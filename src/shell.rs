//! The Shell: the full-terminal screen that `orqa` alone opens (ADR 0004).
//! `Screen` is the plain state the tests drive; `open` wraps it in the
//! terminal and the one draw, poll and tick loop (ADR 0003). The Shell owns
//! the Orchestrator: the scheduler runs on a thread of this process, its
//! Events come over a channel into RECENT and the log, and the TICKETS rows
//! are a snapshot of its State. An Event that asks (a Wake, a blocked
//! session) becomes a Question, whose answer goes back to the Orchestrator
//! for that session; the Orchestrator knows no Shell type.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event as Input, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::DefaultTerminal;

use crate::brainstorm::{self, Brainstorm, Phase};
use crate::graphify;
use crate::on_call::{self, Doorbell, OnCall};
use crate::orchestrator::app::{self, ADDRESS_PR_COMMENTS_COUNTDOWN, RELEASE_ON};
use crate::orchestrator::herdr::{self, herdr};
use crate::orchestrator::judgment::{self, Action};
use crate::orchestrator::manual;
use crate::orchestrator::pr::Item;
use crate::orchestrator::release;
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{
    append_log, plural, pr_ref, Answer, Ask, Config, Event, Orchestrator,
};
use crate::orchestrator::state::{
    acquire_lock, load_state, Lock, Review, State, TicketState, LOCAL, STATUS_MERGED,
    STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING,
};
use crate::setup;
use crate::tools::{Editor, Tools};
use crate::update::{self, Checked, Ready, Releases};
use charted::{StartMap, Tickets};
use idea::Idea;
use summary::Summary;

pub(crate) mod brand;
mod charted;
mod config;
mod demo;
mod draw;
mod idea;
mod summary;

/// The screen redraws every 50 ms while a run is live, for the spinner and
/// the header's panes; at rest every 250 ms.
const TICK: Duration = Duration::from_millis(50);
const IDLE_TICK: Duration = Duration::from_millis(250);
/// How long 'press Ctrl-C again to exit' stands.
const CTRL_C_WINDOW: Duration = Duration::from_secs(2);
const NOTICE_WINDOW: Duration = Duration::from_secs(5);
/// How long non-blocking Manual work's Notice modal stands with no key.
const MANUAL_NOTICE_WINDOW: Duration = Duration::from_secs(60);
/// RECENT keeps this many Events; older ones are in the log.
const KEPT_EVENTS: usize = 1000;
/// An update another process's run keeps from installing is tried this often.
const RETRY: Duration = Duration::from_secs(60);
/// Every command the Shell takes: its name, arguments and what it does. The
/// / list shows it, and the README's table.
const COMMANDS: [(&str, &str, &str); 18] = [
    ("/start-epic", "<epic>", "run every Ticket of an open Epic"),
    (
        "/start-ticket",
        "<ticket>…",
        "run Tickets, or add them to the live Ticket run",
    ),
    (
        "/remove-ticket",
        "<ticket>",
        "take a Ticket out of the live Ticket run",
    ),
    (
        "/continue",
        "[<id>]",
        "resume the saved run, a Parked Ticket, or a Brainstorm",
    ),
    (
        "/brainstorm",
        "",
        "chart an idea into Tickets or a Map, with you",
    ),
    ("/stop-work", "", "stop the run, the panes stay"),
    (
        "/retry",
        "<ticket>",
        "the Ticket's Stage again, in a fresh session",
    ),
    ("/park", "<ticket>", "take a Ticket out to wait for you"),
    (
        "/rebase",
        "<ticket>",
        "rebase a PR that conflicts with main",
    ),
    (
        "/address-pr-comments",
        "<ticket>",
        "open a PR's comments and failing checks for approval",
    ),
    ("/questions", "", "show the hidden Questions"),
    (
        "/manual-work",
        "",
        "the Manual work still open, to mark done",
    ),
    (
        "/config",
        "",
        "the App, model and effort each Stage runs on",
    ),
    (
        "/away",
        "",
        "a Stage's question parks its Ticket, again to turn off",
    ),
    (
        "/summary",
        "[<epic>]",
        "the Epic's PRs, Rounds and Findings",
    ),
    ("/demo", "", "a made-up run to see the Shell at work"),
    ("/stop-demo", "", "end the demo, the Shell as it was"),
    ("/exit", "", "leave Orqadence"),
];

/// An open Epic and its child Tickets, one row each on the TICKETS tree; or,
/// its id empty and last, the no-Epic group: the open Tickets with no open Epic.
pub(crate) struct Epic {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) tickets: Vec<BdIssue>,
    /// What the Epic waits on: its bd blocks dependencies.
    pub(crate) blockers: Vec<String>,
    /// Its own bd labels.
    pub(crate) labels: Vec<String>,
}

/// The live run: the Orchestrator, its scheduler thread and the lock, held
/// until the scheduler has returned and every Ticket thread has left.
struct Run {
    o: Arc<Orchestrator>,
    /// None once joined: the run is stopping, its Ticket threads leaving.
    scheduler: Option<JoinHandle<Result<(), String>>>,
    /// An Epic run; otherwise a Ticket run, over the State's queue. A
    /// finished run clears the state file.
    epic: bool,
    /// The scheduler returned an error: the state file is read back.
    failed: bool,
    /// The summary has opened by itself, which it does once a run, again
    /// after a Ticket joins a Ticket run.
    summarized: bool,
    /// The run ends in a Release: its summary waits for the run's end.
    release: bool,
    _lock: Lock,
}

/// What a yes/no confirmation, or a yes/no Notice modal, does on yes.
pub(crate) enum Pending {
    /// Discard the saved run and start this Epic, or a Ticket run on these
    /// Tickets.
    Start { ids: Vec<String>, epic: bool },
    /// Stop the run and exit.
    Exit,
    /// Close the done Epic in bd, asked in a Notice modal: its completed
    /// State, which poll has cleared from the Shell, for the summary in the
    /// reason, and whether the close comment went in, so a retry never adds
    /// it twice.
    Close { done: State, commented: bool },
    /// Run graphify's Docs pass on this new X.Y tag; no records its X.Y as
    /// handled, so the tag is skipped until the next X.Y.
    DocsPass { tag: String },
    /// Stop the live Brainstorm, saved, then run `line`: /brainstorm or
    /// /continue @<other>.
    Switch { line: String },
}

/// What a Question is about, which decides its options and what an answer does.
pub(crate) enum About {
    /// What the Orchestrator asked of a Ticket. A Wake offers the actions
    /// still unspent (a nudge with either canned prompt, or the one a
    /// Judgment picked; retry; park; wait), then open the pane and a prompt
    /// of your own; a blocked session offers open the pane, park, "I
    /// answered it"; a plan offers approve, feedback of your own, resend
    /// the feedback that was not sent, park, open the pane; a plan
    /// failure offers open the pane, park, retry, resend the feedback.
    Asked(Ask),
    /// A yes/no confirmation; it jumps the queue, but for the Docs pass's,
    /// queued with the others.
    Confirm(Pending),
    /// The /continue checklist: one row per saved Ticket, reset toggled by Space.
    Continue { rows: Vec<(String, bool)> },
}

/// What the Shell puts to the user above the input line when the
/// Orchestrator cannot act alone. One shows at a time, oldest first; a
/// Ticket's Question holds that Ticket alone, and none is ever saved.
pub(crate) struct Question {
    pub(crate) ticket: Option<String>,
    /// The event line it came from, or the confirmation's wording.
    pub(crate) text: String,
    pub(crate) about: About,
    /// The option the cursor is on.
    pub(crate) cursor: usize,
    /// The first row of a plan or a Stage's question shown, or a Wake's
    /// tail's rows up from its last, which the modal's reading keys and the
    /// wheel move; the draw, which knows the width, keeps it inside.
    pub(crate) scroll: Cell<usize>,
    /// When it was asked: its line's time.
    pub(crate) asked: chrono::DateTime<chrono::Local>,
    /// When a plan's modal first drew it, for its count of the lines since.
    pub(crate) opened: OnceCell<chrono::DateTime<chrono::Local>>,
}

impl Question {
    /// The Brainstorm's own, which no run's end drops: Next Waypoint?, or
    /// a Waypoint session's Manual work.
    pub(crate) fn brainstorms(&self) -> bool {
        match &self.about {
            About::Asked(Ask::NextWaypoint { .. }) => true,
            About::Asked(Ask::Manual { stage, .. }) => stage == brainstorm::driver::WAYPOINT,
            _ => false,
        }
    }
}

/// A Notice modal's kind: red and titled ERROR, or green and titled NOTICE.
pub(crate) enum NoticeKind {
    Error,
    Info,
}

/// A Notice modal: a message over whatever is open until Enter or Esc
/// closes it. Unlike the one-line notice it never goes by itself, but for
/// an autoclose no key has stopped.
pub(crate) struct Notice {
    pub(crate) kind: NoticeKind,
    pub(crate) text: String,
    /// How long it stands once it shows, if it closes by itself.
    autoclose: Option<Duration>,
    /// When it closes by itself: set as it shows, cleared for good by any
    /// key but Enter or Esc.
    pub(crate) closes: Option<chrono::DateTime<chrono::Local>>,
    /// The first row of a message longer than the box shown, which ↑↓
    /// move; the draw, which knows the height, keeps it inside.
    pub(crate) scroll: Cell<usize>,
    /// A yes/no one in place of [ OK ]: what yes does.
    pub(crate) yes: Option<Pending>,
    /// The button the cursor is on: 0 yes, 1 no.
    pub(crate) cursor: usize,
}

/// The approval modal: a Ticket's PR comments and failing checks, each
/// row checked or not, for Address PR comments to fix or answer as won't
/// fix. Not a Question: it never joins their queue, nor rings On call.
pub(crate) struct Approval {
    pub(crate) ticket: String,
    /// "PR #12".
    pub(crate) pr: String,
    pub(crate) rows: Vec<(Item, bool)>,
    pub(crate) cursor: usize,
    /// How long it counts down once it shows, if it approves itself.
    countdown: Option<Duration>,
    /// When it approves the rows as they stand: set as it shows, cleared
    /// for good by any key.
    pub(crate) approves: Option<chrono::DateTime<chrono::Local>>,
    /// /address-pr-comments opened it: the runs cap does not hold its run.
    by_hand: bool,
}

/// The /manual-work modal: every Run directory's open Manual work, each
/// row checked or not, to mark done. A blocking row is never checked: only
/// its Question marks it done, since its session waits for the facts.
pub(crate) struct ManualWork {
    /// The Ticket or Waypoint (its Run directory's name), the item, checked.
    pub(crate) rows: Vec<(String, manual::Item, bool)>,
    /// The row the cursor is on.
    pub(crate) cursor: usize,
}

/// What the screen shows, with no terminal in it.
pub(crate) struct Screen {
    pub(crate) folder: String,
    pub(crate) version: String,
    /// COLORTERM says 24-bit; otherwise every color is folded to the 256 cube.
    pub(crate) truecolor: bool,
    pub(crate) epics: Vec<Epic>,
    /// The Brainstorms' Maps, Waypoints and Ideas, kept off the tree.
    brainstorm_issues: Vec<BdIssue>,
    /// The saved Brainstorms, loaded at open; a driver's thread sends each
    /// one it changes back through brainstorm_sender.
    pub(crate) brainstorms: Vec<Brainstorm>,
    /// The Shell's own herdr pane, HERDR_PANE_ID at open: the charting
    /// pane splits it.
    pub(crate) shell_pane: String,
    /// The Brainstorm drivers' state changes, applied in poll().
    brainstorm_sender: Sender<Brainstorm>,
    brainstorm_receiver: Receiver<Brainstorm>,
    /// Set by close(): a driver's thread leaves, its pane running in herdr.
    brainstorm_stop: Arc<AtomicBool>,
    /// The drivers' threads, by idea, joined by close() so a result being
    /// acted on is saved before the process exits.
    pub(crate) brainstorm_threads: Vec<(String, JoinHandle<()>)>,
    /// The suggested command, ghost text in the empty input: Tab fills it
    /// in, running any command clears it.
    pub(crate) suggestion: Option<String>,
    /// The run's State: a snapshot of the live Orchestrator's, or the saved
    /// one; the Overall bar, the TICKETS rows and the resumable mark come
    /// from it.
    pub(crate) state: State,
    /// The panel's lines, oldest first.
    pub(crate) events: Vec<Event>,
    pub(crate) input: String,
    /// The input line's cursor, in chars before its end: 0 types at the end.
    pub(crate) back: usize,
    /// The command lines entered, oldest first; ↑ on an empty line recalls
    /// the last.
    pub(crate) history: Vec<String>,
    /// Where the history is kept for the next Shell, a line per command:
    /// .orqadence-local/history, set at open.
    history_file: Option<PathBuf>,
    /// The history line recalled onto the input, while ↑↓ walk the history.
    recall: Option<usize>,
    /// The open list's cursor row, back to the top on every key that types.
    pub(crate) pick: usize,
    /// The first TICKETS row shown, for a tree taller than its room; the
    /// draw, which knows the height, keeps it inside the tree.
    pub(crate) scroll: Cell<usize>,
    /// RECENT's rows scrolled up from the newest, 0 sticking to the bottom;
    /// the draw, which knows the height, keeps it inside the lines.
    pub(crate) recent: Cell<usize>,
    /// One line above the input, and when it goes.
    pub(crate) notice: Option<(String, Instant)>,
    /// The Notice modals waiting, oldest first; the first shows over
    /// everything and takes every key.
    pub(crate) notices: Vec<Notice>,
    /// The close-Epic Notice modal, held while the Epic summary is open
    /// until Esc on it raises it over the summary.
    pub(crate) held: Option<Notice>,
    ctrl_c: Option<Instant>,
    pub(crate) ticks: u64,
    /// A run is live: the header's panes step, the status row spins.
    pub(crate) running: bool,
    /// The tick the live run started on, where the panes' steps count from.
    pub(crate) started: u64,
    pub(crate) quit: bool,
    /// Every run's Config, cloned for the run. Its exe is the running
    /// binary's real path, resolved once at open: an update renames over it,
    /// and every run's plan hook runs it. Tests point it at a scratch file.
    pub(crate) cfg: Config,
    /// What the preflight found missing at open; a run is refused while
    /// anything is.
    missing: Vec<String>,
    /// The Orchestrator's Events: the Shell holds the receiver, every run's
    /// Config gets the sender.
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    run: Option<Run>,
    /// The Questions waiting, oldest first; the first shows unless hidden.
    pub(crate) questions: Vec<Question>,
    /// The approval modals waiting, one PR's each, oldest first; the first
    /// shows ahead of any Question and takes its keys.
    pub(crate) approvals: Vec<Approval>,
    /// /manual-work, while it is open: it takes every key but Ctrl-C.
    pub(crate) manual_work: Option<ManualWork>,
    /// /brainstorm's idea modal, while it is open: it takes every key but
    /// Ctrl-C.
    pub(crate) idea: Option<Idea>,
    /// The Tickets modal, while charting's Tickets wait on the user: it
    /// takes every key but Ctrl-C.
    pub(crate) tickets: Option<Tickets>,
    /// The start-Map modal, or its Continue form: it takes every key but
    /// Ctrl-C.
    pub(crate) start_map: Option<StartMap>,
    /// Charting's outcomes waiting on the Tickets or start-Map modal open,
    /// oldest first.
    pub(crate) charted: VecDeque<Brainstorm>,
    /// The live Brainstorm's Idea: one at a time, and none at open.
    pub(crate) live: Option<String>,
    /// Ctrl+G on the idea modal: the run loop hands the terminal to the
    /// editor, as it re-execs on reexec.
    pub(crate) editing: bool,
    /// Where Ctrl+G's editor runs: the terminal, or in tests a FakeEditor.
    pub(crate) editor: Arc<dyn Editor>,
    /// Esc hid the Question; /questions or Esc on an empty input line brings it back.
    pub(crate) hidden: bool,
    /// The input line is a prompt of the user's own for the front Question.
    pub(crate) composing: bool,
    /// /config, while it is open: it takes every key but Ctrl-C.
    pub(crate) settings: Option<config::Settings>,
    /// The Ticket /continue @ticket unparked: its next Question goes ahead
    /// of every other Ticket's.
    first: Option<String>,
    /// The docked plan's page, or the summary's, at the last draw, which
    /// PageUp, PageDown and Space move by.
    pub(crate) page: Cell<usize>,
    /// The rows the docked plan's headings, or the summary's Tickets, start
    /// on at the last draw, for Tab and Shift-Tab.
    pub(crate) heads: RefCell<Vec<usize>>,
    /// Where the last draw put TICKETS, RECENT and the docked Question, for
    /// the wheel to scroll the one under the pointer.
    pub(crate) tickets_area: Cell<Rect>,
    pub(crate) recent_area: Cell<Rect>,
    pub(crate) dock_area: Cell<Rect>,
    /// The updater thread's checks, applied between commands in poll().
    update_sender: Sender<Checked>,
    update_receiver: Receiver<Checked>,
    /// The graphify thread's passes, applied in poll() as the checks are.
    graphify_sender: Sender<graphify::Pass>,
    graphify_receiver: Receiver<graphify::Pass>,
    /// The tag a Docs pass runs on: while it does, or its Question waits,
    /// a check's new tag asks nothing.
    pub(crate) docs_tag: Option<String>,
    /// The Docs pass's RECENT lines, a thread's, and whether the pass is over.
    docs_sender: Sender<(String, bool)>,
    docs_receiver: Receiver<(String, bool)>,
    /// The Docs pass's graphify tab once its thread has made it, which
    /// /exit cancels and closes while docs_tag is set.
    docs_tab: Arc<Mutex<graphify::DocsTab>>,
    /// True while a Docs pass's thread runs: the checks leave graph.json to it.
    docs_live: Arc<Mutex<bool>>,
    /// A release downloaded while a run holds the lock: installed when it ends.
    pub(crate) update: Option<Ready>,
    /// When a pending update may try the lock again, from tick().
    pub(crate) retry: Instant,
    /// An idle-Shell update installed: open() re-execs after the terminal is back.
    pub(crate) reexec: bool,
    /// The Epic summary, over the whole terminal while open.
    pub(crate) summary: Option<Summary>,
    /// The State of the last run to finish, which its end cleared: what
    /// /summary alone shows next. In memory only.
    last: State,
    /// /demo's scripted run, in place of the Shell's Epics, State and RECENT.
    pub(crate) demo: Option<demo::Demo>,
    /// The On call settings, loaded once at open: the one copy the On call
    /// state and /config read and change.
    pub(crate) on_call: OnCall,
    /// Where On call pushes go: Moshi, or in tests a FakeDoorbell, so no
    /// test rings the phone; a test keeps its own fake by putting it here.
    pub(crate) doorbell: Arc<dyn Doorbell>,
    /// On call is on: off each time the Shell opens, like Away.
    pub(crate) calling: bool,
    /// When On call last ended: a Question's wait counts from then, if later
    /// than its asking.
    off_since: Option<chrono::DateTime<chrono::Local>>,
    /// Each push's outcome, a thread's, read in poll() as the update checks are.
    ring_sender: Sender<Result<(), String>>,
    ring_receiver: Receiver<Result<(), String>>,
    /// The pushes whose outcome poll() has not read yet.
    pub(crate) pushes: usize,
    /// MOSHI_WEBHOOK_TOKEN was set at open: it wins over a token /config keeps.
    pub(crate) moshi_env: bool,
}

