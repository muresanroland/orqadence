---
name: orqa-brainstorm-epic
description: Orqadence Brainstorm end. Works a Map's build-Epic Waypoint - writes the Epics that build what the Map decided, with Tickets /start-epic can run, opens the docs pull request, and writes a result file. Run by the Orqadence Orchestrator, not by hand.
---

# Brainstorm Epics

You work a Map's last Waypoint with the user, in a fresh session inside the Brainstorm's worktree, on its own branch. Every other Waypoint of the Map is closed: the way to its Destination is clear. You write the Epics that build what the Map decided. The Orchestrator only reads your result file. Your inputs are under **Inputs** at the end.

**MAP** is the Map's id. **WAYPOINT** is its build-Epic Waypoint, the one labelled `brainstorm:epic`. **TICKET LABELS** lists each Ticket label the Target repo has configured: its name, its kind (`area` or `modifier`) and its guidance. **RELEASES** is `on` when the Target repo has releases turned on, else `off`. **RESULT FILE** is where you write your result, last.

## Rules

- Load the orqa-brainstorm-grilling and orqa-brainstorm-domain-modeling skills by name in this session. Every question you put to the user goes through the grilling skill: one question at a time, through your App's question tool.
- Use the {{prose}} skill for your commits and result file: load it by name in this session.
- Read the repo's `CLAUDE.md` (or `AGENTS.md`), `CONTEXT.md` and `docs/adr/` if present. Use the repo's vocabulary.
- **Refer by title.** Wherever the user reads an issue (your questions, the Map, a description), name it by its title, never by a bare id. The id may ride beside the title, never in its place.
- The bd commands are written out below; use them as they are. Never `bd edit`.
- Other sessions may share this worktree, so commit only the paths you changed:

  ```
  git add -- <paths>
  git commit -m '<message>' -- <paths>
  ```

## 1. Claim and read

1. Claim the WAYPOINT before anything else: `bd update <WAYPOINT> --claim`.
2. Read the Map: `bd show <MAP>`. Its Destination, Notes and Decisions so far are the low-resolution view.
3. List its closed Waypoints with `bd list --parent <MAP> --status=closed --json`. Read each one's resolution with `bd show <id> --json --include-comments`: its comments and close reason hold the decision. Zoom into the ones the Epics depend on. Prototype branches and research notes they link can be read with `git show`.
4. Add to Decisions so far any closed Waypoint missing from it, from its close reason: one line each, its title and the gist of its answer. Rewrite the Map's description with `bd update <MAP> --body-file=-` and the whole new body on stdin, changing only those lines.

## 2. Write the Epics

Write the build Epics in build order, one per area: an Epic builds one area's part, and the Epics that come later assume the earlier ones are merged. Grill the user only on what the closed Waypoints leave open.

1. Create each Epic, with no `--parent`: it is not a child of the Map. Its description says what it builds, the decisions it rests on (by Waypoint title), the contract between its Tickets, and which earlier Epics it assumes are merged.

   ```
   bd create --type=epic --priority=2 --title="<title>" --body-file=- <<'EOF'
   <description>
   EOF
   ```

2. Create its Tickets as its children, each sized for one pull request, so /start-epic can run them. Each Ticket has a description, acceptance criteria, and at most one Area label from **TICKET LABELS** (an `area` kind). Add a `modifier` label only when its guidance says it fits.

   ```
   bd create --parent <epic> --no-inherit-labels --type=task --priority=2 --title="<title>" --acceptance="<criteria>" --labels=<area label> --body-file=- <<'EOF'
   <description>
   EOF
   ```

3. Wire the blocking edges between Tickets in a second pass, once every Ticket has its id: `bd dep add <ticket> <the ticket it waits for>`.
4. When no configured label fits a Ticket's type of work, propose one in a `LABEL:` line of your result, and leave the Ticket without an Area label. Never write Orqadence's config and never add the proposed label in bd: the user decides.

## 3. Releases

With **RELEASES** `on`, ask the user which Epics carry `orqa:release`, through the grilling skill, every Epic checked by default. Add the label to each Epic chosen, never to a Ticket: `bd label add <epic> orqa:release`. With **RELEASES** `off`, never add it.

## 4. Commit and open the docs PR

1. Commit the document changes you made (`CONTEXT.md`, `docs/adr/`, notes), if any.
2. If the branch has commits the default branch lacks (`git log --oneline origin/HEAD..HEAD`, falling back to `main`), run the orqa-create-pr skill to open the one docs pull request for this Brainstorm. Its title names the Map. Its body names the Epics, in build order, by title. With no commits, open no PR.

## 5. Resolve the WAYPOINT

1. Post the resolution: `bd comments add <WAYPOINT> "<the Epics in build order, by title and id, and the docs PR>"`.
2. Add its line to the Map's Decisions so far, as in step 1.
3. Close it with a reason naming the Epics: `bd close <WAYPOINT> --reason="Epics: <title> (<id>), <title> (<id>)"`.

Leave the Map open: Orqadence closes it once it has read your result.

## Result file

Write the **RESULT FILE** last, after the WAYPOINT is closed. The Orchestrator parses its first lines; nothing counts until it exists.

```
STATUS: done
EPICS: <id> <id>...
PR: <url>
LABEL: <name> | <guidance> | skills: <a>, <b> | tickets: <ids>

<what each Epic builds, by title, and which carry orqa:release>
```

`EPICS:` lists the Epics in build order, the first to run first. Leave the `PR:` line out when you opened no PR. Write one `LABEL:` line per proposed label, or none. Its name is bare (`docs`, not `orqa:docs`). Its skills are the skills you suggest its Stages load, or empty. Its tickets are the ids that need it.

If you cannot finish (bd fails, the user stops the session), write `STATUS: failed` followed by the reason.
