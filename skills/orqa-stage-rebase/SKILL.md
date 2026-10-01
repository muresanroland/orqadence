---
name: orqa-stage-rebase
description: Orqadence Rebase Stage. Rebases a Ticket's pull request that conflicts with main onto the default branch, keeping both sides' intent, then pushes to the same PR. Run by the Orqadence Orchestrator on the user's command, not by hand.
---

# Rebase Stage

You are in a fresh session inside the kept worktree of a Ticket whose pull request is open and conflicts with the default branch: work merged since changed what it touches. Ask only what the Ticket, both sides' commits, the repo's docs and the Inputs leave open; otherwise decide, and note the answer you took from them. Inputs are under **Inputs** at the end.

## Do

1. Read the **Ticket file**: it reminds you what the change is for; its `## Epic context` is the parent Epic's description: context, not scope. Load every skill under **Label skills** by name in this session, and follow **Label guidance**.
2. `git fetch origin`, then rebase the branch onto the **Default branch**, keeping both sides' intent: never resolve a conflict by dropping the other change.
   Use the {{merge-conflicts}} skill for the rebase.
   When a conflict cannot keep both intents, stop mid-rebase and ask (see the end): the hunk, what each side meant, and the options ours, theirs, or a merge you describe. The answer resumes the rebase. Never abort it.
3. Run the repo's tests until they pass. Commit what they needed.
4. Push to the same PR with `git push --force-with-lease`. Never open a second PR, never merge, never close the Ticket.
5. Check `gh pr view <PR> --json mergeable` reports `MERGEABLE` (GitHub may need a few seconds after the push).

## Result file

Write the **Result file** from Inputs last. The Orchestrator parses only the first line:

```
STATUS: done

<each conflict and how you kept both sides; what the tests needed; the PR's mergeable state>
```

Write `STATUS: failed` with the reason if the tests cannot be made to pass or the push is rejected. A conflict that cannot keep both intents is a question (step 2), never a failure.

To ask, write the **Result file** with `STATUS: question` as its first line, then the question, then its options as the last lines, one per line starting with `- `, and wait: the answer comes into this pane as a prompt. Carry on, and overwrite the Result file with done or failed when you finish.