impl Screen {
    pub(crate) fn new(
        cfg: Config,
        folder: String,
        truecolor: bool,
        epics: Vec<Epic>,
        state: State,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (update_sender, update_receiver) = mpsc::channel();
        let (ring_sender, ring_receiver) = mpsc::channel();
        let (graphify_sender, graphify_receiver) = mpsc::channel();
        let (docs_sender, docs_receiver) = mpsc::channel();
        let (brainstorm_sender, brainstorm_receiver) = mpsc::channel();
        Screen {
            folder,
            version: crate::version::version(),
            truecolor,
            epics,
            brainstorm_issues: Vec::new(),
            brainstorms: Vec::new(),
            shell_pane: String::new(),
            brainstorm_sender,
            brainstorm_receiver,
            brainstorm_stop: Arc::default(),
            brainstorm_threads: Vec::new(),
            suggestion: None,
            state,
            events: Vec::new(),
            input: String::new(),
            back: 0,
            history: Vec::new(),
            history_file: None,
            recall: None,
            pick: 0,
            scroll: Cell::new(0),
            recent: Cell::new(0),
            notice: None,
            notices: Vec::new(),
            held: None,
            ctrl_c: None,
            ticks: 0,
            running: false,
            started: 0,
            quit: false,
            cfg,
            missing: Vec::new(),
            sender,
            receiver,
            run: None,
            questions: Vec::new(),
            approvals: Vec::new(),
            manual_work: None,
            idea: None,
            tickets: None,
            start_map: None,
            charted: VecDeque::new(),
            live: None,
            editing: false,
            #[cfg(not(test))]
            editor: Arc::new(crate::tools::Exec),
            #[cfg(test)]
            editor: Arc::new(crate::tools::fake::FakeEditor::default()),
            hidden: false,
            composing: false,
            settings: None,
            first: None,
            page: Cell::new(0),
            heads: RefCell::new(Vec::new()),
            tickets_area: Cell::default(),
            recent_area: Cell::default(),
            dock_area: Cell::default(),
            update_sender,
            update_receiver,
            graphify_sender,
            graphify_receiver,
            docs_tag: None,
            docs_sender,
            docs_receiver,
            docs_tab: Arc::default(),
            docs_live: Arc::default(),
            update: None,
            retry: Instant::now(),
            reexec: false,
            summary: None,
            last: State::default(),
            demo: None,
            on_call: OnCall::default(),
            #[cfg(not(test))]
            doorbell: Arc::new(on_call::Moshi),
            #[cfg(test)]
            doorbell: Arc::new(on_call::FakeDoorbell::default()),
            calling: false,
            off_since: None,
            ring_sender,
            ring_receiver,
            pushes: 0,
            moshi_env: false,
        }
    }

    /// The idle screen for a Target repo: the open Epics from bd, the saved
    /// run and the preflight.
    pub(crate) fn open(repo: &Path, tools: Arc<dyn Tools>, env: &dyn Fn(&str) -> String) -> Self {
        let home = env("HOME");
        let folder = match repo.strip_prefix(&home) {
            Ok(rest) if !home.is_empty() => Path::new("~").join(rest).display().to_string(),
            _ => repo.display().to_string(),
        };
        let colorterm = env("COLORTERM");
        let truecolor = colorterm == "truecolor" || colorterm == "24bit";
        let state = load_state(repo).unwrap_or_default();
        let missing = setup::preflight(repo, &*tools, env);
        let cfg = Config {
            tools,
            repo: repo.to_path_buf(),
            workspace: env("HERDR_WORKSPACE_ID"),
            api_key: setup::typesafe_key(repo, env).unwrap_or_default(),
            exe: PathBuf::new(),
            typesafe: Arc::new(judgment::Api),
            home: PathBuf::from(&home),
            tick: Duration::from_secs(5),
            poll_prs: Duration::from_secs(30),
            away: Arc::default(), // off each time the Shell opens
            log: Arc::new(Mutex::new(Box::new(io::sink()))),
            events: mpsc::channel().0,
            clock: Arc::new(chrono::Local::now),
            #[cfg(test)]
            timeout: None,
            #[cfg(test)]
            wait: None,
        };
        let mut screen = Screen::new(cfg, folder, truecolor, Vec::new(), state);
        screen.missing = missing;
        screen.shell_pane = env("HERDR_PANE_ID");
        let history = repo.join(LOCAL).join("history");
        screen.history = fs::read_to_string(&history)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        screen.history_file = Some(history);
        screen.on_call = on_call::load(repo, env);
        screen.moshi_env = !env(on_call::TOKEN_VAR).trim().is_empty();
        // every issue: a saved Brainstorm's closed issue may have lost its label
        let issues = screen.reload_issues().unwrap_or_default();
        screen.brainstorms = brainstorm::load(repo, &issues);
        screen.track_docs();
        // nothing is live at open
        screen.suggestion = brainstorm::most_recent(repo, &screen.brainstorms)
            .map(|b| format!("/continue @{}", b.key()));
        match update::exe_path() {
            Ok(exe) => {
                screen.cfg.exe = exe;
                screen.notify_updated();
                screen.check_updates(Arc::new(update::GitHub), update::EVERY);
            }
            Err(err) => screen.say(&format!("update check failed: {err}")),
        }
        screen.refresh_graphify(update::EVERY);
        screen
    }

    /// The update notice: an install that put this version in place, the
    /// last Shell's or init's, left its marker beside the exe.
    pub(crate) fn notify_updated(&mut self) {
        if update::take_marker(&self.cfg.exe, &self.version) {
            let text = format!(
                "Updated to version {}. See release notes: {}",
                self.version,
                update::release_notes(&self.version)
            );
            self.notify(NoticeKind::Info, &text, Some(Duration::from_secs(30)));
        }
    }

    /// The updater thread: one check now, then one every `every`, each handed
    /// to poll(); it never renames, and stops once it has handed over a
    /// release, one replace per process. Each check keeps the prices of the
    /// models in use first, a failure handed over as a failed check. A dev
    /// build's check asks nothing.
    pub(crate) fn check_updates(&self, releases: Arc<dyn Releases>, every: Duration) {
        let (version, exe, repo, tx) = (
            self.version.clone(),
            self.cfg.exe.clone(),
            self.cfg.repo.clone(),
            self.update_sender.clone(),
        );
        thread::spawn(move || loop {
            if let Err(err) = update::refresh_prices(&*releases, &version, &repo) {
                if tx.send(Err(format!("prices: {err}"))).is_err() {
                    return;
                }
            }
            let checked = update::check(&*releases, &version, &exe);
            let done = matches!(checked, Ok(Some(_)));
            if tx.send(checked).is_err() || done {
                return;
            }
            thread::sleep(every);
        });
    }

    /// The graphify thread, while the switch is on at open: one pass in the
    /// checkout now, then one every `every`, each handed to poll(). A switch
    /// turned off since skips the pass and hands over no tag; the thread
    /// stops once the Screen is gone.
    pub(crate) fn refresh_graphify(&self, every: Duration) {
        if !app::graphify(&self.cfg.repo) {
            return;
        }
        let (tools, repo, tx, docs_live) = (
            self.cfg.tools.clone(),
            self.cfg.repo.clone(),
            self.graphify_sender.clone(),
            self.docs_live.clone(),
        );
        thread::spawn(move || loop {
            let pass = match app::graphify(&repo) {
                true => graphify::pass(&*tools, &repo, &docs_live),
                false => graphify::Pass::default(),
            };
            if tx.send(pass).is_err() {
                return;
            }
            thread::sleep(every);
        });
    }

    /// A new X.Y tag's Question, queued with the others; none while a
    /// Question or a pass for a tag is live. Never saved: a check after a
    /// restart asks again, as nothing was recorded.
    // ponytail: a tag found in the demo is dropped, a yes there would start a
    // real session; the next check asks
    fn ask_docs_pass(&mut self, tag: String) {
        let asked = self.questions.iter().any(is_docs_pass);
        // a no answered since the check read the tags has handled it
        let handled = !graphify::is_new(graphify::handled(&self.cfg.repo), &tag);
        if asked || handled || self.docs_tag.is_some() || self.demo.is_some() {
            return;
        }
        if self.questions.is_empty() {
            self.hidden = false;
        }
        self.questions.push(Question {
            ticket: None,
            text: format!("{tag} tagged: run the graphify docs pass now?"),
            about: About::Confirm(Pending::DocsPass { tag }),
            cursor: 1,
            scroll: Cell::new(0),
            asked: chrono::Local::now(),
            opened: OnceCell::new(),
        });
    }

    /// The Docs pass on tag, on a thread of its own that hands its lines to
    /// poll(); it never holds the run.
    fn run_docs_pass(&mut self, tag: String) {
        self.docs_tag = Some(tag.clone());
        *self.docs_tab.lock().unwrap() = graphify::DocsTab::default();
        let (tools, repo, workspace, tick, tx, docs_tab, docs_live) = (
            self.cfg.tools.clone(),
            self.cfg.repo.clone(),
            self.cfg.workspace.clone(),
            self.cfg.tick,
            self.docs_sender.clone(),
            self.docs_tab.clone(),
            self.docs_live.clone(),
        );
        thread::spawn(move || {
            // waits out a check's graph refresh under way, so the pass
            // starts after its graph.json
            *docs_live.lock().unwrap() = true;
            let say = |line| _ = tx.send((line, false));
            let last = graphify::docs_pass(
                &*tools,
                &repo,
                &workspace,
                &tag,
                tick,
                graphify::DOCS_PASS_LIMIT,
                &say,
                &docs_tab,
            );
            *docs_live.lock().unwrap() = false;
            let _ = tx.send((last, true));
        });
    }

    /// A check's outcome: a failure is one line; a release installs at once
    /// on an idle Shell, or waits for the run to release the lock.
    fn updated(&mut self, checked: Checked) {
        match checked {
            Err(err) => self.say(&format!("update check failed: {err}")),
            Ok(None) => {}
            Ok(Some(ready)) => {
                self.update = Some(ready);
                match &self.run {
                    Some(_) => self.say(&format!(
                        "{} downloaded, installs when the run stops",
                        self.update.as_ref().unwrap().tag
                    )),
                    None => self.install(true),
                }
            }
        }
    }

    /// The pending rename over the exe, under the repo lock, which no run of
    /// this Shell may hold: another process's run keeps it pending, tried
    /// again from tick() a minute on. On an idle Shell a done rename says
    /// 'updating to vX' and quits for open() to re-exec; at the end of a run
    /// it says nothing. A refused rename is one line, the temp file gone.
    fn install(&mut self, reexec: bool) {
        let Some(ready) = self.update.take() else {
            return;
        };
        let tag = ready.tag.clone();
        let installed = match acquire_lock(&self.cfg.repo) {
            Ok(_lock) => ready.install(),
            Err(_) => {
                self.update = Some(ready);
                self.retry = Instant::now() + RETRY;
                return;
            }
        };
        match installed {
            Ok(()) if reexec => {
                self.say(&format!("updating to {tag}"));
                self.quit = true;
                self.reexec = true;
            }
            Ok(()) => {}
            Err(err) => self.say(&format!("update check failed: {err}")),
        }
    }

    /// The header's version: with a release waiting on the run, where it goes.
    pub(crate) fn shown_version(&self) -> String {
        match &self.update {
            Some(ready) => format!("{} → {} at stop", self.version, ready.tag),
            None => self.version.clone(),
        }
    }

    /// The Shell's last act, after the terminal is back: the run goes, its
    /// lock with it, and then the release that waited on it installs (/exit
    /// and Ctrl-C twice, with no re-exec). Another process's lock drops it.
    /// A live Docs pass is interrupted, an idle-Shell update's re-exec too:
    /// its tab closes, its session with it, and nothing is recorded, so the
    /// next check asks again.
    pub(crate) fn close(&mut self) {
        // a live charting pane stays running in herdr
        self.brainstorm_stop.store(true, Ordering::SeqCst);
        for (_, driver) in self.brainstorm_threads.drain(..) {
            let _ = driver.join();
        }
        // cancelled under the lock the pass records under: it records nothing
        // after, and closes a tab it makes after
        let tab = {
            let mut docs = self.docs_tab.lock().unwrap();
            docs.cancelled = true;
            docs.tab.take()
        };
        if let (Some(_), Some(tab)) = (&self.docs_tag, tab) {
            let _ = herdr(&*self.cfg.tools, &self.cfg.repo, &["tab", "close", &tab]);
        }
        if let Some(mut run) = self.run.take() {
            run.o.stop();
            if let Some(scheduler) = run.scheduler.take() {
                let _ = scheduler.join();
            }
            self.withdraw(&run);
        }
        self.install(false);
        if let Some(ready) = self.update.take() {
            ready.discard();
        }
    }

    /// The bd cache again: on open, on every /start-epic, after a Ticket
    /// closes, when a run ends and before the Epic summary opens by
    /// itself. Whether bd answered.
    fn reload_epics(&mut self) -> bool {
        self.reload_issues().is_some()
    }

    /// reload_epics, handing back every issue bd listed; None when bd failed.
    fn reload_issues(&mut self) -> Option<Vec<BdIssue>> {
        match bd_list(&self.cfg.repo, &*self.cfg.tools) {
            Ok(issues) => {
                let (epics, brainstorm_issues) = load_epics(issues.clone(), &self.state.queue);
                self.epics = epics;
                self.brainstorm_issues = brainstorm_issues;
                Some(issues)
            }
            Err(err) => {
                self.notice(&format!("bd list failed: {err}"), NOTICE_WINDOW);
                None
            }
        }
    }

