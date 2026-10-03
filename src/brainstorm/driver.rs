//! The Brainstorm's driver: its session, charting an Idea or a Waypoint of
//! its Map, in a pane split from the Shell's, watched on a thread of its
//! own (ADR 0003) until its result file checks out or its pane is gone. The
//! Shell owns it, so it runs beside any run; its lines go through the
//! Shell's Event channel, where a Ticket's go: charting's under the Idea's
//! id, a Waypoint's under its own or the Map's, and into orchestrator.log.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use super::{brainstorms, is_map, labelled, Brainstorm, Phase, EPIC, MAP, RESEARCH};
use crate::orchestrator::app::{self, BRAINSTORM};
use crate::orchestrator::herdr::{self, agent_name, herdr, locate, place_beside_shell};
use crate::orchestrator::manual;
use crate::orchestrator::result::{
    read_manual, read_stage_result, stage_prompt, ResultRequirements, StageResult, MANUAL,
};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{append_log, plural, run_dir, Ask, Config, Event};
use crate::orchestrator::state::Session;
use crate::orchestrator::trust::await_trust;
use crate::shell::{bd_list, suffix};
use crate::skills::manifest::Manifest;
use crate::skills::stage_skill;
use crate::tools::{RunError, Tools};

/// The session charting an Idea, its agent's stage name.
const CHART: &str = "chart";
/// A session with the user on a Waypoint of the Map, its Manual work's
/// stage.
pub(crate) const WAYPOINT: &str = "waypoint";

/// What a live Map's frontier gives next.
enum Next {
    /// A Waypoint for the user.
    Take(BdIssue),
    /// The build-Epic Waypoint, all that is left.
    Epic(BdIssue),
    /// Research running or blocked behind it: the line that says so.
    Wait(String),
}

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
    /// A dead session's pane still open, which the next pane replaces:
    /// kept off the saved Brainstorm meanwhile, so the Shell sees its pane
    /// still opening and refuses to switch.
    previous: String,
    /// CHART or WAYPOINT.
    stage: &'static str,
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
    let mut d = Driver::new(cfg, saved, stop, b, CHART);
    if let Some(result) = d.take_up(shell, |d| d.start(shell).map(Some)) {
        d.watch(&result);
    }
}

/// A session with the user on the live Map of `b`: brainstorm-waypoint, on
/// the brainstorm_waypoint row, in the pane beside the Shell's that
/// replaces the last Brainstorm pane, its PROMPT `prompt` or none. `pick`
/// is the Waypoint /continue @<waypoint> named, started fresh. Without it a
/// saved session is taken up as charting's is, and a fresh one takes the
/// Map's in-progress Waypoint that is not research, else the frontier's
/// first for the user. Its result naming its Waypoint closed in bd closes
/// the pane and asks "Next Waypoint?".
pub(crate) fn waypoint(
    cfg: &Config,
    shell: &str,
    b: Brainstorm,
    saved: &Sender<Brainstorm>,
    stop: &AtomicBool,
    prompt: Option<String>,
    pick: Option<String>,
) {
    let mut d = Driver::new(cfg, saved, stop, b, WAYPOINT);
    let prompt = prompt.unwrap_or_else(|| "none".to_string());
    // a session saved is an interrupted one: a closed Waypoint clears it
    let interrupted = d.b.session.is_some();
    let result = match pick {
        Some(id) => {
            let started = d.fresh(shell, &prompt, Some(id), false);
            d.started(started)
        }
        None => d.take_up(shell, |d| d.fresh(shell, &prompt, None, interrupted)),
    };
    if let Some(result) = result {
        d.watch(&result);
    }
}

impl<'a> Driver<'a> {
    /// The driver of `b`'s `stage` session, nothing set aside.
    fn new(
        cfg: &'a Config,
        saved: &'a Sender<Brainstorm>,
        stop: &'a AtomicBool,
        b: Brainstorm,
        stage: &'static str,
    ) -> Self {
        Driver {
            cfg,
            saved,
            stop,
            b,
            previous: String::new(),
            stage,
        }
    }

