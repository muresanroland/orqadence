//! The Brainstorm's driver: its session in a pane split from the Shell's,
//! watched on a thread of its own (ADR 0003) until its result file checks
//! out or its pane is gone. The Shell owns it, so it runs beside any run;
//! its lines go through the Shell's Event channel under the Idea's id, where
//! a Ticket's goes, and into orchestrator.log.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use super::{brainstorms, is_map, Brainstorm, Phase, EPIC, MAP};
use crate::orchestrator::app::{self, BRAINSTORM};
use crate::orchestrator::herdr::{self, agent_name, herdr, locate, place_beside_shell};
use crate::orchestrator::result::{
    read_stage_result, stage_prompt, ResultRequirements, StageResult,
};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{append_log, plural, Config, Event};
use crate::orchestrator::state::Session;
use crate::orchestrator::trust::await_trust;
use crate::shell::bd_list;
use crate::skills::manifest::Manifest;
use crate::skills::stage_skill;
use crate::tools::{RunError, Tools};

/// One Brainstorm's session as the driver runs it.
struct Driver<'a> {
    /// The Shell's Config, its events the Shell's own channel.
    cfg: &'a Config,
    /// Where each change of the Brainstorm goes back to the Shell.
    saved: &'a Sender<Brainstorm>,
    /// The Shell is closing: the session's pane stays running in herdr.
    stop: &'a AtomicBool,
    /// The Brainstorm charted, as last saved.
    b: Brainstorm,
}

/// Charts the Idea of `b`: brainstorm-chart, on the brainstorm_chart row,
/// in a pane split from the Shell's pane `shell`, cwd the Brainstorm's
/// worktree. A saved pane still alive is watched again; else a saved
/// session is resumed by id in a fresh pane, and only when that fails does
/// brainstorm-chart start fresh. Done only once its result file checks
/// out: idle never counts, and nothing is asked. Its pane gone stops the
/// Brainstorm, saved; `stop` leaves the session running.
pub(crate) fn chart(
    cfg: &Config,
    shell: &str,
    b: Brainstorm,
    saved: &Sender<Brainstorm>,
    stop: &AtomicBool,
) {
    let mut d = Driver {
        cfg,
        saved,
        stop,
        b,
    };
    let session = d.b.session.clone().filter(|s| !s.id.is_empty());
    let started = match (d.watched_again(), session) {
        (Err(err), _) => {
            let text = format!("charting not resumed: herdr did not answer, try again: {err}");
            return d.say(&text);
        }
        (Ok(true), _) => Ok(d.result_file()),
        (Ok(false), Some(session)) => match d.resume(shell, &session) {
            Err(why) if !why.is_empty() => {
                d.say(&format!("charting not resumed: {why}, starting it fresh"));
                d.start(shell)
            }
            resumed => resumed,
        },
        (Ok(false), None) => d.start(shell),
    };
    match started {
        Ok(result) => d.watch(&result),
        Err(why) if why.is_empty() => {} // stopped
        Err(why) => d.say(&format!("charting not started: {why}")),
    }
}

