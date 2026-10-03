---
name: orqa-brainstorm-research
description: Answer one Research Waypoint of a Brainstorm's Map from primary sources, alone - write the findings to docs/research/<name>.md on the Brainstorm branch, post the resolution, close the Waypoint and create the Waypoints its findings raise. Run by Orqadence in the Brainstorm's worktree, or loaded by orqa-brainstorm-waypoint, not by hand.
---

# Brainstorm research

Adapted from the research and wayfinder skills of [mattpocock/skills](https://github.com/mattpocock/skills) (MIT License, Copyright (c) 2026 Matt Pocock).

You answer one Research Waypoint of a Map: a question that research alone settles. You run in the Brainstorm's worktree, `.orqadence-local/worktrees/<idea>` on branch `brainstorm/<idea>`, which other sessions share. The **Map** is a bd epic labelled `brainstorm:map`; its children are the **Waypoints**.

**Nobody is there: never ask.** Where the sources leave a choice open, decide, and say in the findings what you decided and why. The research is yours: do it in this session, and never hand the research to a background agent. A quick lookup inside it (a file search, one page) may use your App's built-in subagents, and you wait for what they find.

Use the {{prose}} skill for your commit, bd comments and result file: load it by name in this session.

## Inputs

Orqadence gives you these:

- **MAP**: the Map's id.
- **WAYPOINT**: the Research Waypoint. Orqadence claimed it for you: do not claim another.
- **RESULT FILE**: the path you write last.

When the orqa-brainstorm-waypoint skill loaded you, MAP and WAYPOINT are its own, and you skip step 6: its session writes the result.

## Do

1. **Read the question.** Run `bd show <MAP>` for the Destination and the Notes, and `bd show <WAYPOINT> --json --include-comments` for the Waypoint; its `## Question` is your whole scope. Fetch any Waypoint or note it names.
2. **Investigate against primary sources**: official docs, source code, specs, first-party APIs, not a secondary write-up of them. Follow every claim back to the source that owns it, and note the version, commit or date you checked.
3. **Write the findings** to one file, `docs/research/<name>.md`, `<name>` a short kebab-case name for the question. Cite each claim's source. Commit that path alone to the Brainstorm's branch:

   ```
   git add -- docs/research/<name>.md
   git commit -m '<message>' -- docs/research/<name>.md
   ```

   If git reports that `index.lock` exists, another session is committing: wait a few seconds and run both commands again. Never push. The note reaches the default branch with the Brainstorm's docs pull request.
4. **Resolve the Waypoint.** Post the answer as its resolution comment, then close it with a one-line reason:

   ```
   bd comments add <WAYPOINT> -f - <<'EOF'
   <the answer in a few lines, and what it leaves open>
   Findings: docs/research/<name>.md on brainstorm/<idea>.
   EOF
   bd close <WAYPOINT> --reason "<one line>"
   ```

   Never edit the Map's description: the next session with the user adds this Waypoint's line to Decisions so far. Anything your findings make dimly visible but not yet sharp goes in the comment, not on the Map.
5. **Create the Waypoints your findings raise.** A question you can state precisely now becomes a Waypoint, a Research one included when research alone settles it. Create them all first, then wire them, since a Waypoint needs its id before another can name it:

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

   Never create a `brainstorm:epic` Waypoint.
6. **Write the RESULT FILE** last, after the commit:

   ```
   STATUS: done
   WAYPOINT: <id>

   <the answer in a few lines; the Waypoints you created; the findings' path>
   ```

   If the question cannot be answered from the sources you can reach, still commit what you found, comment, and close the Waypoint saying so. Write `STATUS: failed` with the reason only when you could not do the work at all.

## Manual work

Anything that needs a credential or a human action you cannot do, such as an account to read a private API, is Manual work: load the orqa-manual-work skill by name and file it. Never run a command it says never to run (`gh secret`, `gh variable`, `gh workflow run` and the rest): a denied attempt is your sign to file it. Your Run directory is `.orqadence-local/runs/<waypoint>/` in the main checkout, `../../runs/<waypoint>/` from this worktree; create it if it is missing. The item's first line is `Ticket: <waypoint> · Stage: research · Blocks: yes|no`.

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