    /// Takes the Events and snapshots the live run's State, after the
    /// scheduler's end so a finished run's snapshot is its last; the summary
    /// opens by itself once every Ticket of the run has its PR, or at the
    /// end of a run that ends in a Release. Once the
    /// scheduler thread has returned the run is stopping until every Ticket
    /// thread has left (each sees stop at its next sleep, and still saves
    /// state after its current Tools call); then the run is over, the lock
    /// goes, a stopped run says so, and a done run clears the saved run, a
    /// done Epic asking whether to close the Epic.
    pub(crate) fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            self.push(event);
        }
        while let Ok(checked) = self.update_receiver.try_recv() {
            self.updated(checked);
        }
        while let Ok((failed, tag)) = self.graphify_receiver.try_recv() {
            for line in failed {
                self.say(&line);
            }
            if let Some(tag) = tag {
                self.ask_docs_pass(tag);
            }
        }
        self.brainstorm_updates();
        // the live charting's driver gone: its pane closed, its session
        // dead, or never started; a Map's gone holding its session was
        // interrupted, while one with none waits on research or a Question.
        // Read again once it is gone: what it sent before it ended counts.
        let ended = self.live.clone().filter(|idea| !self.driving(idea));
        if ended.is_some() {
            self.brainstorm_updates();
        }
        if let (Some(_), Some(b)) = (ended, self.live_brainstorm()) {
            if b.phase == Phase::Charting || b.session.is_some() {
                self.live = None;
            }
        }
        self.open_charted();
        while let Ok((line, over)) = self.docs_receiver.try_recv() {
            self.say(&line);
            if over {
                self.docs_tag = None;
            }
        }
        while let Ok(rung) = self.ring_receiver.try_recv() {
            self.pushes -= 1;
            if let Err(err) = rung {
                self.refuse(&format!("on call: push failed: {err}"));
            }
        }
        self.probed();
        self.finished();
        let Some(run) = &mut self.run else {
            return;
        };
        if run.scheduler.as_ref().is_some_and(JoinHandle::is_finished) {
            let outcome = run.scheduler.take().unwrap().join();
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    run.failed = true;
                    self.notice(&err, NOTICE_WINDOW);
                }
                Err(_) => {
                    run.o.stop();
                    self.notice("the scheduler thread died", NOTICE_WINDOW);
                }
            }
        }
        let before = self.to_unblock();
        self.state = self.run.as_ref().unwrap().o.state.lock().unwrap().clone();
        self.ring_unblock(&before);
        // the cache again once it says done: a Ticket added since has no PR
        let run = self.run.as_ref().unwrap();
        if !run.summarized && self.all_prs_open() && self.reload_epics() && self.all_prs_open() {
            let release = self.ends_in_release();
            let run = self.run.as_mut().unwrap();
            (run.summarized, run.release) = (true, release);
            if !release {
                self.summarize_run();
            }
        }
        let over = self
            .run
            .as_ref()
            .is_some_and(|r| r.scheduler.is_none() && r.o.active.lock().unwrap().is_empty());
        if !over {
            return;
        }
        let run = self.run.take().unwrap();
        // it stopped by itself: a long usage limit, or the scheduler's error,
        // which rings even after the summary rang run done
        if run.failed || run.o.closed() {
            self.ring_end("run stopped");
        }
        self.running = false;
        self.withdraw(&run);
        // never saved: derived again on resume
        let kept = |q: &Question| q.ticket.is_none() || q.brainstorms();
        // composing is the front Question's: kept with it
        self.composing &= self.questions.first().is_some_and(kept);
        self.questions.retain(kept);
        self.first = None;
        if run.o.stopping() {
            // a long usage limit has said it closed the panes
            if !run.o.closed() {
                self.say("stopped, panes left running, /continue resumes");
            }
        } else if run.failed {
            self.state = load_state(&self.cfg.repo).unwrap_or_default();
        } else {
            // the summary held for the Release: after it, or with none after all
            if run.release || self.state.release.is_some() {
                self.summarize_run();
            }
            let done = std::mem::take(&mut self.state); // run done: nothing to resume
            if let Err(err) = self.state.save(&self.cfg.repo) {
                self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
            self.last = done.clone();
            if run.epic {
                let epic = &done.epic;
                let text = match self.epics.iter().find(|e| e.id == *epic) {
                    Some(e) => format!("close Epic {epic} {}?", e.title),
                    None => format!("close Epic {epic}?"),
                };
                let pending = Pending::Close {
                    done,
                    commented: false,
                };
                self.ask_close(&text, pending);
            }
        }
        self.reload_epics();
        drop(run); // the lock goes
        self.install(false); // the last act of /stop-work
    }

    /// The run's end, its scheduler returned: its unanswered approval
    /// modals go, and the offers the scheduler sent after the poll last
    /// read the Events, withdrawn from offered, so the next run's poll
    /// offers them again. The other Events still unread show.
    fn withdraw(&mut self, run: &Run) {
        let (unread, rest): (Vec<Event>, Vec<Event>) =
            self.receiver.try_iter().partition(|e| !e.offer.is_empty());
        for event in rest {
            self.push(event);
        }
        let items = |a: Approval| (a.ticket, a.rows.into_iter().map(|(i, _)| i).collect());
        let unread = unread
            .into_iter()
            .map(|e| (e.ticket.unwrap_or_default(), e.offer));
        run.o
            .withdraw(self.approvals.drain(..).map(items).chain(unread));
    }

    /// Whether every Ticket of the run (its Epic's on the bd tree, or the
    /// Ticket run's queue) has its PR open or merged or is Parked, one at
    /// least with its PR: the summary's DONE, from the live State.
    fn all_prs_open(&self) -> bool {
        let ids: Vec<&str> = match self.saved() {
            Some(epic) => epic.tickets.iter().map(|t| t.id.as_str()).collect(),
            None => self.state.queue.iter().map(String::as_str).collect(),
        };
        let closed = |id: &str| {
            let mut all = self.epics.iter().flat_map(|e| &e.tickets);
            all.any(|t| t.id == id && t.status == "closed")
        };
        let status = |id: &str| match self.state.tickets.get(id) {
            _ if closed(id) => STATUS_MERGED,
            Some(ts) => ts.status.as_str(),
            None => "",
        };
        let out = [STATUS_PR_OPEN, STATUS_MERGED, STATUS_PARKED];
        ids.iter().all(|id| out.contains(&status(id)))
            && ids.iter().any(|id| status(id) != STATUS_PARKED)
    }

    /// Whether the run ends in a Release: one already started, as
    /// release_due resumes it; else, read on the bd cache, releases on and
    /// orqa:release on its Epic, or on any Ticket of a Ticket run.
    fn ends_in_release(&self) -> bool {
        if self.state.release.is_some() {
            return true;
        }
        let labelled = match self.saved() {
            Some(epic) => release::carries(&epic.labels),
            None => (self.state.queue.iter())
                .any(|id| find(&self.epics, id).is_some_and(|t| release::carries(&t.labels))),
        };
        labelled && app::switch(&self.cfg.repo, &RELEASE_ON)
    }

    /// The saved run's Epic on the tree; never the no-Epic group.
    pub(crate) fn saved(&self) -> Option<&Epic> {
        let epic = &self.state.epic;
        self.epics
            .iter()
            .find(|e| !epic.is_empty() && e.id == *epic)
    }

    /// The scheduler has returned and Ticket threads are still leaving.
    pub(crate) fn stopping(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.scheduler.is_none())
    }

    pub(crate) fn tick(&mut self) {
        self.ticks += 1;
        if self.ctrl_c.is_some_and(|at| at.elapsed() >= CTRL_C_WINDOW) {
            self.ctrl_c = None;
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            self.notice = None;
        }
        if self
            .notices
            .first()
            .and_then(|n| n.closes)
            .is_some_and(|at| (self.cfg.clock)() >= at)
        {
            self.close_notice();
        }
        // a Notice modal covers the approval modal and takes its keys
        if self.notices.is_empty()
            && (self.approvals.first().and_then(|a| a.approves))
                .is_some_and(|at| (self.cfg.clock)() >= at)
        {
            self.decide(true, "the countdown");
        }
        if self.run.is_none() && self.update.is_some() && Instant::now() >= self.retry {
            self.install(true);
        }
        self.go_on_call();
        demo::tick(self);
    }

    /// On call turns on once a counted Question (a Ticket's, not a
    /// confirmation or the /continue checklist) has waited its minutes, the
    /// clock started again by the last end; never Away, with no token or in
    /// the demo. Every counted Question waiting rings.
    fn go_on_call(&mut self) {
        let away = self.cfg.away.load(Ordering::SeqCst);
        if self.calling || away || self.on_call.token.is_none() || self.demo.is_some() {
            return;
        }
        let now = (self.cfg.clock)();
        let minutes = self.on_call.minutes;
        let wait = chrono::Duration::minutes(minutes as i64);
        let waited = |q: &Question| {
            let since = self.off_since.map_or(q.asked, |off| off.max(q.asked));
            matches!(q.about, About::Asked(_)) && now - since >= wait
        };
        if !self.questions.iter().any(waited) {
            return;
        }
        self.calling = true;
        self.say(&format!(
            "on call: a Question has waited {minutes} minutes, pushing to your phone"
        ));
        for i in 0..self.questions.len() {
            self.ring_question(i);
        }
        self.ring_unblock(&[]);
    }

    /// Pushes Question `i` when it counts: its Ticket, kind and Ticket
    /// title; its own text and options never leave the Mac.
    fn ring_question(&mut self, i: usize) {
        let q = &self.questions[i];
        let (Some(id), About::Asked(ask)) = (&q.ticket, &q.about) else {
            return;
        };
        let kind = match ask {
            Ask::Wake { .. } => "Wake",
            Ask::Blocked { .. } => "Blocked session",
            Ask::Plan { .. } => "Plan to approve",
            Ask::PlanFailed { .. } => "Plan failed",
            Ask::Limited { .. } => "Usage limit",
            Ask::StageQuestion { .. } => "Stage question",
            Ask::Manual { .. } => "Manual work",
            Ask::TicketStart { .. } => "Start question",
            Ask::Labels { .. } => "Label question",
            Ask::Tag { .. } => "Tag question",
            Ask::ReleaseAgain { .. } => "Release question",
            Ask::Merge { .. } => "Merge question",
            Ask::NextWaypoint { .. } => "Waypoint question",
        };
        let mut message = format!("{id} · {kind}");
        if let Some(title) = self.title(id) {
            message = format!("{message} · {title}");
        }
        self.ring(message);
    }

    /// The Tickets whose PR MERGE TO UNBLOCK lists.
    fn to_unblock(&self) -> Vec<String> {
        let prs = draw::to_unblock(self).into_iter();
        prs.map(|(id, _, _)| id.to_string()).collect()
    }

    /// While On call, pushes each PR MERGE TO UNBLOCK lists but `before`
    /// did: its Ticket, the Tickets it unblocks and its title.
    fn ring_unblock(&mut self, before: &[String]) {
        if !self.calling {
            return;
        }
        let new: Vec<(String, String)> = (draw::to_unblock(self).into_iter())
            .filter(|(id, _, _)| !before.iter().any(|b| b == id))
            .map(|(id, _, waiting)| (id.to_string(), waiting.join(", ")))
            .collect();
        for (id, waiting) in new {
            let mut message = format!("{id} · Merge to unblock {waiting}");
            if let Some(title) = self.title(&id) {
                message = format!("{message} · {title}");
            }
            self.ring(message);
        }
    }

    /// Ends On call with a RECENT line saying why; the clock starts again.
    fn off_call(&mut self, why: &str) {
        if self.calling {
            self.calling = false;
            self.off_since = Some((self.cfg.clock)());
            self.say(&format!("on call: off, {why}"));
        }
    }

    /// While On call, the run's end: the Epic (or 'Ticket run'), what
    /// happened and the Epic's title.
    fn ring_end(&mut self, what: &str) {
        if !self.calling {
            return;
        }
        let message = match self.saved() {
            Some(e) => format!("{} · {what} · {}", e.id, e.title),
            None if self.state.epic.is_empty() => format!("Ticket run · {what}"),
            None => format!("{} · {what}", self.state.epic),
        };
        self.ring(message);
    }

    /// One push to the phone on a thread of its own, its outcome to poll().
    fn ring(&mut self, message: String) {
        let Some(token) = self.on_call.token.clone() else {
            return;
        };
        if self.demo.is_some() {
            return;
        }
        let (bell, tx) = (self.doorbell.clone(), self.ring_sender.clone());
        let title = format!("orqa · {}", self.folder);
        self.pushes += 1;
        thread::spawn(move || _ = tx.send(bell.ring(&token, &title, &message)));
    }

    /// An Event from the Orchestrator; only panel lines show. Any line of
    /// a Ticket closes the Question it had: the Ticket has moved on, and an
    /// answer sent to it meanwhile is dropped. A merge Question closed so
    /// is told to the Orchestrator, whose poll asks it again while it holds. A line that asks, or an ask
    /// with no line of its own, raises the Ticket's Question anew. A notice's
    /// line is shown with its info Notice modal, closing nothing.
    pub(crate) fn push(&mut self, event: Event) {
        if !event.offer.is_empty() {
            let minutes = app::count(&self.cfg.repo, &ADDRESS_PR_COMMENTS_COUNTDOWN) as u64;
            let countdown = (minutes > 0).then(|| Duration::from_secs(60 * minutes));
            let ticket = event.ticket.unwrap_or_default();
            return self.offer(ticket, event.offer, countdown, false);
        }
        // non-blocking Manual work: no Question, so the Ticket's stays open
        if let Some(text) = event.notice.clone() {
            self.show(event);
            return self.notify(NoticeKind::Info, &text, Some(MANUAL_NOTICE_WINDOW));
        }
        if !event.panel && event.ask.is_none() {
            return;
        }
        let (ticket, ask, text) = (event.ticket.clone(), event.ask.clone(), event.text.clone());
        let asked = event.time;
        if event.panel {
            self.show(event);
        }
        let Some(id) = ticket else {
            return;
        };
        if let Some(i) = self
            .questions
            .iter()
            .position(|q| q.ticket.as_deref() == Some(&id))
        {
            let closed = self.questions.remove(i);
            if i == 0 && self.composing {
                self.composing = false; // the prompt was for that Question
                self.input.clear();
            }
            if let Some(run) = &self.run {
                run.o.take_answer(&id, None);
                // a merge Question a line closed is the poll's to ask again
                if ask.is_none() && matches!(closed.about, About::Asked(Ask::Merge { .. })) {
                    run.o.unasked(&id);
                }
            }
        }
        let Some(ask) = ask else {
            return;
        };
        if self.questions.is_empty() {
            self.hidden = false;
        }
        // "stuck in fix 1", without the reason; a prompt line whole
        let short = match ask {
            Ask::Wake { .. } | Ask::PlanFailed { .. } => {
                text.split_once(": ").map_or(text.as_str(), |(s, _)| s)
            }
            Ask::Blocked { .. }
            | Ask::Plan { .. }
            | Ask::Limited { .. }
            | Ask::StageQuestion { .. }
            | Ask::Manual { .. }
            | Ask::TicketStart { .. }
            | Ask::Labels { .. }
            | Ask::Tag { .. }
            | Ask::ReleaseAgain { .. }
            | Ask::Merge { .. }
            | Ask::NextWaypoint { .. } => text.as_str(),
        };
        let asking = format!("asking you: {short}");
        // /continue @ticket's goes after a confirmation, and the Question
        // a prompt is being typed for, only
        let at = match self.first.as_deref() == Some(id.as_str()) {
            true => {
                self.first = None;
                self.hidden = false;
                let confirms = self.questions.iter().take_while(|q| q.ticket.is_none());
                confirms.count().max(usize::from(self.composing))
            }
            false => self.questions.len(),
        };
        self.questions.insert(
            at,
            Question {
                ticket: Some(id.clone()),
                text,
                about: About::Asked(ask),
                cursor: 0,
                scroll: Cell::new(0),
                asked,
                opened: OnceCell::new(),
            },
        );
        self.tell(Some(&id), &asking);
        if self.calling {
            self.ring_question(at);
        }
    }

    /// A line on RECENT.
    fn show(&mut self, event: Event) {
        self.events.push(event);
        // scrolled up, RECENT stays on the lines it shows
        if self.recent.get() > 0 {
            self.recent.set(self.recent.get() + 1);
        }
        if self.events.len() > KEPT_EVENTS {
            self.events.drain(..self.events.len() - KEPT_EVENTS);
        }
    }

    /// A line of the Shell's own, on RECENT and in the log as the
    /// Orchestrator's are, but for the demo's; None is a run-level line.
    fn tell(&mut self, ticket: Option<&str>, text: &str) {
        let time = chrono::Local::now();
        if self.demo.is_none() {
            append_log(&self.cfg.repo, time, ticket.unwrap_or(""), text);
        }
        self.show(Event {
            time,
            ticket: ticket.map(str::to_string),
            text: text.to_string(),
            panel: true,
            ask: None,
            offer: Vec::new(),
            notice: None,
        });
    }

    fn say(&mut self, text: &str) {
        self.tell(None, text);
    }

    /// A refused command: said, and a notice above the input line.
    fn refuse(&mut self, text: &str) {
        self.say(text);
        self.notice(text, NOTICE_WINDOW);
    }

    /// Whether a run is live or stopping, which refuses a start; said so.
    fn busy(&mut self) -> bool {
        match &self.run {
            None => false,
            Some(run) if run.scheduler.is_none() => {
                self.refuse("refused: a run is stopping");
                true
            }
            Some(_) => {
                self.refuse("refused: a run is live, /stop-work first");
                true
            }
        }
    }

    /// Whether a Ticket of the live run waits on the user: it has a Question
    /// waiting, or its last panel line is a trust dialog.
    // ponytail: the trust wait is read off the last Event rather than kept as state.
    pub(crate) fn blocked(&self, id: &str) -> bool {
        self.questions
            .iter()
            .any(|q| q.ticket.as_deref() == Some(id))
            || self
                .events
                .iter()
                .rev()
                .find(|e| e.ticket.as_deref() == Some(id))
                .is_some_and(|e| {
                    e.text.starts_with("waiting: ") && e.text.contains(" does not trust ")
                })
    }

    /// Whether the front Question shows above the input line.
    pub(crate) fn showing(&self) -> bool {
        !self.questions.is_empty() && !self.hidden
    }

    /// Whether the front Question docks in the modal: a plan, a Wake, a
    /// Stage's own question, Manual work, the Ticket's labels or its PR's
    /// merge.
    pub(crate) fn modal(&self) -> bool {
        self.showing()
            && matches!(
                self.questions[0].about,
                About::Asked(
                    Ask::Plan { .. }
                        | Ask::Wake { .. }
                        | Ask::StageQuestion { .. }
                        | Ask::Manual { .. }
                        | Ask::Labels { .. }
                        | Ask::Merge { .. }
                )
            )
    }

    /// The front Question's options, numbered in this order.
    pub(crate) fn options(&self) -> Vec<String> {
        let Some(q) = self.questions.first() else {
            return Vec::new();
        };
        match &q.about {
            About::Asked(Ask::Wake { actions, .. }) => actions
                .iter()
                .map(|action| action.option())
                .chain(["open the pane", "a prompt of your own"].map(str::to_string))
                .collect(),
            About::Asked(Ask::Blocked { .. }) => ["open the pane", "park", "I answered it"]
                .map(str::to_string)
                .to_vec(),
            About::Asked(Ask::Plan { feedback, .. }) => ["approve", "feedback of your own"]
                .map(str::to_string)
                .into_iter()
                .chain(
                    feedback
                        .iter()
                        .map(|f| format!("resend your feedback: {f}")),
                )
                .chain(["park", "open the pane"].map(str::to_string))
                .collect(),
            About::Asked(Ask::PlanFailed { feedback, .. }) => {
                ["open the pane", "park", "retry with a fresh session"]
                    .map(str::to_string)
                    .into_iter()
                    .chain(
                        feedback
                            .iter()
                            .map(|f| format!("resend your feedback: {f}")),
                    )
                    .collect()
            }
            About::Asked(Ask::Limited {
                fallback,
                extra_review,
                ..
            }) => ["wait for the reset".to_string()]
                .into_iter()
                .chain(fallback.iter().map(|f| format!("review with {f}")))
                .chain([format!(
                    "open the PR unreviewed{}",
                    if *extra_review {
                        ", the extra review skipped too"
                    } else {
                        ""
                    }
                )])
                .collect(),
            About::Asked(Ask::StageQuestion { options, .. }) => options
                .iter()
                .cloned()
                .chain(["an answer of your own", "open the pane", "park"].map(str::to_string))
                .collect(),
            About::Asked(Ask::Manual { .. }) => ["done", "done, with facts of your own", "park"]
                .map(str::to_string)
                .to_vec(),
            About::Asked(
                Ask::TicketStart { options }
                | Ask::Labels { options }
                | Ask::Tag { options, .. }
                | Ask::ReleaseAgain { options }
                | Ask::Merge { options, .. }
                | Ask::NextWaypoint { options, .. },
            ) => options.clone(),
            About::Confirm(_) => ["yes", "no"].map(str::to_string).to_vec(),
            About::Continue { rows } => rows
                .iter()
                .map(|(id, reset)| {
                    let ts = self.state.tickets.get(id).cloned().unwrap_or_default();
                    let stage = if ts.round > 0 {
                        format!("{} {}", ts.stage, ts.round)
                    } else {
                        ts.stage.clone()
                    };
                    let parked = if ts.status == STATUS_PARKED {
                        format!("  parked: {}", ts.reason)
                    } else {
                        String::new()
                    };
                    let how = if *reset {
                        "reset to Implement"
                    } else {
                        "resume"
                    };
                    format!("{}  {stage}{parked}  → {how}", self.name(id))
                })
                .collect(),
        }
    }

    /// A Ticket as RECENT names it: its suffix and title, or its id alone.
    pub(crate) fn name(&self, id: &str) -> String {
        match self.title(id) {
            Some(title) => format!("{} {title}", suffix(id)),
            None => id.to_string(),
        }
    }

    /// The list open above the input line, each row what it fills in, its
    /// middle column and its text; empty when none is. '/' with no space yet
    /// lists the commands containing it, else those it is a subsequence of.
    /// '@<query>' ending the line lists the open Epics and Tickets whose id
    /// contains it, then whose title does, then whose id it is a subsequence
    /// of; after a command, only what it takes, after /continue what it
    /// continues. None opens on a prompt of your own.
    pub(crate) fn list(&self) -> Vec<(String, &'static str, String)> {
        if self.composing {
            return Vec::new();
        }
        let input = self.input.to_lowercase();
        if input.starts_with('/') && !input.contains(' ') {
            let containing: Vec<_> = COMMANDS
                .into_iter()
                .filter(|c| c.0.contains(&input))
                .collect();
            let commands = match containing.is_empty() {
                false => containing,
                true => COMMANDS
                    .into_iter()
                    .filter(|c| subsequence(&input, c.0))
                    .collect(),
            };
            return commands
                .into_iter()
                .map(|(name, args, text)| (name.to_string(), args, text.to_string()))
                .collect();
        }
        let Some((before, q)) = input
            .rsplit_once('@')
            .filter(|(before, q)| !q.contains(' ') && (before.is_empty() || before.ends_with(' ')))
        else {
            return Vec::new();
        };
        // What the command before it takes, by its args in COMMANDS.
        let command = before.split(' ').next();
        let takes = COMMANDS
            .iter()
            .find(|c| Some(c.0) == command)
            .map_or("", |c| c.1);
        let (epics, tickets) = (!takes.contains("<ticket>"), !takes.contains("<epic>"));
        let mut found = Vec::new();
        if command == Some("/continue") {
            found = self.continue_rows();
        } else {
            for e in &self.epics {
                if epics && !e.id.is_empty() {
                    found.push((e.id.clone(), "Epic", e.title.clone()));
                }
                for t in e.tickets.iter().filter(|t| tickets && t.status != "closed") {
                    found.push((t.id.clone(), "Ticket", t.title.clone()));
                }
            }
        }
        let rank = |(id, _, title): &(String, &str, String)| {
            let (id, title) = (id.to_lowercase(), title.to_lowercase());
            [id.contains(q), title.contains(q), subsequence(q, &id)]
                .iter()
                .position(|hit| *hit)
        };
        found.retain(|row| rank(row).is_some());
        found.sort_by_key(rank);
        found
    }

    /// What /continue @ takes: each saved Map and its Waypoints, closed
    /// ones last, each
    /// charting Idea, then each Parked Ticket, every text ending in its
    /// state; a Waypoint ready to take ends in its title.
    fn continue_rows(&self) -> Vec<(String, &'static str, String)> {
        let issues = &self.brainstorm_issues;
        let title = |id: &str| {
            issues
                .iter()
                .find(|i| i.id == id)
                .map_or("", |i| i.title.as_str())
        };
        let mut rows = Vec::new();
        let saved = self.brainstorms.iter().filter(|b| b.phase == Phase::Map);
        for b in saved {
            let live = self.live.as_ref() == Some(&b.idea);
            let mut waypoints: Vec<&BdIssue> =
                issues.iter().filter(|i| i.parent == b.map).collect();
            waypoints.sort_by_key(|i| (i.status == "closed", suffix_order(&i.id)));
            let left = waypoints.iter().filter(|i| i.status != "closed").count();
            let counted = format!("{} open", plural(left, "Waypoint"));
            let state = match (live, b.started) {
                (true, _) => format!("{counted}, live"),
                (false, false) => "not started".to_string(),
                (false, true) => format!("{counted}, stopped"),
            };
            rows.push((b.map.clone(), "Map", format!("{} · {state}", title(&b.map))));
            for w in waypoints {
                let research = b.research.iter().find(|r| r.waypoint == w.id);
                let blockers: Vec<&str> = w
                    .blockers()
                    .filter(|id| unfinished(&self.epics, issues, id))
                    .map(suffix)
                    .collect();
                let text = if w.status == "closed" {
                    format!("{} · closed", w.title)
                } else if let Some(r) = research {
                    match r.parked {
                        true => format!("{} · parked, asks you", w.title),
                        false => format!("{} · researching", w.title),
                    }
                } else if !blockers.is_empty() {
                    format!("{} · blocked on {}", w.title, blockers.join(", "))
                } else if brainstorm::labelled(w, brainstorm::EPIC) && left > 1 {
                    format!("{} · last, {} others open", w.title, left - 1)
                } else {
                    w.title.clone() // ready to take
                };
                rows.push((w.id.clone(), "Waypoint", text));
            }
        }
        for b in self
            .brainstorms
            .iter()
            .filter(|b| b.phase == Phase::Charting)
        {
            let state = match self.live.as_ref() == Some(&b.idea) {
                true => "charting",
                false => "charting stopped",
            };
            rows.push((
                b.idea.clone(),
                "Idea",
                format!("{} · {state}", title(&b.idea)),
            ));
        }
        let mut parked: Vec<&String> = self
            .state
            .tickets
            .iter()
            .filter(|(_, ts)| ts.status == STATUS_PARKED)
            .map(|(id, _)| id)
            .collect();
        parked.sort_by_key(|id| suffix_order(id));
        for id in parked {
            let text = format!("{} · parked", self.title(id).unwrap_or(id));
            rows.push((id.clone(), "Ticket", text));
        }
        rows
    }

    /// Tab or Enter on an open list: a command fills in as '<command> ', an
    /// Epic or Ticket id in place of '@<query>', a space after either.
    fn fill(&mut self, picked: &str) {
        let at = self.input.rfind('@').unwrap_or(0);
        self.input.truncate(at);
        self.input.push_str(picked);
        self.input.push(' ');
        self.back = 0;
    }

    /// The input line's cursor as a byte index into it.
    pub(crate) fn at(&self) -> usize {
        let n = self.input.chars().count().saturating_sub(self.back);
        self.input
            .char_indices()
            .nth(n)
            .map_or(self.input.len(), |(i, _)| i)
    }

    pub(crate) fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let held = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        // a cleared or taken line leaves the cursor past its start
        self.back = self.back.min(self.input.chars().count());
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        // A Notice modal takes every key before the approval modal, the
        // summary, /config, a Question or the input: Enter or Esc closes it,
        // ↑↓ scroll a long one; a yes/no one takes y or n, ←→ or Tab move,
        // Enter answers and Esc says no. Any other key stops its countdown
        // for good and is swallowed, but Ctrl-C goes on to exit.
        if let Some(n) = self.notices.first_mut() {
            let ask = n.yes.is_some();
            match key.code {
                KeyCode::Char('y') if ask => return self.answer_notice(true),
                KeyCode::Char('n') | KeyCode::Esc if ask => return self.answer_notice(false),
                KeyCode::Enter if ask => {
                    let yes = n.cursor == 0;
                    return self.answer_notice(yes);
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab if ask => {
                    n.cursor ^= 1
                }
                KeyCode::Enter | KeyCode::Esc => return self.close_notice(),
                KeyCode::Up => scroll_by(&n.scroll, -1),
                KeyCode::Down => scroll_by(&n.scroll, 1),
                _ => {}
            }
            n.closes = None;
            if !ctrl_c {
                return;
            }
        }
        // The approval modal takes every key before the summary, /config, a
        // Question or the input: ↑↓ move, Space toggles a row, Enter fixes
        // the checked ones, Esc cancels. Any key stops its countdown for
        // good; Ctrl-C goes on to exit.
        if let Some(a) = self.approvals.first_mut() {
            a.approves = None;
            match key.code {
                KeyCode::Up => a.cursor = a.cursor.saturating_sub(1),
                KeyCode::Down => a.cursor = (a.cursor + 1).min(a.rows.len() - 1),
                KeyCode::Char(' ') => a.rows[a.cursor].1 ^= true,
                KeyCode::Enter => return self.decide(true, "you"),
                KeyCode::Esc => return self.decide(false, "you"),
                _ => {}
            }
            if !ctrl_c {
                return;
            }
        }
        // The /manual-work modal, as the /continue checklist: ↑↓ move,
        // Space checks a row that does not block, Enter marks the checked
        // done, Esc closes; Ctrl-C goes on.
        if let Some(m) = &mut self.manual_work {
            match key.code {
                KeyCode::Up => m.cursor = m.cursor.saturating_sub(1),
                KeyCode::Down => m.cursor = (m.cursor + 1).min(m.rows.len() - 1),
                KeyCode::Char(' ') => match m.rows.get_mut(m.cursor) {
                    Some((_, item, on)) if !item.blocks => *on = !*on,
                    _ => {}
                },
                KeyCode::Enter => return self.mark_done(),
                KeyCode::Esc => self.manual_work = None,
                _ => {}
            }
            if !ctrl_c {
                return;
            }
        }
        if self.idea.is_some() && !ctrl_c {
            return self.idea_key(key);
        }
        if self.tickets.is_some() && !ctrl_c {
            return self.tickets_key(key);
        }
        if self.start_map.is_some() && !ctrl_c {
            return self.start_map_key(key);
        }
        if ctrl_c {
            if self.ctrl_c.is_some_and(|at| at.elapsed() < CTRL_C_WINDOW) {
                self.quit();
            } else {
                self.ctrl_c = Some(Instant::now());
                self.notice("press Ctrl-C again to exit", CTRL_C_WINDOW);
            }
            return;
        }
        // The Epic summary reads like the plan and takes nothing else; Esc
        // closes it, or raises the close-Epic Notice modal held over it.
        if let Some(summary) = &self.summary {
            match key.code {
                KeyCode::Esc => match self.held.take() {
                    Some(n) => self.raise(n),
                    None => self.summary = None,
                },
                code => self.scroll_rows(&summary.scroll, code),
            }
            return;
        }
        if self.settings.is_some() {
            return self.config_key(key.code, held);
        }
        // With a Question showing and the input line empty the keys are
        // its: arrows or a number pick, Enter answers, Esc hides or cancels,
        // Space toggles a /continue row, y and n answer a confirmation; a
        // plan reads like a pager, ↑↓ a line, PageUp, PageDown and Space a
        // page, Home and End, Tab and Shift-Tab heading to heading, and ←→
        // pick; PageUp and PageDown page a docked Wake's or Stage question's
        // body. A slash starts a command.
        if self.showing() && self.input.is_empty() && !self.composing {
            let n = self.options().len();
            let confirm = matches!(self.questions[0].about, About::Confirm(_));
            let plan = matches!(self.questions[0].about, About::Asked(Ask::Plan { .. }));
            let docked = self.modal();
            let q = &mut self.questions[0];
            match key.code {
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Char(' ')
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Tab
                | KeyCode::BackTab
                    if plan =>
                {
                    self.scroll_rows(&self.questions[0].scroll, key.code)
                }
                KeyCode::PageUp | KeyCode::PageDown if docked => self.page(key.code),
                KeyCode::Up | KeyCode::Left => q.cursor = q.cursor.saturating_sub(1),
                KeyCode::Down | KeyCode::Right => q.cursor = (q.cursor + 1).min(n - 1),
                KeyCode::Char(c @ '0'..='9') => {
                    if let Some(i) = (c as usize).checked_sub('1' as usize).filter(|i| *i < n) {
                        q.cursor = i;
                    }
                }
                KeyCode::Char(' ') => {
                    if let About::Continue { rows } = &mut q.about {
                        rows[q.cursor].1 ^= true;
                    }
                }
                KeyCode::Char('y') if confirm => self.answer(0),
                KeyCode::Char('n') if confirm => self.answer(1),
                KeyCode::Enter => {
                    let cursor = q.cursor;
                    self.answer(cursor);
                }
                // Esc hides a Ticket's Question; it cancels a confirmation
                // or the /continue checklist.
                KeyCode::Esc if q.ticket.is_some() => self.hidden = true,
                KeyCode::Esc => {
                    self.questions.remove(0);
                    self.notice("cancelled", NOTICE_WINDOW);
                }
                KeyCode::Char(c) if !held => {
                    self.input.push(c);
                    self.pick = 0;
                }
                _ => {}
            }
            return;
        }
        // With a list open Up and Down move its cursor, Tab fills in its row
        // and so does Enter, but on a command typed whole that needs no
        // argument Enter runs it. Up on an empty line recalls the last
        // command, then Up and Down walk the history, past its newest back to
        // an empty line, until another key takes the line. Shift with Up and
        // Down scrolls RECENT; on an empty line PageUp and PageDown TICKETS.
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Up => return scroll_by(&self.recent, 1),
                KeyCode::Down => return scroll_by(&self.recent, -1),
                _ => {}
            }
        }
        let recall = self.recall.take();
        let (open, picked, whole, pick) = {
            let list = self.list();
            // a reloaded bd cache may have shortened the list under the cursor
            let pick = self.pick.min(list.len().saturating_sub(1));
            let row = list.get(pick);
            // an optional argument ([...]) may be left out
            let whole =
                row.is_some_and(|(name, args, _)| !args.starts_with('<') && *name == self.input);
            (list.len(), row.map(|row| row.0.clone()), whole, pick)
        };
        self.pick = pick;
        match key.code {
            KeyCode::Char(_) if held => {}
            KeyCode::Char(c) => {
                let at = self.at();
                self.input.insert(at, c);
                self.pick = 0;
            }
            KeyCode::Up if recall.is_some() || (self.input.is_empty() && !self.composing) => {
                let i = recall.unwrap_or(self.history.len()).saturating_sub(1);
                if let Some(line) = self.history.get(i) {
                    self.input.clone_from(line);
                    self.recall = Some(i);
                    self.back = 0;
                }
            }
            KeyCode::Down if recall.is_some() => {
                let i = recall.map_or(0, |i| i + 1);
                self.input = self.history.get(i).cloned().unwrap_or_default();
                self.recall = Some(i).filter(|i| *i < self.history.len());
                self.back = 0;
            }
            KeyCode::Up if open > 0 => self.pick = self.pick.saturating_sub(1),
            KeyCode::Down if open > 0 => self.pick = (self.pick + 1).min(open - 1),
            KeyCode::Tab
                if self.input.is_empty() && !self.composing && self.suggestion.is_some() =>
            {
                self.input = self.suggestion.clone().unwrap_or_default();
                self.back = 0;
            }
            KeyCode::Tab if picked.is_some() => self.fill(&picked.unwrap()),
            KeyCode::Enter if picked.is_some() && !whole => self.fill(&picked.unwrap()),
            KeyCode::PageDown | KeyCode::PageUp if self.composing => {
                if self.modal() {
                    self.page(key.code);
                }
            }
            KeyCode::PageDown if self.input.is_empty() => scroll_by(&self.scroll, 10),
            KeyCode::PageUp if self.input.is_empty() => scroll_by(&self.scroll, -10),
            KeyCode::Left => self.back = (self.back + 1).min(self.input.chars().count()),
            KeyCode::Right => self.back = self.back.saturating_sub(1),
            KeyCode::Backspace => {
                let at = self.at();
                if let Some(c) = self.input[..at].chars().next_back() {
                    self.input.remove(at - c.len_utf8());
                }
                self.pick = 0;
            }
            KeyCode::Esc if self.composing => {
                self.composing = false;
                self.input.clear();
            }
            KeyCode::Esc if self.input.is_empty() => self.hidden = false,
            KeyCode::Esc => self.input.clear(),
            KeyCode::Enter if self.composing => {
                let prompt = std::mem::take(&mut self.input);
                if !prompt.trim().is_empty() {
                    self.composing = false;
                    let word = match self.questions[0].about {
                        About::Asked(Ask::Plan { .. }) => "feedback",
                        About::Asked(Ask::StageQuestion { .. }) => "your answer",
                        About::Asked(Ask::Manual { .. }) => "your facts",
                        _ => "your prompt",
                    };
                    self.reply(word, Answer::Prompt(prompt.trim().to_string()));
                    self.off_call("you answered");
                }
            }
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.input);
                let line = line.trim();
                if !line.is_empty() && self.history.last().is_none_or(|last| last != line) {
                    self.history.push(line.to_string());
                    // ponytail: the file only grows, a line per command typed;
                    // cut it at open if it ever gets long
                    if let Some(file) = &self.history_file {
                        let file = File::options().create(true).append(true).open(file);
                        let _ = file.and_then(|mut file| writeln!(file, "{line}"));
                    }
                }
                self.command(line);
            }
            _ => {}
        }
    }

    /// The wheel scrolls the box under the pointer a row: the Epic summary,
    /// the docked plan, Wake or Stage's question, RECENT or TICKETS. It
    /// never moves a Question's answer or a list's cursor, and /config and a
    /// Notice modal take none of it. Clicks do nothing.
    pub(crate) fn mouse(&mut self, m: MouseEvent) {
        let down = match m.kind {
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::ScrollDown => 1,
            _ => return,
        };
        if !self.notices.is_empty() {
            return;
        }
        let pointer = Position::new(m.column, m.row);
        let under = |area: &Cell<Rect>| area.get().contains(pointer);
        // RECENT and a Wake's tail count their rows up from the newest.
        let (rows, by) = if let Some(summary) = &self.summary {
            (&summary.scroll, down)
        } else if self.settings.is_some() {
            return;
        } else if self.modal() && under(&self.dock_area) {
            let q = &self.questions[0];
            match q.about {
                About::Asked(Ask::Wake { .. }) => (&q.scroll, -down),
                _ => (&q.scroll, down),
            }
        } else if under(&self.recent_area) {
            (&self.recent, -down)
        } else if under(&self.tickets_area) {
            (&self.scroll, down)
        } else {
            return;
        };
        scroll_by(rows, by);
    }

    /// The reading keys of the plan modal and the Epic summary over its
    /// first row shown: a line, a page, either end, the next or previous
    /// heading (a Ticket, in the summary); any other key none. The draw
    /// keeps the row inside.
    fn scroll_rows(&self, scroll: &Cell<usize>, code: KeyCode) {
        let (row, page) = (scroll.get(), self.page.get());
        let heads = self.heads.borrow();
        scroll.set(match code {
            KeyCode::Up => row.saturating_sub(1),
            KeyCode::Down => row.saturating_add(1),
            KeyCode::PageUp => row.saturating_sub(page),
            KeyCode::Home => 0,
            KeyCode::End => usize::MAX,
            KeyCode::Tab => heads.iter().copied().find(|&h| h > row).unwrap_or(row),
            KeyCode::BackTab => heads.iter().copied().rfind(|&h| h < row).unwrap_or(0),
            KeyCode::PageDown | KeyCode::Char(' ') => row.saturating_add(page),
            _ => row,
        });
    }

    /// PageUp or PageDown over the docked Question's body: a Wake's tail
    /// counts its rows up from its last, so the two swap there.
    fn page(&self, code: KeyCode) {
        let q = &self.questions[0];
        let code = match (&q.about, code) {
            (About::Asked(Ask::Wake { .. }), KeyCode::PageUp) => KeyCode::PageDown,
            (About::Asked(Ask::Wake { .. }), KeyCode::PageDown) => KeyCode::PageUp,
            (_, code) => code,
        };
        self.scroll_rows(&q.scroll, code);
    }

    /// The user picked option `choice` of the front Question.
    fn answer(&mut self, choice: usize) {
        // an answer takes the Question; open the pane and composing leave it
        let waiting = self.questions.len();
        match (&self.questions[0].about, choice) {
            // a Wake's actions, then open the pane and a prompt of your own
            (About::Asked(Ask::Wake { actions, pane, .. }), _) => match actions.get(choice) {
                Some(&action) => self.reply(action.word(), Answer::Act(action)),
                None if choice == actions.len() => self.open_pane(pane.clone()),
                None => self.composing = true,
            },
            (About::Asked(Ask::Blocked { pane }), 0) => self.open_pane(pane.clone()),
            (About::Asked(Ask::Blocked { .. }), 1) => self.reply("park", Answer::Act(Action::Park)),
            (About::Asked(Ask::Blocked { .. }), 2) => {
                let q = self.questions.remove(0);
                self.tell(q.ticket.as_deref(), "you answered: I answered it");
            }
            // approve, feedback of your own, resend, park, open the pane
            (About::Asked(Ask::Plan { feedback, pane, .. }), n) => {
                let resend = usize::from(feedback.is_some());
                match n {
                    0 => self.reply("approve", Answer::Approve),
                    1 => self.composing = true,
                    2 if resend == 1 => {
                        let feedback = feedback.clone().unwrap_or_default();
                        self.reply("feedback", Answer::Prompt(feedback));
                    }
                    n if n == 2 + resend => self.reply("park", Answer::Act(Action::Park)),
                    _ => self.open_pane(pane.clone()),
                }
            }
            (About::Asked(Ask::PlanFailed { pane, .. }), 0) => self.open_pane(pane.clone()),
            (About::Asked(Ask::PlanFailed { .. }), 1) => {
                self.reply("park", Answer::Act(Action::Park))
            }
            (About::Asked(Ask::PlanFailed { .. }), 2) => {
                self.reply("retry", Answer::Act(Action::Retry))
            }
            (
                About::Asked(Ask::PlanFailed {
                    feedback: Some(f), ..
                }),
                _,
            ) => {
                let feedback = f.clone();
                self.reply("feedback", Answer::Prompt(feedback));
            }
            // wait, review with the fallback, open the PR unreviewed: the
            // answer stands for every Review on the App until the reset
            (About::Asked(Ask::Limited { app, .. }), n) => {
                let (app, options) = (app.clone(), self.options());
                let answer = match n {
                    0 => Review::Wait,
                    n if n + 1 == options.len() => Review::Unreviewed,
                    _ => Review::Fallback,
                };
                let q = self.questions.remove(0);
                self.tell(
                    q.ticket.as_deref(),
                    &format!("you answered: {}", options[n]),
                );
                if let Some(run) = &self.run {
                    run.o.review(&app, answer);
                }
            }
            // the Stage's options, an answer of your own, open the pane, park
            (About::Asked(Ask::StageQuestion { options, pane, .. }), n) => {
                match (options.get(n).cloned(), n.saturating_sub(options.len())) {
                    (Some(option), _) => self.reply(&option.clone(), Answer::Prompt(option)),
                    (None, 0) => self.composing = true,
                    (None, 1) => self.open_pane(pane.clone()),
                    (None, _) => self.reply("park", Answer::Act(Action::Park)),
                }
            }
            // done, done with your facts (typed in the band), park
            (About::Asked(Ask::Manual { .. }), n) => match n {
                0 => self.reply("done", Answer::Prompt(String::new())),
                1 => self.composing = true,
                _ => self.reply("park", Answer::Act(Action::Park)),
            },
            // its options alone, the one picked sent word for word
            (
                About::Asked(
                    Ask::TicketStart { options }
                    | Ask::Labels { options }
                    | Ask::ReleaseAgain { options }
                    | Ask::Merge { options, .. },
                ),
                n,
            ) => {
                if let Some(option) = options.get(n).cloned() {
                    self.reply(&option.clone(), Answer::Prompt(option));
                }
            }
            // yes, no, yes with a prompt of your own (typed in the band)
            (About::Asked(Ask::NextWaypoint { .. }), 0) => self.reply("yes", Answer::Approve),
            (About::Asked(Ask::NextWaypoint { .. }), 1) => {
                self.reply("no", Answer::Act(Action::Park))
            }
            (About::Asked(Ask::NextWaypoint { .. }), _) => self.composing = true,
            // yes or no, sent word for word; no's Notice says how to tag it
            (About::Asked(Ask::Tag { options, notice }), n) => {
                let notice = (n == 1).then(|| notice.clone());
                if let Some(option) = options.get(n).cloned() {
                    self.reply(&option.clone(), Answer::Prompt(option));
                }
                if let Some(text) = notice {
                    self.notify(NoticeKind::Info, &text, None);
                }
            }
            // answered even when what it confirms asks again
            (About::Confirm(_), 0) => {
                self.off_call("you answered");
                let q = self.questions.remove(0);
                let About::Confirm(pending) = q.about else {
                    unreachable!()
                };
                self.confirmed(&q.text, pending);
            }
            (About::Confirm(Pending::DocsPass { tag }), _) => {
                if let Err(err) = graphify::set_handled(&self.cfg.repo, tag) {
                    self.say(&format!("graphify docs pass: {err}"));
                }
                self.questions.remove(0);
            }
            (About::Confirm(_), _) => {
                self.questions.remove(0);
                self.notice("cancelled", NOTICE_WINDOW);
            }
            (About::Continue { rows }, _) => {
                let rows = rows.clone();
                self.questions.remove(0);
                self.resume(&rows);
            }
            _ => {}
        }
        self.hidden = false;
        if self.questions.len() < waiting {
            self.off_call("you answered");
        }
    }

    /// Closes the done Epic in bd, its summary in the reason, after a comment
    /// that lists each Ticket with its PR, from the 'PR merged: <url>'
    /// poll_merges closed it with; a Ticket closed any other way has no PR.
    /// Both name the version its Release released, and the tag.
    /// The summary is of the run's completed State, Parked Tickets and all.
    /// A failure asks `text` again, to retry what has not gone in yet.
    fn close_epic(&mut self, text: &str, done: State, commented: bool) {
        let epic = done.epic.as_str();
        let tickets = self
            .epics
            .iter()
            .find(|e| e.id == epic)
            .map(|e| &e.tickets[..]);
        let lines: Vec<String> = tickets
            .unwrap_or_default()
            .iter()
            .map(|t| {
                let reason = &t.close_reason;
                let pr = reason
                    .strip_prefix("PR merged: ")
                    .filter(|url| !url.is_empty())
                    .unwrap_or("no PR");
                format!("- {} {}: {pr}", t.id, t.title)
            })
            .collect();
        let released =
            summary::released(&done).map_or(String::new(), |v| format!("; released {v}"));
        let comment = format!("Every Ticket merged{released}:\n{}", lines.join("\n"));
        let (tools, repo) = (&self.cfg.tools, &self.cfg.repo);
        let merged = format!("every Ticket merged{released}");
        let reason = match bd_list(repo, &**tools)
            .and_then(|issues| Summary::build(repo, &self.cfg.home, &issues, &done, epic))
        {
            Ok(summary) => format!("{merged}\n\n{}", draw::plain(&summary)),
            Err(_) => merged, // no evidence: the reason alone
        };
        let mut closed = Ok(String::new());
        if !commented {
            closed = tools.run(repo, &["bd", "comments", "add", epic, &comment]);
        }
        let commented = closed.is_ok();
        let closed =
            closed.and_then(|_| tools.run(repo, &["bd", "close", epic, "--reason", &reason]));
        match closed {
            Ok(_) => _ = self.reload_epics(),
            Err(err) => {
                self.notice(&err.to_string(), NOTICE_WINDOW);
                self.ask_close(text, Pending::Close { done, commented });
            }
        }
    }

    /// Focuses the pane a Question is about; the Question stays.
    fn open_pane(&mut self, pane: String) {
        if self.demo.is_some() {
            return self.notice("demo: no pane behind it", NOTICE_WINDOW);
        }
        let focus = self
            .cfg
            .tools
            .run(&self.cfg.repo, &["herdr", "agent", "focus", &pane]);
        if let Err(err) = focus {
            self.notice(&err.to_string(), NOTICE_WINDOW);
        }
    }

    /// The front Question answered: line one names the answer, which goes
    /// to the Orchestrator for the session the Question was about; line two
    /// is the Orchestrator's, once it acts.
    fn reply(&mut self, word: &str, answer: Answer) {
        let q = self.questions.remove(0);
        let id = q.ticket.clone().unwrap_or_default();
        self.tell(Some(&id), &format!("you answered: {word}"));
        // the Brainstorm's: no run to answer them
        if let About::Asked(Ask::NextWaypoint { map, .. }) = &q.about {
            return self.next_waypoint(map, answer);
        }
        if let About::Asked(Ask::Manual { pane, item, .. }) = &q.about {
            let live = self.live_brainstorm().filter(|b| &b.pane == pane).cloned();
            if let Some(b) = live {
                // not taken: the Question stays, to answer again
                if !self.brainstorm_manual(&id, &b, item, answer) {
                    self.questions.insert(0, q);
                }
                return;
            }
        }
        if self.demo.is_some() {
            return demo::answered(self, &id, &q.about, answer);
        }
        let pane = match &q.about {
            About::Asked(
                Ask::Wake { pane, .. }
                | Ask::Blocked { pane }
                | Ask::Plan { pane, .. }
                | Ask::PlanFailed { pane, .. }
                | Ask::StageQuestion { pane, .. }
                | Ask::Manual { pane, .. },
            ) => pane.as_str(),
            // no session: the Ticket's own, or the Release's
            About::Asked(
                Ask::TicketStart { .. }
                | Ask::Labels { .. }
                | Ask::Tag { .. }
                | Ask::ReleaseAgain { .. }
                | Ask::Merge { .. },
            ) => "",
            _ => return,
        };
        if let Some(run) = &self.run {
            run.o.answer(&id, pane, answer);
        }
    }

    /// A yes/no confirmation, its cursor on no: Enter alone never discards
    /// or exits.
    fn confirm(&mut self, text: &str, pending: Pending) {
        self.ask_first(Question {
            ticket: None,
            text: text.to_string(),
            about: About::Confirm(pending),
            cursor: 1,
            scroll: Cell::new(0),
            asked: chrono::Local::now(),
            opened: OnceCell::new(),
        });
    }

    /// What a confirmation's yes does, asked in `text`.
    fn confirmed(&mut self, text: &str, pending: Pending) {
        match pending {
            Pending::Start { ids, epic } => self.start(&ids, epic, true),
            Pending::Exit => self.quit(),
            Pending::Close { done, commented } => self.close_epic(text, done, commented),
            Pending::DocsPass { tag } => self.run_docs_pass(tag),
            Pending::Switch { line } => {
                if self.stop_live() {
                    self.command(&line);
                }
            }
        }
    }

    /// The drivers' state changes, each Brainstorm's copy replaced. Its
    /// Epics written and its docs PR merged or never opened suggests
    /// /start-epic of the first, once.
    fn brainstorm_updates(&mut self) {
        let built =
            |b: &Brainstorm| b.phase == Phase::Done && !b.epics.is_empty() && !b.docs_waiting();
        while let Ok(b) = self.brainstorm_receiver.try_recv() {
            let saved = self.brainstorms.iter_mut().find(|s| s.idea == b.idea);
            let charting = saved.as_ref().is_some_and(|s| s.phase == Phase::Charting);
            if built(&b) && !saved.as_deref().is_some_and(built) {
                self.suggestion = Some(format!("/start-epic {}", b.epics[0]));
            }
            if b.phase == Phase::Done && self.live.as_ref() == Some(&b.idea) {
                self.live = None;
            }
            match saved {
                Some(saved) => *saved = b.clone(),
                None => self.brainstorms.push(b.clone()),
            }
            // charting's outcome, once; Start Map makes a Map live again
            if charting && b.phase != Phase::Charting && self.live.as_ref() == Some(&b.idea) {
                self.live = None;
            }
            match b.phase {
                Phase::Map if charting => self.charted.push_back(b),
                Phase::Done if charting && !b.tickets.is_empty() => self.charted.push_back(b),
                _ => {}
            }
        }
    }

    /// The Notice for Epic `id` a Brainstorm wrote while its docs PR is
    /// not merged.
    fn docs_unmerged(&mut self, id: &str) {
        let unmerged = |b: &&Brainstorm| b.docs_waiting() && b.epics.iter().any(|e| e == id);
        if let Some(b) = self.brainstorms.iter().find(unmerged) {
            let text = format!(
                "docs PR {} not merged; Tickets won't see its docs",
                b.docs_pr
            );
            self.notify(NoticeKind::Info, &text, None);
        }
    }

    /// The live Brainstorm, saved.
    fn live_brainstorm(&self) -> Option<&Brainstorm> {
        let live = self.live.as_ref()?;
        self.brainstorms.iter().find(|b| &b.idea == live)
    }

    /// Whether a driver thread still runs for `idea`, live or stopping.
    fn driving(&self, idea: &str) -> bool {
        self.brainstorm_threads
            .iter()
            .any(|(i, t)| i == idea && !t.is_finished())
    }

    /// Whether `line` waits on the switch Question: a Brainstorm other
    /// than `key` is live, and only one is at a time.
    fn switch_asked(&mut self, key: &str, line: &str) -> bool {
        let live = self.live_brainstorm().map(|b| b.key().to_string());
        let Some(live) = live.filter(|l| l != key) else {
            return false;
        };
        let text = format!("{line}: {live} is live. Stop {live} and start this one?");
        self.confirm(
            &text,
            Pending::Switch {
                line: line.to_string(),
            },
        );
        true
    }

    /// Stops the live Brainstorm, saved: its pane closed, so a charting
    /// driver sees it gone and saves it. False, said so, when the pane
    /// does not close.
    fn stop_live(&mut self) -> bool {
        // a driver's new pane, sent but not yet taken
        self.brainstorm_updates();
        let Some(b) = self.live_brainstorm().cloned() else {
            self.live = None;
            self.questions.retain(|q| !q.brainstorms());
            return true;
        };
        // its driver between charting and its pane: that pane would open
        // after the next Brainstorm starts
        if b.pane.is_empty() && self.driving(&b.idea) {
            let text = format!("refused: {}'s pane is still opening, try again", b.key());
            self.refuse(&text);
            return false;
        }
        if b.pane.is_empty() {
            self.tell(Some(b.key()), "stopped; Brainstorm saved");
        } else if let Err(err) = herdr(
            &*self.cfg.tools,
            &self.cfg.repo,
            &["pane", "close", &b.pane],
        ) {
            if !herdr::pane_gone(&err) {
                self.tell(
                    Some(b.key()),
                    &format!("not stopped: its pane did not close: {err}"),
                );
                return false;
            }
        }
        self.live = None;
        // its Questions go: /continue @<map> asks them again
        self.questions.retain(|q| !q.brainstorms());
        true
    }

    /// Asks in a yes/no Notice modal whether to close the done Epic, its
    /// cursor on no: over the Epic summary once Esc closes it, at once
    /// with none open.
    fn ask_close(&mut self, text: &str, pending: Pending) {
        let n = Notice {
            yes: Some(pending),
            cursor: 1,
            ..notice_modal(NoticeKind::Info, text, None)
        };
        match self.summary {
            Some(_) => self.held = Some(n),
            None => self.raise(n),
        }
    }

    /// The front yes/no Notice modal answered: it closes, and the Epic
    /// summary under it; yes does what it asks.
    fn answer_notice(&mut self, yes: bool) {
        let n = self.notices.remove(0);
        self.show_notice();
        self.summary = None;
        self.off_call("you answered");
        if let (true, Some(pending)) = (yes, n.yes) {
            self.confirmed(&n.text, pending);
        }
    }

    /// A confirmation or the /continue checklist: shown at once, ahead of
    /// every Ticket's Question and the Docs pass's, in place of one still
    /// waiting.
    fn ask_first(&mut self, question: Question) {
        self.questions
            .retain(|q| q.ticket.is_some() || is_docs_pass(q));
        self.questions.insert(0, question);
        self.hidden = false;
    }

    /// The /continue checklist answered, once the lock is held: a reset
    /// Ticket starts Implement over, a Parked one set to resume is unparked
    /// at its Stage, and the rest resume as saved. A Brainstorm's issue a
    /// run saved before they were kept out is dropped: it never enters the
    /// Pipeline.
    fn resume(&mut self, rows: &[(String, bool)]) {
        let Some(prepared) = self.prepare() else {
            return;
        };
        let o = prepared.1.clone();
        let kept_out = |id: &String| self.brainstorm_issues.iter().any(|i| &i.id == id);
        let rows: Vec<&(String, bool)> = rows.iter().filter(|(id, _)| !kept_out(id)).collect();
        {
            let mut state = o.state.lock().unwrap();
            state.queue.retain(|id| !kept_out(id));
            state.tickets.retain(|id, _| !kept_out(id));
            state.removed.retain(|id, _| !kept_out(id));
        }
        for (id, reset) in rows {
            if *reset {
                if let Err(err) = reset_ticket(&o, id) {
                    self.notice(&format!("{id} not reset: {err}"), NOTICE_WINDOW);
                }
            } else if o.ticket(id).status == STATUS_PARKED {
                o.update(id, |ts| ts.status = STATUS_RUNNING.to_string());
            }
        }
        let epic = o.state.lock().unwrap().epic.clone();
        self.spawn(prepared, !epic.is_empty(), move |o| o.run(&epic));
    }

    /// One input line: a slash command, or y/n typed at a confirmation.
    pub(crate) fn command(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        self.suggestion = None;
        if matches!(self.questions.first(), Some(q) if matches!(q.about, About::Confirm(_))) {
            match line {
                "y" | "yes" => return self.answer(0),
                "n" | "no" => return self.answer(1),
                _ => {}
            }
        }
        let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
        let query = rest.trim();
        let query = query.strip_prefix('@').unwrap_or(query); // '@<id>' typed, not picked
        let starts = [
            "/start-epic",
            "/start-ticket",
            "/continue",
            "/summary",
            "/demo",
            "/brainstorm",
        ];
        if self.demo.is_some() && starts.contains(&name) {
            return self.refuse("refused: the demo is on, /stop-demo ends it");
        }
        match name {
            "/start-epic" => {
                if self.busy() {
                    return;
                }
                self.reload_epics();
                if let Some(text) = brainstorm::refusal(&self.brainstorm_issues, query) {
                    return self.refuse(&text);
                }
                if let Some(id) = self.resolve(query, true) {
                    match self.epic_waits(&id, &|_| false) {
                        Some(text) => self.refuse(&text),
                        None => {
                            self.docs_unmerged(&id);
                            self.start(&[id], true, false)
                        }
                    }
                }
            }
            // on a live Ticket run it adds to the run
            "/start-ticket" => {
                let adding = self.run.as_ref().is_some_and(|r| !r.epic) && !self.stopping();
                if !adding && self.busy() {
                    return;
                }
                self.reload_epics();
                let Some(ids) = self.tickets_named(query) else {
                    return;
                };
                match adding {
                    true => self.add(&ids),
                    false => self.start(&ids, false, false),
                }
            }
            "/remove-ticket" => match self.run.as_ref().map(|r| (r.epic, r.o.clone())) {
                _ if query.is_empty() => {
                    self.notice("usage: /remove-ticket <ticket>", NOTICE_WINDOW)
                }
                None => self.refuse("refused: no Ticket run is live"),
                Some(_) if self.stopping() => self.refuse("refused: a run is stopping"),
                Some((true, _)) => {
                    self.refuse("refused: an Epic run takes every Ticket of its Epic")
                }
                Some(_) if !self.state.queue.iter().any(|t| t == query) => self.refuse(&format!(
                    "refused: Ticket {} is not in the run",
                    suffix(query)
                )),
                Some((false, o)) => {
                    self.reload_epics();
                    match self.waits_on(query) {
                        Some(w) => {
                            self.refuse(&format!("refused: {w} waits on {query}, remove {w} first"))
                        }
                        None => o.command(&format!("remove-{query}")),
                    }
                }
            },
            "/continue" if !query.is_empty() => {
                let saved = self
                    .brainstorms
                    .iter()
                    .find(|b| b.phase != Phase::Done && b.key() == query);
                // a Waypoint of a saved Map
                let of_map = self
                    .brainstorm_issues
                    .iter()
                    .find(|i| i.id == query)
                    .and_then(|w| {
                        let map = |b: &&Brainstorm| b.phase == Phase::Map && b.map == w.parent;
                        self.brainstorms.iter().find(map)
                    });
                match (saved.cloned(), of_map.cloned()) {
                    (Some(b), _) => self.continue_brainstorm(b),
                    (None, Some(b)) => self.continue_waypoint(b, query),
                    (None, None) => self.continue_ticket(query),
                }
            }
            "/continue" => {
                if self.busy() {
                    return;
                }
                let mut rows: Vec<(String, bool)> = self
                    .state
                    .tickets
                    .iter()
                    .filter(|(_, ts)| ts.status == STATUS_RUNNING || ts.status == STATUS_PARKED)
                    .map(|(id, _)| (id.clone(), false))
                    .collect();
                rows.sort_by_key(|(id, _)| suffix_order(id));
                // a Ticket run whose Tickets all wait on their merges resumes too
                let waiting = !self.state.queue.is_empty()
                    || self
                        .state
                        .tickets
                        .values()
                        .any(|ts| ts.status == STATUS_PR_OPEN);
                if rows.is_empty() && self.state.epic.is_empty() && !waiting {
                    self.refuse("refused: no saved Ticket to continue");
                } else if rows.is_empty() {
                    self.resume(&rows);
                } else {
                    self.ask_first(Question {
                        ticket: None,
                        text: "continue the saved run: each Ticket resumes at its Stage, or is reset to Implement".to_string(),
                        about: About::Continue { rows },
                        cursor: 0,
                        scroll: Cell::new(0),
                        asked: chrono::Local::now(),
                        opened: OnceCell::new(),
                    });
                }
            }
            // An Epic by its id, as the @ list fills it in; closed ones too.
            // Alone: the live or saved run, or the last to finish.
            "/summary" => {
                let ran = |state: &State| !state.epic.is_empty() || !state.queue.is_empty();
                let (state, epic) = if !query.is_empty() {
                    // the last run's State when it is that Epic's: its Release
                    let state = match self.state.epic != query && self.last.epic == query {
                        true => &self.last,
                        false => &self.state,
                    };
                    (state.clone(), query.to_string())
                } else if ran(&self.state) {
                    (self.state.clone(), self.state.epic.clone())
                } else if ran(&self.last) {
                    (self.last.clone(), self.last.epic.clone())
                } else {
                    return self
                        .notice("no run yet, /summary @<epic> shows an Epic", NOTICE_WINDOW);
                };
                self.summarize(&state, &epic);
            }
            "/manual-work" => self.open_manual_work(),
            "/brainstorm" if self.switch_asked("", "/brainstorm") => {}
            "/brainstorm" => self.idea = Some(Idea::default()),
            "/questions" => match self.questions.is_empty() {
                true => self.notice("no questions waiting", NOTICE_WINDOW),
                false => self.hidden = false,
            },
            "/stop-work" | "/stop-demo" if self.demo.is_some() => demo::stop(self),
            "/stop-demo" => self.notice("no demo is running", NOTICE_WINDOW),
            // A confirmation answered in the demo would act on the real bd.
            "/demo" if self.questions.iter().any(|q| q.ticket.is_none()) => {
                self.refuse("refused: answer the waiting question first")
            }
            "/demo" if !self.busy() => demo::start(self),
            "/demo" => {}
            "/stop-work" => self.stop_work(),
            "/config" => self.open_config(),
            "/away" => {
                let away = !self.cfg.away.fetch_xor(true, Ordering::SeqCst);
                self.say(match away {
                    true => "away: on, a Stage's question parks its Ticket",
                    false => "away: off",
                });
                if away {
                    self.off_call("away is on");
                }
            }
            "/retry" | "/park" | "/rebase" | "/address-pr-comments" => {
                let waiting = self
                    .questions
                    .iter()
                    .any(|q| q.ticket.as_deref() == Some(query));
                match self.run.as_ref().map(|run| run.o.clone()) {
                    _ if query.is_empty() => {
                        self.notice(&format!("usage: {name} <ticket>"), NOTICE_WINDOW)
                    }
                    None => {
                        self.refuse("refused: no run is live, /start-epic or /continue starts one")
                    }
                    Some(_) if waiting => self.refuse(&format!(
                        "refused: Ticket {} has a Question waiting",
                        suffix(query)
                    )),
                    // by hand: every item still open, no countdown
                    // ponytail: gh is asked on the Shell's thread, which
                    // stalls the screen that long, as bd's calls here do
                    Some(o) if name == "/address-pr-comments" => match o.open_items(query) {
                        Ok(items) if items.is_empty() => {
                            self.refuse(&format!("refused: {query} has no open PR comments"))
                        }
                        Ok(items) => self.offer(query.to_string(), items, None, true),
                        Err(err) => self.refuse(&err),
                    },
                    Some(o) => o.command(&format!("{}-{query}", &name[1..])),
                }
            }
            "/exit" => match &self.run {
                None => self.quit(),
                Some(_) => self.confirm("stop the run and exit?", Pending::Exit),
            },
            _ => self.notice(&format!("unknown command: {line}"), NOTICE_WINDOW),
        }
    }

    /// /continue @<idea> or @<map>: another Brainstorm live asks first; a
    /// Map opens its Continue form at the saved answer, an Idea resumes its
    /// charting.
    fn continue_brainstorm(&mut self, b: Brainstorm) {
        let key = b.key().to_string();
        if self.switch_asked(&key, &format!("/continue @{key}")) {
            return;
        }
        match b.phase {
            // a driver still running, live or stopping, owns its pane
            Phase::Map if self.driving(&b.idea) => {
                self.refuse(&format!("refused: a Waypoint session of {key} is running"))
            }
            Phase::Map => self.open_start_map(&b, true),
            _ if self.driving(&b.idea) => {
                self.refuse(&format!("refused: {key} is already charting"))
            }
            _ => self.chart(b),
        }
    }

    /// /continue @<waypoint> of saved Map `b`: another Brainstorm live asks
    /// first; refused while a session of the Map's runs, and for the
    /// Waypoint's own reasons; else its Map live and brainstorm-waypoint
    /// started on it with WAYPOINT set.
    fn continue_waypoint(&mut self, mut b: Brainstorm, id: &str) {
        let key = b.key().to_string();
        if self.switch_asked(&key, &format!("/continue @{id}")) {
            return;
        }
        if self.driving(&b.idea) {
            return self.refuse(&format!("refused: a Waypoint session of {key} is running"));
        }
        // a failed bd list is said, never read as a Waypoint to take
        let Some(issues) = self.reload_issues() else {
            return;
        };
        if let Some(text) = brainstorm::waypoint_refusal(&issues, &b, id) {
            return self.refuse(&text);
        }
        b.started = true;
        if let Err(err) = b.save(&self.cfg.repo) {
            return self.tell(Some(&key), &format!("Brainstorm state not saved: {err}"));
        }
        if let Some(saved) = self.brainstorms.iter_mut().find(|s| s.idea == b.idea) {
            *saved = b.clone();
        }
        self.work_map(b, None, Some(id.to_string()));
    }

    /// "Next Waypoint?" answered for `map`, while it is the live Map: yes
    /// (Approve) starts a fresh session, with your prompt as its PROMPT
    /// when you typed one; no stops the Brainstorm, saved, for /continue
    /// @<map>.
    fn next_waypoint(&mut self, map: &str, answer: Answer) {
        let live = self.live_brainstorm();
        let Some(b) = live
            .filter(|b| b.map == map && b.phase == Phase::Map)
            .cloned()
        else {
            return self.tell(Some(map), "not acted on: the Map is no longer live");
        };
        match answer {
            Answer::Approve => self.work_map(b, None, None),
            Answer::Prompt(text) => self.work_map(b, Some(text), None),
            Answer::Act(_) => {
                self.live = None;
                let text = "Brainstorm stopped: you answered no to next Waypoint?";
                self.tell(Some(&b.map), text);
                self.suggestion = Some(format!("/continue @{}", b.map));
            }
        }
    }

    /// A Waypoint session's Manual work answered: done, with your facts or
    /// none, goes into its pane as Manual work <n> done: <facts>, its
    /// result removed first, and the item is marked done; park stops the
    /// Brainstorm, saved, so /continue @<map> asks it again. Whether the
    /// answer was taken: not when the prompt or the stop failed.
    fn brainstorm_manual(
        &mut self,
        id: &str,
        b: &Brainstorm,
        item: &manual::Item,
        answer: Answer,
    ) -> bool {
        let Answer::Prompt(facts) = answer else {
            if !self.stop_live() {
                return false;
            }
            let next = format!("/continue @{}", b.map);
            self.tell(
                Some(id),
                &format!("parked: Brainstorm stopped, {next} asks again"),
            );
            self.suggestion = Some(next);
            return true;
        };
        let (tools, repo) = (&*self.cfg.tools, &self.cfg.repo);
        let kept = fs::read(&b.result);
        let _ = fs::remove_file(&b.result);
        let prompt = manual::done_prompt(&item.folder, &facts);
        if let Err(err) = herdr(tools, repo, &["agent", "prompt", &b.pane, &prompt]) {
            // put back unless written anew
            if let Ok(kept) = kept {
                let _ = File::options()
                    .write(true)
                    .create_new(true)
                    .open(&b.result)
                    .and_then(|mut f| f.write_all(&kept));
            }
            self.tell(Some(id), &format!("never took your answer: {err}"));
            return false;
        }
        let number = manual::number(&item.folder);
        let marked = manual::done(tools, repo, id, item, &facts);
        self.tell(Some(id), &format!("sent manual work {number} done"));
        if let Err(err) = marked {
            self.tell(
                Some(id),
                &format!("manual work {number} not marked done: {err}"),
            );
        }
        true
    }

    /// /continue @ticket: unparks that one Ticket at its Stage, watching its
    /// live session, and puts its next Question first; Away goes off, the
    /// user being back. With no run live the saved run resumes; in a live
    /// run it is the scheduler's command, as /retry is.
    fn continue_ticket(&mut self, id: &str) {
        let parked = self
            .state
            .tickets
            .get(id)
            .is_some_and(|ts| ts.status == STATUS_PARKED);
        match self.run.as_ref().map(|run| run.o.clone()) {
            _ if !parked => {
                return self.refuse(&format!("refused: Ticket {} is not parked", suffix(id)))
            }
            None => {}
            Some(_) if self.stopping() => return self.refuse("refused: a run is stopping"),
            Some(o) => o.command(&format!("continue-{id}")),
        }
        self.first = Some(id.to_string());
        if self.cfg.away.swap(false, Ordering::SeqCst) {
            self.say("away: off");
        }
        if self.run.is_none() {
            self.resume(&[(id.to_string(), false)]);
        }
    }

    /// The one open Epic (or Ticket) an argument names, a leading @ stripped:
    /// its id exactly, or the only one whose id or title contains it.
    /// Anything else is a notice.
    fn resolve(&mut self, query: &str, epics: bool) -> Option<String> {
        let query = query.strip_prefix('@').unwrap_or(query);
        let candidates: Vec<(&str, &str)> = if epics {
            self.epics
                .iter()
                .filter(|e| !e.id.is_empty())
                .map(|e| (e.id.as_str(), e.title.as_str()))
                .collect()
        } else {
            self.epics
                .iter()
                .flat_map(|e| &e.tickets)
                .filter(|t| t.status != "closed")
                .map(|t| (t.id.as_str(), t.title.as_str()))
                .collect()
        };
        if let Some((id, _)) = candidates.iter().find(|(id, _)| *id == query) {
            return Some(id.to_string());
        }
        let q = query.to_lowercase();
        let found: Vec<String> = candidates
            .iter()
            .filter(|(id, title)| {
                id.to_lowercase().contains(&q) || title.to_lowercase().contains(&q)
            })
            .map(|(id, title)| format!("{id} {title}"))
            .collect();
        let what = if epics { "open Epic" } else { "open Ticket" };
        match found.as_slice() {
            [one] => Some(one.split(' ').next().unwrap().to_string()),
            [] => {
                self.notice(&format!("no {what} matches {query:?}"), NOTICE_WINDOW);
                None
            }
            many => {
                self.notice(&format!("matches: {}", many.join("  ·  ")), NOTICE_WINDOW);
                None
            }
        }
    }

    /// The open Tickets /start-ticket names: every word an open Ticket's id
    /// exactly, as the @ list fills them in, or else the one Ticket resolve
    /// finds. A Brainstorm's Waypoint or Idea never enters the Pipeline:
    /// refused. A Ticket blocked by an open one that is neither named nor in
    /// the run would never start: refused, as is one whose Epic waits so.
    fn tickets_named(&mut self, query: &str) -> Option<Vec<String>> {
        let words: Vec<&str> = query
            .split_whitespace()
            .map(|w| w.strip_prefix('@').unwrap_or(w))
            .collect();
        let refusal = words
            .iter()
            .find_map(|w| brainstorm::refusal(&self.brainstorm_issues, w));
        if let Some(text) = refusal {
            self.refuse(&text);
            return None;
        }
        // one off the tree is closed: the tree keeps every open Ticket
        let open = |epics: &[Epic], id: &str| find(epics, id).is_some_and(|t| t.status != "closed");
        let ids = match words.len() > 1 && words.iter().all(|w| open(&self.epics, w)) {
            true => config::distinct(words.iter().map(|w| w.to_string())),
            false => vec![self.resolve(query, false)?],
        };
        let theirs = |b: &str| ids.iter().chain(&self.state.queue).any(|t| t == b);
        let waits = ids.iter().find_map(|id| {
            let t = find(&self.epics, id)?;
            match t
                .blockers()
                .find(|b| unfinished(&self.epics, &self.brainstorm_issues, b) && !theirs(b))
            {
                Some(b) => Some(format!(
                    "refused: {id} waits on {b}, which is not in the run"
                )),
                None => self.epic_waits(&t.parent, &theirs),
            }
        });
        match waits {
            Some(text) => {
                self.refuse(&text);
                None
            }
            None => Some(ids),
        }
    }

    /// What `epic` waits on, open and not `theirs`, as a refusal: bd hides a
    /// blocked Epic's children from bd ready, so a run on them would idle.
    fn epic_waits(&self, epic: &str, theirs: &dyn Fn(&str) -> bool) -> Option<String> {
        let e = self.epics.iter().find(|e| e.id == epic)?;
        let waits: Vec<&str> = e
            .blockers
            .iter()
            .map(String::as_str)
            .filter(|b| unfinished(&self.epics, &self.brainstorm_issues, b) && !theirs(b))
            .collect();
        let which = if waits.len() == 1 {
            "which is"
        } else {
            "which are"
        };
        (!waits.is_empty()).then(|| {
            format!(
                "refused: {epic} waits on {}, {which} not in the run",
                waits.join(", ")
            )
        })
    }

    /// An open Ticket of the Ticket run that waits on `ticket`, itself open,
    /// or whose Epic does: removed, its merge would go unpolled and that one
    /// never start.
    fn waits_on(&self, ticket: &str) -> Option<String> {
        let open = |id: &str| find(&self.epics, id).filter(|t| t.status != "closed");
        open(ticket)?;
        let epic_waits = |t: &BdIssue| {
            let epic = self.epics.iter().find(|e| e.id == t.parent);
            epic.is_some_and(|e| e.blockers.iter().any(|b| b == ticket))
        };
        let mut queue = self.state.queue.iter();
        let waits = |t: &&String| {
            open(t).is_some_and(|t| t.blockers().any(|b| b == ticket) || epic_waits(t))
        };
        queue.find(waits).cloned()
    }

    /// /start-epic, and /start-ticket with no run live, which starts a Ticket
    /// run on its Tickets, or adds them to the saved one's queue. Over a
    /// different saved Epic, or any saved Epic for a Ticket run, it asks
    /// before discarding the saved run, which goes only once the lock is
    /// held. An Epic run resumes every running Ticket, so over a saved Ticket
    /// run it asks too when one running, Parked (removed from the run or not)
    /// or queued is not the Epic's. A saved Ticket run stopped in its Release
    /// asks before anything, Epic or Ticket, takes it up: its Release
    /// belongs to it alone.
    fn start(&mut self, ids: &[String], epic: bool, discard: bool) {
        let parent = |ticket: &str| {
            self.epics
                .iter()
                .flat_map(|e| &e.tickets)
                .find(|t| t.id == ticket)
                .map(|t| t.parent.clone())
                .unwrap_or_default()
        };
        let release = self.state.release.as_ref().map(|r| r.id.as_str());
        let saved = if epic && self.state.epic.is_empty() {
            // running or Parked, removed from the run too, or queued and
            // not started: the Epic run takes up the removed and clears the
            // queue
            let (tickets, removed) = (&self.state.tickets, &self.state.removed);
            let unfinished = |t: &str| match tickets.get(t).or(removed.get(t)) {
                Some(ts) => ts.status == STATUS_RUNNING || ts.status == STATUS_PARKED,
                None => true,
            };
            let mut stray: Vec<&str> = tickets
                .keys()
                .chain(removed.keys())
                .chain(&self.state.queue)
                .map(String::as_str)
                .chain(release)
                .filter(|t| unfinished(t) && parent(t) != ids[0])
                .collect();
            stray.sort();
            stray.dedup();
            stray.join(", ")
        } else if self.state.epic.is_empty() {
            release.unwrap_or_default().to_string()
        } else {
            self.state.epic.clone()
        };
        let other = !saved.is_empty() && !(epic && saved == ids[0]);
        if other && !discard {
            let pending = Pending::Start {
                ids: ids.to_vec(),
                epic,
            };
            return self.confirm(&format!("discard the saved run on {saved}?"), pending);
        }
        let Some(prepared) = self.prepare() else {
            return;
        };
        let o = prepared.1.clone();
        if other {
            if let Err(err) = State::default().save(&self.cfg.repo) {
                return self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
            *o.state.lock().unwrap() = State::default();
        }
        if epic {
            let id = ids[0].clone();
            self.spawn(prepared, true, move |o| o.run(&id));
        } else {
            o.enqueue(ids);
            self.spawn(prepared, false, |o| o.run(""));
        }
    }

    /// /start-ticket on a live Ticket run: its Tickets join the queue, and
    /// the summary opens by itself again once every one has its PR.
    fn add(&mut self, ids: &[String]) {
        let new: Vec<String> = ids
            .iter()
            .filter(|id| !self.state.queue.contains(id))
            .cloned()
            .collect();
        if new.is_empty() {
            return self.notice("already in the run", NOTICE_WINDOW);
        }
        let Some(run) = &mut self.run else {
            return;
        };
        if !run.o.enqueue(&new) {
            return self.refuse("refused: the Ticket run is ending, /start-ticket once it has");
        }
        run.summarized = false;
        self.state = run.o.state.lock().unwrap().clone();
        for id in &new {
            self.tell(Some(id), "added to the run");
        }
    }

    /// Opens the live run's summary by itself, and rings On call.
    fn summarize_run(&mut self) {
        let state = self.state.clone();
        self.summarize(&state, &state.epic);
        self.ring_end("run done");
    }

    /// Opens the summary of an Epic, or with none of the State's Ticket
    /// run, built fresh from bd, that State and the Run directories; a
    /// failure, or a run with no evidence, is a notice.
    fn summarize(&mut self, state: &State, epic: &str) {
        let repo = &self.cfg.repo;
        let built = bd_list(repo, &*self.cfg.tools)
            .and_then(|issues| Summary::build(repo, &self.cfg.home, &issues, state, epic));
        match built {
            Ok(summary) => self.summary = Some(summary),
            Err(err) => self.notice(&err, NOTICE_WINDOW),
        }
    }

    /// Takes the lock and makes the run's Orchestrator over the state
    /// file; None, said in a notice, when a run cannot start. The callers
    /// have checked that no run is live.
    fn prepare(&mut self) -> Option<(Lock, Arc<Orchestrator>)> {
        if let Some(missing) = self.missing.first() {
            self.notice(&format!("refused: {missing}"), NOTICE_WINDOW);
            return None;
        }
        let repo = &self.cfg.repo;
        let lock = match acquire_lock(repo) {
            Ok(lock) => lock,
            Err(err) => {
                self.notice(&err.to_string(), NOTICE_WINDOW);
                return None;
            }
        };
        let log: Box<dyn io::Write + Send> = match File::options()
            .create(true)
            .append(true)
            .open(repo.join(LOCAL).join("orchestrator.log"))
        {
            Ok(file) => Box::new(file),
            Err(_) => Box::new(io::sink()),
        };
        match Orchestrator::new(Config {
            log: Arc::new(Mutex::new(log)),
            events: self.sender.clone(),
            ..self.cfg.clone()
        }) {
            Ok(o) => Some((lock, o)),
            Err(err) => {
                self.notice(&err.to_string(), NOTICE_WINDOW);
                None
            }
        }
    }

    /// Runs `work` over the prepared Orchestrator on the scheduler thread;
    /// it returns when the run ends and poll sees it.
    fn spawn(
        &mut self,
        (lock, o): (Lock, Arc<Orchestrator>),
        epic: bool,
        work: impl FnOnce(&Arc<Orchestrator>) -> Result<(), String> + Send + 'static,
    ) {
        self.state = o.state.lock().unwrap().clone();
        let scheduler = {
            let o = o.clone();
            thread::spawn(move || work(&o))
        };
        self.run = Some(Run {
            o,
            scheduler: Some(scheduler),
            epic,
            failed: false,
            summarized: false,
            release: false,
            _lock: lock,
        });
        self.running = true;
        self.started = self.ticks;
    }

    /// /stop-work: scheduling ends, live panes stay, the state file holds
    /// every Ticket as saved, and the lock goes once every thread has left.
    fn stop_work(&mut self) {
        match &self.run {
            Some(run) => run.o.stop(),
            None => self.notice("nothing is running", NOTICE_WINDOW),
        }
    }

    /// Ends the Shell; a live run is stopped first and its panes stay.
    fn quit(&mut self) {
        if let Some(run) = &self.run {
            run.o.stop();
        }
        self.quit = true;
    }

    fn notice(&mut self, text: &str, span: Duration) {
        self.notice = Some((text.to_string(), Instant::now() + span));
    }

    /// Raises a Notice modal; one raised while another shows waits behind it.
    /// An autoclose counts down only while it shows.
    pub(crate) fn notify(&mut self, kind: NoticeKind, text: &str, autoclose: Option<Duration>) {
        self.raise(notice_modal(kind, text, autoclose));
    }

    /// Raises a Notice modal built whole, as notify does.
    fn raise(&mut self, n: Notice) {
        self.notices.push(n);
        if self.notices.len() == 1 {
            self.show_notice();
        }
    }

    /// Closes the front Notice modal; the next one waiting shows. The last
    /// one gone, the approval modal's countdown starts again, unless a key
    /// has stopped it.
    fn close_notice(&mut self) {
        self.notices.remove(0);
        self.show_notice();
        if self.notices.is_empty() && self.approvals.first().is_some_and(|a| a.approves.is_some()) {
            self.show_approval();
        }
    }

    /// The front Notice modal shows: its autoclose, if any, counts from now.
    fn show_notice(&mut self) {
        let now = (self.cfg.clock)();
        if let Some(n) = self.notices.first_mut() {
            n.closes = n.autoclose.map(|length| now + length);
        }
    }

    /// Raises the approval modal for a Ticket's PR comments, every row
    /// checked; one raised while another shows waits behind it.
    fn offer(
        &mut self,
        ticket: String,
        items: Vec<Item>,
        countdown: Option<Duration>,
        by_hand: bool,
    ) {
        let pr = self.state.tickets.get(&ticket).map(|ts| pr_ref(&ts.pr));
        self.approvals.push(Approval {
            ticket,
            pr: pr.unwrap_or_default(),
            rows: items.into_iter().map(|item| (item, true)).collect(),
            cursor: 0,
            countdown,
            approves: None,
            by_hand,
        });
        if self.approvals.len() == 1 {
            self.show_approval();
        }
    }

    /// The front approval modal shows: its countdown, if any, runs from now.
    fn show_approval(&mut self) {
        let now = (self.cfg.clock)();
        if let Some(a) = self.approvals.first_mut() {
            a.approves = a.countdown.map(|length| now + length);
        }
    }

    /// The front approval modal answered, by `who`: fix queues Address PR
    /// comments with the checked rows approved and the rest won't fix;
    /// cancel starts nothing. Its items stay offered either way, so the
    /// poll does not reopen them. One the poll raised is then told to the
    /// Orchestrator as answered, which the merge Question waits for. The
    /// next one shows.
    fn decide(&mut self, fix: bool, who: &str) {
        let a = self.approvals.remove(0);
        let n = plural(a.rows.len(), "PR comment");
        let raised = (!a.by_hand).then(|| a.ticket.clone());
        if fix {
            let (on, off): (Vec<_>, Vec<_>) = a.rows.into_iter().partition(|(_, on)| *on);
            let text = format!("{who} approved {} of {n}", on.len());
            self.tell(Some(&a.ticket), &text);
            let items = |rows: Vec<(Item, bool)>| rows.into_iter().map(|(item, _)| item).collect();
            if let Some(run) = &self.run {
                run.o
                    .approve_comments(&a.ticket, items(on), items(off), a.by_hand);
            }
        } else {
            let text = format!("{who} cancelled {n}, /address-pr-comments opens them");
            self.tell(Some(&a.ticket), &text);
        }
        // after the approval, so the flow never reads as over in between
        if let (Some(run), Some(ticket)) = (&self.run, raised) {
            run.o.decided(&ticket);
        }
        self.show_approval();
    }

    /// Opens the /manual-work modal over the open items of every Run
    /// directory under .orqadence-local/runs, a Ticket's or a Waypoint's,
    /// live run or none; a notice when there are none.
    fn open_manual_work(&mut self) {
        let runs = self.cfg.repo.join(LOCAL).join("runs");
        let mut dirs: Vec<_> = fs::read_dir(runs)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                // a /continue reset's <id>.reset-<n> keeps its items under <id>
                let id = name.split(".reset-").next().unwrap_or_default();
                (id.to_string(), entry.path())
            })
            .collect();
        dirs.sort();
        let rows: Vec<_> = dirs
            .iter()
            .flat_map(|(id, dir)| {
                manual::open(dir).into_iter().map(|mut item| {
                    item.blocks |= self.waited_on(&item.folder);
                    (id.clone(), item, false)
                })
            })
            .collect();
        match rows.is_empty() {
            true => self.notice("no Manual work open", NOTICE_WINDOW),
            false => self.manual_work = Some(ManualWork { rows, cursor: 0 }),
        }
    }

    /// A Manual work Question waits on `folder`: its session blocks on it,
    /// whatever its first line says.
    fn waited_on(&self, folder: &Path) -> bool {
        let Ok(real) = folder.canonicalize() else {
            return false;
        };
        self.questions.iter().any(|q| match &q.about {
            About::Asked(Ask::Manual { item, .. }) => {
                item.folder.canonicalize().ok() == Some(real.clone())
            }
            _ => false,
        })
    }

    /// Mark done on the /manual-work modal: each checked item's bd comment
    /// with its What, its folder deleted, said on RECENT; the modal closes.
    /// Each is read again first: one that came to block while the modal was
    /// open is left to its Question, so its session still hears it is done.
    fn mark_done(&mut self) {
        let Some(m) = self.manual_work.take() else {
            return;
        };
        for (id, item, _) in m.rows.into_iter().filter(|(_, _, on)| *on) {
            let n = manual::number(&item.folder);
            // its own Run directory, a reset's archive too: <dir>/manual-work/<n>
            let dir = item.folder.ancestors().nth(2).unwrap_or(&item.folder);
            let now = manual::read(dir, &item.folder);
            let marked = match now {
                Ok(item) if item.blocks || self.waited_on(&item.folder) => {
                    Err("it blocks now, its Question marks it done".to_string())
                }
                Ok(item) => manual::done(&*self.cfg.tools, &self.cfg.repo, &id, &item, ""),
                Err(err) => Err(err),
            };
            let text = match marked {
                Ok(()) => format!("marked Manual work {n} done: {}", item.what),
                Err(err) => format!("Manual work {n} not marked done: {err}"),
            };
            self.tell(Some(&id), &text);
        }
    }

    /// The title of a Ticket on the idle tree, for the RECENT Ticket column.
    pub(crate) fn title(&self, id: &str) -> Option<&str> {
        self.epics
            .iter()
            .flat_map(|e| &e.tickets)
            .find(|t| t.id == id)
            .map(|t| t.title.as_str())
    }
}