impl Driver<'_> {
    /// A RECENT line under the Idea's id, and the log's.
    fn say(&self, text: &str) {
        let time = chrono::Local::now();
        append_log(&self.cfg.repo, time, &self.b.idea, text);
        let _ = self.cfg.events.send(Event {
            time,
            ticket: Some(self.b.idea.clone()),
            text: text.to_string(),
            panel: true,
            ask: None,
            offer: Vec::new(),
            notice: None,
        });
    }

    /// The Brainstorm's state file, and the Shell's copy.
    fn save(&self) {
        if let Err(err) = self.b.save(&self.cfg.repo) {
            self.say(&format!("Brainstorm state not saved: {err}"));
        }
        let _ = self.saved.send(self.b.clone());
    }

    /// Its pane closed by the user: the Brainstorm stopped, saved.
    fn gone(&mut self) {
        self.b.pane.clear();
        self.save();
        self.say("charting stopped: its pane is gone; Brainstorm saved");
    }

    /// Its session dead in a pane still open: the Brainstorm stopped,
    /// saved, its pane kept so a resume replaces it.
    fn died(&self) {
        self.save();
        self.say("charting stopped: its session is gone; Brainstorm saved");
    }

    /// One tick. False once the Shell is closing, which it checks every
    /// 50ms so close() joins this thread promptly.
    fn sleep(&self) -> bool {
        let end = Instant::now() + self.cfg.tick;
        while !self.stop.load(Ordering::SeqCst) {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            thread::sleep(left.min(Duration::from_millis(50)));
        }
        false
    }

    /// Whether herdr still names its agent in its saved pane: the session
    /// is watched again, said so. Only agent_not_found says it is gone;
    /// another failure is herdr's, so a live session is never replaced.
    fn watched_again(&self) -> Result<bool, RunError> {
        if self.b.pane.is_empty() {
            return Ok(false);
        }
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let name = agent_name(&self.b.idea, "chart");
        let alive = match herdr(tools, repo, &["agent", "get", &name]) {
            Ok(reply) => reply.result.agent.pane_id == self.b.pane,
            Err(err) if herdr::agent_gone(&err) => false,
            Err(err) => return Err(err),
        };
        if alive {
            let at = locate(tools, repo, &self.cfg.workspace, &self.b.pane);
            self.say(&format!("charting watched again {at}"));
        }
        Ok(alive)
    }

    /// Its result file, in the Brainstorm's folder.
    fn result_file(&self) -> PathBuf {
        brainstorms(&self.cfg.repo)
            .join(&self.b.idea)
            .join("chart.md")
    }

    /// A pane beside the Shell's, cwd the worktree, saved as the
    /// Brainstorm's with `session`; the folder trusted, then the agent
    /// started there with `args`: where it is. Why not, or empty once
    /// stopped.
    fn launch(
        &mut self,
        shell: &str,
        row: &app::Row,
        session: Session,
        args: &[String],
    ) -> Result<String, String> {
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let worktree = PathBuf::from(&self.b.worktree);
        let tree = worktree.display().to_string();
        let pane = place_beside_shell(tools, repo, shell, &self.b.pane, &["--cwd", &tree])
            .map_err(|err| format!("got no pane: {err}"))?;
        self.b.pane = pane.clone();
        self.b.session = Some(session);
        self.save();
        let at = locate(tools, repo, &self.cfg.workspace, &pane);
        // closed during the wait, the pane stops the Brainstorm
        let sleep = || pane_alive(tools, repo, &pane) && self.sleep();
        let trusted = await_trust(
            row.app,
            &self.cfg.home,
            &worktree,
            repo,
            &at,
            |t| self.say(t),
            sleep,
        );
        if trusted.is_err() {
            if !self.stop.load(Ordering::SeqCst) {
                self.gone();
            }
            return Err(String::new());
        }
        let name = agent_name(&self.b.idea, "chart");
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
        let give_up = Instant::now() + 6 * self.cfg.tick;
        herdr::start_agent(tools, repo, &argv, give_up, || self.sleep())
            .map_err(|err| format!("session did not start: {err}"))?;
        Ok(at)
    }

    /// The session's own args: its folder's and the row's model and effort.
    fn args(&self, row: &app::Row) -> Vec<String> {
        let dir = brainstorms(&self.cfg.repo).join(&self.b.idea);
        let mut args = (row.app.worktree_args)(&dir.display().to_string());
        args.extend(row.flags());
        args
    }

    /// The saved `session` resumed by id in a fresh pane and told to
    /// continue, only while the brainstorm_chart row's App is unchanged:
    /// its result file's path. Why not, or empty once stopped.
    fn resume(&mut self, shell: &str, session: &Session) -> Result<PathBuf, String> {
        let row = app::row(&self.cfg.repo, BRAINSTORM[0], &[])?;
        if row.app.name != session.app {
            return Err(format!("its App is now {}", row.app.name));
        }
        let mut args = row.resume(&session.id);
        args.extend(self.args(&row));
        let at = self.launch(shell, &row, session.clone(), &args)?;
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        herdr(tools, repo, &["agent", "prompt", &self.b.pane, "continue"])
            .map_err(|err| format!("never took continue: {err}"))?;
        self.say(&format!("charting resumed: {} {at}", row.said()));
        Ok(self.result_file())
    }

    /// The session started in its pane and prompted: its result file's
    /// path. Why not, or empty once stopped.
    fn start(&mut self, shell: &str) -> Result<PathBuf, String> {
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let row = app::row(repo, BRAINSTORM[0], &[])?;
        let worktree = PathBuf::from(&self.b.worktree);
        // committed on its base, as a Stage skill is (ADR 0006)
        let skill = stage_skill(&worktree, "orqa-brainstorm-chart").unwrap_or_else(|| {
            Err("has no brainstorm-chart skill on its base branch: commit and merge .orqadence/skills (orqa init writes them)".to_string())
        })?;
        let manifest = Manifest::load(repo)?;
        let have = manifest.have(repo, &worktree, &self.cfg.home, tools, row.app);
        let skill = manifest
            .fill_jobs(&skill, &have, row.app.built_in, row.app.mention)
            .0;
        let labels = ticket_labels(repo)?;
        let result = self.result_file();
        fs::create_dir_all(result.parent().unwrap()).map_err(|err| err.to_string())?;
        let _ = fs::remove_file(&result); // an earlier session's
        let session = Session {
            app: row.app.name.to_string(),
            ..Default::default()
        };
        let args = self.args(&row);
        let at = self.launch(shell, &row, session, &args)?;
        self.say(&format!("charting started: {} {at}", row.said()));
        let shown = result.display().to_string();
        let inputs = [
            ("IDEA", self.b.idea.as_str()),
            ("TICKET LABELS", &labels),
            ("RESULT FILE", &shown),
        ];
        let prompt = stage_prompt(&skill, &inputs);
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        herdr(tools, repo, &["agent", "prompt", &self.b.pane, &prompt])
            .map_err(|err| format!("never took the skill: {err}"))?;
        Ok(result)
    }

    /// Each tick: a result file that checks out ends the session; one that
    /// does not says why, once per reason, the pane and the Idea left for
    /// the session to rewrite it. Its pane gone stops the Brainstorm,
    /// saved, and so does its agent gone from it; a herdr read failing
    /// otherwise keeps watching. A new session id herdr reports is saved as
    /// the session's.
    fn watch(&mut self, result: &Path) {
        let name = agent_name(&self.b.idea, "chart");
        let mut said = String::new();
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        loop {
            // The status before the result: a result written as the
            // session exits, between the two reads, is still taken.
            let known = self.b.session.as_ref().map(|s| s.id.clone());
            let watched = herdr::watch(tools, repo, &self.b.pane, &name, known.as_deref());
            if let Some((_, Some(id))) = &watched {
                if let Some(session) = &mut self.b.session {
                    session.id = id.clone();
                }
                self.save();
            }
            if result.exists() {
                match self.done(result) {
                    Ok(()) => return,
                    Err(why) if why != said => {
                        self.say(&format!("charting result not taken: {why}"));
                        said = why;
                    }
                    Err(_) => {}
                }
            }
            // None is also a failed agent get: only a gone pane, or a pane
            // herdr says has no agent, stops it
            if watched.is_none() {
                if !pane_alive(tools, repo, &self.b.pane) {
                    return self.gone();
                }
                if agent_dead(tools, repo, &self.b.pane) {
                    return self.died();
                }
            }
            if !self.sleep() {
                return;
            }
        }
    }

    /// The result's ids checked in bd and acted on: the Idea closed with
    /// what came out, the pane closed, a Map kept with its worktree, or
    /// Tickets ending the Brainstorm, its worktree removed and its branch
    /// kept. Why not, leaving everything as it was.
    fn done(&mut self, result: &Path) -> Result<(), String> {
        let (r, why) = read_stage_result(result, ResultRequirements::default());
        if !why.is_empty() {
            return Err(why);
        }
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let issues = bd_list(repo, tools).map_err(|err| format!("bd list failed: {err}"))?;
        let reason = outcome(&issues, &r)?;
        tools
            .run(repo, &["bd", "close", &self.b.idea, "--reason", &reason])
            .map_err(|err| format!("Idea not closed: {err}"))?;
        // Only a closed or already gone pane is forgotten; another
        // failure keeps its id so the pane stays tracked.
        match herdr(tools, repo, &["pane", "close", &self.b.pane]) {
            Err(err) if !herdr::pane_gone(&err) => {
                self.say(&format!("charting pane not closed: {err}"));
            }
            _ => self.b.pane.clear(),
        }
        self.b.label_lines = r.labels;
        let line = if r.map.is_empty() {
            let remove = ["git", "worktree", "remove", "--force", &self.b.worktree];
            match tools.run(repo, &remove) {
                Ok(_) => self.b.worktree.clear(),
                Err(err) => self.say(&format!("worktree not removed: {err}")),
            }
            self.b.phase = Phase::Done;
            self.b.tickets = r.tickets;
            let pr = match r.pr.as_str() {
                "" => String::new(),
                pr => format!("; docs PR {pr}"),
            };
            format!("charting done: {reason}; Idea closed{pr}")
        } else {
            let waypoints = issues.iter().filter(|i| i.parent == r.map).count();
            self.b.phase = Phase::Map;
            self.b.map = r.map;
            format!(
                "charting done: {reason}, {}; Idea closed",
                plural(waypoints, "Waypoint")
            )
        };
        self.save();
        self.say(&line);
        Ok(())
    }
}

