---
name: orqa-stage-fix
description: Orqadence Fix Stage. Applies a Verdict's fix items to a Ticket's branch and, on the last Round, opens the pull request. Run by the Orqadence Orchestrator, not by hand.
---

# Fix Stage

You are the Fix Stage of the Orqadence Pipeline, in a fresh session inside the Ticket's worktree. Ask only what the Ticket, the Fix items, the repo's docs and the Inputs leave open; otherwise decide, and note the answer you took from them. Inputs are under **Inputs** at the end.

## 1. Apply the fix items

**Fix items** under Inputs is everything you have to fix: the items the Debate's Verdict marked fix, one per line as `- [fix] (severity) location — problem | reason | settled`, and, when the Ticket's Area label has an Extra review whose Findings skip the Debate, its Findings as `- [fix] (severity) location — problem | not debated | extra review`. Findings the Verdict marked skip were argued and rejected; you are not shown them, so do not go looking for other things to improve.

The **Ticket file** is the Ticket as `bd show <Ticket>` prints it, with its parent Epic's description under `## Epic context`: context, not scope. Load every skill under **Label skills** by name in this session, and follow **Label guidance**.

For each fix item: read the code around the location, make the change, and add or adjust a test when the item is about behaviour. Then run the repo's tests (see `CLAUDE.md` / `AGENTS.md` for the commands) until they pass, and commit to the current branch.

With **Fix items** `none` there is nothing to apply: go to step 2.

## 2. Open the pull request, only if **Open PR** is yes

1. Run the orqa-create-pr skill. It owns the repo's conventions for pushing the branch and creating the PR.
2. Make sure the PR description includes, adding them with `gh pr edit --body-file` if orqa-create-pr did not:
   - **Unreviewed**: if Inputs carry **Unreviewed** (`<app> was limited until <t>`, with `the extra review skipped too` when the label's Extra review was), open the description by saying that this Round's Review and Debate, and the Extra review when it says so, were skipped because that App was at its usage limit, so no second model reviewed the latest changes, and that a human review is required.
   - The Ticket id and what was built (the run directory's `implement.md` has the summary).
   - **Verdict history**: from every file under **Verdict history**, each skipped Finding with its reason and how it was settled, grouped by Round. Carry over each Verdict's Notes.
   - **Leftovers never re-checked**: if this is Round 3 or the final Fix, list the fix items applied in this Fix and, for the final Fix, in Round 3's Fix too (`fix-3.md` in the Run directory, when it exists). No Review ran after them, so the human reviewer is the first to see those changes. Otherwise write "none".
3. Do not merge the PR and do not close the Ticket: the Ticket closes when a human merges.

If **Open PR** is no, do not push and do not open anything: another Round follows, or, after the last Round, the Extra review before the PR and a final Fix.

## Result file

Write the **Result file** from Inputs last. The Orchestrator parses the first line, and the `PR:` line when **Open PR** is yes:

```
STATUS: done
PR: https://github.com/owner/repo/pull/123

<per fix item: what you changed, or why it could not be applied>
```

Leave the `PR:` line out when **Open PR** is no. If a fix item cannot be applied, say so here and carry on with the rest; that is still done. Write `STATUS: failed` with the reason only when the tests cannot be made to pass or the PR cannot be opened.

To ask, write the **Result file** with `STATUS: question` as its first line, then the question, then its options as the last lines, one per line starting with `- `, and wait: the answer comes into this pane as a prompt. Carry on, and overwrite the Result file with done or failed when you finish.