/// Whether a Question is the Docs pass's.
fn is_docs_pass(q: &Question) -> bool {
    matches!(q.about, About::Confirm(Pending::DocsPass { .. }))
}

/// A /continue reset: the Ticket's panes close, its run directory moves
/// aside as <id>.reset-<n> (the evidence stays, and no result in it is
/// accepted again), and it starts over at Implement. A directory that
/// cannot move refuses the reset.
fn reset_ticket(o: &Orchestrator, id: &str) -> io::Result<()> {
    for pane in o.ticket(id).panes.values() {
        let _ = o.herdr(&["pane", "close", pane]);
    }
    let dir = o.run_dir(id);
    if dir.exists() {
        let aside = (1..)
            .map(|n| dir.with_file_name(format!("{id}.reset-{n}")))
            .find(|path| !path.exists())
            .unwrap();
        fs::rename(&dir, aside)?;
    }
    o.update(id, |ts| {
        *ts = TicketState {
            status: STATUS_RUNNING.to_string(),
            stage: "implement".to_string(),
            ..Default::default()
        }
    });
    Ok(())
}

/// Whether `q`'s characters appear in `text` in order.
fn subsequence(q: &str, text: &str) -> bool {
    let mut chars = text.chars();
    q.chars().all(|c| chars.any(|t| t == c))
}

