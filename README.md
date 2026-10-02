<p align="center">
  <img src="assets/brand/orqadence-banner.gif" alt="Orqadence — terminal orchestrator" width="640">
</p>

Drives a [beads](https://github.com/gastownhall/beads) Epic through a fixed multi-agent pipeline, from Tickets to open pull requests, with every agent visible in [herdr](https://herdr.dev).

Each Ticket gets its own git worktree and herdr tab. It passes through:

1. **Implement**: Claude Code writes a plan, which is approved before any edit, then implements it.
2. **Review**: Codex reviews the change.
3. **Debate**: a Claude moderator settles each Finding as fix or skip.
4. **Fix**: Claude Code applies the fix items.

Review, Debate and Fix repeat for up to 3 rounds, then a pull request opens. You merge it, or with Agent merge on Orqadence does. Orqadence then closes the Ticket and cleans up its worktree. Vocabulary: [CONTEXT.md](CONTEXT.md).

## Requirements

- macOS on Apple Silicon or Linux x86_64 (other platforms: build from source)

| Tool | Used for | Setup |
|---|---|---|
| [herdr](https://herdr.dev) | Runs every agent in a visible pane | Start `orqa` from a pane inside herdr (`HERDR_ENV=1`) |
| [beads (`bd`)](https://github.com/gastownhall/beads) | The Epic and its Tickets | `bd init` in the Target repo, which `orqa init` offers to run |
| [git](https://git-scm.com) | Branches and worktrees | The repo needs a remote on GitHub |
| [GitHub CLI (`gh`)](https://cli.github.com) | Opens PRs, watches for merges | `gh auth login` |
| [Claude Code (`claude`)](https://claude.com/claude-code) | Implement, Debate, Fix, Rebase, Address PR comments | Logged in |
| [Codex CLI (`codex`)](https://github.com/openai/codex) | Review | Logged in |
| [TypeSafe](https://docs.typesafe.ai) API key | Optional: the Judgment approves plans and handles stuck sessions, and the Debate settles disputed Findings | `TYPESAFE_API_KEY`, or say yes and paste it at `orqa init` |

With TypeSafe off, Orqadence works the same but asks you instead: every plan approval and every stuck session becomes a Question in the Shell, and a Finding the Debate's sides still dispute is skipped and listed in the PR.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/muresanroland/orqadence/main/install.sh | sh
```

This installs the latest release as `~/.local/bin/orqa`. Set `ORQADENCE_INSTALL_DIR` to install somewhere else. Keep that directory writable by you and on your `PATH`.

**Updates are automatic.** A release build checks GitHub Releases each time the Shell opens, and once a day while it stays open. It downloads a newer version and swaps it in when no run is live in that repo. `orqa --version` prints the version, for example `v1.0.0`.

To build from source instead, run `cargo install --git https://github.com/muresanroland/orqadence`. That gives a `-dev` build, which never updates itself.

## Set up a Target repo

A Target repo is the repository whose Epic you want worked on. Once, from a herdr pane inside it:

```bash
orqa init
```

`init` does these things:
- It installs the Stage skills, `orqa-create-pr`, `orqa-infra-review`, `orqa-address-pr-comments` and `orqa-manual-work` in `.orqadence/skills`, each linked from `.agents/skills` and `.claude/skills` by a relative folder link. Commit the skills and the links. They are yours to edit from then on, and running `init` again asks before touching them. A Stage skill the repo already has in `.agents/skills` moves into `.orqadence/skills` as it is, with a link left behind.
- Every skill it installs is named `orqa-<name>`, in its folder and its SKILL.md, so none collides with a skill of your own; an install from before that is renamed, its links, picks and record with it. Commit the rename and merge it: a Ticket runs the skills on its base.
- In a checkout an older `init` set up, it moves the skills that `init` put in `.agents/skills` into `.orqadence/skills` and leaves links behind; the repo's own skills stay where they are. Skills it put at user level (`~/.agents/skills`) are copied in as they are, edits kept, and the `~` copies stay. It also takes out the lines an older Orqadence added to `.git/info/exclude` to hide the skill links.
- With no `bd` workspace, it offers to run `bd init`.
- It offers to write the beads `docs/agents` setup the skills read, only what is missing: `docs/agents/issue-tracker.md`, `triage-labels.md`, `domain.md`, and an Agent skills block in `CLAUDE.md`, or `AGENTS.md` when there is no `CLAUDE.md`. A non-interactive `init` writes them too.
- It asks whether to use TypeSafe, yes by default. Yes asks for the key, which it keeps in `.orqadence-local/typesafe-key`, and installs the `orqa-typesafe-ai` skill; an empty key is no. With `TYPESAFE_API_KEY` set it is on without asking. Without it, a non-interactive `init` leaves TypeSafe on only where it is already on with a kept key, and off everywhere else. On or off is kept in `.orqadence/config.json`.
- It installs each job's default skill, pinned by commit: `orqa-tdd`, `orqa-code-review`, `orqa-ponytail`, `orqa-caveman`, `orqa-ponytail-review` and `orqa-resolving-merge-conflicts`; the PR comments job's default is the shipped `orqa-address-pr-comments`. A job picks from these, the repo's own skills and the Apps' built-in ones; your personal skills (`~/.claude/skills`, `~/.agents/skills` and Claude Code plugins') join them once you turn them on, for yourself alone, on `/config`'s Skills page.
- It asks whether to rebase PRs that conflict with main by themselves, and whether to open PR comments and failing checks for approval by themselves. Both default to no and are kept in `.orqadence/config.json` as `rebase_auto` and `address_pr_comments_auto`. A re-run keeps what is set, and a checkout whose settings are committed is not asked.
- With PR comments opened by themselves, it asks whether to merge Ticket PRs by themselves (Agent merge; security, db and infra Tickets still wait for you), no by default, kept as `agent_merge`. Without PR comments opened by themselves it is not asked and stays off. Then, Agent merge on or off, it asks which review bots the repo has (coderabbit, greptile), kept as `review_bots`; under Agent merge the minutes it waits for a bot, `bot_wait`, stay at 30 until `/config` changes them. A re-run keeps what is set, and a checkout whose settings are committed is not asked. Init tells each bot listed to skip a pull request labelled `orqa:no-review` (one whose Ticket carries `orqa:no-review`, one that changes only Markdown or skills, or the Release's version pull request): `"!orqa:no-review"` in `reviews.auto_review.labels` of `.coderabbit.yaml`, and `orqa:no-review` in `disabledLabels` of `greptile.json`, or of `.greptile/config.json` when the repo has one. A missing file is created, as is an empty `.coderabbit.yaml`, a JSON one keeps every other key (sorted), and init names each file it wrote so you commit it. A file init cannot edit, as a `.coderabbit.yaml` with settings of its own, is left as it is, and init says what to add by hand.
- It keeps on GitHub the labels Orqadence puts on pull requests: `orqa:human-merge`, `orqa:no-review`, each Ticket label in `.orqadence/config.json` (Area labels one colour, Modifier labels another) and, with releases on, `orqa:release`. A missing one is made and one whose colour or description differs is set again; none is deleted. Without a GitHub remote or a `gh` login it says so and goes on. `/config` does the same for a label it adds, renames or turns from Area to Modifier. bd needs nothing: a bd label exists once an issue carries it.
- It asks whether to turn on releases (the `orqa:release` label), yes by default, kept in `.orqadence/config.json` as `release_on`. A non-interactive `init` turns it on only where it is not set yet; a re-run keeps what is set, and a checkout whose settings are committed is not asked.
- It offers to install herdr's integration for claude or codex when it is not installed or outdated, listing what each install writes. Without it herdr does not know a session's id, so `/continue` starts those Stages fresh instead of resuming them.
- It runs a preflight that reports anything still missing: the `bd` workspace, `gh` auth, the git remote, the `orqa-create-pr` skill, a job's skill, an App a Stage runs on that is not on `PATH`, or herdr. It also warns about a personal skill that shadows an installed one on Claude, and about the superpowers plugin being enabled.

`.orqadence/` holds what gets committed: `config.json`, `skills.json`, `installed-skills.json` and the skill files. What depends on the machine, the person or the run goes in `.orqadence-local/`, which `init` makes with a `.gitignore` of `*` inside, so the folder ignores itself; `init` adds nothing to the repo's `.gitignore`, and takes out the `.orqadence/` line an older `init` added. In a checkout an older Orqadence set up, `init` first lists the run files it left in `.orqadence/`, naming each worktree with uncommitted changes, and deletes them once you answer yes: the worktrees are removed, their branches kept, and a saved run is lost. No, or nobody answering, stops `init` with nothing changed. `orqa` opens the Shell only once `.orqadence-local/` exists.

## Run an Epic

1. Create the Epic and its Tickets in beads. Each Ticket should say what to build and how to check it. Blockers work as usual: a Ticket that depends on another waits for that Ticket's PR to merge.

   ```bash
   epic=$(bd create --silent --type=epic --title="...")
   bd create --parent "$epic" --type=task --title="..." --description="..." --acceptance="..."
   ```

2. Open the Shell from a herdr pane in the Target repo:

   ```bash
   orqa
   ```

   It shows the open Epics and their Tickets. Type `/` to pick a command, then `@` to pick an Epic or Ticket by its id or part of its title; Tab or Enter fills in the one under the cursor.

3. Watch progress in the Shell. Each Ticket's panes appear in its own herdr tab. Answer Questions as they come up: plan approvals, sessions waiting at a prompt, and Wakes the Judgment wasn't sure about.

4. Review and merge the PRs on GitHub. With Agent merge on, Orqadence merges a PR itself once its checks are green and, unless it is a No-review pull request (labelled `orqa:no-review`), the review bots have reviewed it and every PR comment is answered. A PR comment still open once Address PR comments is done with the PR, or a listed bot with no review `bot_wait` minutes after the PR opened, is a Question: merge, park, or for the bot keep waiting. While you are Away TypeSafe answers it over the diff and the open items, merging at or above the Wake floor and parking otherwise. A merge GitHub refuses parks the Ticket, and security, db and infra PRs still wait for you. Orqadence closes each merged Ticket and starts the Tickets that were waiting on it. When every Ticket is closed, the Epic is done.

### Shell commands

| Command | What it does |
|---|---|
| `/start-epic <epic>` | Run every Ticket of the Epic, at most `max_tickets` at once (3 unless `/config` says otherwise) |
| `/start-ticket <ticket>…` | Start a Ticket run over one or more Tickets (`/start-ticket @a @b @c`), with or without an Epic. It stays live until every PR is merged, closing each Ticket as its PR merges; while it runs, `/start-ticket` adds Tickets to it |
| `/remove-ticket <ticket>` | Take a Ticket out of the live Ticket run: a queued or Parked one, or one whose PR is open (the PR stays open). `/start-ticket` adds it back where it left off. A working one is refused: `/park` it first; so is one another Ticket of the run waits on |
| `/continue [<ticket>]` | Resume the saved run, e.g. after `/stop-work` or a restart. With a Ticket: unpark that Ticket at its Stage and put its question to you first |
| `/stop-work` | Stop scheduling. Agent panes keep running and the state is saved |
| `/retry <ticket>` | Rerun the Ticket's failed Stage with a fresh session |
| `/park <ticket>` | Take the Ticket out of the pipeline; the others keep going |
| `/rebase <ticket>` | Rebase the Ticket's open PR onto main, keeping both sides' intent or asking you; refused unless the last poll saw it conflict with main |
| `/address-pr-comments <ticket>` | Open the approval modal over every PR comment and failing check still open on the Ticket's PR, with no countdown: Space unchecks a row, Enter starts Address PR comments, which fixes the checked ones, answers the rest as won't fix and pushes to the same PR; Esc cancels. With `address_pr_comments_auto` on, the modal opens by itself once the PR's head is quiet (a run per PR up to `address_pr_comments_runs`), and approves the rows as they stand when its countdown ends unless a key stops it; under Away it approves them all without opening |
| `/questions` | Show the Questions waiting for you |
| `/manual-work` | Open the Manual work still open in every Run directory, one row per item with its Ticket, What and folder: Space checks a row, Enter adds a bd comment with each checked item's What and deletes its folder, Esc changes nothing. A blocking item shows but cannot be checked: its Question marks it done. An item stays open until it is marked done, even after its PR merges |
| `/config` | Pick the App, model and effort each Stage runs on, and a plan model other than Implement's; every change saves at once to `.orqadence/config.json`, and during a run the Stages that start after it use it. A change that breaks a check (the Review on Implement's model, the Debate's sides in one family) is refused; the Apps page shows which Apps are installed. Each Stage's page also picks its jobs' Delegate skills; the Skills page adds (`a`), updates (`u`, `U` for all) and removes (`d`) the skills Orqadence installed; the TypeSafe page turns TypeSafe on or off and asks the key when there is none; the Run page sets how many Tickets a run has in the Pipeline at once, and `max_pr_sessions`, how many Rebase and Address PR comments sessions run at once, which the live run takes up at once; the Rebase and Address PR comments pages turn each on by itself, and the latter sets the approval countdown in minutes (0 waits) and the runs per PR, and under AGENT MERGE whether Ticket PRs merge by themselves (refused while PR comments are not opened by themselves, and turned off with them), the repo's review bots and the minutes to wait for one; the Release page turns releases on and picks the Release's App, model and effort |
| `/away` | Toggle Away: a Stage's question parks its Ticket, with a bd comment, until you `/continue @ticket` it, and a PR's new comments are approved without the modal |
| `/summary [<epic>]` | Show the run's PRs, Rounds, Findings, cost and time (an Epic's, or alone the live, saved or last run's, Ticket runs too); it also opens by itself once every Ticket has its PR or is Parked, or, for a run carrying `orqa:release`, once its Release ends, with a Version line |
| `/demo` | Play a made-up run to see the Shell at work: Tickets moving through their Stages, RECENT filling, each kind of Question waiting for your answer, a Ticket waiting on a merge, a usage limit, and the Epic summary at the end. Nothing starts and nothing is written |
| `/stop-demo` | End the demo; the Shell is back as it was. `/stop-work` does the same |
| `/exit` | Leave the Shell (asks first during a run). Ctrl-C twice does the same |

Leaving the Shell never kills agent panes.

### What it writes

Committed, in `.orqadence/` in the Target repo: `config.json`, `skills.json`, `installed-skills.json` and the skills.

Local, in `.orqadence-local/`, which ignores itself:
- `orchestrator.log`: one line per event.
- `state.json`: the run, so it can be resumed.
- `lock`: one run per Target repo.
- `runs/<ticket>/`: each Stage's results, diffs and Debate transcripts.
- `worktrees/`: one worktree per Ticket.
- `typesafe-key`: the TypeSafe key, readable only by you.

## Releasing

Cargo.toml's `version` is the source of truth. After the merge, push the matching `vX.Y.Z` tag (`git tag vX.Y.Z origin/main && git push origin vX.Y.Z`). That starts the release workflow, which builds both binaries on GitHub and creates the release with them and `THIRD-PARTY-LICENSES.txt`, the license notices of the crates they link. Don't create the release on GitHub yourself: releases are immutable once published, so it would go out with no binaries and nothing can attach them later. If a tag's run fails before the release is made, retry it from the Actions tab (**release**, then **Run workflow**) with that tag, or run `gh workflow run release.yml -f tag=vX.Y.Z`. Feature tickets bump minor, fixes bump patch.

## Build

See the build and test commands in [AGENTS.md](AGENTS.md).

## License

[PolyForm Noncommercial 1.0.0](LICENSE.md): free for personal, hobby, research and nonprofit use. Any commercial use, selling it included, needs a separate license; contact [@muresanroland](https://github.com/muresanroland).
