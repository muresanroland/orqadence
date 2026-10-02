# The Orchestrator merges a Ticket's pull request, unless its label says a human must

ADR 0002 left every merge to a human, so an Epic paused at each dependency edge until the user looked, even for a glossary line or a version bump. With Agent merge on, the Orchestrator merges a Ticket's pull request itself once Address PR comments' flow has finished on a quiet head: checks green, every bot the repo lists in `review_bots` has reviewed, and no PR comment left unanswered. A PR comment counts as answered when it is fixed, answered as covered by another open Ticket of the run or its Epic, or answered as won't fix with the reason; a bot reading only the diff flags work a later Ticket builds (PR #78 against harness-we9.3), and deleting that work would be wrong. Anything still open, or a listed bot that has not reviewed within `bot_wait`, is a Question: merge or park. A Ticket whose label is configured `human_merge` (security, db and infra as shipped), or that carries `orqa:human-merge`, is never merged by the Orchestrator. A pull request whose every changed file is Markdown or a skill, and the Release's version pull request, is a No-review pull request: the review bots are told to skip it and it merges once its checks are green.

## Considered Options

- **Keep every merge human (ADR 0002 as it was).** Rejected: the bots and Address PR comments already review and fix what a human would skim, and the wait is the run's slowest step.
- **A Stage session merges, following a merge skill.** Rejected: a session can't see the gates (labels, quiet head, open items, bot reviews), so a session never merges; only the Orchestrator does, in code.
- **GitHub auto-merge.** Rejected: it knows branch protection, not Orqadence's labels, bots or open PR comments.
- **Agent merge without automatic Address PR comments.** Rejected: nothing would answer the bots, so every reviewed pull request would end in the Question. The switch requires it.
- **An agent judging which pull requests are small enough to skip review.** Rejected for a fixed path check the agent cannot talk past; a version bump done as a Ticket touches the build file and is reviewed.

## Consequences

- Agent merge is a team setting, off by default; turning it on also asks for `review_bots` and `bot_wait`.
- The Orchestrator merges with the repo's own method (squash, else rebase, else a merge commit). GitHub refusing it, as branch protection may, parks the Ticket with gh's reason. A human's changes-requested review blocks the merge.
- Under Away, TypeSafe answers the merge Question; unsure, or TypeSafe off, parks the Ticket.
- With Agent merge on, the Release's version pull request is merged and its tag pushed without the tag Question.
- `orqa init` writes the No-review exclusion into the Target repo's `.coderabbit.yaml` and Greptile's config.
- Dependent Tickets still wait for the merge (ADR 0002); they just wait less.
- Amended (harness-yfx): a Ticket carrying the built-in `orqa:no-review` makes its pull request No-review too, its Review, Extra review and Debate skipped. Human-merge and No-review are independent, so a pull request may carry both: nothing reviews it and a human merges it. `orqa init` asks for `review_bots`, and writes their exclusion, whether Agent merge is on or not.
