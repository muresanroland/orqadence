---
name: orqa-brainstorm-waypoint
description: Work one Waypoint of a Brainstorm's Map with the user - grilling, a prototype, a task, or research when it does not run in the background - and record its decision on the Map. Run by Orqadence in the Brainstorm's worktree, not by hand.
---

# Brainstorm Waypoint

Adapted from the wayfinder and prototype skills of [mattpocock/skills](https://github.com/mattpocock/skills) (MIT License, Copyright (c) 2026 Matt Pocock).

You work one Waypoint of a Map with the user, in a fresh session inside the Brainstorm's worktree, `.orqadence-local/worktrees/<idea>` on branch `brainstorm/<idea>`. The **Map** is a bd epic labelled `brainstorm:map`; its description holds the Destination, the Notes, the Decisions so far, what is Not yet specified and what is Out of scope. Its children are the **Waypoints**, one question each, labelled `brainstorm:grilling`, `brainstorm:prototype`, `brainstorm:task`, `brainstorm:research` (a Research Waypoint) or `brainstorm:epic` (the one build-Epic Waypoint). Resolve exactly one Waypoint this session.

Use the {{prose}} skill for your commits, bd comments and result file: load it by name in this session.

## Inputs

Orqadence gives you these:

- **MAP**: the Map's id.
- **BACKGROUND**: `on` or `off`. On, Orqadence runs the Research Waypoints in sessions of their own.
- **PROMPT**: what the user said when starting this session, or `none`.
- **WAYPOINT**: optional. Set, it is the Waypoint you take.
- **RESULT FILE**: the path you write last.

## Rules for the whole session

- **One question at a time.** Ask the user through your App's own question tool (AskUserQuestion on claude), never as text in your reply, and never several questions at once. Put your recommended answer first.
- **Refer by name.** In anything the user reads, name a Map or Waypoint by its title; the id rides beside it, never in place of it.
- **Plan, don't do.** A Waypoint ends in a decision, not a deliverable. The Map's Notes may lift this for this Map.
- **bd as written here.** Every bd command you need is written out below; there is no tracker doc to look up.
- **Commit only your paths.** Other sessions share this worktree. Commit to the Brainstorm's branch the paths you changed, and only those:

  ```
  git add -- <paths>
  git commit -m '<message>' -- <paths>
  ```

  If git reports that `index.lock` exists, another session is committing: wait a few seconds and run both commands again. Never push.
- **Manual work.** Anything that needs a credential or a human action you cannot do is Manual work: load the orqa-manual-work skill by name and file it. Never run a command it says never to run (`gh secret`, `gh variable`, `gh workflow run` and the rest): a denied attempt is your sign to file it. Your Run directory is `.orqadence-local/runs/<waypoint>/` in the main checkout, `../../runs/<waypoint>/` from this worktree; create it if it is missing. The item's first line is `Ticket: <waypoint> · Stage: waypoint · Blocks: yes|no`.

## Do

### 1. Load the Map

Run `bd show <MAP>`. Orient to its Destination before anything else, and load by name every skill its Notes name. Do not read every Waypoint: fetch one in full only when you need it, with `bd show <id> --json --include-comments`.

### 2. Bring Decisions so far up to date

List the Map's closed Waypoints:

```
bd list --parent <MAP> --status closed --json
```

Each one that is in neither Decisions so far nor Out of scope gets a line in Decisions so far, from its close reason: `- <title> (<id>): <close reason>`. Research sessions never write the Map, so their Waypoints arrive here. Read the description again just before you write it, then write the whole description back:

```
bd update <MAP> --body-file - <<'EOF'
<the Map's whole description>
EOF
```

Skip the write when nothing was missing.

### 3. Choose and claim your Waypoint

With **WAYPOINT** set, take that one. Otherwise run the frontier query:

```
bd ready --parent <MAP> --unassigned --json
```

It lists the open, unblocked, unclaimed Waypoints in map order, the order Orqadence reads too. Take the first one in that list, except: skip `brainstorm:research` when BACKGROUND is on, since Orqadence runs and claims those itself; and never take `brainstorm:epic`, since Orqadence starts the build-Epic Waypoint's own session when it is all that is left. If nothing is left to take, write `STATUS: failed` with that reason and stop.

Claim it before any work, so concurrent sessions skip it:

```
bd update <id> --claim
```

If the claim fails because another session holds it, run the frontier query again; with **WAYPOINT** set, write `STATUS: failed` with that reason instead.

### 4. Work it

Read the Waypoint: `bd show <id> --json --include-comments`. Its `## Question` is your whole scope. **PROMPT**, when not `none`, is the user's first word on it. Fetch any related or closed Waypoint in full as you need it. By its label:

- **`brainstorm:grilling`**, and whenever in doubt: load the orqa-brainstorm-grilling skill and the orqa-brainstorm-domain-modeling skill by name, and grill the user until the question is decided.
- **`brainstorm:task`**: nothing to decide; this work unblocks a decision. Do what you can yourself. The checklist the user must do is Manual work: file it, blocking when the Waypoint cannot resolve without it. The answer records what was done and the facts later Waypoints depend on (where a credential lives, a new URL, a row count; never a secret's value).
- **`brainstorm:research`**, only when BACKGROUND is off: load the orqa-brainstorm-research skill by name and do the research as it says, here in this session. It posts the resolution comment, closes the Waypoint and creates the Waypoints its findings raise; you then do only step 6's points 3 and 8 (its Decisions-so-far line, the Map write and your commit) and write your own result file.
- **`brainstorm:prototype`**: build a prototype, below.

### 5. Prototype

A prototype is **throwaway code that answers a question**; the question decides its shape. It is the one place a Brainstorm writes code.

1. Name it: `<name>`, a short kebab-case name for the question. `<idea>` is this worktree's folder name.
2. Give it a worktree of its own, `.orqadence-local/worktrees/<idea>-proto-<name>`, on a new branch `prototype/<name>` from the Brainstorm branch. Never switch branch in the Brainstorm's worktree: research sessions work in it.

   ```
   git worktree add ../<idea>-proto-<name> -b prototype/<name> brainstorm/<idea>
   ```

   Write the prototype only in there.
3. Use the {{working-mode}} skill for the prototype's code: load it by name in this session.
4. Pick the branch from the question, asking the user when it is unclear:
   - **"Does this logic or state model feel right?"**: [LOGIC.md](LOGIC.md). A single shareable HTML file that pushes the state model through the cases hard to reason about on paper.
   - **"What should this look like?"**: [UI.md](UI.md). Several radically different UI variations on one route, switchable from a floating bar.
5. Rules for both:
   - **Throwaway, and clearly marked so.** Put it next to what it prototypes, named so a reader sees it is a prototype. For a UI route, follow the project's routing convention.
   - **Trivial to run**: one command in the project's task runner, or one HTML file to double-click.
   - **No persistence** unless persistence is the question; then a scratch DB or a local file with a clear "PROTOTYPE, wipe me" name.
   - **Skip the polish**: no tests, no error handling beyond what makes it run, no abstractions.
   - **Surface the state**: after every action, or on every variant switch, show the full relevant state.
6. Hand it to the user: say how to run it, and grill them on what it shows until the question is decided.
7. Capture it: commit the prototype in its worktree (the commit rules above, run there) and link the branch on the Waypoint:

   ```
   bd comments add <id> -f - <<'EOF'
   Prototype: branch prototype/<name>, run with <command>. See it with git show prototype/<name>:<path>.
   EOF
   ```

   Nothing folds into real code: the decision goes on the Waypoint (step 6).
8. Remove the worktree before you write your result, and keep the branch: local, never pushed, never merged.

   ```
   git worktree remove ../<idea>-proto-<name>
   ```

   If it refuses over untracked build output, run it again with `--force`: the branch already holds the commit.

### 6. Resolve

1. Post the answer as the **resolution comment**: the decision and why, in a few lines, with a pointer to any file or branch that holds the detail.

   ```
   bd comments add <id> -f - <<'EOF'
   <the decision and why>
   EOF
   ```

2. Close it with a one-line gist: `bd close <id> --reason "<gist>"`.
3. Add its line to the Map's Decisions so far, `- <title> (<id>): <gist>`. The Map is an index: gist the decision and point at the Waypoint, never restate it.
4. Add the Waypoints the answer surfaced. Create them all first, then wire them, since a Waypoint needs its id before another can name it:

   ```
   bd create --parent <MAP> --no-inherit-labels --labels brainstorm:<grilling|prototype|task|research> --title "<the question, as a name>" --body-file - <<'EOF'
   ## Question

   <the decision or investigation this Waypoint resolves>
   EOF
   ```

   Then make each new Waypoint block the build-Epic Waypoint, and wire any edge between Waypoints:

   ```
   bd list --parent <MAP> --label brainstorm:epic --json
   bd dep add <build-Epic Waypoint> <new Waypoint>
   bd dep add <blocked Waypoint> <Waypoint it waits on>
   ```

   Never create a second `brainstorm:epic` Waypoint.
5. **Graduate the fog.** Not yet specified holds what is in scope but not sharp enough to be a Waypoint. Make it a Waypoint when you can state the question precisely now, even if you cannot answer it yet; leave it as fog when you cannot. Each patch the answer has made specifiable becomes its new Waypoints and leaves Not yet specified; add any fog the answer revealed.
6. **Out of scope.** When the answer shows that a Waypoint, this one or another, sits beyond the Destination, close it, `bd close <id> --reason "Out of scope: <why>"`, and add one line under Out of scope: `- <title> (<id>): <gist and why>`. It stays out of Decisions so far.
7. When the decision invalidates other open Waypoints, update them (`bd update <id> --body-file -`) or close them with a reason.
8. Write the Map's description back once with all of it (step 2's command, re-reading first), and commit the files you changed (CONTEXT.md, docs/adr/, notes) with the commit rules above.

### 7. Result file

Write the **RESULT FILE** last, after the commit. Its first lines are what Orqadence parses:

```
STATUS: done
WAYPOINT: <id>

<the decision in a few lines; Waypoints created or closed; files and branches committed>
```

`WAYPOINT` is the one you took. If you cannot finish, write `STATUS: failed` followed by the reason and what you tried.

For Manual work you cannot go on without, write the **RESULT FILE** with `STATUS: manual` as its first line and the item's folder path as its second, and wait: the answer comes into this pane as the prompt `Manual work <n> done: <facts>`. Carry on with those facts, and overwrite the **RESULT FILE** with done or failed when you finish.

<!--
mattpocock/skills (https://github.com/mattpocock/skills), its licence:

MIT License

Copyright (c) 2026 Matt Pocock

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
-->
