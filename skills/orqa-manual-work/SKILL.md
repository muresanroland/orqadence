---
name: orqa-manual-work
description: File Manual work for the user, never do it - anything a session needs that takes a credential or a human action it cannot do (repo secrets and variables, workflow runs, dashboards, cloud accounts, terraform against real state). Writes a folder in the Run directory with written steps and, as the task needs, a wizard or a prompt for a separate session. Planned for loading by Orqadence's Stage and Brainstorm sessions once Manual work support ships, not by hand.
---

# Manual work

Adapted from the wizard skill of [mattpocock/skills](https://github.com/mattpocock/skills) (MIT License, Copyright (c) 2026 Matt Pocock); [template.sh](template.sh) is its template, vendored with the full notice in its header.

**Manual work** is something your session needs done that it cannot do itself, above all anything that needs a credential. You file it for the user as a folder of written steps and, as the task needs, a wizard to run or a prompt for a separate session. You write every file; the Orchestrator composes no text. You never do the work yourself, even when a tool you hold could.

## When to file

File anything that needs a credential or a human action you cannot do. Never run these; file them as Manual work instead:

- `gh secret`, in any form
- `gh variable`, in any form
- `gh workflow run`
- `gh api` writes to repo settings
- cloud CLIs acting on real accounts (gcloud, aws, az and the like)
- `terraform plan` or `terraform apply` against real state

gh is signed in for you, so these would work: that is why they are never run. A denied attempt is your sign to file Manual work, not to find another form of the same call.

Private registry or git modules that make `terraform init` need a credential are Manual work too. Then terraform validate and test cannot run: say so in your result, so the pull request says it.

## The folder

Each item is one folder: `<Run directory>/manual-work/<n>/`, `<n>` the next number not used there (1 when there is none). It holds:

- `manual-work.md`, always. Its first line is `Ticket: <id> · Stage: <stage> · Blocks: yes|no` (a Brainstorm session puts its Waypoint's id), then these headings:
  - `## What`: the work, in one line the user reads in a list.
  - `## Why`: what in the Ticket needs it, and what it unblocks.
  - `## How`: which files of the folder to use, in what order, and the steps they do not cover.
  - `## Report back`: the facts your session needs back, such as a value's name or an ID (never a secret's value). Empty when there are none.
- `wizard.sh`, as the task needs: template.sh's library plus the task's stages (see **The wizard**). You write it and never run it.
- `prompt.md`, as the task needs: a prompt for a separate agent session that has the user's logins, such as gcloud. It names the repo, the goal, the exact commands or the outcome, and what to report back.

Which fits:

- dashboards, secrets and logins: a wizard.
- work that needs the user's CLI logins: a prompt.
- otherwise: Markdown alone.

A task can take both, a wizard for the secrets and a prompt for the cloud work: say the order under `## How`.

## Blocking or not

You decide whether your work can go on without the item.

- **Not blocking**: write the folder and carry on. The pull request lists it. You can file several.
- **Blocking (planned)**: once the Stage loader and Orchestrator support Manual work, at most one at a time, since you wait on it; put related steps in one item. Write the folder, then the Result file with `STATUS: manual` as its first line and the folder path as its second, and wait in your pane. The answer comes into the pane as the prompt `Manual work <n> done: <facts>`. Carry on with those facts, and overwrite the Result file when you finish.

## The wizard

A **wizard** is a bash script that walks the user, step by step, through a manual procedure that's tedious to do by hand and tedious to re-explain to an AI every time. It opens each URL, says exactly what to click and copy, captures the values, writes them where they belong (`.env`, GitHub secrets), confirms at every stage, and shows how many stages are left.

The delightful UX is already solved by [template.sh](template.sh): stage-by-stage progress, confirmation gates, cross-platform URL opening (including WSL), hidden secret entry, idempotent `.env` upserts, `gh secret`/`gh variable` writes, and a closing summary. **Your job is only to scope the procedure and author its stages.** The library above the `STAGES` marker is identical in every wizard; that consistency is the point: never hand-edit it. A wizard's stage is one step of the script, not an Orqadence Stage.

### 1. Scope the procedure

Work out every manual step the user must take and every value that gets captured along the way. Nobody is there to confirm the stages with you: the scope comes from the Ticket and the repo. Read:

- For setup: `.env`, `.env.example`, `.env.*`, `README`, `docker-compose*`, framework config, and `.github/workflows/*` (every `secrets.*` / `vars.*` reference is a value the wizard must produce).
- For a migration or transition: the current state, the target state, and the irreversible actions between them.

**Done when:** every stage is named in order, and for each captured value you know (a) where the user gets it, (b) where it's written (`.env`, a GitHub secret, both, or nowhere; some stages are pure actions), and (c) whether it's secret (hidden entry) or public.

### 2. Map each stage's journey

For each stage, write the precise path a user follows: which URL to open, what to do there, where a value is shown, which variable it fills: e.g. "Dashboard → Developers → API keys → Reveal test key → copy". Where you don't actually know the current UI or the exact command, check the docs, and where they leave it open, say so in the stage: never invent steps that may not exist.

**Done when:** every stage traces to concrete instructions a stranger could follow.

### 3. Author the wizard

Copy `template.sh` (beside this file) to `wizard.sh` in the item's folder. Replace the example stage with one `stage` per step, in dependency order. Use the library helpers: `stage`, `say`/`step`, `open_url`, `ask`/`ask_secret`, `write_env`, `set_secret`/`set_var`, `pause`/`confirm`. Set `TOTAL_STAGES` to the number of stages you wrote.

Hold the bar the template sets: open the URL before asking for its value, use `ask_secret` for anything secret, `write_env` every persisted value, `set_secret` only the values CI actually needs, and `confirm` before any irreversible action. Each `stage` clears the screen so only the current step is visible: keep a stage to one focused task so nothing the user needs scrolls away. Don't touch the library above the marker.

### 4. Verify

- `bash -n wizard.sh`; run `shellcheck` if available.
- `chmod +x wizard.sh`.
- Never run it: it opens browsers, blocks on human input and acts with the user's credentials. Trace it statically instead: every value from step 1 is captured and lands where step 1 said, and every `set_secret` name exactly matches a `secrets.*` reference in CI.
- Say under `## How` how to run it: from which folder, with which `ENV_FILE` if not the repo's `.env`.