    /// What its lines call its session.
    fn noun(&self) -> &'static str {
        match self.stage {
            CHART => "charting",
            _ => "Waypoint",
        }
    }

    /// Its App row.
    fn row(&self) -> Result<app::Row, String> {
        let name = match self.stage {
            CHART => BRAINSTORM[0],
            _ => BRAINSTORM[1],
        };
        app::row(&self.cfg.repo, name, &[])
    }

    /// The saved session watched again while its agent lives in its pane,
    /// else resumed by id in a fresh pane, else `fresh`: its result file,
    /// None when nothing is watched.
    fn take_up(
        &mut self,
        shell: &str,
        fresh: impl FnOnce(&mut Self) -> Result<Option<PathBuf>, String>,
    ) -> Option<PathBuf> {
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let noun = self.noun();
        let session = self.b.session.clone().filter(|s| !s.id.is_empty());
        // off the saved Brainstorm before the lookup, so no switch closes the
        // pane while herdr is asked whether its agent lives
        if !self.b.pane.is_empty() {
            self.previous = std::mem::take(&mut self.b.pane);
            self.save();
        }
        // a Waypoint's pane kept with no result to watch is replaced
        let watched = self
            .watched_again()
            .map(|alive| alive && (self.stage == CHART || !self.b.result.is_empty()));
        if !matches!(watched, Ok(false)) {
            self.b.pane = std::mem::take(&mut self.previous);
            self.save();
        }
        let started = match (watched, session) {
            (Err(err), _) => {
                let text = format!("{noun} not resumed: herdr did not answer, try again: {err}");
                self.say(&text);
                return None;
            }
            (Ok(true), _) => {
                let at = locate(tools, repo, &self.cfg.workspace, &self.b.pane);
                self.say(&format!("{noun} watched again {at}"));
                Ok(Some(self.result_file()))
            }
            (Ok(false), Some(session)) => match self.resume(shell, &session) {
                // its new pane closed meanwhile, by a switch or the user
                Err(why)
                    if !why.is_empty()
                        && !self.b.pane.is_empty()
                        && !pane_alive(tools, repo, &self.b.pane) =>
                {
                    self.say(&format!("{noun} not resumed: {why}"));
                    self.gone();
                    Err(String::new())
                }
                Err(why) if !why.is_empty() => {
                    self.say(&format!("{noun} not resumed: {why}, starting it fresh"));
                    fresh(self)
                }
                resumed => resumed.map(Some),
            },
            (Ok(false), None) => fresh(self),
        };
        // no new pane took its place: the old one is the Brainstorm's again
        if !self.previous.is_empty() {
            self.b.pane = std::mem::take(&mut self.previous);
            self.save();
        }
        self.started(started)
    }

    /// The result file a start gave, or why not said.
    fn started(&self, started: Result<Option<PathBuf>, String>) -> Option<PathBuf> {
        match started {
            Ok(result) => result,
            Err(why) if why.is_empty() => None, // stopped
            Err(why) => {
                self.say(&format!("{} not started: {why}", self.noun()));
                None
            }
        }
    }

    /// A line under `id`, and the log's; `panel` false keeps a Question
    /// off RECENT.
    fn send(&self, id: &str, text: &str, panel: bool, ask: Option<Ask>) {
        let time = chrono::Local::now();
        append_log(&self.cfg.repo, time, id, text);
        let _ = self.cfg.events.send(Event {
            time,
            ticket: Some(id.to_string()),
            text: text.to_string(),
            panel,
            ask,
            offer: Vec::new(),
            notice: None,
        });
    }

    /// A RECENT line under `id`.
    fn tell(&self, id: &str, text: &str) {
        self.send(id, text, true, None);
    }

    /// A RECENT line under the Idea's id while charting, else the Map's.
    fn say(&self, text: &str) {
        match self.stage {
            CHART => self.tell(&self.b.idea, text),
            _ => self.tell(&self.b.map, text),
        }
    }

    /// `text` under `id` with `ask`, unless it is the line said last.
    fn once(&self, said: &mut String, id: &str, text: &str, ask: Option<Ask>) {
        if said != text {
            self.send(id, text, true, ask);
            *said = text.to_string();
        }
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
        self.stopped("its pane is gone");
    }

    /// The Brainstorm stopped, saved, `why` said; a pane still open is kept
    /// so a resume replaces it.
    fn stopped(&self, why: &str) {
        self.save();
        self.say(&format!("{} stopped: {why}; Brainstorm saved", self.noun()));
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

    /// Whether herdr still names its agent in its saved pane, set aside in
    /// `previous`. Only
    /// agent_not_found says it is gone; another failure is herdr's, so a
    /// live session is never replaced.
    fn watched_again(&self) -> Result<bool, RunError> {
        if self.previous.is_empty() {
            return Ok(false);
        }
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let name = agent_name(&self.b.idea, self.stage);
        let alive = match herdr(tools, repo, &["agent", "get", &name]) {
            Ok(reply) => reply.result.agent.pane_id == self.previous,
            Err(err) if herdr::agent_gone(&err) => false,
            Err(err) => return Err(err),
        };
        Ok(alive)
    }

    /// Its result file, in the Brainstorm's folder: chart.md, or the
    /// Waypoint session's as saved.
    fn result_file(&self) -> PathBuf {
        match self.stage {
            CHART => brainstorms(&self.cfg.repo)
                .join(&self.b.idea)
                .join("chart.md"),
            _ => PathBuf::from(&self.b.result),
        }
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
        let previous = match self.b.pane.is_empty() {
            true => &self.previous,
            false => &self.b.pane,
        };
        let pane = place_beside_shell(tools, repo, shell, previous, &["--cwd", &tree])
            .map_err(|err| format!("got no pane: {err}"))?;
        self.previous.clear();
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
        let name = agent_name(&self.b.idea, self.stage);
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
    /// continue, only while its row's App is unchanged and its result file
    /// is known: that file's path. Why not, or empty once stopped.
    fn resume(&mut self, shell: &str, session: &Session) -> Result<PathBuf, String> {
        let row = self.row()?;
        if self.stage == WAYPOINT && self.b.result.is_empty() {
            return Err("its result file was not saved".to_string());
        }
        if row.app.name != session.app {
            return Err(format!("its App is now {}", row.app.name));
        }
        let mut args = row.resume(&session.id);
        args.extend(self.args(&row));
        let at = self.launch(shell, &row, session.clone(), &args)?;
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        herdr(tools, repo, &["agent", "prompt", &self.b.pane, "continue"])
            .map_err(|err| format!("never took continue: {err}"))?;
        self.say(&format!("{} resumed: {} {at}", self.noun(), row.said()));
        Ok(self.result_file())
    }

    /// The skill orqa-brainstorm-<name> as committed on the worktree's
    /// base, as a Stage skill is (ADR 0006), its jobs filled for `row`.
    fn skill(&self, row: &app::Row, name: &str) -> Result<String, String> {
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let worktree = PathBuf::from(&self.b.worktree);
        let skill = stage_skill(&worktree, &format!("orqa-brainstorm-{name}")).unwrap_or_else(|| {
            Err(format!("has no brainstorm-{name} skill on its base branch: commit and merge .orqadence/skills (orqa init writes them)"))
        })?;
        let manifest = Manifest::load(repo)?;
        let have = manifest.have(repo, &worktree, &self.cfg.home, tools, row.app);
        Ok(manifest
            .fill_jobs(&skill, &have, row.app.built_in, row.app.mention)
            .0)
    }

    /// The session started in its pane and prompted: its result file's
    /// path. Why not, or empty once stopped.
    fn start(&mut self, shell: &str) -> Result<PathBuf, String> {
        let repo = &self.cfg.repo;
        let row = self.row()?;
        let skill = self.skill(&row, "chart")?;
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
        let name = agent_name(&self.b.idea, self.stage);
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
                let taken = match self.stage {
                    CHART => self
                        .done(result)
                        .map_err(|why| {
                            let idea = self.b.idea.clone();
                            let text = format!("charting result not taken: {why}");
                            self.once(&mut said, &idea, &text, None);
                        })
                        .is_ok(),
                    _ => self.waypoint_done(result, &mut said),
                };
                if taken {
                    return;
                }
            } else if self.stage == WAYPOINT {
                said.clear(); // Manual work answered: a new item is asked afresh
            }
            // None is also a failed agent get: only a gone pane, or a pane
            // herdr says has no agent, stops it
            if watched.is_none() {
                if !pane_alive(tools, repo, &self.b.pane) {
                    return self.gone();
                }
                if agent_dead(tools, repo, &self.b.pane) {
                    return self.stopped("its session is gone");
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
            self.b.session = None; // charting's: no Waypoint session resumes it
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

/// A Waypoint session's own steps.
impl Driver<'_> {
    /// A fresh session: on `pick`, else, in place of an `interrupted` one,
    /// the Map's in-progress Waypoint that is not research, else the
    /// frontier's next for the user; its result file. None, said, when
    /// research or the build-Epic Waypoint is all that is left. Why not, or
    /// empty once stopped.
    fn fresh(
        &mut self,
        shell: &str,
        prompt: &str,
        pick: Option<String>,
        interrupted: bool,
    ) -> Result<Option<PathBuf>, String> {
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let issues = bd_list(repo, tools).map_err(|err| format!("bd list failed: {err}"))?;
        let map = self.b.map.as_str();
        let claimed = issues.iter().find(|i| {
            interrupted
                && i.parent == map
                && i.status == "in_progress"
                && !labelled(i, RESEARCH)
                && !labelled(i, EPIC)
        });
        let pick = pick.or_else(|| claimed.map(|i| i.id.clone()));
        let set = pick.is_some();
        let w = match pick {
            Some(id) => match issues.iter().find(|i| i.id == id) {
                Some(w) if labelled(w, EPIC) => Next::Epic(w.clone()),
                Some(w) => Next::Take(w.clone()),
                None => return Err(format!("{id} is not in bd")),
            },
            None => self.frontier(&issues)?,
        };
        match w {
            Next::Take(w) => self.start_waypoint(shell, &w.id, set, prompt).map(Some),
            Next::Epic(e) => {
                let line = format!("only the build-Epic Waypoint is left: {}", suffix(&e.id));
                self.idle(&line);
                Ok(None)
            }
            Next::Wait(line) => {
                self.idle(&line);
                Ok(None)
            }
        }
    }

    /// No session runs, said: the Brainstorm stays live with none saved.
    fn idle(&mut self, line: &str) {
        self.b.session = None;
        self.b.result.clear();
        self.save();
        self.say(line);
    }

    /// What the Map's frontier gives next, read from bd in map order.
    fn frontier(&self, issues: &[BdIssue]) -> Result<Next, String> {
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let map = self.b.map.as_str();
        let argv = ["bd", "ready", "--parent", map, "--unassigned", "--json"];
        let out = tools
            .run(repo, &argv)
            .map_err(|err| format!("bd ready failed: {err}"))?;
        let ready = serde_json::from_str::<Option<Vec<BdIssue>>>(&out)
            .map_err(|err| format!("bd ready failed: {err}"))?
            .unwrap_or_default();
        Ok(next(&ready, issues, &self.b))
    }

    /// brainstorm-waypoint started on Waypoint `id`, WAYPOINT given when
    /// `set`: its result file, waypoint-<n>.md with n the first unused,
    /// saved for a resume. Why not, or empty once stopped.
    fn start_waypoint(
        &mut self,
        shell: &str,
        id: &str,
        set: bool,
        prompt: &str,
    ) -> Result<PathBuf, String> {
        let row = self.row()?;
        let skill = self.skill(&row, "waypoint")?;
        let dir = brainstorms(&self.cfg.repo).join(&self.b.idea);
        fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        let file = |n: usize| dir.join(format!("waypoint-{n}.md"));
        let result = (1..).map(file).find(|f| !f.exists()).unwrap();
        self.b.result = result.display().to_string();
        let session = Session {
            app: row.app.name.to_string(),
            ..Default::default()
        };
        let args = self.args(&row);
        let at = self.launch(shell, &row, session, &args)?;
        self.tell(id, &format!("Waypoint started: {} {at}", row.said()));
        let background = if self.b.background { "on" } else { "off" };
        let shown = self.b.result.clone();
        let mut inputs = vec![
            ("MAP", self.b.map.as_str()),
            ("BACKGROUND", background),
            ("PROMPT", prompt),
            ("RESULT FILE", &shown),
        ];
        if set {
            inputs.push(("WAYPOINT", id));
        }
        let prompt = stage_prompt(&skill, &inputs);
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        herdr(tools, repo, &["agent", "prompt", &self.b.pane, &prompt])
            .map_err(|err| format!("never took the skill: {err}"))?;
        Ok(result)
    }

    /// A Waypoint result read: its WAYPOINT closed in bd closes the pane
    /// and asks "Next Waypoint?", or says the frontier waits on research;
    /// still open, the pane is kept. Manual work is put to the user once.
    /// Whether it ended the session; each line not taken said once.
    fn waypoint_done(&mut self, result: &Path, said: &mut String) -> bool {
        let map = self.b.map.clone();
        let (r, why) = read_stage_result(result, ResultRequirements::default());
        if why == MANUAL {
            self.manual(result, said);
            return false;
        }
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let issues = match (why.is_empty(), bd_list(repo, tools)) {
            (false, _) => Err(why),
            (true, Err(err)) => Err(format!("bd list failed: {err}")),
            (true, Ok(issues)) => Ok(issues),
        };
        let found = issues.and_then(|issues| {
            let w = issues
                .iter()
                .find(|i| i.id == r.waypoint && i.parent == map);
            match (w.cloned(), r.waypoint.as_str()) {
                (Some(w), _) => Ok((w, issues)),
                (None, "") => Err("it names no WAYPOINT".to_string()),
                (None, id) => Err(format!("{id} is not a Waypoint of {map}")),
            }
        });
        let (w, issues) = match found {
            Ok(found) => found,
            Err(why) => {
                self.once(
                    said,
                    &map,
                    &format!("Waypoint result not taken: {why}"),
                    None,
                );
                return false;
            }
        };
        if w.status != "closed" {
            let at = locate(tools, repo, &self.cfg.workspace, &self.b.pane);
            let at = at.trim_start_matches('(').trim_end_matches(')');
            let text = format!(
                "result written, but {} is still open in bd: {at} kept",
                suffix(&w.id)
            );
            self.once(said, &w.id, &text, None);
            return false;
        }
        // Only a closed or already gone pane is forgotten.
        match herdr(tools, repo, &["pane", "close", &self.b.pane]) {
            Err(err) if !herdr::pane_gone(&err) => {
                self.tell(&w.id, &format!("Waypoint pane not closed: {err}"));
            }
            _ => self.b.pane.clear(),
        }
        self.b.session = None;
        self.b.result.clear();
        self.save();
        self.tell(&w.id, "closed; its pane closes");
        let asked = format!("{} {} closed. Next Waypoint?", suffix(&w.id), w.title);
        match self.frontier(&issues) {
            Ok(Next::Take(n) | Next::Epic(n)) => {
                let options = vec![
                    format!("yes: the next on the Map, {} {}", suffix(&n.id), n.title),
                    format!("no: the Brainstorm stops, /continue @{map} picks it up"),
                    "yes, with a prompt of your own in place of the default".to_string(),
                ];
                self.send(
                    &w.id,
                    &asked,
                    false,
                    Some(Ask::NextWaypoint { options, map }),
                );
            }
            Ok(Next::Wait(line)) => self.say(&line),
            Err(why) => self.say(&format!("Next Waypoint not asked: {why}")),
        }
        true
    }

    /// STATUS: manual: the item, in the Run directory of the Waypoint its
    /// folder names, put to the user once as Manual work's Question.
    fn manual(&mut self, result: &Path, said: &mut String) {
        let filed = read_manual(result).unwrap_or_default();
        let folder = PathBuf::from(&self.b.worktree).join(&filed);
        let id = folder
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        match manual::read(&run_dir(repo, &id), &folder) {
            Ok(item) => {
                let at = locate(tools, repo, &self.cfg.workspace, &self.b.pane);
                let ask = Ask::Manual {
                    pane: self.b.pane.clone(),
                    stage: WAYPOINT.to_string(),
                    item,
                };
                self.once(
                    said,
                    &id,
                    &format!("manual work in Waypoint {at}"),
                    Some(ask),
                );
            }
            Err(err) => {
                let text = format!("Waypoint result not taken: its Manual work: {err}");
                let map = self.b.map.clone();
                self.once(said, &map, &text, None);
            }
        }
    }
}

/// What `ready`, the Map's frontier in map order, gives next among
/// `issues`: its first Waypoint for the user, never the build-Epic one nor
/// research while it runs in the background; else, with every other
/// Waypoint closed, the build-Epic one; else the line saying what the
/// research waits on.
fn next(ready: &[BdIssue], issues: &[BdIssue], b: &Brainstorm) -> Next {
    let mine = |w: &&BdIssue| !labelled(w, EPIC) && !(b.background && labelled(w, RESEARCH));
    if let Some(w) = ready.iter().find(mine) {
        return Next::Take(w.clone());
    }
    let open = |i: &&BdIssue| i.parent == b.map && i.status != "closed";
    let (epic, open): (Vec<&BdIssue>, Vec<&BdIssue>) =
        issues.iter().filter(open).partition(|i| labelled(i, EPIC));
    if let (true, Some(e)) = (open.is_empty(), epic.first()) {
        return Next::Epic((*e).clone());
    }
    let unfinished = |id: &&str| issues.iter().any(|i| i.id == *id && i.status != "closed");
    let running: Vec<&str> = open
        .iter()
        .filter(|i| labelled(i, RESEARCH) && !i.blockers().any(|id| unfinished(&id)))
        .map(|i| suffix(&i.id))
        .collect();
    let mut parts = Vec::new();
    if !running.is_empty() {
        parts.push(format!("{} running", running.join(", ")));
    }
    for i in &open {
        let on: Vec<&str> = i.blockers().filter(unfinished).map(suffix).collect();
        if !on.is_empty() {
            parts.push(format!("{} is blocked on {}", suffix(&i.id), on.join(", ")));
        }
    }
    Next::Wait(match parts.is_empty() {
        true => "waiting: no Waypoint is ready for you".to_string(),
        false => format!("waiting for research: {}", parts.join(", ")),
    })
}

/// The Idea's close reason when the result checks out in `issues`: its MAP
/// an open epic labelled brainstorm:map with exactly one open
/// brainstorm:epic child, or its TICKETS open issues. Why not otherwise.
fn outcome(issues: &[BdIssue], r: &StageResult) -> Result<String, String> {
    let open = |id: &str| issues.iter().find(|i| i.id == id && i.status != "closed");
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