/// The Idea's close reason when the result checks out in `issues`: its MAP
/// an open epic labelled brainstorm:map with exactly one open
/// brainstorm:epic child, or its TICKETS open issues. Why not otherwise.
fn outcome(issues: &[BdIssue], r: &StageResult) -> Result<String, String> {
    let open = |id: &str| issues.iter().find(|i| i.id == id && i.status != "closed");
    let labelled = |i: &BdIssue, label: &str| i.labels.iter().any(|l| l == label);
    if !r.map.is_empty() {
        let map = r.map.as_str();
        if !open(map).is_some_and(is_map) {
            return Err(format!("{map} is not an open epic labelled {MAP}"));
        }
        let epics = issues
            .iter()
            .filter(|i| i.parent == map && i.status != "closed" && labelled(i, EPIC))
            .count();
        if epics != 1 {
            return Err(format!(
                "Map {map} has {epics} open {EPIC} Waypoints, not one"
            ));
        }
        return Ok(format!("Map {map}"));
    }
    if r.tickets.is_empty() {
        return Err("it names no MAP and no TICKETS".to_string());
    }
    if let Some(id) = r.tickets.iter().find(|id| open(id).is_none()) {
        return Err(format!("{id} is not an open issue"));
    }
    Ok(format!("Tickets {}", r.tickets.join(", ")))
}

