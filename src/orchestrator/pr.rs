//! A Ticket's PR as the poll reads it, one GraphQL query per open PR
//! (docs/research/pr-bot-comments.md §6 on research/pr-bot-comments):
//! whether its head is quiet, and its open items.

use regex::Regex;
use serde::Deserialize;

/// The research's §6 query, asked by the PR's URL, as deep as threads.sh
/// reads for its marker rule. It costs 1 point: GitHub counts connections,
/// not their size.
pub(crate) const QUERY: &str = "query($url:URI!){resource(url:$url){...on PullRequest{
  state mergeable reviewDecision isMergeQueueEnabled headRefOid createdAt mergeCommit{oid}
  statusCheckRollup{commit{oid} state contexts(first:100){pageInfo{hasNextPage} nodes{
    ...on CheckRun{name status conclusion startedAt
      checkSuite{app{slug} workflowRun{event workflow{name}}}}
    ...on StatusContext{context state description creator{login}}}}}
  reviewThreads(first:100){pageInfo{hasNextPage} nodes{id isResolved path line
    comments(first:50){pageInfo{hasNextPage} nodes{databaseId author{login __typename} body}}}}
  reviews(last:50){pageInfo{hasPreviousPage} nodes{databaseId url author{login __typename} body}}
  comments(last:100){pageInfo{hasPreviousPage} nodes{databaseId url author{login __typename} body}}}}}";

/// The PR, the reply's `resource`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Pr {
    pub(crate) state: String,
    pub(crate) mergeable: String,
    /// CHANGES_REQUESTED while a human's review asks for changes.
    pub(crate) review_decision: Option<String>,
    /// Its base branch has a merge queue: `gh pr merge` would queue it.
    pub(crate) is_merge_queue_enabled: bool,
    pub(crate) head_ref_oid: String,
    /// When it opened: what bot_wait counts from.
    pub(crate) created_at: Option<chrono::DateTime<chrono::Local>>,
    merge_commit: Option<Commit>,
    status_check_rollup: Option<Rollup>,
    review_threads: Nodes<Thread>,
    reviews: Nodes<Post>,
    comments: Nodes<Post>,
}

/// A connection's nodes, and whether it has more than the poll read.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Nodes<T> {
    nodes: Vec<T>,
    page_info: PageInfo,
}