/// The child suffix of a bd id: harness-kqe.9 is 9.
pub(crate) fn suffix(id: &str) -> &str {
    id.rsplit_once('.').map_or(id, |(_, s)| s)
}

/// Where an id sorts among its siblings: by its child suffix as a number,
/// one with none last.
fn suffix_order(id: &str) -> usize {
    suffix(id).parse().unwrap_or(usize::MAX)
}

/// A Ticket on the tree by its id.
fn find<'a>(epics: &'a [Epic], id: &str) -> Option<&'a BdIssue> {
    epics.iter().flat_map(|e| &e.tickets).find(|t| t.id == id)
}

/// A blocker still open: an open Epic, an open Ticket, or an open Map,
/// Waypoint or Idea kept off the tree, which bd ready honors all the same;
/// any other off the tree is closed, as the tree keeps every open Epic and
/// Ticket.
fn unfinished(epics: &[Epic], kept_out: &[BdIssue], id: &str) -> bool {
    epics.iter().any(|e| e.id == id)
        || find(epics, id).is_some_and(|t| t.status != "closed")
        || kept_out.iter().any(|i| i.id == id && i.status != "closed")
}

/// Every issue, Epics and closed ones too, from one bd list call: --limit 0,
/// as bd's default 50 would cut off a Waypoint's Map.
pub(crate) fn bd_list(repo: &Path, tools: &dyn Tools) -> Result<Vec<BdIssue>, String> {
    let out = tools
        .run(
            repo,
            &["bd", "list", "--json", "--brief", "--all", "--limit", "0"],
        )
        .map_err(|err| err.to_string())?;
    let issues: Option<Vec<BdIssue>> =
        serde_json::from_str(&out).map_err(|err| format!("unreadable reply: {err}"))?;
    Ok(issues.unwrap_or_default())
}

