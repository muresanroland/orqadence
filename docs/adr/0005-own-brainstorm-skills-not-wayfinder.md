# The Brainstorm runs Orqadence's own skills, not wayfinder

A Brainstorm charts a Map in bd and works its Map tickets one session each, modelled on mattpocock's wayfinder and the skills it calls (grilling, domain-modeling, research, prototype). Orqadence ships its own Brainstorm skills for this, installed like the Stage skills, rather than driving wayfinder or any other third-party skill through a configurable setting. Wayfinder is user-invoked, hardcodes its labels, writes no done signal, asks the user instead of creating Tickets when no Map is needed, and fires research itself. Each of those would need an override the session might not follow: the research found none of the overrides documented, and whether a session prefers a label table over wayfinder's literal labels unverified (docs/research/brainstorm-skills.md). No other brainstorming skill surveyed (superpowers, compound-engineering, spec-kit) charts a Map at all, so a swap setting would have had nothing to swap to.

## Considered Options

- **Wayfinder by default, swappable in /config**, with renamable labels, a label table in the tracker doc, and overrides restated in every prompt and the Map's Notes. Rejected: every override is fragile, and it all serves a swap nobody can make.
- **Own skills with per-job Delegate skills** (a third-party grilling or research skill chosen per job). Delegate skills already exist for Stages, and Ticket labels pick them per job (harness-bsg.7). The Brainstorm skills own their jobs to start with. Whether a Brainstorm job takes a Delegate skill through the same mechanism is left to the Brainstorm skills' design (harness-bsg.17), not ruled out.

## Consequences

- A Target repo changes a Brainstorm by editing its installed copy of the Brainstorm skills, as it does a Stage skill.
- Each Brainstorm session's prompt is the skill body plus Inputs, the same as a Stage's, on every App. The skills write their own result files and use Orqadence's own labels.
- No agent definition files are shipped: each Brainstorm session runs in its own pane, where the Orchestrator can see it, resume it and count its cost. In-session lookups use the App's built-in subagents.
- Upstream improvements to mattpocock's skills are not picked up automatically; they are ported by hand when wanted.
