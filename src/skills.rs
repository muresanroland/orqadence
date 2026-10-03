//! The skills Orqadence ships: one Stage skill per Stage, plus orqa-create-pr,
//! orqa-infra-review with its fetch.sh, orqa-address-pr-comments with its
//! scripts/threads.sh, orqa-manual-work with its template.sh, and the two
//! skills every Brainstorm skill loads, orqa-brainstorm-grilling and
//! orqa-brainstorm-domain-modeling with its ADR and CONTEXT formats. 'orqa init'
//! copies them into a Target repo. A new skill is one more line here, and
//! one more in EXTRA_FILES for each file beside its SKILL.md.

use std::fs;
use std::io;
use std::iter;
use std::path::Path;

/// (name, body of skills/<name>/SKILL.md), sorted by name. Every skill
/// Orqadence installs is named orqa-<name>, so that none shares a name with
/// a skill of the user's own.
pub(crate) const SKILLS: &[(&str, &str)] = &[
    (
        "orqa-address-pr-comments",
        include_str!("../skills/orqa-address-pr-comments/SKILL.md"),
    ),
    (
        "orqa-brainstorm-domain-modeling",
        include_str!("../skills/orqa-brainstorm-domain-modeling/SKILL.md"),
    ),
    (
        "orqa-brainstorm-grilling",
        include_str!("../skills/orqa-brainstorm-grilling/SKILL.md"),
    ),
    (CREATE_PR, include_str!("../skills/orqa-create-pr/SKILL.md")),
    (
        "orqa-infra-review",
        include_str!("../skills/orqa-infra-review/SKILL.md"),
    ),
    (
        "orqa-manual-work",
        include_str!("../skills/orqa-manual-work/SKILL.md"),
    ),
    (
        "orqa-stage-address-pr-comments",
        include_str!("../skills/orqa-stage-address-pr-comments/SKILL.md"),
    ),
    (
        "orqa-stage-fix",
        include_str!("../skills/orqa-stage-fix/SKILL.md"),
    ),
    (
        "orqa-stage-implement",
        include_str!("../skills/orqa-stage-implement/SKILL.md"),
    ),
    (
        "orqa-stage-moderate",
        include_str!("../skills/orqa-stage-moderate/SKILL.md"),
    ),
    (
        "orqa-stage-rebase",
        include_str!("../skills/orqa-stage-rebase/SKILL.md"),
    ),
    (
        "orqa-stage-release",
        include_str!("../skills/orqa-stage-release/SKILL.md"),
    ),
    (
        "orqa-stage-review",
        include_str!("../skills/orqa-stage-review/SKILL.md"),
    ),
];

/// (skill, file, body): the files a Shipped skill carries beside its
/// SKILL.md, by their path in its folder. init writes the scripts among
/// them executable.
pub(crate) const EXTRA_FILES: &[(&str, &str, &str)] = &[
    (
        "orqa-address-pr-comments",
        "scripts/threads.sh",
        include_str!("../skills/orqa-address-pr-comments/scripts/threads.sh"),
    ),
    (
        "orqa-brainstorm-domain-modeling",
        "ADR-FORMAT.md",
        include_str!("../skills/orqa-brainstorm-domain-modeling/ADR-FORMAT.md"),
    ),
    (
        "orqa-brainstorm-domain-modeling",
        "CONTEXT-FORMAT.md",
        include_str!("../skills/orqa-brainstorm-domain-modeling/CONTEXT-FORMAT.md"),
    ),
    (
        "orqa-infra-review",
        "fetch.sh",
        include_str!("../skills/orqa-infra-review/fetch.sh"),
    ),
    (
        "orqa-manual-work",
        "template.sh",
        include_str!("../skills/orqa-manual-work/template.sh"),
    ),
];

/// The create-pr skill the Fix Stage runs.
pub(crate) const CREATE_PR: &str = "orqa-create-pr";

/// A Stage skill's installed copy under `repo`, a Ticket's worktree (the
/// one its Stage runs) or the checkout: Orqadence's in .orqadence/skills,
/// else one of the repo's own in .agents/skills. Only an absent copy falls
/// through: one that cannot be read is said. A base that orqa init has
/// not yet renamed on has it by its old name, without orqa-.
// ponytail: the old name is for bases not merged since the rename; drop it
// once Target repos have moved.
pub(crate) fn stage_skill(repo: &Path, name: &str) -> Option<Result<String, String>> {
    let names = iter::once(name).chain(name.strip_prefix(manifest::PREFIX));
    names.into_iter().find_map(|name| {
        [manifest::FILES, ".agents/skills"].iter().find_map(|dir| {
            let path = repo.join(dir).join(name).join("SKILL.md");
            match fs::read_to_string(&path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => None,
                read => Some(read.map_err(|err| format!("cannot read {}: {err}", path.display()))),
            }
        })
    })
}

pub(crate) mod manifest;

#[cfg(test)]
mod manifest_test;
#[cfg(test)]
mod skills_test;
