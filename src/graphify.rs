//! The graphify commands Orqadence runs.

use std::path::Path;
use std::time::Duration;

use crate::orchestrator::app;
use crate::tools::{RunError, Tools};

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

#[cfg(test)]
mod graphify_test;
