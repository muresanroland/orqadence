---
name: orqa-stage-release
description: Orqadence Release Stage. Raises the Target repo's version once a run's Tickets are merged, adds a changelog entry when the repo keeps one, and opens the version pull request. Run by the Orqadence Orchestrator at a run's end, not by hand.
---

# Release Stage

You are the Release Stage of an Orqadence run, in a fresh session inside its own worktree (**Worktree**), on a new branch off the default branch: every Ticket of the run is merged into it. Ask only what the repo's docs and the Inputs leave open; otherwise decide, and note the answer you took from them. Inputs are under **Inputs** at the end.

**Bump** is `minor` (an Epic's run: vX.Y.Z to vX.Y+1.0) or `patch` (a Ticket run: vX.Y.Z to vX.Y.Z+1). **Epic** is the Epic's id and title, or `none` in a Ticket run. **Tickets** lists each merged Ticket's id, title and PR url, one per line.

## 1. Find the version

1. Find every place the repo keeps its own version: `Cargo.toml` with `Cargo.lock`, `package.json` with its lockfile, `pyproject.toml`, a `VERSION` file, and the like. Then `git grep` the version you found to catch the rest (a constant the build reads, a chart). Only the repo's own version: a dependency's equal number is not it.
2. `git fetch origin --tags`, and take the highest `vX.Y.Z` tag, pre-release tags such as `v1.5.0-rc.1` aside (none: v0.0.0).
3. The base is the highest of the versions found and the tag.
4. Raise the base as **Bump** says.

A repo that keeps its version only in tags (no file holds it): change no file, commit nothing and open no PR; go to the Result file with the next version from the latest tag.

## 2. Raise it

1. Write the new version in every place step 1 found. A lockfile's own entry is updated with the ecosystem's own command (`cargo update --workspace --offline`, `npm install --package-lock-only`, `pnpm install --lockfile-only`, `uv lock`), never by hand-editing unrelated lines. If that command is missing or fails, change only the lockfile's own entry by hand and say so in the PR body.
2. When the repo keeps a changelog (`CHANGELOG.md`, `CHANGES.md`, `HISTORY.md`, `NEWS.md`, in any case, at the root or in `docs/`), add an entry for the new version in its own format: its heading and date style, its sections, an `Unreleased` section moved under the new version. The entry says what the run's **Tickets** built. Never create a changelog.
3. Read `git diff`: every changed line is the version, a lockfile entry of the repo's own, or the changelog entry.
4. Commit to the current branch, in the repo's commit style (for example `chore: bump version to 1.5.0`).

## 3. Open the version pull request

Run the orqa-create-pr skill. Its title names the new version. Its body says the new version, the base and why that base (the manifest and the tag disagreeing, when they did), and where you changed the version; names the **Epic** when it is not `none`; and lists the run's **Tickets**, each with its PR.

This Stage never merges, never creates or pushes a tag and never creates a GitHub Release: the merge and the tag are the Orchestrator's.

## Result file

Write the **Result file** from Inputs last. The Orchestrator parses the first line, the `VERSION:` line and the `PR:` line:

```
STATUS: done
VERSION: v1.5.0
PR: https://github.com/owner/repo/pull/123

<where you changed the version, and why that base>
```

Leave the `PR:` line out in a repo that keeps its version only in tags. Write `STATUS: failed` with the reason only when the repo's checks cannot be made to pass or the PR cannot be opened.

To ask, write the **Result file** with `STATUS: question` as its first line, then the question, then its options as the last lines, one per line starting with `- `, and wait: the answer comes into this pane as a prompt. Carry on, and overwrite the Result file with done or failed when you finish.
