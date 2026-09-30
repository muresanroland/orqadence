# Every skill Orqadence installs is named orqa-; personal skills are opt-in

ADR 0006 kept loading Delegate skills by name and put a personal skill of the same name to the user when a Ticket starts. In practice the user's own `~/.claude/skills` held the very skills Orqadence installs as job defaults (`tdd`, `code-review`, `caveman`...), so every Ticket asked about each of them, though whichever ran was the same text. Every skill Orqadence installs, the Shipped skills and the Delegate skills it fetches, is now named `orqa-<name>`, in its folder and in its SKILL.md's `name`, since claude and codex name a skill by that line. Nothing Orqadence installs can share a name with a skill of the user's own. A job picks from Orqadence's skills, the repo's own committed skills and the Apps' built-in ones; the user's personal skills, at home and from Claude Code plugins, join them only once that person turns them on, a per-person switch in `.orqadence-local/config.json`.

## Considered Options

- **Keep the names; skip the Question when the personal copy is byte-identical.** Kept as well, for the repo's own picks, but a personal copy one commit behind still asks, and a Stage would still run the personal one.
- **Prefix only the fetched Delegate skills.** Rejected: `create-pr` collided too, which is why init asked whether to keep the repo's own; with `orqa-create-pr` that question is gone.
- **The switch committed, for the team.** Rejected: each person's `~` holds different skills, so a team switch would load different text per machine.

## Consequences

- `orqa init` renames an install from before: folders, links, the Skill manifest's entries and picks, and init's record; `orqadence-create-pr` becomes `orqa-create-pr`. The rename is a change to commit and merge; until it is on a Ticket's base, the Stage skill is found under its old name, and a pick not on the base is the usual Question.
- The Fix Stage always runs `orqa-create-pr`; a repo that wants its own conventions edits its committed copy.
- A skill added from a source in /config is installed as `orqa-<name>` too.
- A personal skill picked while the switch is off is not installed: its line is left out, as for any pick missing.
