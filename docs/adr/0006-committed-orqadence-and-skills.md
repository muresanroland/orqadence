# .orqadence and its skills are committed; what a run leaves is not

A Target repo's `.orqadence/` is committed and holds its Orqadence settings: config.json (the Stage rows, Ticket labels, switches and caps), the Skill manifest, init's record of the skill text it wrote, and the skill files themselves in `.orqadence/skills`, reached through committed relative folder links from `.agents/skills` and `.claude/skills`. What depends on the machine, the person or the run lives in `.orqadence-local/`, which ignores itself: the state file, lock, log, Run directories, worktrees, Brainstorm state, graphify's last X.Y handled, and the per-person settings (the TypeSafe key and the On call page). We did this so each repo keeps its own workflow, and so that no skill can change how Stages behave without a change someone reviews: a third-party update that adds one line ("don't go on until the user answers") shows up as a diff in a pull request, not as a new commit hash in a manifest.

## Considered Options

- **Uncommitted skills fetched at their pinned commit**, per checkout or at user level. Rejected: the text that runs is never reviewed, and user-level skills are shared by every repo on the machine, so updating one for repo A changes repo B.
- **Stages read Delegate skills by path**, so a personal skill of the same name can't shadow the committed one (Claude Code runs a personal skill over a project one; codex lists both). Rejected: a skill read as a file loses what invoking it does in claude (`!` lines, `${CLAUDE_*}` placeholders, `context: fork`, hooks, allowed-tools). Delegates are still loaded by name, and a personal skill with the same name is put to the user at Ticket start instead.
- **A local override for any setting**, so a teammate without codex could swap a row. Rejected for now: only the secrets and On call are per person.
- **Moving an existing checkout's run into `.orqadence-local`.** Rejected: a saved Stage's conversation holds its old absolute paths, and codex stops at a directory prompt when resumed elsewhere, so a move needed resume code that exists for one upgrade. Init deletes the old run files after asking, keeping the worktrees' branches, as the `.harness` rename did.

## Consequences

- A Ticket runs the skills committed on its base branch. A skill added or updated from /config takes effect once it is merged; rows and labels still apply at once from the checkout's config.json. A pick missing on the base is a Question when the Ticket starts.
- /config edits leave the checkout with uncommitted changes; the user commits them like any other file.
- `orqa` opens only once `.orqadence-local/` exists. In a fresh clone, init skips the team's questions and does only the per-machine and per-person steps.
- The Skill location question is gone: skills always live in `.orqadence/skills`.
