//! The graphify commands Orqadence runs: a new worktree's graph, and
//! graphify in the checkout (harness-bsg.13), where the Shell's background
//! pass at open and every 24h refreshes the checkout's code graph, upgrades graphify
//! and finds a new vX.Y tag for the Docs pass. The last X.Y handled, built
//! or declined, is one value in .orqadence-local/graphify-docs-pass, never
//! in state.json, which a finished run clears.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::orchestrator::app::{self, DOCS_PASS, ROWS};
use crate::orchestrator::herdr::{agent_name, herdr, locate, start_agent};
use crate::orchestrator::stage::SETTLE_TICKS;
use crate::orchestrator::state::{local_dir, LOCAL};
use crate::tools::{RunError, Tools};
use crate::update::semver;

/// How long graphify update . may run (it takes seconds) before it is
/// killed, so a stalled one cannot hold up the Ticket.
const UPDATE_LIMIT: Duration = Duration::from_secs(300);

/// How long a Docs pass may run before it is given up unfinished: an LLM
/// over the checkout takes minutes, tens of them on a large one, so two
/// hours bounds a stuck or blocked session with room for a slow one.
pub(crate) const DOCS_PASS_LIMIT: Duration = Duration::from_secs(2 * 60 * 60);

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

/// A pass's failed steps, one RECENT line each, and the highest tag it read.
pub(crate) type Pass = (Vec<String>, Option<String>);