impl<T> Nodes<T> {
    fn more(&self) -> bool {
        self.page_info.has_next_page || self.page_info.has_previous_page
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Rollup {
    commit: Option<Commit>,
    /// The rollup's first 100 contexts, and whether it has more.
    contexts: Nodes<Context>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    /// Of a `last:` connection: it has older nodes.
    has_previous_page: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Commit {
    oid: String,
}

/// A CheckRun (name, status, conclusion, started_at, check_suite) or a
/// StatusContext (context, state, creator).
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Context {
    name: String,
    status: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    check_suite: serde_json::Value,
    context: String,
    state: String,
    creator: serde_json::Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Thread {
    id: String,
    is_resolved: bool,
    path: String,
    line: Option<u64>,
    comments: Nodes<Post>,
}

/// A thread's comment, a review or a PR comment.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Post {
    database_id: u64,
    url: String,
    author: Option<Author>,
    body: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Author {
    login: String,
    #[serde(rename = "__typename")]
    typename: String,
}

/// One open item of a PR. Serialized for the merge Judgment's state.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct Item {
    /// The same on every poll: what the Ticket's offered set keeps.
    pub(crate) id: String,
    /// thread, outside, comment or check.
    pub(crate) kind: &'static str,
    pub(crate) author: String,
    /// One line, its title.
    pub(crate) summary: String,
    /// The bot's own severity, a human's login, "failed" for a check.
    pub(crate) rating: String,
    /// path:line, the path alone, or "".
    pub(crate) place: String,
}

/// The PR in gh's reply to QUERY.
pub(crate) fn parse(reply: &str) -> Result<Pr, String> {
    let mut reply: serde_json::Value =
        serde_json::from_str(reply).map_err(|err| err.to_string())?;
    let pr = reply
        .pointer_mut("/data/resource")
        .map(serde_json::Value::take)
        .filter(|pr| !pr.is_null())
        .ok_or("no pull request at that url")?;
    serde_json::from_value(pr).map_err(|err| err.to_string())
}

impl Pr {
    /// The commit its merge made; empty until it merges.
    pub(crate) fn merge_commit(&self) -> &str {
        self.merge_commit.as_ref().map_or("", |c| c.oid.as_str())
    }

    /// The rollup's contexts if they are the head's: a rollup of another
    /// commit holds nothing, nor does a bot seen only on an earlier one.
    /// Like gh, only the newest run of a check counts; a queued run, not
    /// started yet, is the newest, but a skipped one never started is the
    /// oldest.
    fn contexts(&self) -> Vec<&Context> {
        let mut newest: Vec<&Context> = Vec::new();
        let Some(rollup) = &self.status_check_rollup else {
            return newest;
        };
        if rollup
            .commit
            .as_ref()
            .is_none_or(|c| c.oid != self.head_ref_oid)
        {
            return newest;
        }
        for c in &rollup.contexts.nodes {
            match newest.iter_mut().find(|n| n.key() == c.key()) {
                Some(n) if c.age() > n.age() => *n = c,
                Some(_) => {}
                None => newest.push(c),
            }
        }
        newest
    }

    /// A check or a bot is at work on the head: a context in gh's pending
    /// bucket.
    pub(crate) fn busy(&self) -> bool {
        self.contexts().iter().any(|c| pending(c.state()))
    }

    /// What every merge of the Orchestrator's waits for: its checks green,
    /// each passed, skipped or neutral (a cancelled one never passed), none
    /// past the 100 the poll reads, and the rollup the head's own (a rollup
    /// of another commit is not its checks), GitHub calling it mergeable,
    /// and no review asking for changes.
    pub(crate) fn ready(&self) -> bool {
        let unread = self.status_check_rollup.as_ref().is_some_and(|r| {
            r.contexts.more() || r.commit.as_ref().is_none_or(|c| c.oid != self.head_ref_oid)
        });
        let passed = |c: &&Context| matches!(c.state(), "SUCCESS" | "SKIPPED" | "NEUTRAL");
        let green = !unread && self.contexts().iter().all(passed);
        let blocked = self.review_decision.as_deref() == Some("CHANGES_REQUESTED");
        green && self.mergeable == "MERGEABLE" && !blocked
    }

    /// Whether the review bot, as review_bots names it, has reviewed the
    /// PR, on any commit: a review of its own, not a PR comment, which may
    /// only say it was skipped or rate limited.
    pub(crate) fn reviewed_by(&self, bot: &str) -> bool {
        let login = match bot {
            "coderabbit" => "coderabbitai",
            "greptile" => "greptile-apps",
            other => other,
        };
        self.reviews.nodes.iter().any(|r| r.by(login))
    }

    /// The bots of `bots`, as review_bots names them, that have not
    /// reviewed the PR.
    pub(crate) fn silent<'a>(&self, bots: &[&'a str]) -> Vec<&'a str> {
        let silent = bots.iter().filter(|bot| !self.reviewed_by(bot));
        silent.copied().collect()
    }

    /// The poll read every thread, thread comment, review and PR comment:
    /// with any past its page, items() may miss an open one.
    pub(crate) fn items_complete(&self) -> bool {
        let threads = &self.review_threads;
        !(threads.more()
            || threads.nodes.iter().any(|t| t.comments.more())
            || self.reviews.more()
            || self.comments.more())
    }

