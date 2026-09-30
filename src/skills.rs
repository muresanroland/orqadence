//! The skills Orqadence ships: one Stage skill per Stage, plus orqa-create-pr. 'orqa init' copies them into a Target repo. A new skill
//! is one more line here.

use std::fs;
use std::io;
use std::iter;
use std::path::Path;

/// (name, body of skills/<name>/SKILL.md), sorted by name. Every skill
/// Orqadence installs is named orqa-<name>, so that none shares a name with
/// a skill of the user's own.
pub(crate) const SKILLS: &[(&str, &str)] = &[
    (CREATE_PR, include_str!("../skills/orqa-create-pr/SKILL.md")),
    (
        "orqa-stage-address",
        include_str!("../skills/orqa-stage-address/SKILL.md"),
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
        "orqa-stage-review",
        include_str!("../skills/orqa-stage-review/SKILL.md"),
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