/// The last X.Y handled; none recorded, or one that cannot be read, is None.
pub(crate) fn handled(repo: &Path) -> Option<(u64, u64)> {
    let raw = fs::read_to_string(repo.join(LOCAL).join(HANDLED)).ok()?;
    let (x, y) = raw.trim().split_once('.')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// Records tag's X.Y as handled; a tag semver cannot read refuses.
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
/// A patch-only tag never counts.
pub(crate) fn is_new(handled: Option<(u64, u64)>, tag: &str) -> bool {
    semver(tag).is_some_and(|(x, y, _)| handled != Some((x, y)))
}

/// One pass in the checkout: the code graph refreshed, graphify upgraded
/// and its skill installed for claude and codex as the rows run them, then
/// the highest tag merged into origin's default branch. A failed step is
/// one line, and the pass goes on to the next. While `docs_live` holds
/// true a Docs pass owns graph.json and the refresh is skipped, so a newer
/// graph.json at its end is its own.
pub(crate) fn pass(tools: &dyn Tools, repo: &Path, docs_live: &Mutex<bool>) -> Pass {
    let mut failed = Vec::new();
    let mut run = |argv: &[&str]| {
        tools
            .run(repo, argv)
            .map_err(|err| failed.push(format!("graphify check failed: {err}")))
    };
    // held through the refresh: a Docs pass setting it waits the refresh
    // out, its start then after this graph.json
    let live = docs_live.lock().unwrap();
    if !*live {
        let _ = run(&["graphify", "update", "."]);
    }
    drop(live);
    // upgraded by what installed it: uv when uv lists it, else pipx; a
    // failed listing upgrades nothing
    let from_uv = match tools.run(repo, &["which", "uv"]) {
        Ok(_) => run(&["uv", "tool", "list"]).map(|list| {
            list.lines()
                .any(|line| line.split_whitespace().next() == Some("graphifyy"))
        }),
        Err(_) => Ok(false),
    };
    let installer = match from_uv {
        Ok(true) => Some(["uv", "tool", "upgrade", "graphifyy"].as_slice()),
        Ok(false) => Some(["pipx", "upgrade", "graphifyy"].as_slice()),
        Err(()) => None,
    };
    if installer.is_some_and(|argv| run(argv).is_ok()) {
        for platform in platforms(repo) {
            let _ = run(&["graphify", "install", "--platform", platform]);
        }
    }
    // offline, the local tags still count
    let _ = run(&["git", "fetch", "origin", "--tags"]);
    // follows the remote's default branch; offline it fails as the fetch
    // did, and the local origin/HEAD still serves
    let _ = tools.run(repo, &["git", "remote", "set-head", "origin", "--auto"]);
    let tags = run(&["git", "tag", "--merged", "origin/HEAD", "--list", "v*"]);
    let tag = tags.ok().and_then(|tags| latest(&tags));
    (failed, tag)
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

/// The Docs pass on tag, a session the user can watch: the docs_pass row's
/// App in the root pane of a graphify tab of `workspace`, the checkout as
/// it is checked out its cwd, prompted with graphify's skill over `.
/// --update`. Done once the session, idle after working, has left a
/// graph.json newer than the pass: the X.Y is recorded and the tab closes.
/// Anything else, past `limit` too or after close() has cancelled it,
/// records nothing, so the next check asks again, and leaves the tab open
/// to read why. `say` takes the started line and a start's error,
/// `docs_tab` the tab once made; the pass's last RECENT line comes back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn docs_pass(
    tools: &dyn Tools,
    repo: &Path,
    workspace: &str,
    tag: &str,
    tick: Duration,
    limit: Duration,
    say: &dyn Fn(String),
    docs_tab: &Mutex<DocsTab>,
) -> String {
    let unfinished =
        format!("graphify docs pass on {tag} did not finish: asked again at the next check");
    let since = SystemTime::now();
    let deadline = Instant::now() + limit;
    let opened = |tab: &str| docs_tab.lock().unwrap().tab = Some(tab.to_string());
    let (tab, pane) = match start_docs_pass(tools, repo, workspace, tag, tick, say, &opened) {
        Ok(started) => started,
        Err(err) => {
            say(format!("graphify docs pass on {tag} did not start: {err}"));
            return unfinished;
        }
    };
    // idle before the session has worked, or settled, is not believed
    let mut worked = false;
    for ticks in 1.. {
        if Instant::now() >= deadline {
            return unfinished;
        }
        thread::sleep(tick);
        let status = herdr(tools, repo, &["agent", "get", &pane]).map(|r| r.result.agent.status);
        match status.as_deref() {
            Err(_) | Ok("") => return unfinished,
            Ok("idle" | "done") if worked || ticks > SETTLE_TICKS => break,
            Ok("working") => worked = true,
            Ok(_) => {}
        }
    }
    let graph = repo.join("graphify-out").join("graph.json");
    let built = fs::metadata(graph).and_then(|m| m.modified());
    if !worked || !built.is_ok_and(|at| at > since) {
        return unfinished;
    }
    // under the lock close() cancels with, so a pass it interrupted is
    // never recorded
    let docs = docs_tab.lock().unwrap();
    if docs.cancelled {
        return unfinished;
    }
    if let Err(err) = set_handled(repo, tag) {
        say(format!("graphify docs pass on {tag}: {err}"));
        return unfinished;
    }
    drop(docs);
    let _ = herdr(tools, repo, &["tab", "close", &tab]);
    format!("graphify docs pass done on {tag}")
}

/// A Docs pass's graphify tab once its thread has made it, and whether the
/// Shell's close() has interrupted it: shared by the Shell and the thread.
#[derive(Default)]
pub(crate) struct DocsTab {
    pub(crate) tab: Option<String>,
    pub(crate) cancelled: bool,
}

/// The graphify tab and its root pane, the session started and prompted
/// there: its tab and pane.
fn start_docs_pass(
    tools: &dyn Tools,
    repo: &Path,
    workspace: &str,
    tag: &str,
    tick: Duration,
    say: &dyn Fn(String),
    opened: &dyn Fn(&str),
) -> Result<(String, String), String> {
    let row = app::row(repo, DOCS_PASS, &[])?;
    let checkout = repo.display().to_string();
    let create = [
        "tab",
        "create",
        "--workspace",
        workspace,
        "--label",
        "graphify",
        "--cwd",
        &checkout,
        "--no-focus",
    ];
    let reply = herdr(tools, repo, &create).map_err(|err| err.to_string())?;
    let (tab, pane) = (reply.result.tab.tab_id, reply.result.root_pane.pane_id);
    opened(&tab);
    // a failed pass's agent may live on in its tab: a name of its own
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let name = agent_name("graphify", &format!("docs-{}", secs.as_secs()));
    let mut args = (row.app.worktree_args)(&checkout);
    args.extend(row.flags());
    let mut argv = vec![
        "agent",
        "start",
        &name,
        "--kind",
        row.app.name,
        "--pane",
        &pane,
        "--",
    ];
    argv.extend(args.iter().map(String::as_str));
    let give_up = Instant::now() + 6 * tick;
    start_agent(tools, repo, &argv, give_up, || {
        thread::sleep(tick);
        true
    })
    .map_err(|err| err.to_string())?;
    // the Docs pass runs on claude or codex alone (runs_on): claude's
    // mention is in words, so its slash command
    let mention = match row.app.mention {
        "" => "/",
        mention => mention,
    };
    let prompt = format!("{mention}graphify . --update");
    herdr(tools, repo, &["agent", "prompt", &pane, &prompt]).map_err(|err| err.to_string())?;
    let at = locate(tools, repo, workspace, &pane);
    say(format!(
        "graphify docs pass started on {tag}: {} {at}",
        row.said()
    ));
    Ok((tab, pane))
}

#[cfg(test)]
mod graphify_test;