/// Every open Epic expanded into its Tickets, then the no-Epic group when it
/// has one, from one bd list's issues. A closed Ticket with no open Epic is left
/// out, but for one still in the Ticket run's queue. A Brainstorm's Maps,
/// Waypoints and Ideas never enter the Pipeline: kept off the tree, they
/// come back beside it.
fn load_epics(mut issues: Vec<BdIssue>, queue: &[String]) -> (Vec<Epic>, Vec<BdIssue>) {
    let brainstorms: Vec<bool> = issues
        .iter()
        .map(|i| brainstorm::kind(&issues, i).is_some())
        .collect();
    let mut brainstorms = brainstorms.into_iter();
    // extract_if visits each issue once, in order
    let kept_out: Vec<BdIssue> = issues
        .extract_if(.., |_| brainstorms.next().unwrap_or(false))
        .collect();
    let mut epics: Vec<Epic> = issues
        .iter()
        .filter(|i| i.issue_type == "epic" && i.status != "closed")
        .map(|i| Epic {
            id: i.id.clone(),
            title: i.title.clone(),
            tickets: Vec::new(),
            blockers: i.blockers().map(String::from).collect(),
            labels: i.labels.clone(),
        })
        .collect();
    let mut no_epic = Epic {
        id: String::new(),
        title: "no Epic".to_string(),
        tickets: Vec::new(),
        blockers: Vec::new(),
        labels: Vec::new(),
    };
    issues.sort_by_key(|i| suffix_order(&i.id));
    for issue in issues {
        if issue.issue_type == "epic" {
            continue;
        }
        match epics.iter_mut().find(|e| e.id == issue.parent) {
            Some(epic) => epic.tickets.push(issue),
            None if issue.status != "closed" || queue.contains(&issue.id) => {
                no_epic.tickets.push(issue)
            }
            None => {}
        }
    }
    if !no_epic.tickets.is_empty() {
        epics.push(no_epic);
    }
    (epics, kept_out)
}

