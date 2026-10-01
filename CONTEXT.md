# Orqadence

Drives a beads epic through a fixed multi-agent pipeline, from tickets to open pull requests, with every agent visible in herdr.

## Language

**Orqadence**:
The globally installed tool as a whole: the Orchestrator and the Shell.

**Target repo**:
The repository whose epic is being worked on. Orqadence is run from inside it; it scaffolds the repo's agent setup once, and the repo owns its conventions from then on.
_Avoid_: Project, host repo

**Stage skill**:
A skill owned and shipped by Orqadence that holds the instructions for one Stage. Once installed for a Target repo, the installed copy is the one that runs and may be edited there.
_Avoid_: Prompt, template

**Brainstorm skill**:
A skill owned and shipped by Orqadence that holds the instructions for one kind of Brainstorm session, such as charting a Map or answering a Research Waypoint. Orqadence's own, modelled on mattpocock's wayfinder and the skills it calls; a Target repo changes a Brainstorm by editing the installed copy, not by swapping in another skill.
_Avoid_: Wayfinder, brainstorming skill

**Delegate skill**:
A skill, third-party or Shipped, a Stage skill or Brainstorm skill runs for one job (test-first implementing, self review, the over-engineering audit, merge conflicts, PR comments, how it writes...), chosen per job by the user: one Orqadence installed, the repo's own, one built into an App, or, once the user turns their personal skills on, one of their own at home or from a Claude Code plugin. A Stage can have several. The skill that runs it still owns its result; with no Delegate skill for a job it follows its own instructions. The Brainstorm's own jobs, grilling, domain modeling, research and prototyping, never take one.
_Avoid_: Override, replacement, work skill

**Shipped skill**:
Any skill Orqadence ships and installs for a Target repo, committed with the repo's Orqadence settings: the Stage skills, the Brainstorm skills, plus orqa-create-pr, which the Fix Stage and the Release run, orqa-infra-review, orqa:infra's Extra review skill, and orqa-address-pr-comments, the Address PR comments Stage's default Delegate skill. Like every skill Orqadence installs, fetched Delegate skills too, its name starts with orqa-, so none shares a name with a skill of the repo's or the user's own.

**Personal override**:
One setting of the Target repo's Orqadence settings that one person keeps for themselves, read over the repo's committed value on their machine only. It never leaves the machine and is never reviewed, so it covers the settings that change how a run is scheduled and which App, model or effort a Stage uses, never a skill or a Delegate pick.
_Avoid_: Local config, user config, override file

**Skill manifest**:
The Target repo's record of the skills Orqadence installs for it: the Shipped skills, plus third-party skills named by their pinned source, and which of them is each Stage's Delegate skill. It belongs to the repo and is committed beside the skill files themselves, so every checkout runs the same text, and a new version of any skill reaches the repo only as a change someone reviews.
_Avoid_: Config, lockfile

**Epic**:
The beads epic handed to Orqadence. Its child Tickets are the whole scope of one run.

**Ticket**:
A beads issue, and the unit that moves through the Pipeline. Most are children of the Epic; a Ticket can also run in a Ticket run, whether it has an Epic or not. It ends as one pull request and closes only when that pull request is merged.
_Avoid_: Task, issue, story

**Ticket run**:
A run over Tickets the user names instead of an Epic: its scope is a queue they add to and take from while it runs, and it ends once every Ticket in it is merged (or none is left), after its Release when it carries the Release label. Like an Epic run it takes at most max_tickets at once, polls for merges and resumes; only one run, of either kind, is live in a Target repo.
_Avoid_: Batch, single-Ticket run

**Saved run**:
A run, an Epic run or a Ticket run, that was stopped before it ended, kept so it can go on later where it left off: each Ticket at its Stage. A Target repo keeps every Saved run until it ends; still, only one run is live at a time, so a Saved run resumes only while none is.
_Avoid_: Stopped run, history entry, paused epic

**Ticket label**:
A bd label `orqa:<name>` that the Target repo has configured, changing how its Ticket runs: the skills and guidance the Stages that write its code get, the App, model or effort of any Stage, the template its pull request is written from, and possibly an Extra review. A Ticket carries at most one Area label and any number of Modifier labels; labels that clash are put to the user before the Ticket goes on.
_Avoid_: Tag, kind

