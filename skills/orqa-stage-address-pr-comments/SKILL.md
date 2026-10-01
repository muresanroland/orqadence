---
name: orqa-stage-address-pr-comments
description: Orqadence Address PR comments Stage. Acts on a Ticket's pull request comments and failing checks, then pushes to the same PR. Run by the Orqadence Orchestrator on the user's command, not by hand.
---

# Address PR comments Stage

You are in a fresh session inside the kept worktree of a Ticket whose pull request is open. A reviewer, human or bot, left PR comments on it, or a check failed. Ask only what the Ticket, the PR comments, the repo's docs and the Inputs leave open; otherwise decide, and note the answer you took from them. Inputs are under **Inputs** at the end.

## Do

1. Read the feedback. **PR metadata (gh JSON)** holds the PR's reviews, conversation comments and checks (`statusCheckRollup`). Inline review comments are not in it; fetch them with `gh api repos/{owner}/{repo}/pulls/<number>/comments --paginate` (`{owner}` and `{repo}` are filled in by gh). Read each failing check in `statusCheckRollup` with `gh run view <run id> --log-failed`; the run id is in its `detailsUrl` (`.../actions/runs/<run id>/...`). The **Ticket file** reminds you what the change is for; its `## Epic context` is the parent Epic's description: context, not scope. Load every skill under **Label skills** by name in this session, and follow **Label guidance**.
2. Apply each requested change, and fix what each failing check reports. A comment that asks a question, or that you judge wrong, gets no code change: answer it in the result file instead and let the human decide.
3. Run the repo's tests until they pass. Commit.
4. Push to the same PR with `git push`. Never open a second PR, never merge, never close the Ticket.
5. Check `gh pr view <PR> --json mergeable` reports `MERGEABLE` (GitHub may need a few seconds after the push).

## Result file

Write the **Result file** from Inputs last. The Orchestrator parses only the first line:

```
STATUS: done

<per PR comment: what you changed, or your answer; per failing check: what you fixed; the PR's mergeable state>
```

Write `STATUS: failed` with the reason if the tests cannot be made to pass or the push is rejected.

To ask, write the **Result file** with `STATUS: question` as its first line, then the question, then its options as the last lines, one per line starting with `- `, and wait: the answer comes into this pane as a prompt. Carry on, and overwrite the Result file with done or failed when you finish.
