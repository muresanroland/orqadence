---
name: orqa-brainstorm-chart
description: Orqadence Brainstorm charting. Turns the user's Idea into Tickets in bd, or into a Map of Waypoints when the work is too big to see the way through, and writes a result file. Run by the Orqadence Orchestrator, not by hand.
---

# Brainstorm charting

You chart one Brainstorm with the user, in a fresh session inside the Brainstorm's worktree, on its own branch. The user's idea arrived loose and wrapped in fog: the way from here to the **Destination** isn't visible yet. You find that way. When the way is already clear, the Idea becomes Tickets. When it isn't, the Idea becomes a **Map**: a bd epic whose children are **Waypoints**, one question each, worked in later sessions until the last one writes the Epics. The Orchestrator only reads your result file. Your inputs are under **Inputs** at the end.

**IDEA** is the Idea's id; `bd show <IDEA>` prints the user's text. **TICKET LABELS** lists each Ticket label the Target repo has configured: its name, its kind (`area` or `modifier`) and its guidance. **RESULT FILE** is where you write your result, last.

## Rules

- Load the orqa-brainstorm-grilling and orqa-brainstorm-domain-modeling skills by name in this session. Every question you put to the user goes through the grilling skill: one question at a time, through your App's question tool.
- Use the {{prose}} skill for your commits and result file: load it by name in this session.
- Read the repo's `CLAUDE.md` (or `AGENTS.md`), `CONTEXT.md` and `docs/adr/` if present. Use the repo's vocabulary.
- **Plan, don't do.** You produce decisions and issues, not the build. The pull to just do the work is the sign you have reached the edge of the plan.
- **Refer by title.** Wherever the user reads an issue (your questions, the Map, a description), name it by its title, never by a bare id. The id may ride beside the title, never in its place.
- **Never fire research.** Never start a session or background agent to resolve a Waypoint, and never resolve one yourself: charting resolves nothing. Your App's built-in subagents may look up facts for your own questions.
- The bd commands are written out below; use them as they are. Never `bd edit`.
- Other sessions may share this worktree: commit only the paths you changed, as orqa-brainstorm-domain-modeling's "Where you write, and committing" says.

## 1. Name the Destination

With the grilling and domain-modeling skills, pin down what this effort is finding its way to: the spec, the decision, or the change. It is one or two lines. The Destination fixes the scope, so settle it first.

## 2. Map the frontier

Grill again, **breadth-first** this time. Fan out across the whole space rather than deep on any one thread. Surface the open decisions and the first steps you can take now.

Then decide between Tickets and a Map with the **fog-or-ticket test**: can you state each open question precisely now? You do not need to be able to answer it now.

- **No fog.** The way to the Destination is already clear: nothing is left to decide that a Ticket's session cannot decide alone. Go to step 3.
- **Fog.** Decisions are still open, or you can tell they are coming but cannot phrase them yet. Go to step 4.

## 3. No fog: Tickets

1. Write the Tickets. Each Ticket is one pull request's worth of work, sized for one session. Each Ticket has a description, acceptance criteria, and at most one Area label from **TICKET LABELS** (an `area` kind), by its configured name such as `orqa:fe`. Add a `modifier` label only when its guidance says it fits. Leave `--labels` out when no label fits. Never add `orqa:release`.

   ```
   bd create --type=task --priority=2 --title="<title>" --acceptance="<criteria>" --labels=<area label> --body-file=- <<'EOF'
   <description>
   EOF
   ```

2. Work that spans areas (a screen and the API behind it, say) is an Epic with one Ticket per area. The Epic's description holds the contract each side expects of the other; Orqadence hands it to each Ticket's sessions as Epic context. Create the Epic first, then its Tickets as its children:

   ```
   bd create --type=epic --priority=2 --title="<title>" --body-file=- <<'EOF'
   <what it builds, and the contract between its Tickets>
   EOF
   bd create --parent <epic> --no-inherit-labels --type=task --priority=2 --title="<title>" --acceptance="<criteria>" --labels=<area label> --body-file=- <<'EOF'
   <description>
   EOF
   ```

