---
name: orqa-stage-address-pr-comments
description: Orqadence Address PR comments Stage. Acts on a Ticket's pull request comments and failing checks, pushes to the same PR, and keeps the PR body's screenshots and label sections current. Run by the Orqadence Orchestrator on the user's command, not by hand.
---

# Address PR comments Stage

You are in a fresh session inside the kept worktree of a Ticket whose pull request is open. A reviewer, human or bot, left PR comments on it, or a check failed. Ask only what the Ticket, the PR comments, the repo's docs and the Inputs leave open; otherwise decide, and note the answer you took from them. Inputs are under **Inputs** at the end.

## Do

1. Read the **Ticket file**: it reminds you what the change is for; its `## Epic context` is the parent Epic's description: context, not scope. Load every skill under **Label skills** by name in this session, and follow **Label guidance**.
   Use the {{pr-comments}} skill for steps 2 to 5 in place of them: give it the PR, the **Approved** and **Won't fix** lists when Inputs carry them, the **Other open Tickets**, and each failing check in **PR metadata (gh JSON)**'s `statusCheckRollup`. Keep its report for the result file, then go on at step 6.
2. Read the feedback. **PR metadata (gh JSON)** holds the PR's reviews, conversation comments and checks (`statusCheckRollup`). Inline review comments are not in it; fetch them with `gh api repos/{owner}/{repo}/pulls/<number>/comments --paginate` (`{owner}` and `{repo}` are filled in by gh). Read each failing check in `statusCheckRollup` with `gh run view <run id> --log-failed`; the run id is in its `detailsUrl` (`.../actions/runs/<run id>/...`).
3. Apply each requested change, and fix what each failing check reports. When Inputs carry the **Approved** and **Won't fix** lists, change only the Approved items, answer each Won't fix item in the result file, and leave comments on neither list alone. A comment that asks a question, or that you judge wrong, gets no code change: answer it in the result file instead and let the human decide.
   Among the comments you would change, one that asks for work one of the **Other open Tickets** does, as a bot reading only the diff asks to delete what a later Ticket builds on, gets no change either. Answer it on the PR, not in the result file: `Covered by <id>: <title>. <!-- address-pr-comments -->`. Reply on its review thread with `gh api repos/{owner}/{repo}/pulls/<number>/comments/<comment id>/replies -f body=<text>`, then resolve the thread. Its id is the `id` of the `reviewThreads` node whose first comment has that `databaseId` (`gh api graphql --paginate`, the pull request's `reviewThreads(first:100,after:$endCursor){pageInfo{hasNextPage endCursor} nodes{id comments(first:1){nodes{databaseId}}}}` in a query that declares `$endCursor:String`, so gh reads every page); resolve it with `gh api graphql -F id=<thread id> -f query='mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{isResolved}}}'`. A comment with no thread gets a line in one PR comment (`gh pr comment <PR> --body-file -`) that ends with the marker: `- **<its title>**: Covered by <id>: <title>.` for a bot's, `- @<author>, on [your comment](<url>): Covered by <id>: <title>.` for a human's. Never delete or revert what an open Ticket there depends on.
4. Run the repo's tests until they pass. Commit.
5. Push to the same PR with `git push`. Never open a second PR, never merge, never close the Ticket.
6. Check `gh pr view <PR> --json mergeable` reports `MERGEABLE` (GitHub may need a few seconds after the push).
7. **Screenshots**: when **Label** names `orqa:fe` and this run's changes touched a screen, capture the changed screens again as the last Fix did, after the change only. Empty `<Run directory>/pr/`, start the app in the worktree the way the repo's docs say, capture every screen this run changed into `<Run directory>/pr/` (the repo's Playwright: `npx playwright screenshot <url> <file>`; else headless Chrome under a timeout), then stop the app. Light and dark only if the change touched theming. If `gh --version` is older than 2.99 or the remote is a GitHub Enterprise Server (`gh repo view --json url` shows a host other than github.com or `*.ghe.com`), `--attach` cannot upload there: skip it. Otherwise add every file with one `gh pr edit --attach` call on the PR, not retried if it fails. Images are never committed: they stay in the Run directory, outside the worktree.
   - The app cannot start: capture and attach nothing; step 8 says why.
8. **The PR body**: start from the PR's live body (`gh pr view <PR> --json body -q .body`), where gh appended the uploaded files' URLs, and change only these sections:
   - `## Screenshots`, when step 7 had screens to capture: the URLs gh appended, moved here in place of the earlier captures of the same screens. When they were not attached, the section says they were not attached and why (gh older than 2.99, a GitHub Enterprise Server remote, the error gh printed, or why the app could not start). A body without the section gets it before `## Orqadence run`.
   - Each label section this run's changes made stale: `## Contract`, `## Schema`, `## Structure`, `## Threat note`, `## Infra`. Rewrite it to match the branch now, as that section's comment in the template at **PR template** says, else as the section reads now. **PR template**, when Inputs carry it, is the Ticket's PR template, resolved as the last Fix's was, in the Target repo's checkout rather than your worktree.

   The rest of the body stays as it is: the other sections and `## Orqadence run`. Write it back with one `gh pr edit <PR> --body-file <file>`. With none of these to change, leave the body alone.

## Result file

Write the **Result file** from Inputs last. The Orchestrator parses only the first line:

```
STATUS: done

<per PR comment: what you changed, your answer, or the open Ticket that covers it; per failing check: what you fixed; the PR's mergeable state; the screens captured and attached, or why not; the label sections rewritten>
```

Write `STATUS: failed` with the reason if the tests cannot be made to pass or the push is rejected.

To ask, write the **Result file** with `STATUS: question` as its first line, then the question, then its options as the last lines, one per line starting with `- `, and wait: the answer comes into this pane as a prompt. Carry on, and overwrite the Result file with done or failed when you finish.
