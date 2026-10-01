#!/usr/bin/env bash
# Prints a JSON array of the PR's open review comments, one object per comment:
#   kind, thread_id, comment_id, author, url, path, line, outdated, title, body, prompt, replies[{author, body}]
# body is the comment without its HTML comments or CodeRabbit's static-analysis transcripts.
# prompt is the bot's own prompt for AI agents found in body: Greptile's "Prompt To Fix With AI"
#   block or CodeRabbit's "🤖 Prompt for AI Agents" block. null when body has none, as with
#   Greptile's outside findings and human comments.
# kind "thread": an unresolved inline review thread, human or bot; body is its root comment. A
#   thread is answered, and left out, once a comment in it carries <!-- address-pr-comments -->
#   (or the older <!-- address-greptile -->) and only bots have written after it; a human reply
#   brings it back.
# kind "outside": one bot finding with no thread: an entry of Greptile's "Comments Outside Diff"
#   PR comment, or an outside-diff or nitpick finding with a severity header in a CodeRabbit
#   review body. thread_id is null, comment_id is that PR comment's or review's id, path/line
#   come from the finding (null if it names none), outdated is always true since GitHub does not
#   track those lines, and replies is empty. A finding is answered, and left out, once a PR
#   comment carries the marker and its title in bold.
# kind "comment": a PR comment or a review's summary written by a human, without the marker.
#   thread_id, path and line are null, comment_id is the comment's or review's id, url is its
#   link, outdated is false and replies is empty. It is answered, and left
#   out, once a PR comment carries the marker and links that url, `(<url>)`. Many ask for no
#   change at all.
# Threads come first, then outside findings, then comments; url is null for the first two.
# Run inside the repo checkout. Usage: threads.sh <pr-number>
set -euo pipefail
# gh's --jq is gojq: Go regex syntax.
FILTER='
def marked: test("<!-- address-(pr-comments|greptile) -->");
# A bot`s login is plain in GraphQL, with [bot] in REST; a deleted account`s author is null.
def bot($login): .author.login // "" | sub("\\[bot\\]$"; "") == $login;
def title: (capture("\\*\\*(?<t>[^*\\n]+)\\*\\*").t
  // (split("\n") | map(select(test("\\S"))) | first // "" | .[:80]));
# HTML comments and CodeRabbit`s "Supported by static analysis" transcripts are noise to a fixer.
def clean: gsub("<!--[\\s\\S]*?-->"; "") | gsub("<details>\\s*<summary>🔎[\\s\\S]*?</details>\\s*"; "") | sub("\\s+$"; "");
# CodeRabbit ends each review-body finding with <!-- cr-comment:v1:<id> -->, and opens it with a
# `path:L-L` line (outside diff) or a `L-L`: line under an earlier <summary>path (n)</summary>
# shared by that file`s nitpicks, so the last such path carries forward. Only an entry with a
# severity header (_🟠 Major_ and the like) is a finding, not a 🔇 "Additional comments" LGTM.
def coderabbit:
  split("<!-- cr-comment:v1:") as $c
  | reduce range(0; ($c | length) - 1) as $i ({file: null, out: []};
      ($c[$i] | sub("^[0-9a-f]+ -->"; "") | "\n" + . | gsub("\\n> ?"; "\n")) as $t
      | .file = (([$t | scan("<summary>([^<\\n]+?) \\([0-9]+\\)</summary>")] | last | .[0]?) // .file)
      | (($t | capture("\\n(?<f>`(?:(?<p>[^`:\\s]+):)?(?<l>[0-9]+)(?:-[0-9]+)?`[\\s\\S]*)"))
         // {f: $t, p: null, l: null}) as $f
      | .out += [{path: ($f.p // .file), line: ($f.l | if . then tonumber else null end),
                  text: ($f.f | clean)}])
  | .out[] | select(.text | test("_\\S+ (Critical|Major|Minor|Trivial|Info)_"));
# A bot`s prompt for AI agents, or null: Greptile fences it in `````markdown, CodeRabbit in ```
# under its "🤖 Prompt for AI Agents" summary.
def ai_prompt: (capture("`````markdown\\n(?<p>[\\s\\S]*?)\\n`````")
  // capture("<summary>🤖 Prompt for AI Agents</summary>\\s*```[^\\n]*\\n(?<p>[\\s\\S]*?)\\n```") // {p: null}).p;
.data.repository.pullRequest as $pr
| [$pr.comments.nodes[].body | select(marked)] as $answered
| [$pr.reviewThreads.nodes[]
   | select(.isResolved | not)
   | .comments.nodes as $c
   | ([range($c | length) | select($c[.].body | marked)] | last) as $m
   | select($m == null or any($c[$m + 1:][]; .author.__typename != "Bot"))
   | {kind: "thread", thread_id: .id, comment_id: $c[0].databaseId, author: $c[0].author.login,
      url: null, path, line, outdated: .isOutdated, title: ($c[0].body | title),
      body: ($c[0].body | clean), prompt: ($c[0].body | ai_prompt),
      replies: [$c[1:][] | {author: .author.login, body}]}]
+ [$pr.comments.nodes[]
   | select(bot("greptile-apps") and (.body | contains("<!-- greptile_outside_diff -->")))
   | .databaseId as $id
   | .body | split("\n- <img ")[1:][]
   | (capture("\\*\\*(?<t>[^*]+)\\*\\*").t | gsub("\\\\"; "")) as $title
   | select(any($answered[]; contains("**" + $title + "**")) | not)
   | ((capture("`(?<p>[^`:\\s]+):(?<l>[0-9]+)") | {path: .p, line: (.l | tonumber)}) // {path: null, line: null}) as $loc
   | {kind: "outside", thread_id: null, comment_id: $id, author: "greptile-apps", url: null, path: $loc.path,
      line: $loc.line, outdated: true, title: $title,
      body: ("**" + $title + "**\n\n" + (split("\n")[1:] | join("\n") | gsub("(?m)^  "; "") | ltrimstr("\n"))), prompt: null,
      replies: []}]
+ [$pr.reviews.nodes[]
   | select(bot("coderabbitai") and (.body | contains("<!-- cr-comment:v1:")))
   | .databaseId as $id
   | .body | coderabbit
   | (.text | title) as $title
   | select(any($answered[]; contains("**" + $title + "**")) | not)
   | {kind: "outside", thread_id: null, comment_id: $id, author: "coderabbitai", url: null, path, line,
      outdated: true, title: $title, body: .text, prompt: (.text | ai_prompt), replies: []}]
+ [($pr.comments.nodes[] | select(.body | marked | not)), ($pr.reviews.nodes[] | select(.body | test("\\S")))
   | select(.author.__typename != "Bot")
   | .url as $url
   | select(any($answered[]; contains("(" + $url + ")")) | not)
   | {kind: "comment", thread_id: null, comment_id: .databaseId, author: .author.login, url: $url,
      path: null, line: null, outdated: false, title: (.body | title), body: (.body | clean), prompt: (.body | ai_prompt), replies: []}]'
# ponytail: first 100 threads, 50 comments per thread, the last 100 PR comments and 50 reviews; paginate if a PR ever exceeds that.
gh api graphql -F owner='{owner}' -F repo='{repo}' -F pr="$1" -f query='
query($owner:String!,$repo:String!,$pr:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$pr){
  reviewThreads(first:100){nodes{id isResolved isOutdated path line
    comments(first:50){nodes{databaseId author{login __typename} body}}}}
  reviews(last:50){nodes{databaseId url author{login __typename} body}}
  comments(last:100){nodes{databaseId url author{login __typename} body}}}}}' --jq "$FILTER"