3. Wire the blocking edges in a second pass, once every Ticket has its id: `bd dep add <ticket> <the ticket it waits for>`.
4. When no configured label fits a Ticket's type of work, propose one in a `LABEL:` line of your result, and leave the Ticket without an Area label. Never write Orqadence's config and never add the proposed label in bd: the user decides.
5. Commit the document changes you made (`CONTEXT.md`, `docs/adr/`, notes), if any. If you committed anything, run the orqa-create-pr skill to open the one docs pull request for this branch. Its title and body say which Tickets it serves. With no changes, open no PR.

## 4. Fog: a Map

1. Create the Map. Its body holds five sections:

   ```
   bd create --type=epic --priority=2 --title="<title>" --labels=brainstorm:map --body-file=- <<'EOF'
   ## Destination

   <what reaching the end of this Map looks like, in one or two lines>

   ## Notes

   <the domain; standing preferences for this effort; skills every session should load>

   ## Decisions so far

   <!-- one line per closed Waypoint: its title, then the gist of its answer -->

   ## Not yet specified

   <the fog: questions you can tell are coming but cannot yet phrase sharply>

   ## Out of scope

   <work ruled beyond the Destination, one line each with why>
   EOF
   ```

   Not yet specified is in scope, just not sharp enough to be a Waypoint. Do not pre-slice it into Waypoint-sized pieces. Out of scope is what the Destination rules out, and it never comes back as a Waypoint.

2. Create a Waypoint for each question you can state precisely now, even one that is blocked. Each is sized for one session and carries exactly one type label:
   - `brainstorm:grilling`: a decision made in conversation with the user. The default.
   - `brainstorm:prototype`: a cheap, rough artifact to react to (a stub, a screen, some logic) when how it looks or behaves is the question.
   - `brainstorm:task`: work that must happen before a decision can be made, such as getting access to an API to judge it.
   - `brainstorm:research`: a fact that a decision waits on, found in docs, third-party APIs or the codebase, with no user needed.

   ```
   bd create --parent <map> --no-inherit-labels --type=task --priority=2 --title="<title>" --labels=brainstorm:<type> --body-file=- <<'EOF'
   ## Question

   <the decision or investigation this Waypoint resolves>
   EOF
   ```

   `--no-inherit-labels` is required: without it bd copies `brainstorm:map` onto the Waypoint.

3. Create exactly one build-Epic Waypoint, which a later session works last:

   ```
   bd create --parent <map> --no-inherit-labels --type=task --priority=2 --title="Write the Epics that build what this Map decided" --labels=brainstorm:epic --body-file=- <<'EOF'
   ## Question

   Which Epics, with which Tickets, build what this Map decided?
   EOF
   ```

4. Wire the blocking edges in a second pass, once every Waypoint has its id:
   - `bd dep add <waypoint> <the waypoint it waits for>` for each dependency between Waypoints;
   - `bd dep add <build-Epic waypoint> <waypoint>` for every other Waypoint, so the build-Epic Waypoint is blocked by all of them.
5. Commit the document changes you made, if any. Open no PR: the session that writes the Epics opens it.

## Result file

Write the **RESULT FILE** last, after the commit and the PR. The Orchestrator parses its first lines; nothing counts until it exists. After a Map:

```
STATUS: done
MAP: <id>

<the Destination, and the Waypoints on the frontier, by title>
```

After Tickets:

```
STATUS: done
TICKETS: <id> <id>...
PR: <url>
LABEL: <name> | <guidance> | skills: <a>, <b> | tickets: <ids>

<what each Ticket builds, by title>
```

`TICKETS:` lists the Tickets, never their Epic. Leave the `PR:` line out when you opened no PR. Write one `LABEL:` line per proposed label, or none. Its name is bare (`docs`, not `orqa:docs`). Its skills are the skills you suggest its Stages load, or empty. Its tickets are the ids that need it.

If you cannot finish (bd fails, the user stops the charting), write `STATUS: failed` followed by the reason.