/// Opens the Shell over the Target repo and returns when the user exits.
pub(crate) fn open(
    repo: &Path,
    tools: Arc<dyn Tools>,
    env: &dyn Fn(&str) -> String,
) -> io::Result<()> {
    let mut screen = Screen::open(repo, tools, env);
    let mut terminal = ratatui::try_init()?;
    terminal_modes(true);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        terminal_modes(false);
        hook(info)
    }));
    let result = run(&mut terminal, &mut screen);
    terminal_modes(false);
    ratatui::restore();
    screen.close();
    if screen.reexec {
        // An idle-Shell update: the same argv comes back under the new version.
        eprintln!("orqa: {}", update::reexec(&screen.cfg.exe));
    }
    result
}

/// Mouse reporting of presses and the wheel, SGR-encoded, on or off, so a
/// wheel notch comes as itself and not as the ↑ or ↓ the terminal's
/// alternate scroll sends (harness-7lz). Plain drag no longer selects text;
/// Shift or Option held does. Not crossterm's EnableMouseCapture, which
/// reports every move too, a redraw each. Bracketed paste with it, so a
/// pasted newline reaches the idea modal as itself and not as Enter.
fn terminal_modes(on: bool) {
    let codes = match on {
        true => "\x1b[?1000h\x1b[?1006h\x1b[?2004h",
        false => "\x1b[?2004l\x1b[?1006l\x1b[?1000l",
    };
    let _ = crossterm::execute!(io::stdout(), crossterm::style::Print(codes));
}

