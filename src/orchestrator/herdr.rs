//! What the Orchestrator reads from herdr and how it names things there.

use std::path::Path;
use std::time::Instant;

use serde::Deserialize;
#[cfg(test)]
use serde::Serialize;

use super::stage::Orchestrator;
use crate::tools::{RunError, Tools};

#[derive(Clone, Debug, Default, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(default)]
pub(crate) struct TabInfo {
    pub(crate) tab_id: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(default)]
pub(crate) struct PaneInfo {
    pub(crate) pane_id: String,
    pub(crate) tab_id: String,
}

/// One pane's place in its tab, in terminal cells.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct PaneRect {
    pub(crate) pane_id: String,
    pub(crate) rect: Rect,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Rect {
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// Every herdr response shape the Orchestrator reads.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct HerdrReply {
    pub(crate) result: HerdrResult,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct HerdrResult {
    pub(crate) tab: TabInfo,
    pub(crate) root_pane: PaneInfo,
    pub(crate) pane: PaneInfo,
    pub(crate) tabs: Vec<TabInfo>,
    pub(crate) panes: Vec<PaneInfo>,
    pub(crate) layout: Layout,
    pub(crate) agent: Agent,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Layout {
    pub(crate) panes: Vec<PaneRect>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Agent {
    #[serde(rename = "agent_status")]
    pub(crate) status: String,
    /// The pane the agent lives in.
    pub(crate) pane_id: String,
    /// The session id herdr's integration reports; null or missing without
    /// one.
    pub(crate) agent_session: Option<AgentSession>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AgentSession {
    pub(crate) value: String,
}

impl Orchestrator {
    /// A failed command's RunError comes back whole, so a caller can read
    /// herdr's stderr (agent_pane_busy, agent_not_ready, ...).
    pub(crate) fn herdr(&self, args: &[&str]) -> Result<HerdrReply, RunError> {
        herdr(&*self.cfg.tools, &self.cfg.repo, args)
    }

    /// Names a pane the way every event line does: "(pane 2-1)".
    pub(crate) fn locate(&self, pane_id: &str) -> String {
        locate(
            &*self.cfg.tools,
            &self.cfg.repo,
            &self.cfg.workspace,
            pane_id,
        )
    }

    /// The last `lines` lines of the agent's output, its target a pane id
    /// or an agent's name; empty when herdr cannot say.
    pub(crate) fn tail(&self, target: &str, lines: usize) -> String {
        let lines = lines.to_string();
        let read = [
            "herdr",
            "agent",
            "read",
            target,
            "--source",
            "recent-unwrapped",
            "--lines",
            &lines,
        ];
        self.cfg
            .tools
            .run(&self.cfg.repo, &read)
            .unwrap_or_default()
    }

    /// Gives a new pane in `tab`, split out of its roomiest pane, or in a new
    /// tab labelled `label` when `tab` is empty or has no panes left: a
    /// Ticket's tab, a research-<map> tab. `never` is a pane it must never
    /// split: a tab holding only that pane counts as having none left.
    /// `placement` is the --cwd, --no-focus, ... the pane starts with.
    /// Returns the tab and the pane.
    pub(crate) fn place_pane(
        &self,
        tab: &str,
        label: &str,
        placement: &[&str],
        never: &str,
    ) -> Result<(String, String), RunError> {
        let mut in_tab = Vec::new();
        if !tab.is_empty() {
            if let Ok(panes) = self.herdr(&["pane", "list", "--workspace", &self.cfg.workspace]) {
                in_tab.extend(
                    panes
                        .result
                        .panes
                        .into_iter()
                        .filter(|p| p.tab_id == tab && p.pane_id != never)
                        .map(|p| p.pane_id),
                );
            }
        }
        let Some(last) = in_tab.last() else {
            let mut argv = vec![
                "tab",
                "create",
                "--workspace",
                &self.cfg.workspace,
                "--label",
                label,
            ];
            argv.extend_from_slice(placement);
            let reply = self.herdr(&argv)?;
            return Ok((reply.result.tab.tab_id, reply.result.root_pane.pane_id));
        };
        let (mut target, mut direction) = (last.clone(), "right".to_string());
        if let Ok(mut layout) = self.herdr(&["pane", "layout", "--pane", &in_tab[0]]) {
            layout.result.layout.panes.retain(|p| p.pane_id != never);
            let (p, d) = split_target(&layout.result.layout.panes);
            if !p.is_empty() {
                (target, direction) = (p, d);
            }
        }
        let mut argv = vec!["pane", "split", &target, "--direction", &direction];
        argv.extend_from_slice(placement);
        let reply = self.herdr(&argv)?;
        Ok((tab.to_string(), reply.result.pane.pane_id))
    }

    /// Gives a Brainstorm pane in the Shell's own tab. `shell` is the Shell's
    /// pane (HERDR_PANE_ID); its tab is read live, since the pane can have
    /// moved. `previous`, the last Brainstorm pane, is replaced when it is
    /// still in that tab: split, then closed. Otherwise the Shell's pane is
    /// split, once. The panes the user opened there are never split.
    #[allow(dead_code)] // the Brainstorm's charting pane calls it (harness-1n3.8)
    pub(crate) fn place_beside_shell(
        &self,
        shell: &str,
        previous: &str,
        placement: &[&str],
    ) -> Result<String, RunError> {
        let tab = self.herdr(&["pane", "get", shell])?.result.pane.tab_id;
        // Only pane_not_found means the previous pane is gone; another
        // failed lookup must not split the Shell a second time.
        let beside = !previous.is_empty()
            && previous != shell
            && match self.herdr(&["pane", "get", previous]) {
                Ok(r) => r.result.pane.tab_id == tab,
                Err(err) if err.to_string().contains("pane_not_found") => false,
                Err(err) => return Err(err),
            };
        if beside {
            let mut argv = vec!["pane", "split", previous, "--direction", "right"];
            argv.extend_from_slice(placement);
            let pane = self.herdr(&argv)?.result.pane.pane_id;
            // A previous pane left open would be untracked: take the new
            // one back so the caller still holds a valid `previous`.
            match self.herdr(&["pane", "close", previous]) {
                Err(err) if !err.to_string().contains("pane_not_found") => {
                    let _ = self.herdr(&["pane", "close", &pane]);
                    return Err(err);
                }
                _ => return Ok(pane),
            }
        }
        let direction = self
            .herdr(&["pane", "layout", "--pane", shell])
            .ok()
            .and_then(|r| {
                r.result
                    .layout
                    .panes
                    .into_iter()
                    .find(|p| p.pane_id == shell)
            })
            .map_or("right", |p| cut(&p.rect));
        let mut argv = vec![
            "pane",
            "split",
            shell,
            "--direction",
            direction,
            "--ratio",
            SHELL_RATIO,
        ];
        argv.extend_from_slice(placement);
        Ok(self.herdr(&argv)?.result.pane.pane_id)
    }

    /// The herdr lifecycle state of the agent in a pane; None when no agent
    /// lives there any more.
    pub(crate) fn agent_status(&self, pane_id: &str) -> Option<String> {
        self.herdr(&["agent", "get", pane_id])
            .ok()
            .map(|reply| reply.result.agent.status)
    }
}

/// herdr `args` run in dir, its reply read; a failed command's RunError
/// comes back whole.
pub(crate) fn herdr(tools: &dyn Tools, dir: &Path, args: &[&str]) -> Result<HerdrReply, RunError> {
    let mut argv = vec!["herdr"];
    argv.extend_from_slice(args);
    let out = tools.run(dir, &argv)?;
    serde_json::from_str(&out).map_err(|err| RunError {
        command: format!("herdr {}", args[0]),
        status: "unreadable reply".to_string(),
        stderr: err.to_string(),
        stdout: String::new(),
    })
}

/// herdr agent start's `argv` run in dir, giving a pane that has just been
/// created the moment it needs to get a shell: until it has one herdr
/// refuses with agent_pane_busy, which is not the pane being unusable.
/// `give_up` bounds that patience; `sleep` waits a tick between tries, and
/// its false ends it.
pub(crate) fn start_agent(
    tools: &dyn Tools,
    dir: &Path,
    argv: &[&str],
    give_up: Instant,
    sleep: impl Fn() -> bool,
) -> Result<(), RunError> {
    loop {
        let err = match herdr(tools, dir, argv) {
            Ok(_) => return Ok(()),
            Err(err) => err,
        };
        if !err.to_string().contains("agent_pane_busy") || Instant::now() > give_up || !sleep() {
            return Err(err);
        }
    }
}

/// A pane of `workspace` as event lines name it: "(pane 2-1)".
pub(crate) fn locate(tools: &dyn Tools, dir: &Path, workspace: &str, pane_id: &str) -> String {
    let tabs = herdr(tools, dir, &["tab", "list", "--workspace", workspace]);
    let panes = herdr(tools, dir, &["pane", "list", "--workspace", workspace]);
    let at = match (tabs, panes) {
        (Ok(tabs), Ok(panes)) => location(&tabs.result.tabs, &panes.result.panes, pane_id),
        _ => "?".to_string(),
    };
    format!("(pane {at})")
}

/// The share of its pane the Shell keeps when a Brainstorm pane is split out
/// of it.
#[allow(dead_code)] // with place_beside_shell
const SHELL_RATIO: &str = "0.6";

/// A terminal cell is about twice as tall as it is wide, so a pane of equal
/// rows and columns is a tall sliver on screen, not a square.
const CELL_ASPECT: usize = 2;

/// Picks which pane a new Stage pane is split out of, and which way to cut
/// it: the roomiest pane, along its longer side. Splitting the same pane every
/// time halves it again and again, and four Stage panes in a tab that was only
/// ever cut one way are four slivers too narrow for an agent to draw in.
pub(crate) fn split_target(panes: &[PaneRect]) -> (String, String) {
    let mut best = 0;
    let (mut pane, mut direction) = (String::new(), String::new());
    for p in panes {
        let area = p.rect.width * p.rect.height;
        if area <= best {
            continue;
        }
        best = area;
        pane = p.pane_id.clone();
        direction = cut(&p.rect).to_string();
    }
    (pane, direction)
}

/// Which way to split a pane: along its longer side on screen.
fn cut(rect: &Rect) -> &'static str {
    if rect.width > CELL_ASPECT * rect.height {
        "right"
    } else {
        "down"
    }
}

/// Names a pane as <tab>-<pane> by its position in herdr's tab and pane
/// lists, because herdr ids are opaque and never reused.
pub(crate) fn location(tabs: &[TabInfo], panes: &[PaneInfo], pane_id: &str) -> String {
    let tab_id = panes
        .iter()
        .rev()
        .find(|p| p.pane_id == pane_id)
        .map_or("", |p| p.tab_id.as_str());
    for (t, tab) in tabs.iter().enumerate() {
        if tab.tab_id != tab_id {
            continue;
        }
        let mut n = 0;
        for p in panes {
            if p.tab_id == tab_id {
                n += 1;
            }
            if p.pane_id == pane_id {
                return format!("{}-{n}", t + 1);
            }
        }
    }
    "?".to_string()
}

/// Builds a herdr agent name: [a-z][a-z0-9_-]{0,31}, unique per live agent.
/// The tail of the ticket id is its most distinctive part, so that is what
/// survives truncation.
pub(crate) fn agent_name(ticket: &str, stage: &str) -> String {
    let mut name = String::new();
    let mut in_run = false; // a run of other characters becomes one dash
    for c in format!("{ticket}-{stage}").to_lowercase().chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
        if ok {
            name.push(c);
        } else if !in_run {
            name.push('-');
        }
        in_run = !ok;
    }
    if name.len() > 30 {
        name = name[name.len() - 30..].to_string();
    }
    format!("h-{}", name.trim_start_matches(['-', '_']))
}
