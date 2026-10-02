//! The graphify commands Orqadence runs: a new worktree's graph, and
//! graphify in the checkout (harness-bsg.13), where the Shell's background
//! pass at open and every 24h refreshes the checkout's code graph, upgrades graphify
//! and finds a new vX.Y tag for the Docs pass. The last X.Y handled, built
//! or declined, is one value in .orqadence-local/graphify-docs-pass, never
//! in state.json, which a finished run clears.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::orchestrator::app::{self, ROWS};
use crate::orchestrator::state::{local_dir, LOCAL};
use crate::tools::{RunError, Tools};
use crate::update::semver;

/// How long graphify update . may run (it takes seconds) before it is
/// killed, so a stalled one cannot hold up the Ticket.
const UPDATE_LIMIT: Duration = Duration::from_secs(300);

/// A new worktree's own code graph, while the "graphify" switch is on: the
/// checkout's graphify-out/ copied in when it has a graph, then graphify
/// update . there, which keeps the Docs pass's nodes whose files exist.
pub(crate) fn prepare_worktree(
    tools: &dyn Tools,
    checkout: &Path,
    worktree: &Path,
) -> Result<(), RunError> {
    if !app::graphify(checkout) {
        return Ok(());
    }
    let out = checkout.join("graphify-out");
    if out.join("graph.json").exists() {
        let from = out.display().to_string();
        let to = worktree.join("graphify-out").display().to_string();
        tools.run(worktree, &["cp", "-R", &from, &to])?;
    }
    tools.run_within(worktree, &["graphify", "update", "."], UPDATE_LIMIT)?;
    Ok(())
}

/// The file under LOCAL holding the last X.Y handled, as "1.3".
const HANDLED: &str = "graphify-docs-pass";

/// A pass's failed steps, one RECENT line each, and the new tag it found.
pub(crate) type Pass = (Vec<String>, Option<String>);

/// The last X.Y handled; none recorded, or one that cannot be read, is None.
pub(crate) fn handled(repo: &Path) -> Option<(u64, u64)> {
    let raw = fs::read_to_string(repo.join(LOCAL).join(HANDLED)).ok()?;
    let (x, y) = raw.trim().split_once('.')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// Records tag's X.Y as handled; a tag semver cannot read refuses.
#[allow(dead_code)] // the Docs pass Question (harness-u4f.5) writes it
pub(crate) fn set_handled(repo: &Path, tag: &str) -> Result<(), String> {
    let (x, y, _) = semver(tag).ok_or_else(|| format!("unreadable tag {tag:?}"))?;
    local_dir(repo)
        .and_then(|dir| fs::write(dir.join(HANDLED), format!("{x}.{y}")))
        .map_err(|err| format!("{HANDLED}: {err}"))
}

/// The highest vX.Y.Z of git tag's output; a tag semver cannot read
/// (v2.0.0-rc1) is ignored.
fn latest(tags: &str) -> Option<String> {
    tags.lines()
        .map(str::trim)
        .filter_map(|tag| Some((semver(tag)?, tag)))
        .max()
        .map(|(_, tag)| tag.to_string())
}

/// Whether tag is new: its X or Y differs from the X.Y handled, or none is.
/// A patch-only tag never counts, and no tag is never new.
fn is_new(handled: Option<(u64, u64)>, tag: Option<&str>) -> bool {
    match tag.and_then(semver) {
        Some((x, y, _)) => handled != Some((x, y)),
        None => false,
    }
}

/// One pass in the checkout: the code graph refreshed, graphify upgraded
/// and its skill installed for claude and codex as the rows run them, then
/// the highest tag merged into origin's default branch. A failed step is
/// one line, and the pass goes on to the next.
pub(crate) fn pass(tools: &dyn Tools, repo: &Path) -> Pass {
    let mut failed = Vec::new();
    let mut run = |argv: &[&str]| {
        tools
            .run(repo, argv)
            .map_err(|err| failed.push(format!("graphify check failed: {err}")))
    };
    let _ = run(&["graphify", "update", "."]);
    // the installer init used: uv, or pipx with no uv on PATH
    let installer = match tools.run(repo, &["which", "uv"]) {
        Ok(_) => ["uv", "tool", "upgrade", "graphifyy"].as_slice(),
        Err(_) => ["pipx", "upgrade", "graphifyy"].as_slice(),
    };
    if run(installer).is_ok() {
        for platform in platforms(repo) {
            let _ = run(&["graphify", "install", "--platform", platform]);
        }
    }
    // offline, the local tags still count
    let _ = run(&["git", "fetch", "origin", "--tags"]);
    let merged = ["git", "tag", "--merged", "origin/HEAD", "--list", "v*"];
    let tags = match tools.run(repo, &merged) {
        Ok(tags) => Ok(tags),
        // origin/HEAD unset: set it once from the remote and read again
        Err(_) => {
            run(&["git", "remote", "set-head", "origin", "--auto"]).and_then(|_| run(&merged))
        }
    };
    let tag = tags.ok().and_then(|tags| latest(&tags));
    let new = is_new(handled(repo), tag.as_deref())
        .then_some(tag)
        .flatten();
    (failed, new)
}

/// claude and codex, each that a row of config.json runs on: graphify
/// installs its skill for those two.
fn platforms(repo: &Path) -> BTreeSet<&'static str> {
    let doc = app::read(repo).map(|(_, doc)| doc).unwrap_or_default();
    ROWS.iter()
        .filter_map(|key| app::field(&doc, key, "app").ok())
        .filter_map(|name| ["claude", "codex"].into_iter().find(|p| *p == name))
        .collect()
}

#[cfg(test)]
mod graphify_test;