    /// Its open items, most severe first: unresolved threads, the bots'
    /// findings in bodies, human PR comments and review bodies, failing
    /// checks. What address-pr-comments answered is left out by its
    /// threads.sh's rule.
    pub(crate) fn items(&self) -> Vec<Item> {
        let answers: Vec<&str> = (self.comments.nodes.iter())
            .filter(|c| c.marked())
            .map(|c| c.body.as_str())
            .collect();
        let answered = |needle: String| answers.iter().any(|a| a.contains(&needle));
        let mut items = Vec::new();

        // a thread is answered once only bots wrote after its last marker
        for t in &self.review_threads.nodes {
            let comments = &t.comments.nodes;
            let Some(root) = comments.first() else {
                continue;
            };
            let replied = match comments.iter().rposition(Post::marked) {
                Some(m) => comments[m + 1..].iter().any(|c| !c.bot()),
                None => true,
            };
            if t.is_resolved || !replied {
                continue;
            }
            let place = match t.line {
                Some(line) => format!("{}:{line}", t.path),
                None => t.path.clone(),
            };
            items.push(Item {
                id: t.id.clone(),
                kind: "thread",
                author: root.login().to_string(),
                summary: title(&root.body),
                rating: root.rating(root.body.lines().next().unwrap_or("")),
                place,
            });
        }

        // CodeRabbit ends each review-body entry with a cr-comment marker and
        // opens it with a `L-L`: line under a <summary>path (n)</summary>
        // (a nitpick) or a `path:L-L`: line (outside the diff). Only a
        // nitpick or outside-diff entry whose line carries a severity is a
        // finding: not 🔇 LGTM, nor a ♻️ duplicate of a thread
        let heading = Regex::new(r"<summary>([^<\n]+?) \([0-9]+\)</summary>").unwrap();
        let header = Regex::new(r"(?m)^`(?:([^`:\s]+):)?([0-9]+)(?:-[0-9]+)?`:.*").unwrap();
        let quote = Regex::new(r"(?m)^> ?").unwrap();
        for review in self.reviews.nodes.iter().filter(|r| r.by("coderabbitai")) {
            let entries: Vec<&str> = review.body.split("<!-- cr-comment:v1:").collect();
            let (mut section, mut file) = (String::new(), String::new());
            for pair in entries.windows(2) {
                // past the first, an entry opens with the previous one's id
                let entry = match pair[0].split_once(" -->") {
                    Some((id, entry)) if id.bytes().all(|b| b.is_ascii_hexdigit()) => entry,
                    _ => pair[0],
                };
                let entry = quote.replace_all(entry, "");
                for h in heading.captures_iter(&entry) {
                    match &h[1] {
                        s if s.ends_with(" comments") => section = s.to_string(),
                        s => file = s.to_string(),
                    }
                }
                if !section.contains("Nitpick") && !section.contains("Outside diff") {
                    continue;
                }
                let Some(h) = header.captures(&entry) else {
                    continue;
                };
                let Some(rating) = severity(&h[0]) else {
                    continue;
                };
                let summary = title(&entry[h.get(0).unwrap().start()..]);
                if answered(format!("**{summary}**")) {
                    continue;
                }
                let path = h.get(1).map_or(file.as_str(), |p| p.as_str());
                let place = match path {
                    "" => String::new(),
                    path => format!("{path}:{}", &h[2]),
                };
                items.push(Item {
                    id: format!("cr:{}", pair[1].split(" -->").next().unwrap_or("")),
                    kind: "outside",
                    author: review.login().to_string(),
                    summary,
                    rating,
                    place,
                });
            }
        }

        // Greptile: the outside-diff comment's entries, and the summary's
        // Findings but those that link a thread of this reply
        let roots: Vec<u64> = (self.review_threads.nodes.iter())
            .filter_map(|t| Some(t.comments.nodes.first()?.database_id))
            .collect();
        let finding = Regex::new(r"(?m)^[0-9]+\. .*").unwrap();
        let discussion = Regex::new(r"#discussion_r([0-9]+)").unwrap();
        let located = Regex::new(r"`([^`:\s]+:[0-9]+)").unwrap();
        for c in self.comments.nodes.iter().filter(|c| c.by("greptile-apps")) {
            let mut lines: Vec<&str> = Vec::new();
            if c.body.contains("<!-- greptile_outside_diff -->") {
                let entries = c.body.split("\n- <img ").skip(1);
                lines.extend(entries.map(|e| e.lines().next().unwrap_or("")));
            }
            if let Some((_, findings)) = c.body.split_once("<h2>Findings</h2>") {
                let findings = findings.split("<details>").next().unwrap_or("");
                lines.extend(finding.find_iter(findings).map(|m| m.as_str()).filter(|l| {
                    let linked = discussion.captures(l).and_then(|d| d[1].parse().ok());
                    !linked.is_some_and(|id| roots.contains(&id))
                }));
            }
            for line in lines {
                let summary = title(line).replace('\\', "");
                if answered(format!("**{summary}**")) {
                    continue;
                }
                let place = located.captures(line);
                items.push(Item {
                    // ponytail: the title is Greptile's only handle on a
                    // finding, as it is threads.sh's: reworded, it is offered again
                    id: format!("{}:{summary}", c.database_id),
                    kind: "outside",
                    author: c.login().to_string(),
                    rating: c.rating(line),
                    summary,
                    place: place.map_or(String::new(), |p| p[1].to_string()),
                });
            }
        }

        // a human's PR comment or review body, answered once a marker
        // comment links it
        let comments = self.comments.nodes.iter().map(|c| ("comment", c));
        let reviews = self.reviews.nodes.iter().map(|r| ("review", r));
        for (kind, post) in comments.chain(reviews) {
            if post.bot() || post.marked() || post.body.trim().is_empty() {
                continue;
            }
            if answered(format!("({})", post.url)) {
                continue;
            }
            items.push(Item {
                id: format!("{kind}:{}", post.database_id),
                kind: "comment",
                author: post.login().to_string(),
                summary: title(&post.body),
                rating: post.login().to_string(),
                place: String::new(),
            });
        }

        for c in self.contexts().into_iter().filter(|c| failed(c.state())) {
            let (name, workflow, event) = c.key();
            items.push(Item {
                id: format!("check:{}:{name}:{workflow}:{event}", self.head_ref_oid),
                kind: "check",
                author: c.author().to_string(),
                summary: name.to_string(),
                rating: "failed".to_string(),
                place: String::new(),
            });
        }

        items.sort_by_key(|i| rank(&i.rating));
        items
    }
}