/// Moves a scroll row by `by`, never under 0; the draw keeps it inside.
fn scroll_by(rows: &Cell<usize>, by: isize) {
    rows.set(rows.get().saturating_add_signed(by));
}

/// A Notice modal with one [ OK ], not yet shown.
fn notice_modal(kind: NoticeKind, text: &str, autoclose: Option<Duration>) -> Notice {
    Notice {
        kind,
        text: text.to_string(),
        autoclose,
        closes: None,
        scroll: Cell::new(0),
        yes: None,
        cursor: 0,
    }
}

/// The screen thread: take the Events and the State, draw, poll for a key
/// until the next tick, tick. A redraw follows every key, Event and tick; at
/// rest the tick is 250 ms, in a live run 50 ms. It never waits
/// on the Orchestrator: the State mutex is held for a clone, nothing longer.
fn run(terminal: &mut DefaultTerminal, screen: &mut Screen) -> io::Result<()> {
    let mut last = Instant::now();
    while !screen.quit {
        screen.poll();
        if screen.quit {
            break; // an idle update installed: no key may start a run the re-exec ends
        }
        terminal.draw(|f| draw::draw(f, screen))?;
        let tick = if screen.running { TICK } else { IDLE_TICK };
        if !event::poll(tick.saturating_sub(last.elapsed()))? {
            screen.tick();
            last = Instant::now();
            continue;
        }
        match event::read()? {
            Input::Key(key) => screen.key(key),
            Input::Mouse(m) => screen.mouse(m),
            Input::Paste(text) => screen.paste(&text),
            _ => {}
        }
        if screen.editing {
            screen.editing = false;
            edit(terminal, screen)?;
        }
    }
    Ok(())
}

/// Ctrl+G's editor over the whole terminal: the Shell's modes off and the
/// terminal restored, the editor run, then raw mode, the alternate screen
/// and the modes back, and a clear so the next draw paints every cell.
fn edit(terminal: &mut DefaultTerminal, screen: &mut Screen) -> io::Result<()> {
    terminal_modes(false);
    ratatui::try_restore()?;
    screen.edit_idea(&|k| std::env::var(k).unwrap_or_default());
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    terminal_modes(true);
    terminal.clear()
}

#[cfg(test)]
mod approval_test;
#[cfg(test)]
mod chart_test;
#[cfg(test)]
mod charted_test;
#[cfg(test)]
mod config_test;
#[cfg(test)]
mod continue_test;
#[cfg(test)]
mod demo_test;
#[cfg(test)]
mod epic_test;
#[cfg(test)]
mod graphify_test;
#[cfg(test)]
mod idea_test;
#[cfg(test)]
mod shell_test;
#[cfg(test)]
mod waypoint_test;