/// Whether herdr still has `pane`: only pane_not_found says it is gone.
fn pane_alive(tools: &dyn Tools, repo: &Path, pane: &str) -> bool {
    !herdr(tools, repo, &["pane", "get", pane]).is_err_and(|err| herdr::pane_gone(&err))
}

/// Whether herdr has no agent in `pane`: only agent_not_found says so.
fn agent_dead(tools: &dyn Tools, repo: &Path, pane: &str) -> bool {
    herdr(tools, repo, &["agent", "get", pane]).is_err_and(|err| herdr::agent_gone(&err))
}

/// TICKET LABELS: each label config.json configures, as "orqa:<name>
/// (<kind>): <guidance>", "; " between them; "none" without one. An entry
/// that cannot be read is left out.
fn ticket_labels(repo: &Path) -> Result<String, String> {
    let doc = app::read(repo)?.1;
    let labels: Vec<String> = app::labels(&doc)
        .into_iter()
        .filter_map(|(name, label)| label.ok().map(|l| (name, l)))
        .map(|(name, l)| match l.guidance.as_str() {
            "" => format!("orqa:{name} ({})", l.kind),
            guidance => format!("orqa:{name} ({}): {guidance}", l.kind),
        })
        .collect();
    Ok(if labels.is_empty() {
        "none".to_string()
    } else {
        labels.join("; ")
    })
}
