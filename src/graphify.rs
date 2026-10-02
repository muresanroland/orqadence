//! The graphify commands Orqadence runs.

use std::path::Path;

use crate::orchestrator::app;
use crate::tools::{RunError, Tools};

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
    tools.run(worktree, &["graphify", "update", "."])?;
    Ok(())
}

#[cfg(test)]
mod graphify_test;