**Area label**:
The Ticket label naming the one type of work a Ticket does, such as fe, be, db, security, architecture or infra. Work that spans areas is an Epic with a Ticket per area, the Epic's description saying what each side expects of the other.
_Avoid_: Type (bd's issue type), domain

**Modifier label**:
A Ticket label that only changes which App, model or effort runs a Stage, such as codex-review, and combines with an Area label.
_Avoid_: Flag, option

**Human-merge label**:
A Ticket label configured `human_merge`, as security, db and infra ship, or the built-in `orqa:human-merge` on one Ticket: the Orchestrator never merges that Ticket's pull request, which carries the GitHub label `orqa:human-merge`, even under Agent merge.
_Avoid_: Protected label, manual label

**Release label**:
The label `orqa:release`, on an Epic or on any Ticket of a Ticket run, asking that the run end in a Release, when the Target repo has releases turned on. An Epic's run raises the minor version, a Ticket run the patch version. Unlike a Ticket label it is read on the Epic, not its Tickets, and changes how the run ends, not how a Ticket runs.
_Avoid_: Version bump label, release tag

**Brainstorm**:
Planning work with the user before it is built: one session turns the user's idea into Tickets, or into a Map when the work is big. A Map's Waypoints then get a session each, the ones that need the user one after another, the Research Waypoints alongside, until the Waypoint that writes the Epic closes.
_Avoid_: Wayfinding, grilling, planning run

**Idea**:
The beads issue a Brainstorm opens for the user's idea the moment charting starts, in progress while it is charted, so a stopped charting can be found and continued. Once charting ends it closes, naming the Map or the Tickets that came out; it never enters the Pipeline.
_Avoid_: Start ticket, brainstorm ticket

**Map**:
The beads epic a Brainstorm charts: where the work is headed, the decisions made so far, and the Waypoints still open. Its last Waypoint writes the Epic that builds what it decided.
_Avoid_: Brainstorm epic, wayfinder epic, coding epic (that is the Epic)

**Waypoint**:
One question on a Map, closed by the decision recorded on it rather than by a pull request. It never enters the Pipeline.
_Avoid_: Map ticket, Ticket, decision ticket

**Research Waypoint**:
A Waypoint answered by research alone, needing no user, so Orqadence runs it in its own session while the user works another Waypoint or is Away; the next session with the user reads what it found. Only research runs without the user; anything needing a credential or a human action is Manual work.
_Avoid_: Background Waypoint, Background Map ticket, AFK ticket (AFK is avoided for Away), unattended ticket

**Pipeline**:
The fixed sequence of Stages every Ticket passes through: Implement, Review, Debate, Fix, then a pull request, plus an Extra review when its Area label carries one.
_Avoid_: Workflow, flow

**Extra review**:
A second review an Area label adds to its Tickets, with its own skill, App, model and effort: the Review's instructions with the label's own review skill, such as a security review or infra's offline checks. As the label says, it runs after the Review in every Round, in the first Round only, or once after the last Round before the pull request, and its Findings join the Debate or go straight to the Fix. Like the Review it never changes the code, and a Round opened unreviewed skips it too.
_Avoid_: Extra Stage, label Stage, security Stage

**Round**:
One pass of Review, Debate and Fix over a Ticket, with any Extra review beside the Review. Rounds repeat until a Verdict has no fix items or the cap is reached, after which the pull request opens with any leftover Findings listed.
_Avoid_: Iteration, loop, cycle

**Stage**:
One step of the Pipeline, carried out by a fresh agent session in its own pane.
_Avoid_: Step, phase

**App**:
An agent CLI a Stage can run on, such as claude or codex, picked per Stage by the user. Each App starts its own sessions, reads the Target repo's skills, and has its own usage limits.
_Avoid_: Agent, kind, CLI, provider

**Stage result**:
The recorded outcome of a Stage, carrying its completion status and, as appropriate, Findings, a Verdict, an opened pull request, a Plan to approve, or a question the Stage needs the user to answer before it can go on. The Orchestrator uses it together with the session's state to decide whether the Stage can advance.

**Run directory**:
The Ticket's directory under `.orqadence-local/runs/`, holding its Stages' evidence: the result files, diffs and Debate transcripts, all flat text. It doubles as the Review's sandbox, so build scratch lands there too, and the last Fix's screenshots wait in its `pr/` folder to be attached; both are pruned when the pull request opens. Address PR comments puts its new captures of an orqa:fe Ticket's screens there again.
_Avoid_: Logs, workdir, artifacts

**Finding**:
One claimed problem with a Ticket's changes, raised by the Review, an Extra review or the over-engineering audit, and the unit the Debate argues over.
_Avoid_: Comment, issue, point

**PR comment**:
What a reviewer, human or bot, leaves on a Ticket's pull request once it is open: a review thread, or a finding in a review's body or a bot's summary. The Address PR comments Stage acts on them. Unlike a Finding, it comes from outside Orqadence.
_Avoid_: Finding (that is Orqadence's own Review), feedback

**Rebase**:
The Stage that brings a Ticket's open pull request back onto the default branch when it conflicts, keeping both sides' intent or asking the user. It runs outside the Pipeline, by itself when the Target repo turns it on, and otherwise on the user's command.
_Avoid_: Address, conflict fix, merge

**Address PR comments**:
The Stage that acts on a Ticket's open pull request once its checks and bots are done: it fixes the PR comments and failing checks the user approved, answers the others as won't fix, and pushes to the same pull request. It runs outside the Pipeline, after the user approves or a countdown or Away approves for them.
_Avoid_: Address (alone), Fix (that is the Pipeline's), review response

**Agent merge**:
The Target repo's switch, off by default and only with automatic Address PR comments on, that lets the Orchestrator merge a Ticket's pull request once Address PR comments' flow has finished on a quiet head: checks green, the repo's review bots done, and every PR comment fixed or answered. Anything still open is a Question, merge or park. Never for a Human-merge label; never by a Stage session.
_Avoid_: Auto-merge (GitHub's), self-merge

**No-review pull request**:
The Release's version pull request, or one whose every changed file is Markdown or a skill. It carries the GitHub label `orqa:no-review`, which the review bots are configured to skip, and under Agent merge it merges once its checks are green.
_Avoid_: Trivial PR, docs PR

**Release**:
The Stage that ends a run carrying the Release label, once every Ticket is merged: it raises the version wherever the Target repo keeps it, adds a changelog entry when the repo keeps a changelog, and opens the version pull request. When that is merged, a Question asks whether to tag the new version; yes pushes the tag, and either answer ends the run. Under Agent merge the version pull request is merged and tagged without the Question. A repo that keeps its version only in tags gets no pull request, only the Question. It runs outside the Pipeline and belongs to the run, not to a Ticket.
_Avoid_: Version bump, bump, publish

**Moderator**:
The neutral session that runs the Debate between side A and side B. It never argues a position of its own, and settles Findings the sides still dispute by an outside score.
_Avoid_: Judge, Debby

**Verdict**:
The Debate's result: every Finding marked fix or skip, with a severity and the reason. A high Finding is never debated and is always fix. Only fix items reach the Fix Stage, with any Extra review Findings that skip the Debate.
_Avoid_: Synthesis, summary, report

**Epic summary**:
The Shell's read-only page over one run, an Epic's or a Ticket run's: each Ticket's pull request, Rounds and Findings fixed, skipped and left on the pull request, then the Parked Tickets with their reasons and the Manual work not yet done, with the run's cost and time. It opens by itself once every Ticket has its pull request or is Parked, or, for a run carrying the Release label, only once its Release ends, then naming the new version; /summary opens it again, built fresh from bd, the state file and the Run directories.
_Avoid_: Report, recap

**Brainstorm summary**:
The Shell's page at the end of a Brainstorm, once the Waypoint that writes the Epics closes: each Epic in build order with its Tickets and their labels, then the Manual work not yet done. It offers to start the first Epic, but only once the docs pull request is merged and no other run is going; /summary @<map> opens it again.
_Avoid_: Map summary, Brainstorm report

**Wake**:
The Orchestrator's request for judgment about a Stage that cannot advance by rule, answered by a Judgment or, failing that, by the user through a Question.
_Avoid_: Alert, escalation

**Nudge**:
One canned prompt sent into a woken Stage's live session, at most once per session, by a Judgment or the user's answer: **write the result**, when the work is done but the result file is not, or **carry on**, when the session stopped to ask and nobody will answer, so it decides for itself.
_Avoid_: Continue (that is /continue, resuming a saved run), poke, reminder

**Judgment**:
The Orchestrator's answer to a Wake or a Plan, taken from a typed model over the evidence: for a Wake one of a fixed set of actions with a score each, for a Plan a yes or no score on each criterion (it covers the Ticket, it stays in scope, it asks the user a question). It is acted on at or above a confidence floor, a Wake's and a Plan's each kept in config.json, and shown on the Shell.
_Avoid_: LLM call, Main session

**Plan**:
What an Implement session writes before it may edit: the changes and tests it intends for its Ticket, the decisions it made with the answer taken, and an open question only when it has one. A Judgment approves it when it covers every acceptance criterion, stays in scope and asks nothing; otherwise the user reads it and answers, and the session revises it. An open question always goes to the user; while they are Away it parks its Ticket until they continue it.

**Question**:
What the Shell puts to the user when the Orchestrator cannot act alone: a Wake the Judgment was unsure about, a blocked session, a plan to approve, a Stage's own question, a pick not yet merged on a Ticket's base or a personal skill shadowing a committed one when the Ticket starts, an Extra review skill's fetch.sh that failed, Manual work its session waits on, the Review's App at its usage limit, or a confirmation. It holds only its Ticket (the Review's limit, every Ticket reaching the Review on that App until it is answered), is answered from a fixed set of options or a line of the user's own text, and is never saved: on resume it is derived again from the live session or the Stage result.
_Avoid_: Prompt, dialog, alert, form, popup

**Parked**:
A Ticket taken out of the Pipeline to wait for the user, after a Wake that a Judgment or the user settled as park, or after its Stage, its start or a failed fetch.sh asked a question while the user was Away. Other Tickets keep running. Parked from Rebase or Address PR comments, it keeps its pull request, still polled for its merge, and /continue takes it back to that Stage, never to the Pipeline. A Research Waypoint whose session needs the user while they are Away is parked the same way, and the other research goes on.
_Avoid_: Stuck, paused, failed

**Manual work**:
Something a code-editing Stage or a Brainstorm session needs done that it cannot do itself, above all anything that needs a credential, filed for the user as written steps plus, as the task needs, a wizard to run or a prompt for a separate session outside Orqadence. When the session cannot go on without it, its Ticket or Waypoint waits on a Question until the user marks it done; otherwise the work goes on, the pull request lists it, and it stays open until the user marks it done. Unlike a Question, it asks for an action, not an answer.
_Avoid_: Manual step, human task, hand-off

**Away**:
What the user declares in the Shell when nobody will answer for a while, such as overnight. A Stage's question, a Plan's open question, one at a Ticket's start or one on a failed fetch.sh, then parks its Ticket instead of waiting, and is put to the user when they continue that Ticket. Nothing is pushed to the phone, and the Shell never goes On call. Nothing else changes: Judgments still answer what they can.
_Avoid_: AFK, offline, unattended mode

**On call**:
What the Shell turns on by itself when a Question has waited five minutes unanswered and the user is not Away: they are away from the desk but their phone reaches them. That Question, every Question after it and the end of the run are pushed to the phone at once, and the user answers in the Shell. Answering any Question ends it, from wherever it was typed. Unlike Away, nothing parks for it.
_Avoid_: Away by phone, remote mode, paged

**Limited**:
A Ticket held because the App its Stage runs on hit its provider's usage limit. The limit holds every Stage on that App, whichever Ticket it belongs to: a short one resumes at the reset; a long one (a reset more than a day away) ends the run with every session saved, and /continue resumes each where it stopped. A limit on the Review is put to the user once, through a Question whose answer stands for every Ticket until the reset, except a Ticket carrying orqa:codex-review, which waits it out like any Stage; a limit on one Debate side settles the Findings without that side. Unlike Parked, nothing in the Ticket's own work went wrong.
_Avoid_: Rate-limited, cooling down, throttled

**Ticket tab**:
The herdr tab belonging to one running Ticket, holding one pane per Stage.

**Docs pass**:
graphify's LLM pass over a Target repo's docs and images, which adds what they say to the code graph Orqadence keeps current on its own. It runs only when a new major or minor version tag reaches the default branch and the user says yes to the Question, in an App's session they can watch.
_Avoid_: Rebuild, graph build, reindex

**Tools**:
The one seam every external command (herdr, bd, gh, git) goes through; a test double stands behind it so tests never start a process.
_Avoid_: Runner, exec, shell

**Orchestrator**:
The deterministic process that owns ticket state, pane placement, and stage transitions. Its only judgment calls are bounded, logged Judgments through a typed model, and it never composes text.
_Avoid_: Script, runner, daemon

**Shell**:
The full-terminal screen that `orqa` alone opens: it lists the Epics and the open Tickets with no Epic, takes slash commands, runs the Orchestrator inside its own process, and is where every event and Question appears.
_Avoid_: TUI, dashboard, Main session, attach