impl Post {
    fn login(&self) -> &str {
        self.author.as_ref().map_or("", |a| a.login.as_str())
    }

    /// A bot wrote it: GraphQL's __typename says so, or REST's [bot].
    fn bot(&self) -> bool {
        (self.author.as_ref()).is_some_and(|a| a.typename == "Bot" || a.login.ends_with("[bot]"))
    }

    /// `bot` wrote it, its login with [bot] or without.
    fn by(&self, bot: &str) -> bool {
        self.login().trim_end_matches("[bot]") == bot
    }

    /// address-pr-comments wrote it, as an answer.
    fn marked(&self) -> bool {
        ["<!-- address-pr-comments -->", "<!-- address-greptile -->"]
            .iter()
            .any(|m| self.body.contains(m))
    }

    /// A bot's own severity in `line`, else its author's login.
    fn rating(&self, line: &str) -> String {
        match self.bot() {
            true => severity(line).unwrap_or_else(|| self.login().to_string()),
            false => self.login().to_string(),
        }
    }
}

/// CodeRabbit's `_🟠 Major_` or Greptile's `alt="P1"` badge in `line`.
fn severity(line: &str) -> Option<String> {
    let re = Regex::new(r#"_\S+ (Critical|Major|Minor|Trivial|Info)_|alt="(P[0-2])""#).unwrap();
    let caps = re.captures(line)?;
    Some(caps.get(1).or(caps.get(2))?.as_str().to_string())
}

/// Its title as threads.sh takes it: the first bold run once HTML comments
/// and CodeRabbit's static-analysis transcripts are gone, else its first
/// line, cut to 80 chars.
fn title(body: &str) -> String {
    let noise = Regex::new(r"(?s)<!--.*?-->|<details>\s*<summary>🔎.*?</details>").unwrap();
    let body = noise.replace_all(body, "");
    if let Some(bold) = Regex::new(r"\*\*([^*\n]+)\*\*").unwrap().captures(&body) {
        return bold[1].to_string();
    }
    let line = body.lines().map(str::trim).find(|l| !l.is_empty());
    line.unwrap_or("").chars().take(80).collect()
}

/// Most severe first: failing checks with Critical and P0, then Major, P1
/// and humans' logins, then Minor and P2, then Trivial, then Info.
fn rank(rating: &str) -> u8 {
    match rating {
        "failed" | "Critical" | "P0" => 0,
        "Minor" | "P2" => 2,
        "Trivial" => 3,
        "Info" => 4,
        _ => 1,
    }
}

impl Context {
    /// What gh tells runs of one check apart by: name, workflow and event.
    fn key(&self) -> (&str, &str, &str) {
        let run = &self.check_suite["workflowRun"];
        let workflow = run["workflow"]["name"].as_str().unwrap_or("");
        let name = if self.name.is_empty() {
            &self.context
        } else {
            &self.name
        };
        (name, workflow, run["event"].as_str().unwrap_or(""))
    }

    /// Orders the runs of a check, the newest greatest: a run not started
    /// and not completed is queued, so newer than any started run.
    fn age(&self) -> (bool, Option<&str>) {
        let queued = self.started_at.is_none() && self.status != "COMPLETED";
        (queued, self.started_at.as_deref())
    }

    /// The app that ran a check, or who set a status.
    fn author(&self) -> &str {
        let app = self.check_suite["app"]["slug"].as_str();
        app.or(self.creator["login"].as_str()).unwrap_or("")
    }

    /// Its state as gh reads it: a status's state, a check run's conclusion
    /// once completed, its status before.
    fn state(&self) -> &str {
        if !self.state.is_empty() {
            &self.state
        } else if self.status == "COMPLETED" {
            self.conclusion.as_deref().unwrap_or("")
        } else {
            &self.status
        }
    }
}

/// gh's fail bucket (pkg/cmd/pr/checks/aggregate.go).
fn failed(state: &str) -> bool {
    matches!(state, "ERROR" | "FAILURE" | "TIMED_OUT" | "ACTION_REQUIRED")
}

/// gh's pending bucket: everything but pass, skipping, fail and cancel.
fn pending(state: &str) -> bool {
    !failed(state) && !matches!(state, "SUCCESS" | "SKIPPED" | "NEUTRAL" | "CANCELLED")
}
