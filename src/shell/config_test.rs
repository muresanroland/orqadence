//! /config over fake Tools and the fake world: the docked modal, its pick
//! lists, the probe, and a change saved during a run.

use super::brand::{PURPLE, RED};
use super::config::{
    put, Field, LabelItem, ADDRESS_PR_COMMENTS_PAGE, APPS_PAGE, LABELS_PAGE, ON_CALL_PAGE,
    REBASE_PAGE, RELEASE_PAGE, RUN_PAGE, SKILLS_PAGE, TYPESAFE_PAGE,
};
use super::shell_test::{
    asking, await_line, cols, find, key, logged, notice_modal, render, row, rows, screen_at, shell,
    type_in, type_line,
};
use super::{NoticeKind, Screen};
use crate::orchestrator::stage::Ask;
use crate::orchestrator::world::{new_world, succeed, BdTicket};
use crate::orchestrator::write_file;
use crate::skills::manifest::{add, personal, set_personal, Manifest, NONE};
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use crate::tools::{RunError, Tools};
use crossterm::event::KeyCode;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// codex's catalog as `codex debug models --bundled` prints it, a hidden
/// model among the listed ones.
const CATALOG: &str = r#"{"models": [
  {"slug": "gpt-6-sol", "visibility": "list", "supported_reasoning_levels":
    [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}]},
  {"slug": "gpt-5.6-sol", "visibility": "list", "supported_reasoning_levels": [{"effort": "low"}]},
  {"slug": "codex-auto-review", "visibility": "hide", "supported_reasoning_levels": []}
]}"#;

/// Fake Tools answering codex's catalog, `fail` with stderr, the rest "".
fn apps(fail: &'static str) -> std::sync::Arc<Fake> {
    Fake::new(move |_, argv| match argv.join(" ") {
        cmd if cmd == "codex debug models --bundled" => Ok(CATALOG.to_string()),
        cmd if !fail.is_empty() && cmd.contains(fail) => Err(format!("model {fail} not found")),
        _ => Ok(String::new()),
    })
}

fn keys(s: &mut Screen, codes: &[KeyCode]) {
    for code in codes {
        s.key(key(*code));
    }
}

/// Polls until /config's probe has answered.
fn await_probe(s: &mut Screen) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.settings.as_ref().unwrap().probe.is_some() {
        assert!(Instant::now() < deadline, "the probe never answered");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn config_json(repo: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo.join(".orqadence/config.json")).unwrap())
        .unwrap()
}

/// The foot's note, "" when none.
fn note(s: &Screen) -> String {
    let st = s.settings.as_ref().unwrap();
    st.note
        .as_ref()
        .map_or(String::new(), |(text, _)| text.clone())
}

/// /config docks in the right 58% as a thick purple box titled /config,
/// the live Shell in the left 42%; under 110 columns it folds to a rounded
/// box over the dimmed Shell, leaving it the input line.
#[test]
fn config_opens_docked_at_160x45_and_as_a_box_at_100x30() {
    let repo = TempDir::new();
    let mut s = screen_at(Fake::quiet(), repo.path());
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert_eq!(find(&buf, "┏"), Some((67, 0)), "{:#?}", rows(&buf));
    assert_eq!(buf[(67, 0)].fg, PURPLE);
    assert!(row(&buf, 0).contains("┏ /config ━"), "{:?}", row(&buf, 0));
    assert_eq!(find(&buf, "PIPELINE"), Some((69, 1)), "{:#?}", rows(&buf));
    let (x, _) = find(&buf, "── RECENT").unwrap();
    assert!(x < 67, "RECENT is not in the Shell's left part");

    let buf = render(&s, 100, 30);
    assert_eq!(buf[(8, 2)].symbol(), "╭", "{:#?}", rows(&buf));
    assert!(row(&buf, 2).contains("╭ /config ─"), "{:?}", row(&buf, 2));
    assert_eq!(find(&buf, "PIPELINE"), Some((10, 3)), "{:#?}", rows(&buf));
    assert!(row(&buf, 29).starts_with('›'), "{:#?}", rows(&buf));
}

/// The Review from claude to codex: picking the App leads into codex's
/// models, read from its catalog (the hidden one left out); the model is
/// probed and saves with the App, the effort back to default though codex
/// lists `high` too, and the effort, from that model's levels, saves at once.
#[test]
fn review_to_codex_a_listed_model_and_an_effort_save_all_three() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    write_file(
        &file,
        r#"{"review": {"app": "claude", "model": "opus", "effort": "high"}}"#,
    );
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "Review model · codex (new App)").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "gpt-5.6-sol").is_some(), "{:#?}", rows(&buf));
    assert!(
        find(&buf, "codex-auto-review").is_none(),
        "{:#?}",
        rows(&buf)
    );
    type_in(&mut s, "6-sol");
    s.key(key(KeyCode::Enter));
    await_probe(&mut s);
    assert!(
        tools
            .calls()
            .contains(&"codex exec --sandbox read-only -m gpt-6-sol Reply with ok".to_string()),
        "{:#?}",
        tools.calls()
    );
    assert_eq!(
        config_json(repo.path()),
        json!({"review": {"app": "codex", "model": "gpt-6-sol", "effort": "default"}})
    );
    // gpt-6-sol's levels: default, low, medium, high, xhigh.
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    keys(
        &mut s,
        &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
    );
    assert_eq!(
        config_json(repo.path()),
        json!({"review": {"app": "codex", "model": "gpt-6-sol", "effort": "high"}})
    );
    assert_eq!(
        note(&s),
        "saved: Review codex gpt-6-sol/high, uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "saved: Review codex gpt-6-sol/high, uncommitted in .orqadence/config.json"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(s.settings.as_ref().unwrap().saved.is_some());
}

/// A typed id is probed on the row's App before it saves; one the App
/// refuses keeps the old value, and the App's error shows in a red Notice
/// modal, not the foot. Enter closes it and /config is where it was.
#[test]
fn a_typed_id_whose_probe_fails_keeps_the_old_value_and_shows_the_error() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    let before = r#"{"implement": {"model": "opus"}}"#;
    write_file(&file, before);
    let tools = apps("claude-nope");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(
        &mut s,
        &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
    );
    type_in(&mut s, "type"); // only 'type an id…' is left
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "claude-nope");
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "Implement model id › claude-nope▏").is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "probing claude-nope on claude with a one-line prompt…"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    await_probe(&mut s);
    let probe = tools
        .calls()
        .into_iter()
        .find(|c| c.contains("claude-nope"))
        .unwrap();
    assert!(
        probe.starts_with("claude --tools Read,Grep,Glob,Skill --add-dir ")
            && probe.ends_with(" --model claude-nope -p Reply with ok"),
        "{probe}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    let error = "claude refused claude-nope: model claude-nope not found. Nothing changed.";
    assert_eq!(notice_modal(&s), error);
    assert_eq!(note(&s), "");
    let buf = render(&s, 160, 45);
    let at = find(&buf, "╭ ERROR ─").expect("no ERROR box");
    assert_eq!(buf[at].fg, RED);
    assert!(
        find(&buf, "claude refused claude-nope:").is_some(),
        "{:#?}",
        rows(&buf)
    );
    let place = |s: &Screen| {
        let st = s.settings.as_ref().unwrap();
        (st.section, st.open, st.setting, st.pick.is_some())
    };
    let was = place(&s);
    s.key(key(KeyCode::Enter));
    assert_eq!(notice_modal(&s), "");
    assert_eq!(place(&s), was);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "opus").is_some(), "the old model is not shown");
}

/// A model probed once saves unprobed the next time it is picked, till
/// /config closes: opened anew, it is probed again.
#[test]
fn a_model_is_probed_once_while_config_is_open() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"implement": {"model": "opus"}}"#,
    );
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    let pick = |s: &mut Screen, id: &str| {
        s.key(key(KeyCode::Enter));
        type_in(s, "type");
        s.key(key(KeyCode::Enter));
        type_in(s, id);
        s.key(key(KeyCode::Enter));
        await_probe(s);
        assert_eq!(
            config_json(repo.path())["implement"]["model"],
            json!(id),
            "{:#?}",
            tools.calls()
        );
    };
    let probes = |id: &str| {
        let tail = format!(" --model {id} -p Reply with ok");
        tools.calls().iter().filter(|c| c.ends_with(&tail)).count()
    };
    let to_model = [KeyCode::Enter, KeyCode::Down, KeyCode::Down];
    type_line(&mut s, "/config");
    keys(&mut s, &to_model);
    pick(&mut s, "claude-a");
    pick(&mut s, "claude-b");
    pick(&mut s, "claude-a");
    assert_eq!((probes("claude-a"), probes("claude-b")), (1, 1));
    while s.settings.is_some() {
        s.key(key(KeyCode::Esc));
    }
    type_line(&mut s, "/config");
    keys(&mut s, &to_model);
    pick(&mut s, "claude-b");
    assert_eq!(probes("claude-b"), 2);
}

/// none, typed for a row other than the Review's fallback, is refused
/// before any probe: it would run as a model.
#[test]
fn none_typed_off_the_fallback_is_refused_unprobed() {
    let repo = TempDir::new();
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(
        &mut s,
        &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
    );
    type_in(&mut s, "type");
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "none");
    let calls = tools.calls().len();
    s.key(key(KeyCode::Enter));
    assert!(s.settings.as_ref().unwrap().probe.is_none());
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());
    let file = repo.path().join(".orqadence/config.json");
    assert_eq!(
        note(&s),
        format!(
            "Refused: {}: implement model none: only review_if_limited takes none. \
             Nothing changed.",
            file.display()
        )
    );
    assert!(!file.exists());
}

/// During a run a change saves at once and RECENT and the log say so; the
/// Stage that starts after it runs on it.
#[test]
fn a_change_during_a_run_logs_config_and_the_next_stage_starts_on_it() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.hook(|_, argv| {
        (argv.join(" ") == "codex debug models --bundled").then(|| Ok(CATALOG.to_string()))
    });
    // Implement holds until the change is saved.
    let (release, held) = channel::<()>();
    let held = Mutex::new(held);
    w.session(move |p| {
        if p.stage == "implement" {
            let _ = held.lock().unwrap().recv();
        }
        succeed(p)
    });
    let mut s = shell(&w);
    s.command("/start-ticket hx-1");
    await_line(&mut s, "hx-1 implement started: claude");
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(row(&buf, 0).contains(" run live "), "{:?}", row(&buf, 0));
    // Review, its model, gpt-6-sol
    keys(
        &mut s,
        &[KeyCode::Down, KeyCode::Enter, KeyCode::Down, KeyCode::Enter],
    );
    type_in(&mut s, "6-sol");
    s.key(key(KeyCode::Enter));
    await_probe(&mut s);
    assert_eq!(
        note(&s),
        "saved: Review codex gpt-6-sol, uncommitted. Stages that start from now use it; running ones keep theirs."
    );
    await_line(&mut s, "config: Review codex → codex gpt-6-sol");
    assert!(logged(&w, "config: Review codex → codex gpt-6-sol"));
    drop(release);
    await_line(&mut s, "hx-1 review 1 started: codex gpt-6-sol");
    let start = &w.called("herdr agent start h-hx-1-review")[0];
    assert!(
        start.ends_with("-- --sandbox workspace-write -m gpt-6-sol"),
        "{start}"
    );
}

/// The Pipeline list with each section's summary, a section's page with its
/// rows grouped, and a pick list with each model's family and the current
/// one marked.
#[test]
fn the_pipeline_list_a_stage_page_and_a_pick_list_render() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"implement": {"model": "opus", "effort": "high"}, "side_b": {"model": "gpt-6-sol"}}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    let blocked = Ask::Blocked {
        pane: "2-1".to_string(),
    };
    s.push(asking("harness-kqe.10", "blocked in fix 1", blocked));
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(
        row(&buf, 0).ends_with("━ 1 waiting ┓"),
        "{:?}",
        row(&buf, 0)
    );
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let left: Vec<String> = (1..21).map(|y| text(&buf, y, 69, 97)).collect();
    assert_eq!(
        left,
        [
            "PIPELINE",
            "▸ Plan+Impl claude opus",
            "  │",
            "  Review    codex",
            "  │",
            "  Debate    claude+codex",
            "  │",
            "  Fix       claude",
            "  │",
            "  Rebase    claude",
            "  │",
            "  Comments  claude",
            "  │",
            "  Release   claude",
            "────────────────────────────",
            "  Apps      6 of 6 installed",
            "  Skills    0 installed",
            "  Labels    0 labels",
            "  TypeSafe  on",
            "  Run       3 at once",
        ]
    );
    assert_eq!(buf[(71, 2)].fg, PURPLE, "the picked section is not marked");

    // The Review's page: its fallback starts as none.
    keys(&mut s, &[KeyCode::Down]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "  if limited: model     none  no fallback").is_some(),
        "{:#?}",
        rows(&buf)
    );

    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Down]);
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..24).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Debate  claude, codex",
            "The Moderator puts each disputed Finding to side A and side",
            "B, two models from different families; TypeSafe scores what",
            "they still dispute.",
            "",
            "  Moderator app         claude",
            "▸ Moderator model       default  Anthropic",
            "  Moderator effort      default",
            "",
            "  side A app            claude",
            "  side A model          default  Anthropic",
            "  side A effort         default",
            "",
            "  side B app            codex",
            "  side B model          gpt-6-sol  OpenAI",
            "  side B effort         default",
            "",
            "DELEGATE SKILLS",
            "  over-engineering audit  orqa-ponytail-review  not install",
            "",
            "CHECKS",
            "  ✓ The sides come from two families: Anthropic and OpenAI.",
            "",
        ]
    );
    assert!(
        row(&buf, 42).contains(
            "The Moderator's pane: runs the Debate and settles each Finding. Default passes no flag."
        ),
        "{:#?}",
        rows(&buf)
    );

    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Up, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..9).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Implement model · claude   filter › ▏",
            "  default           claude's own · Anthropic",
            "  fable             Anthropic",
            "▸ opus              Anthropic                    ✓ current",
            "  sonnet            Anthropic",
            "  haiku             Anthropic",
            "  type an id…       probed before it saves",
            "",
        ]
    );
    assert!(
        row(&buf, 44).contains("↑↓ move · type to filter · Enter picks · Esc back"),
        "{:?}",
        row(&buf, 44)
    );
}

/// Tools that answer codex's catalog and refuse every probe with `0`.
struct Refusing(RunError);

impl Tools for Refusing {
    fn run(&self, _: &Path, argv: &[&str]) -> Result<String, RunError> {
        match argv {
            ["codex", "debug", ..] => Ok(CATALOG.to_string()),
            [.., "Reply with ok"] => Err(self.0.clone()),
            _ => Ok(String::new()),
        }
    }
}

/// The App's own error, as each App prints it: claude -p on stdout under a
/// warning on stderr, codex exec a JSON line ending its stderr; it shows in
/// a Notice modal, not the foot.
#[test]
fn a_refused_probe_shows_what_the_app_said() {
    let claude = RunError {
        command: String::new(),
        status: "exit status 1".to_string(),
        stderr: "\"opus\" isn't described by this version's model catalog".to_string(),
        stdout: "There's an issue with the selected model (opus). It may not exist.\n".to_string(),
    };
    let codex = RunError {
        stderr: "codex v0.156.1\nmodel: gpt-6-sol\nERROR: {\"type\":\"error\",\"status\":400,\
            \"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'gpt-6-sol' model \
            is not supported when using Codex with a ChatGPT account.\"}}"
            .to_string(),
        stdout: String::new(),
        ..claude.clone()
    };
    for (err, section, want) in [
        (
            claude,
            0,
            "claude refused opus: There's an issue with the selected model (opus). \
             It may not exist. Nothing changed.",
        ),
        (
            codex,
            1,
            "codex refused gpt-6-sol: The 'gpt-6-sol' model is not supported when \
             using Codex with a ChatGPT account. Nothing changed.",
        ),
    ] {
        let repo = TempDir::new();
        let mut s = screen_at(Arc::new(Refusing(err)), repo.path());
        type_line(&mut s, "/config");
        for _ in 0..section {
            s.key(key(KeyCode::Down));
        }
        keys(&mut s, &[KeyCode::Enter, KeyCode::Down]);
        if section == 0 {
            s.key(key(KeyCode::Down)); // past the toggle
        }
        s.key(key(KeyCode::Enter));
        type_in(&mut s, if section == 0 { "opus" } else { "6-sol" });
        s.key(key(KeyCode::Enter));
        await_probe(&mut s);
        assert_eq!(notice_modal(&s), want);
        assert_eq!(note(&s), "");
        assert!(!repo.path().join(".orqadence/config.json").exists());
    }
}

/// An App whose model listing failed raises a red Notice modal with its
/// error as its Model pick list opens, never as /config opens nor for
/// another App's list; the list behind it has no error row, still offers
/// default and 'type an id…', and stays open once Enter closes the notice.
#[test]
fn a_failed_model_listing_raises_a_notice_as_its_model_list_opens() {
    let repo = TempDir::new();
    let mut s = screen_at(apps("--list-models"), repo.path()); // pi's listing
    type_line(&mut s, "/config");
    assert_eq!(notice_modal(&s), "", "opening /config raised a notice");
    // The Review's App list, from codex: pi leads into its model list.
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    assert_eq!(
        notice_modal(&s),
        "pi could not list its models: pi --list-models: exit status 1: \
         model --list-models not found"
    );
    assert!(matches!(s.notices[0].kind, NoticeKind::Error));
    let st = s.settings.as_ref().unwrap();
    let names: Vec<String> = st
        .entries(st.pick.as_ref().expect("no pick list behind the notice"))
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["default", "type an id…"]);

    let buf = render(&s, 160, 45);
    let at = find(&buf, "╭ ERROR ─").expect("no ERROR box");
    assert_eq!(buf[at].fg, RED);
    let (_, y) = find(&buf, "pi could not list its models:").expect("no error text");
    assert_eq!(y, at.1 + 1, "{:#?}", rows(&buf));
    assert!(
        find(&buf, "Review model · pi (new App)").is_some(),
        "{:#?}",
        rows(&buf)
    );

    s.key(key(KeyCode::Enter));
    assert_eq!(notice_modal(&s), "");
    assert!(
        s.settings.as_ref().unwrap().pick.is_some(),
        "the list closed"
    );
    // Implement's model list, on claude.
    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Left, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    assert!(
        s.settings.as_ref().unwrap().pick.is_some(),
        "no list opened"
    );
    assert_eq!(notice_modal(&s), "", "another App's list raised a notice");
}

/// A model that does not list the row's effort takes it back to default;
/// picking the value a row already has changes nothing, not even a probe.
#[test]
fn a_model_without_the_effort_resets_it_and_the_current_pick_changes_nothing() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    write_file(
        &file,
        r#"{"review": {"model": "gpt-6-sol", "effort": "xhigh"}}"#,
    );
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(
        &mut s,
        &[KeyCode::Down, KeyCode::Enter, KeyCode::Down, KeyCode::Enter],
    );
    type_in(&mut s, "5.6-sol"); // it lists low alone
    s.key(key(KeyCode::Enter));
    await_probe(&mut s);
    assert_eq!(
        config_json(repo.path()),
        json!({"review": {"model": "gpt-5.6-sol", "effort": "default"}})
    );
    let (calls, saved) = (tools.calls().len(), std::fs::read_to_string(&file).unwrap());
    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]); // the cursor opens on the current one
    assert!(s.settings.as_ref().unwrap().probe.is_none());
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
    assert_eq!(note(&s), "");
}

/// A row on an App the table does not have still lists the Apps to move it
/// to one.
#[test]
fn a_row_on_an_unknown_app_can_be_moved_to_a_listed_one() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"fix": {"app": "gemini"}}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let open_fix = [
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Enter,
    ];
    keys(&mut s, &open_fix);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "▸ claude").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "  codex").is_some(), "{:#?}", rows(&buf));
    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]); // claude, then its default
    assert_eq!(
        config_json(repo.path()),
        json!({"fix": {"app": "claude", "model": "default"}})
    );
}

/// TypeSafe reads off while config.json turns it off, the key set or not.
#[test]
fn typesafe_reads_off_while_config_json_turns_it_off() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"typesafe": false}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "TypeSafe  off").is_some(), "{:#?}", rows(&buf));
}

/// A config.json that is not an object is refused, never replaced: /config
/// will not open on it, and a change after a hand edit to one saves nothing.
#[test]
fn a_config_json_not_an_object_is_refused_not_replaced() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    write_file(&file, r#"{"fix": {"app": "gemini"}}"#);
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    write_file(&file, "[1]");
    let fix_to_claude_default = [
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Enter,
        KeyCode::Enter,
        KeyCode::Enter,
    ];
    keys(&mut s, &fix_to_claude_default);
    let why = format!("{}: not a JSON object", file.display());
    assert_eq!(note(&s), format!("Refused: {why}. Nothing changed."));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[1]");
    s.key(key(KeyCode::Esc));
    s.key(key(KeyCode::Esc));
    s.key(key(KeyCode::Esc));
    type_line(&mut s, "/config");
    assert!(s.settings.is_none());
    assert_eq!(super::shell_test::notice(&s), why);
}

/// A config.json edited by hand to break a rule refuses the run, naming
/// the rule, and /config marks it: ✗ on the Review, the badge counting it,
/// the rule spelt out under CHECKS.
#[test]
fn a_hand_edited_config_that_breaks_a_rule_refuses_the_run_and_shows_it() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    write_file(
        &w.repo.join(".orqadence/config.json"),
        r#"{"implement": {"model": "claude-opus-5-5"}, "review": {"app": "claude", "model": "opus"}}"#,
    );
    let mut s = shell(&w);
    s.command("/start-epic hx");
    let rule = "The Review would run on Implement's model, claude-opus-5-5: it must not review its own work";
    assert_eq!(super::shell_test::notice(&s), rule);
    assert!(s.run.is_none() && w.called("bd worktree create").is_empty());

    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(row(&buf, 0).contains("━ ✗ 1 check ┓"), "{:?}", row(&buf, 0));
    assert!(
        find(&buf, "  Review    claude opus ✗").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert_eq!(buf[(93, 4)].fg, super::brand::RED);
    keys(&mut s, &[KeyCode::Down]);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "CHECKS").is_some(), "{:#?}", rows(&buf));
    assert!(
        find(&buf, &format!("✗ {}", &rule[..40])).is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// A pick that would break a rule is refused before any probe: the rule
/// shows in the foot and config.json stays byte for byte as it was.
#[test]
fn a_refused_pick_says_the_rule_and_leaves_config_json_as_it_was() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    let before = "{ \"implement\":{\"model\":\"opus\"} }\n";
    write_file(&file, before);
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    // The Review's App, claude, then its opus-5.5: Implement's opus.
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Up, KeyCode::Enter]);
    type_in(&mut s, "type");
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "opus-5.5");
    s.key(key(KeyCode::Enter));
    assert!(s.settings.as_ref().unwrap().probe.is_none());
    assert!(
        !tools.calls().iter().any(|c| c.ends_with("Reply with ok")),
        "{:#?}",
        tools.calls()
    );
    assert_eq!(
        note(&s),
        "Refused: The Review would run on Implement's model, claude-opus-5-5: \
         it must not review its own work. Nothing changed."
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
}

/// Implement's fields put into config.json: a plan other than Implement's
/// model splits, both halves by full id; the plan on Implement's own model,
/// or a new App for Plan + Implement, plans on one model again.
// ponytail: a unit test, as Implement cannot leave claude on screen until
// harness-7nq.12 lifts runs_on; a screen test of the App change then.
#[test]
fn a_split_takes_full_ids_and_an_app_change_plans_on_one_model_again() {
    let implement = |doc: Value, fields: &[(Field, &str)]| {
        let mut doc = doc;
        let fields: Vec<_> = fields.iter().map(|(f, v)| (*f, v.to_string())).collect();
        put(&mut doc, "implement", &fields);
        doc
    };
    let split = implement(
        json!({"implement": {"model": "opus", "effort": "high"}}),
        &[(Field::Plan, "fable")],
    );
    assert_eq!(
        split,
        json!({"implement": {"model": "claude-opus-5-5", "plan_model": "claude-fable-5-1", "effort": "high"}})
    );
    let one = json!({"implement": {"model": "claude-opus-5-5", "effort": "high"}});
    assert_eq!(implement(split.clone(), &[(Field::Plan, "opus")]), one);
    assert_eq!(implement(split.clone(), &[(Field::Plan, "default")]), one);
    assert_eq!(
        implement(split, &[(Field::App, "codex"), (Field::Model, "gpt-6-sol")]),
        json!({"implement": {"app": "codex", "model": "gpt-6-sol", "effort": "high"}})
    );
    // Another row keeps what it is given.
    let mut doc = json!({});
    put(&mut doc, "review", &[(Field::Model, "opus".to_string())]);
    assert_eq!(doc, json!({"review": {"model": "opus"}}));
}

/// 'Same model for plan and implementation' is on by default; turning it
/// off with Implement's model at default is refused, as the split needs a
/// named model for each half.
#[test]
fn the_toggle_off_with_implement_at_default_is_refused() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ [x] Same model for plan and implementation").is_some(),
        "{:#?}",
        rows(&buf)
    );
    for code in [KeyCode::Enter, KeyCode::Char(' ')] {
        s.key(key(code));
        assert!(s.settings.as_ref().unwrap().pick.is_none());
        assert_eq!(
            note(&s),
            "Pick Implement's model first: the split needs a named model for each half."
        );
    }
    assert!(!repo.path().join(".orqadence/config.json").exists());
}

/// Turning the toggle off opens the plan's model list, Implement's App's
/// models of its family; Esc keeps one model. A plan model picked is probed
/// and splits, both halves saved by full id, with plan model and implement
/// model rows and the rule under CHECKS; the plan on Implement's own model
/// is one model again, and so is the toggle turned on.
#[test]
fn the_toggle_splits_on_a_plan_model_and_joins_again() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    write_file(
        &file,
        r#"{"implement": {"model": "opus", "effort": "high"}}"#,
    );
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Char(' ')]);
    let buf = render(&s, 160, 45);
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let right: Vec<String> = (1..8).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Implement plan model · claude   filter › ▏",
            "▸ fable             Anthropic",
            "  opus              Anthropic",
            "  sonnet            Anthropic",
            "  haiku             Anthropic",
            "  type an id…       probed before it saves",
            "",
        ]
    );
    assert!(
        find(
            &buf,
            "Pick the plan's model to split planning from implementing; Esc keeps one model for both."
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Esc));
    assert_eq!(
        config_json(repo.path()),
        json!({"implement": {"model": "opus", "effort": "high"}})
    );

    s.key(key(KeyCode::Char(' ')));
    s.key(key(KeyCode::Enter)); // fable
    await_probe(&mut s);
    assert!(
        tools
            .calls()
            .iter()
            .any(|c| c.ends_with("--model claude-fable-5-1 -p Reply with ok")),
        "{:#?}",
        tools.calls()
    );
    let split = json!({"implement": {
        "model": "claude-opus-5-5", "plan_model": "claude-fable-5-1", "effort": "high"
    }});
    assert_eq!(config_json(repo.path()), split);
    assert_eq!(
        note(&s),
        "saved: Implement claude claude-fable-5-1→claude-opus-5-5/high, uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (5..16).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "  app                   claude",
            "▸ [ ] Same model for plan and implementation",
            "  plan model            claude-fable-5-1  Anthropic",
            "  implement model       claude-opus-5-5  Anthropic",
            "  effort                high",
            "",
            "DELEGATE SKILLS",
            "  test-first              orqa-tdd  not installed",
            "  self review             orqa-code-review  not installed",
            "  working mode            orqa-ponytail  not installed",
            "  prose                   orqa-caveman  not installed",
        ]
    );

    // Implement's model list offers no default: the split needs a named one.
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "Implement model · claude").is_some() && find(&buf, "claude's own").is_none(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Esc, KeyCode::Up, KeyCode::Up]);

    // The plan on Implement's own model: one model again, nothing probed.
    let calls = tools.calls().len();
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "opus");
    s.key(key(KeyCode::Enter));
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());
    let one = json!({"implement": {"model": "claude-opus-5-5", "effort": "high"}});
    assert_eq!(config_json(repo.path()), one);

    // Split again, then the toggle turned on.
    keys(&mut s, &[KeyCode::Up, KeyCode::Char(' '), KeyCode::Enter]);
    await_probe(&mut s);
    assert_eq!(config_json(repo.path()), split);
    s.key(key(KeyCode::Enter));
    assert_eq!(config_json(repo.path()), one);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ [x] Same model for plan and implementation").is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// A pick list marks each model that would break a rule: every claude
/// model for side B, as side A is Anthropic; Implement's model in the
/// Review's list; the Review's model in Implement's.
#[test]
fn a_pick_list_marks_each_model_that_would_break_a_rule() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"implement": {"model": "opus"}, "review": {"app": "claude", "model": "sonnet"}}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    // side B's App, claude
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down; 6]);
    keys(&mut s, &[KeyCode::Enter, KeyCode::Up, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..8).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Debate side B model · claude (new App)   filter › ▏",
            "▸ default           claude's own · Anth… ✗ side A's family",
            "  fable             Anthropic            ✗ side A's family",
            "  opus              Anthropic            ✗ side A's family",
            "  sonnet            Anthropic            ✗ side A's family",
            "  haiku             Anthropic            ✗ side A's family",
            "  type an id…       probed before it sa…",
        ]
    );
    let (x, y) = find(&buf, "✗ side A's family").unwrap();
    assert_eq!(buf[(x, y)].fg, super::brand::RED);

    // The Review's model list: Implement's opus.
    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Left, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        text(&buf, find(&buf, "  opus ").unwrap().1, 99, 158)
            .ends_with("Anthropic          ✗ Implement's model"),
        "{:#?}",
        rows(&buf)
    );
    // Implement's: the Review's sonnet.
    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Left, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        text(&buf, find(&buf, "  sonnet ").unwrap().1, 99, 158)
            .ends_with("Anthropic         ✗ the Review's model"),
        "{:#?}",
        rows(&buf)
    );
    assert!(
        text(&buf, find(&buf, "▸ opus ").unwrap().1, 99, 158).ends_with("✓ current"),
        "{:#?}",
        rows(&buf)
    );
}

/// Fake Tools where codex is not on PATH and claude says its version.
fn codex_missing() -> Arc<Fake> {
    Fake::new(|_, argv| match argv.join(" ").as_str() {
        "which codex" | "which cursor-agent" => Err(format!("{} not found", argv[1])),
        "claude --version" => Ok("2.1.282 (Claude Code)\n".to_string()),
        "pi --version" => Ok("0.87.1\n".to_string()),
        _ => Ok(String::new()),
    })
}

/// The Apps page: each App of the table installed with its version, or
/// greyed not installed with its homepage, found by its binary (cursor's is
/// cursor-agent); the four from their docs under experimental, unverified.
#[test]
fn the_apps_page_renders_each_app_installed_or_not() {
    let repo = TempDir::new();
    let mut s = screen_at(codex_missing(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; APPS_PAGE]);
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    assert_eq!(text(&buf, 16, 69, 97), "▸ Apps      4 of 6 installed");
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..15).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Apps  4 of 6 installed",
            "The agent CLIs a Stage runs on, found on PATH when /config",
            "opened; orqa init installs herdr's integration for each.",
            "",
            "▸ claude      installed      2.1.282 (Claude Code)",
            "  codex       not installed  https://developers.openai.com…",
            "",
            "  experimental, unverified: from their docs, never run here",
            "  pi          installed      0.87.1",
            "  opencode    installed",
            "  copilot     installed",
            "  cursor      not installed  https://cursor.com/cli",
            "",
            "",
        ]
    );
    let (x, y) = find(&buf, "codex       not installed").unwrap();
    assert_eq!(buf[(x, y)].fg, super::brand::MUTED);
}

/// An App not installed is greyed in an App list; picking it says where
/// to get it and changes nothing. So does the Apps page's foot on it.
#[test]
fn an_app_not_installed_says_where_to_get_it() {
    let repo = TempDir::new();
    let tools = codex_missing();
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    // The Review's App list, codex: its current App, greyed.
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    let (x, y) = find(&buf, "codex").unwrap();
    assert_eq!(buf[(x, y)].fg, super::brand::MUTED);
    assert!(
        cols(&buf, y, 99, 158).contains("not installed"),
        "{:#?}",
        rows(&buf)
    );
    let calls = tools.calls().len();
    s.key(key(KeyCode::Enter));
    let where_ = "codex is not on PATH: get it at https://developers.openai.com/codex";
    assert_eq!(note(&s), where_);
    assert!(s.settings.as_ref().unwrap().pick.is_none());
    let buf = render(&s, 160, 45);
    assert!(find(&buf, where_).is_some(), "{:#?}", rows(&buf));
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());

    // From the Apps page.
    s.key(key(KeyCode::Left));
    keys(&mut s, &[KeyCode::Down; APPS_PAGE - 1]);
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down]);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, where_).is_some(), "{:#?}", rows(&buf));
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());
    assert!(!repo.path().join(".orqadence/config.json").exists());
}

/// A config.json broken by hand on two rules mends one rule at a time: a
/// change that leaves a rule broken as it was saves, one that breaks a
/// rule that held is refused.
#[test]
fn a_config_broken_on_two_rules_mends_one_at_a_time() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    write_file(
        &file,
        r#"{"implement": {"model": "opus"}, "review": {"app": "claude", "model": "opus"},
            "side_b": {"app": "claude"}}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(
        row(&buf, 0).contains("━ ✗ 2 checks ┓"),
        "{:?}",
        row(&buf, 0)
    );
    // The Review to codex, its default: the Debate stays broken.
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(
        config_json(repo.path())["review"],
        json!({"app": "codex", "model": "default"})
    );
    let buf = render(&s, 160, 45);
    assert!(
        row(&buf, 0).contains(" ✗ 1 check · saved "),
        "{:?}",
        row(&buf, 0)
    );
    // The Review back on Implement's model breaks a rule that held.
    let saved = std::fs::read_to_string(&file).unwrap();
    keys(&mut s, &[KeyCode::Enter, KeyCode::Up, KeyCode::Enter]);
    type_in(&mut s, "opus");
    s.key(key(KeyCode::Enter));
    assert!(note(&s).starts_with("Refused: The Review would run on Implement's model"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
}

/// An experimental App is marked so in an App list. On an App that runs
/// several, each model shows its own family, and its default, whose family
/// cannot be told, is marked where a rule reads the row.
#[test]
fn a_side_on_pi_shows_each_models_family_and_its_default_unknown() {
    let repo = TempDir::new();
    let tools = Fake::new(|_, argv| match argv.join(" ").as_str() {
        "pi --list-models" => Ok("provider   model            context\n\
             anthropic  claude-opus-5-5  200K\n\
             openai     gpt-6            400K\n"
            .to_string()),
        _ => Ok(String::new()),
    });
    let mut s = screen_at(tools, repo.path());
    type_line(&mut s, "/config");
    // The Debate, side A's App.
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down; 3]);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    let (_, y) = find(&buf, "▸ claude").unwrap();
    assert!(
        cols(&buf, y + 2, 99, 158).contains("pi          experimental, unverified"),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    for (model, family) in [
        ("default", "pi's own · family u… ? family unknown"),
        ("anthropic/claude", "Anthropic"),
        ("openai/gpt-6", "OpenAI"),
    ] {
        let (_, y) = find(&buf, model).unwrap();
        assert!(
            cols(&buf, y, 99, 158).contains(family),
            "{model}: {:#?}",
            rows(&buf)
        );
    }
    let (_, y) = find(&buf, "openai/gpt-6").unwrap();
    assert!(cols(&buf, y, 99, 158).contains("✗ side B's family"));
}

const TDD: &str = "---\nname: tdd\n---\ntest first\n";

/// A two-skill pack as a clone finds it.
const PACK: &[(&str, &str)] = &[
    ("skills/engineering/tdd/SKILL.md", TDD),
    (
        "skills/engineering/code-review/SKILL.md",
        "---\nname: code-review\n---\n",
    ),
];

/// Fake Tools where git clone writes the pack into its destination and
/// rev-parse answers abc1234def; everything else is "".
fn clones() -> Arc<Fake> {
    Fake::new(|_, argv| {
        if argv.contains(&"clone") {
            let dest = Path::new(argv.last().unwrap());
            for (path, text) in PACK {
                write_file(&dest.join(path), text);
            }
            Ok(String::new())
        } else if argv.contains(&"rev-parse") {
            Ok("abc1234def\n".to_string())
        } else {
            Ok(String::new())
        }
    })
}

/// Polls until /config's clone or update has finished.
fn await_busy(s: &mut Screen) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.settings.as_ref().unwrap().busy.is_some() {
        assert!(Instant::now() < deadline, "the clone never finished");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn manifest(repo: &Path) -> Manifest {
    Manifest::load(repo).unwrap()
}

/// /config open on the Skills page.
fn skills_page(tools: Arc<Fake>, repo: &Path) -> Screen {
    let mut s = screen_at(tools, repo);
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; SKILLS_PAGE]);
    s.key(key(KeyCode::Enter));
    s
}

/// A bare name is not a source: refused at once with where to find one,
/// the text kept to mend, nothing cloned.
#[test]
fn adding_a_bare_name_is_refused_before_any_clone() {
    let repo = TempDir::new();
    let tools = clones();
    let mut s = skills_page(tools.clone(), repo.path());
    s.key(key(KeyCode::Char('a')));
    type_in(&mut s, "tdd");
    s.key(key(KeyCode::Enter));
    assert!(note(&s).contains("skills.sh"), "{}", note(&s));
    assert!(s.settings.as_ref().unwrap().typing.is_some());
    assert!(
        !tools.calls().iter().any(|c| c.contains("clone")),
        "{:#?}",
        tools.calls()
    );
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "source › tdd▏").is_some(), "{:#?}", rows(&buf));
}

/// Removing a skill a job uses asks first, naming the job; no keeps it,
/// yes removes it and sets that job to none.
#[test]
fn removing_a_skill_a_job_uses_asks_then_sets_the_job_to_none() {
    let repo = TempDir::new();
    let tools = clones();
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    let mut s = skills_page(tools, repo.path());
    keys(&mut s, &[KeyCode::Down, KeyCode::Down]); // the location, yours, then orqa-tdd
    s.key(key(KeyCode::Char('d')));
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "orqa-tdd is the Delegate skill for Plan + Implement test-first"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Char('n')));
    assert!(manifest(repo.path()).skills.contains_key("orqa-tdd"));
    s.key(key(KeyCode::Char('d')));
    s.key(key(KeyCode::Char('y')));
    let m = manifest(repo.path());
    assert!(!m.skills.contains_key("orqa-tdd"));
    assert_eq!(m.pick("test-first"), NONE);
    assert!(!repo.path().join(".agents/skills/orqa-tdd").exists());
    assert_eq!(note(&s), "removed orqa-tdd; Plan + Implement test-first is none; saved uncommitted: Tickets take the change once it is merged");
}

/// A source with two skills, one installed from it already: the checklist
/// shows that one ticked; the other ticked installs, and the note says so.
#[test]
fn adding_a_pack_shows_the_checklist_and_installs_the_ticked_skill() {
    let repo = TempDir::new();
    let tools = clones();
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    let mut s = skills_page(tools.clone(), repo.path());
    s.key(key(KeyCode::Char('a')));
    type_in(&mut s, "mattpocock/skills");
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "cloning mattpocock/skills…").is_some(),
        "{:#?}",
        rows(&buf)
    );
    await_busy(&mut s);
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..5).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Skills in mattpocock/skills",
            "▸ [ ] code-review",
            "  [x] tdd                     installed",
            "",
        ]
    );
    s.key(key(KeyCode::Enter));
    assert_eq!(note(&s), "Space ticks a skill to install.");
    keys(&mut s, &[KeyCode::Char(' '), KeyCode::Enter]);
    await_busy(&mut s);
    let m = manifest(repo.path());
    assert_eq!(
        m.skills.keys().collect::<Vec<_>>(),
        ["orqa-code-review", "orqa-tdd"],
        "{m:?}"
    );
    assert_eq!(m.skills["orqa-code-review"].commit, "abc1234def");
    assert_eq!(
        note(&s),
        "installed orqa-code-review from mattpocock/skills @ abc1234; saved uncommitted: Tickets take the change once it is merged"
    );
    assert!(s.settings.as_ref().unwrap().listing.is_none());
}

/// A job's suggestion not installed: picking it clones its source, installs
/// it and picks it.
#[test]
fn picking_a_suggestion_not_installed_clones_it_and_picks_it() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/skills.json"),
        r#"{"picks": {"test-first": "none"}}"#,
    );
    let tools = clones();
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "  orqa-tdd ")
            .is_some_and(|(_, y)| cols(&buf, y, 99, 158).contains("not installed")),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "▸ none").is_some(), "{:#?}", rows(&buf));
    // from none, the current pick, up past the other two suggestions
    keys(
        &mut s,
        &[KeyCode::Up, KeyCode::Up, KeyCode::Up, KeyCode::Enter],
    );
    await_busy(&mut s);
    assert!(
        tools
            .calls()
            .iter()
            .any(|c| c.contains("clone") && c.contains("https://github.com/mattpocock/skills")),
        "{:#?}",
        tools.calls()
    );
    let m = manifest(repo.path());
    assert!(m.skills.contains_key("orqa-tdd"), "{m:?}");
    assert_eq!(m.pick("test-first"), "orqa-tdd");
    assert!(repo
        .path()
        .join(".agents/skills/orqa-tdd/SKILL.md")
        .is_file());
    assert_eq!(
        note(&s),
        "installed orqa-tdd from mattpocock/skills @ abc1234; Plan + Implement test-first uses it; saved uncommitted: Tickets take the change once it is merged"
    );
}

/// A job lists only the skills its Stage's App loads: the Review on codex
/// hides a personal skill only Claude loads, and lists codex's built-in;
/// Implement on claude lists it.
#[test]
fn the_review_job_on_codex_hides_a_claude_only_skill() {
    let (repo, home) = (TempDir::new(), TempDir::new());
    write_file(
        &home.path().join(".claude/skills/grilling/SKILL.md"),
        "---\nname: grilling\n---\n",
    );
    write_file(
        &home.path().join(".agents/skills/archify/SKILL.md"),
        "---\nname: archify\n---\n",
    );
    set_personal(repo.path(), true).unwrap();
    let mut s = screen_at(apps(""), repo.path());
    s.cfg.home = home.path().to_path_buf();
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down; 6]);
    s.key(key(KeyCode::Enter));
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..10).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Review review · codex   filter › ▏",
            "  SUGGESTED",
            "  review-agent            built into codex built in",
            "  orqa-requesting-code-r… obra/superpowers not installed",
            "▸ none                    the Stage skill… ✓ current",
            "  YOUR OTHER SKILLS CODEX CAN SEE",
            "  archify                 ~/.agents/skills yours",
            "",
            "",
        ]
    );
    assert!(find(&buf, "grilling").is_none(), "{:#?}", rows(&buf));

    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Left, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down; 4]);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "grilling").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "archify").is_none(), "{:#?}", rows(&buf));
}

/// The Address PR comments page lists its PR comments job at the Shipped
/// address-pr-comments, installed, and its pick list suggests it.
#[test]
fn the_comments_page_lists_the_pr_comments_job_at_the_shipped_skill() {
    let (repo, home) = (TempDir::new(), TempDir::new());
    crate::setup::install_skills(
        repo.path(),
        home.path(),
        false,
        &mut std::io::sink(),
        &mut std::io::empty(),
        false,
    )
    .unwrap();
    let mut s = screen_at(apps(""), repo.path());
    s.cfg.home = home.path().to_path_buf();
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; 5]);
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..30).map(|y| text(&buf, y, 99, 158)).collect();
    let at = right.iter().position(|l| l == "DELEGATE SKILLS");
    let job = at.and_then(|at| right.get(at + 1));
    assert_eq!(
        job.map(String::as_str),
        Some("  PR comments             orqa-address-pr-comments  shipped"),
        "{right:#?}"
    );

    s.key(key(KeyCode::Enter));
    keys(&mut s, &[KeyCode::Down; 10]);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..5).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right[1..3],
        [
            "  SUGGESTED",
            "▸ orqa-address-pr-commen… shipped              installed ✓",
        ],
        "{right:#?}"
    );
}

/// The Skills page: where the skills live, read-only, then each skill, a
/// Shipped one said so, a fetched one with its source @ commit and the jobs
/// using it.
#[test]
fn the_skills_page_renders_the_location_and_each_skill() {
    let repo = TempDir::new();
    let tools = clones();
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    let mut m = manifest(repo.path());
    m.skills.insert(
        "orqa-create-pr".to_string(),
        crate::skills::manifest::Installed {
            shipped: true,
            ..Default::default()
        },
    );
    m.save(repo.path()).unwrap();
    let mut s = skills_page(tools, repo.path());
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    assert_eq!(text(&buf, 17, 69, 97), "▸ Skills    1 installed");
    let right: Vec<String> = (1..14).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Skills  1 installed",
            "The skills Orqadence installed, from their sources; the",
            "Skill manifest is .orqadence/skills.json.",
            "",
            "▸ location    .orqadence/skills, committed",
            "    linked from .agents/skills and .claude/skills",
            "  yours       off: Orqadence's and the repo's only",
            "    ~/.claude/skills, ~/.agents/skills and Claude Code",
            "      plugins",
            "",
            "  orqa-create-pr          shipped with Orqadence",
            "  orqa-tdd                mattpocock/skills @ abc1234",
            "    ← Plan + Implement test-first",
        ]
    );
    assert!(
        find(
            &buf,
            "Committed: a skill change here takes effect for Tickets once it is merged."
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Char('d')]);
    assert_eq!(
        note(&s),
        "orqa-create-pr is a Shipped skill: it cannot be removed."
    );
    assert!(s.settings.as_ref().unwrap().confirm.is_none());
}

/// Your personal skills' switch, off by default, turns on and off from the
/// Skills page, in the per-person config.json, and the pick lists follow.
#[test]
fn the_skills_page_turns_your_personal_skills_on_and_off() {
    let (repo, home) = (TempDir::new(), TempDir::new());
    write_file(
        &home.path().join(".claude/skills/grilling/SKILL.md"),
        "---\nname: grilling\n---\n",
    );
    let tools = clones();
    let mut s = screen_at(tools.clone(), repo.path());
    s.cfg.home = home.path().to_path_buf();
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; SKILLS_PAGE]);
    s.key(key(KeyCode::Enter));
    let seen = |s: &Screen| {
        s.settings
            .as_ref()
            .unwrap()
            .found
            .iter()
            .any(|(n, _)| n == "grilling")
    };
    assert!(!seen(&s), "a personal skill listed while off");

    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    assert!(personal(repo.path()));
    assert!(seen(&s), "a personal skill not listed once on");
    assert_eq!(
        note(&s),
        "your personal skills on: the jobs can pick them, saved in .orqadence-local/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ yours       on, for you alone").is_some(),
        "{:#?}",
        rows(&buf)
    );

    s.key(key(KeyCode::Char(' ')));
    assert!(!personal(repo.path()));
    assert!(!seen(&s));
}

/// /config open on the TypeSafe page.
fn typesafe_page(s: &mut Screen) {
    type_line(s, "/config");
    keys(s, &[KeyCode::Down; TYPESAFE_PAGE]);
    s.key(key(KeyCode::Enter));
}

/// TypeSafe on: turning it off asks first, saying what changes; yes saves
/// off in config.json.
#[test]
fn typesafe_off_asks_then_saves_off() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    typesafe_page(&mut s);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "TypeSafe  on").is_some(), "{:#?}", rows(&buf));
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "Turn TypeSafe off? Every Wake and Plan becomes a Question"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Char('n')));
    assert!(!repo.path().join(".orqadence/config.json").exists());
    s.key(key(KeyCode::Enter));
    s.key(key(KeyCode::Char('y')));
    assert_eq!(config_json(repo.path()), json!({"typesafe": false}));
    assert_eq!(
        note(&s),
        "TypeSafe off, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "TypeSafe  off").is_some(), "{:#?}", rows(&buf));
}

/// TypeSafe on without a key asks the key, shown as dots; it is kept as
/// init keeps it, readable only by you, and TypeSafe saves on.
#[test]
fn typesafe_on_without_a_key_asks_it_masked() {
    use std::os::unix::fs::PermissionsExt;
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"typesafe": false}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    s.cfg.api_key = String::new();
    typesafe_page(&mut s);
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "ts_live_51c9d0e7");
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "TypeSafe key › ••••••••••••••••▏").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "ts_live").is_none(), "{:#?}", rows(&buf));
    assert!(
        find(&buf, "kept in .orqadence-local/typesafe-key").is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Enter));
    let file = repo.path().join(".orqadence-local/typesafe-key");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "ts_live_51c9d0e7\n"
    );
    let mode = std::fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(config_json(repo.path()), json!({"typesafe": true}));
    assert_eq!(s.cfg.api_key, "ts_live_51c9d0e7");
    assert_eq!(
        note(&s),
        "TypeSafe on, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "••••d0e7").is_some(), "{:#?}", rows(&buf));
}

/// The TypeSafe page shows both floors, a missing one as its default; Enter
/// types one, saved at once to config.json; a value that is not a number
/// from 0 to 1 is refused, the text kept to mend; nothing typed puts the
/// default back.
#[test]
fn the_typesafe_page_shows_both_floors_and_saves_one_at_once() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"plan_floor": 0.6}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    typesafe_page(&mut s);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "  wake floor            0.70  default").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(
        find(&buf, "  plan floor            0.60 ").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "0.60  default").is_none(), "{:#?}", rows(&buf));

    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "0.8");
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "wake floor › 0.8▏").is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path()),
        json!({"plan_floor": 0.6, "wake_floor": 0.8})
    );
    assert_eq!(
        note(&s),
        "wake floor 0.80, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ wake floor            0.80").is_some(),
        "{:#?}",
        rows(&buf)
    );

    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "1.5");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: 1.5 is not a number from 0 to 1. Nothing changed."
    );
    assert_eq!(
        config_json(repo.path()),
        json!({"plan_floor": 0.6, "wake_floor": 0.8})
    );
    let typing = &s.settings.as_ref().unwrap().typing;
    assert!(matches!(typing, Some((_, text)) if text == "1.5"));

    // Esc, then nothing typed on the wake floor puts its default back.
    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Up, KeyCode::Enter, KeyCode::Enter],
    );
    assert_eq!(config_json(repo.path()), json!({"plan_floor": 0.6}));
    assert_eq!(
        note(&s),
        "wake floor 0.70, its default, saved uncommitted in .orqadence/config.json"
    );
}

/// A floor in config.json that is not a number from 0 to 1 is flagged: the
/// title's check badge, a ✗ on the TypeSafe line, its value in red, and the
/// check on the TypeSafe page.
#[test]
fn a_floor_that_is_not_a_number_from_0_to_1_is_flagged() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"wake_floor": "high"}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(row(&buf, 0).contains("━ ✗ 1 check ┓"), "{:?}", row(&buf, 0));
    assert!(find(&buf, "TypeSafe  on ✗").is_some(), "{:#?}", rows(&buf));

    keys(&mut s, &[KeyCode::Down; TYPESAFE_PAGE]);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    let (x, y) = find(&buf, "\"high\"").unwrap_or_else(|| panic!("{:#?}", rows(&buf)));
    assert_eq!(buf[(x, y)].fg, RED);
    assert!(
        find(
            &buf,
            "✗ wake_floor is not a number from 0 to 1: its Judgments"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// The Run page keeps the Tickets a run takes at once: its default while
/// config.json has none; Enter types it, saved at once; anything but a
/// whole number of at least 1 is refused, the text kept to mend; nothing
/// typed puts the default back. A bad one in config.json is flagged.
#[test]
fn the_run_page_keeps_the_tickets_a_run_takes_at_once() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; RUN_PAGE]);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ tickets at once       3  default").is_some(),
        "{:#?}",
        rows(&buf)
    );

    s.key(key(KeyCode::Enter));
    type_in(&mut s, "0");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: 0 is not a whole number of at least 1. Nothing changed."
    );
    s.key(key(KeyCode::Backspace));
    type_in(&mut s, "5");
    s.key(key(KeyCode::Enter));
    assert_eq!(config_json(repo.path()), json!({"max_tickets": 5}));
    assert_eq!(
        note(&s),
        "tickets at once: 5, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ Run       5 at once").is_some(),
        "{:#?}",
        rows(&buf)
    );

    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(config_json(repo.path()), json!({}));
    assert_eq!(
        note(&s),
        "tickets at once: 3, its default, saved uncommitted in .orqadence/config.json"
    );

    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"max_tickets": "lots"}"#,
    );
    keys(&mut s, &[KeyCode::Esc, KeyCode::Esc]);
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "Run       \"lots\" ✗").is_some(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Down; RUN_PAGE]);
    let buf = render(&s, 160, 45);
    let (x, y) =
        find(&buf, "tickets at once       \"lots\"").unwrap_or_else(|| panic!("{:#?}", rows(&buf)));
    assert_eq!(buf[(x + 22, y)].fg, RED);
    assert!(
        find(
            &buf,
            "✗ max_tickets is not a whole number of at least 1: its"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// The Rebase or the Address PR comments page, open.
fn pr_page(s: &mut Screen, section: usize) {
    type_line(s, "/config");
    keys(s, &vec![KeyCode::Down; section]);
    s.key(key(KeyCode::Enter));
}

/// The Rebase and Address PR comments pages: each its row, then its switch,
/// off by default, and Address PR comments' countdown and runs cap at
/// their defaults.
#[test]
fn the_rebase_and_address_pr_comments_pages_render_their_row_switch_and_numbers() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    pr_page(&mut s, REBASE_PAGE);
    let buf = render(&s, 160, 45);
    for line in [
        "▸ app                   claude",
        "  effort                default",
        "  [ ] Rebase PRs that conflict with main by themselves",
    ] {
        assert!(find(&buf, line).is_some(), "{line:?}: {:#?}", rows(&buf));
    }

    keys(&mut s, &[KeyCode::Esc, KeyCode::Esc]);
    pr_page(&mut s, ADDRESS_PR_COMMENTS_PAGE);
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..15)
        .map(|y| cols(&buf, y, 99, 158).trim_end().to_string())
        .collect();
    let at = right
        .iter()
        .position(|l| l.starts_with("▸ app"))
        .unwrap_or_else(|| panic!("{right:#?}"));
    assert_eq!(
        right[at..at + 9],
        [
            "▸ app                   claude",
            "  model                 default  Anthropic",
            "  effort                default",
            "",
            "  [ ] Open PR comments and failing checks for approval by",
            "      themselves",
            "  countdown minutes     5  default",
            "  runs per PR           3  default",
            "",
        ],
        "{right:#?}"
    );
}

/// Agent merge's settings on the Address PR comments page, under their own
/// heading: the switch off, no review bot ticked and bot_wait at 30.
#[test]
fn the_address_pr_comments_page_renders_agent_merge_under_its_heading() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    pr_page(&mut s, ADDRESS_PR_COMMENTS_PAGE);
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..30)
        .map(|y| cols(&buf, y, 99, 158).trim_end().to_string())
        .collect();
    let at = right
        .iter()
        .position(|l| l.starts_with("  runs per PR"))
        .unwrap_or_else(|| panic!("{right:#?}"));
    assert_eq!(
        right[at + 1..at + 7],
        [
            "",
            "AGENT MERGE",
            "  [ ] Merge Ticket PRs by themselves",
            "  [ ] review bot: coderabbit",
            "  [ ] review bot: greptile",
            "  bot wait minutes      30  default",
        ],
        "{right:#?}"
    );
}

/// Agent merge turns on only with automatic Address PR comments on: off,
/// /config refuses and says why; and turning that off turns Agent merge off
/// with a RECENT line.
#[test]
fn agent_merge_is_refused_without_automatic_address_pr_comments_and_goes_off_with_it() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    pr_page(&mut s, ADDRESS_PR_COMMENTS_PAGE);
    keys(&mut s, &[KeyCode::Down; 6]);
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Agent merge off: it needs PR comments and failing checks opened for approval by themselves (address_pr_comments_auto). Nothing changed."
    );
    assert!(!repo.path().join(".orqadence/config.json").exists());

    keys(&mut s, &[KeyCode::Up, KeyCode::Up, KeyCode::Up]);
    s.key(key(KeyCode::Enter));
    keys(&mut s, &[KeyCode::Down; 3]);
    s.key(key(KeyCode::Char(' ')));
    assert_eq!(
        config_json(repo.path()),
        json!({"address_pr_comments_auto": true, "agent_merge": true})
    );
    assert_eq!(
        note(&s),
        "Merge Ticket PRs by themselves: on, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ [x] Merge Ticket PRs by themselves").is_some(),
        "{:#?}",
        rows(&buf)
    );

    keys(&mut s, &[KeyCode::Up, KeyCode::Up, KeyCode::Up]);
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path()),
        json!({"address_pr_comments_auto": false, "agent_merge": false})
    );
    await_line(
        &mut s,
        "config: Merge Ticket PRs by themselves: off, as PR comments no longer open by themselves",
    );
}

/// Space or Enter on a review bot ticks it, saved at once in review_bots,
/// and again unticks it; bot_wait refuses 0 and keeps a whole number.
#[test]
fn ticking_a_review_bot_saves_review_bots_and_bot_wait_refuses_0() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    pr_page(&mut s, ADDRESS_PR_COMMENTS_PAGE);
    keys(&mut s, &[KeyCode::Down; 8]);
    s.key(key(KeyCode::Char(' ')));
    assert_eq!(
        config_json(repo.path()),
        json!({"review_bots": ["greptile"]})
    );
    assert_eq!(
        note(&s),
        "review bots: greptile, saved uncommitted in .orqadence/config.json"
    );
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path()),
        json!({"review_bots": ["coderabbit", "greptile"]})
    );
    let buf = render(&s, 160, 45);
    for line in ["▸ [x] review bot: coderabbit", "  [x] review bot: greptile"] {
        assert!(find(&buf, line).is_some(), "{line:?}: {:#?}", rows(&buf));
    }
    s.key(key(KeyCode::Char(' ')));
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    assert_eq!(config_json(repo.path()), json!({"review_bots": []}));
    assert_eq!(
        note(&s),
        "review bots: none, saved uncommitted in .orqadence/config.json"
    );

    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "0");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: 0 is not a whole number of at least 1. Nothing changed."
    );
    s.key(key(KeyCode::Esc));
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "45");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path()),
        json!({"review_bots": [], "bot_wait": 45})
    );
}

/// A review_bots that is not a list of the known bots is flagged on the
/// Address PR comments page, and ticking a bot writes the list anew.
#[test]
fn a_review_bots_that_cannot_be_read_is_flagged_and_a_tick_mends_it() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"review_bots": ["codrabbit"]}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    pr_page(&mut s, ADDRESS_PR_COMMENTS_PAGE);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "✗ review_bots is not a list of coderabbit, greptile").is_some(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Down; 7]);
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path()),
        json!({"review_bots": ["coderabbit"]})
    );
}

/// Enter or Space on a switch turns it on or off, saved at once.
#[test]
fn toggling_a_switch_saves_config_json_at_once() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    pr_page(&mut s, REBASE_PAGE);
    keys(&mut s, &[KeyCode::Down; 3]);
    s.key(key(KeyCode::Enter));
    assert_eq!(config_json(repo.path()), json!({"rebase_auto": true}));
    assert_eq!(
        note(&s),
        "Rebase PRs that conflict with main by themselves: on, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "▸ [x] Rebase PRs that conflict with main by themselves"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Char(' ')));
    assert_eq!(config_json(repo.path()), json!({"rebase_auto": false}));

    keys(&mut s, &[KeyCode::Esc, KeyCode::Down, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down; 3]);
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path()),
        json!({"rebase_auto": false, "address_pr_comments_auto": true})
    );
}

/// The Release page: its row, then its switch, off by default. Toggling the
/// switch and picking a model each save config.json at once.
#[test]
fn the_release_page_lists_its_row_and_switch_and_each_saves_at_once() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    pr_page(&mut s, RELEASE_PAGE);
    let buf = render(&s, 160, 45);
    for line in [
        "Release",
        "▸ app                   claude",
        "  model                 default  Anthropic",
        "  effort                default",
        "  [ ] Turn on releases (the orqa:release label)",
    ] {
        assert!(find(&buf, line).is_some(), "{line:?}: {:#?}", rows(&buf));
    }

    keys(&mut s, &[KeyCode::Down; 3]);
    s.key(key(KeyCode::Enter));
    assert_eq!(config_json(repo.path()), json!({"release_on": true}));
    assert_eq!(
        note(&s),
        "Turn on releases (the orqa:release label): on, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ [x] Turn on releases (the orqa:release label)").is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Char(' ')));
    assert_eq!(config_json(repo.path()), json!({"release_on": false}));

    keys(&mut s, &[KeyCode::Up, KeyCode::Up, KeyCode::Enter]);
    type_in(&mut s, "type");
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "claude-a");
    s.key(key(KeyCode::Enter));
    await_probe(&mut s);
    assert_eq!(
        config_json(repo.path()),
        json!({"release_on": false, "release": {"model": "claude-a"}})
    );
}

/// The countdown takes 0, no countdown, and refuses -1 and text, the text
/// kept to mend; the runs cap refuses 0. A bad one in config.json is
/// flagged on the page.
#[test]
fn the_countdown_takes_0_and_the_runs_cap_refuses_it() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    pr_page(&mut s, ADDRESS_PR_COMMENTS_PAGE);
    keys(&mut s, &[KeyCode::Down; 4]);
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "0");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path()),
        json!({"address_pr_comments_countdown": 0})
    );
    assert_eq!(
        note(&s),
        "countdown minutes: 0, saved uncommitted in .orqadence/config.json"
    );
    for typed in ["-1", "abc"] {
        s.key(key(KeyCode::Enter));
        type_in(&mut s, typed);
        s.key(key(KeyCode::Enter));
        assert_eq!(
            note(&s),
            format!("Refused: {typed} is not a whole number. Nothing changed.")
        );
        let st = s.settings.as_ref().unwrap();
        assert_eq!(st.typing.as_ref().map(|(_, t)| t.as_str()), Some(typed));
        s.key(key(KeyCode::Esc));
    }

    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "0");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: 0 is not a whole number of at least 1. Nothing changed."
    );
    s.key(key(KeyCode::Esc));
    assert_eq!(
        config_json(repo.path()),
        json!({"address_pr_comments_countdown": 0})
    );

    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"address_pr_comments_runs": 0}"#,
    );
    keys(&mut s, &[KeyCode::Esc, KeyCode::Esc]);
    pr_page(&mut s, ADDRESS_PR_COMMENTS_PAGE);
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "✗ address_pr_comments_runs is not a whole number of at"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// The Run page keeps max_pr_sessions under max_tickets: 0 is refused, a
/// whole number saved at once, and nothing typed puts the default 2 back.
#[test]
fn max_pr_sessions_refuses_0_and_a_blank_puts_the_default_back() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; RUN_PAGE]);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "  PR sessions at once   2  default").is_some(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "0");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: 0 is not a whole number of at least 1. Nothing changed."
    );
    s.key(key(KeyCode::Backspace));
    type_in(&mut s, "4");
    s.key(key(KeyCode::Enter));
    assert_eq!(config_json(repo.path()), json!({"max_pr_sessions": 4}));
    assert_eq!(
        note(&s),
        "PR sessions at once: 4, saved uncommitted in .orqadence/config.json"
    );
    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(config_json(repo.path()), json!({}));
    assert_eq!(
        note(&s),
        "PR sessions at once: 2, its default, saved uncommitted in .orqadence/config.json"
    );
}

/// u fetches a skill's source again and says the new commit; U fetches
/// every one, which here is up to date.
#[test]
fn updating_says_the_new_commit_or_up_to_date() {
    let repo = TempDir::new();
    let tools = clones();
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    let mut m = manifest(repo.path());
    m.skills.get_mut("orqa-tdd").unwrap().commit = "0000000old".to_string();
    m.save(repo.path()).unwrap();
    let mut s = skills_page(tools, repo.path());
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Char('u')]);
    await_busy(&mut s);
    assert_eq!(note(&s), "updated orqa-tdd 0000000 → abc1234; saved uncommitted: Tickets take the change once it is merged");
    assert_eq!(
        manifest(repo.path()).skills["orqa-tdd"].commit,
        "abc1234def"
    );
    s.key(key(KeyCode::Char('U')));
    await_busy(&mut s);
    assert_eq!(note(&s), "every skill is up to date");
}

/// A suggested skill you have from another source is used as it is:
/// picked, nothing cloned.
#[test]
fn a_suggestion_installed_from_a_fork_is_picked_as_it_is() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/skills.json"),
        r#"{"picks": {"test-first": "none"}}"#,
    );
    let tools = clones();
    add(repo.path(), &*tools, "someone/fork", Some("tdd")).unwrap();
    let calls = tools.calls().len();
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    let (_, y) = find(&buf, "  orqa-tdd ").unwrap();
    assert!(
        cols(&buf, y, 99, 158).contains("someone/fork     installed"),
        "{:#?}",
        rows(&buf)
    );
    keys(
        &mut s,
        &[KeyCode::Up, KeyCode::Up, KeyCode::Up, KeyCode::Enter],
    );
    assert!(s.settings.as_ref().unwrap().busy.is_none());
    assert_eq!(manifest(repo.path()).pick("test-first"), "orqa-tdd");
    assert_eq!(
        note(&s),
        "Plan + Implement test-first picks orqa-tdd, saved uncommitted in .orqadence/skills.json"
    );
    let cloned = tools.calls()[calls..].iter().any(|c| c.contains("clone"));
    assert!(!cloned, "{:#?}", tools.calls());
}

/// A garbled Skill manifest still opens /config, saying why no skills
/// show, and a pick refuses rather than save over it.
#[test]
fn a_garbled_manifest_opens_config_and_refuses_a_pick() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/skills.json");
    write_file(&file, "{");
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    assert!(
        note(&s).starts_with(".orqadence/skills.json: "),
        "{}",
        note(&s)
    );
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "none");
    s.key(key(KeyCode::Enter));
    assert!(note(&s).ends_with("Nothing changed."), "{}", note(&s));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{");
}

/// /config open on the On call page.
fn on_call_page(s: &mut Screen) {
    type_line(s, "/config");
    keys(s, &[KeyCode::Down; ON_CALL_PAGE]);
    s.key(key(KeyCode::Enter));
}

/// The per-person config.json's On call object.
fn on_call_json(repo: &Path) -> Value {
    let path = repo.join(".orqadence-local/config.json");
    serde_json::from_str::<Value>(&std::fs::read_to_string(path).unwrap()).unwrap()["on_call"]
        .clone()
}

/// The On call page: the token, masked or not set, the minutes and the test
/// push, with docs/on-call.md by its GitHub URL at its foot, whole on a
/// wide terminal.
#[test]
fn the_on_call_page_renders_its_three_rows_and_the_doc_url() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    on_call_page(&mut s);
    let buf = render(&s, 200, 45);
    for text in [
        "On call   off",
        "▸ token                 not set",
        "  minutes               5",
        "  send a test push",
        "https://github.com/muresanroland/orqadence/blob/main/docs/on-call.md",
    ] {
        assert!(find(&buf, text).is_some(), "{text:?}: {:#?}", rows(&buf));
    }
}

/// A short terminal drops the PIPELINE's connectors so the left's last
/// lines, Run and On call, still show.
#[test]
fn a_short_terminal_shows_every_line_on_the_left() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 24);
    for text in ["Run", "On call   off"] {
        assert!(find(&buf, text).is_some(), "{text:?}: {:#?}", rows(&buf));
    }
}

/// A typed token is shown as dots, saved at once, and the Screen's settings
/// read it; an empty entry clears it: On call off.
#[test]
fn a_typed_moshi_token_saves_masked_and_an_empty_one_clears_it() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    on_call_page(&mut s);
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "moshi_tok_51c9d0e7");
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "moshi_tok").is_none(), "{:#?}", rows(&buf));
    assert!(
        find(&buf, "Moshi token › ••••").is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Enter));
    assert_eq!(on_call_json(repo.path())["token"], "moshi_tok_51c9d0e7");
    assert_eq!(s.on_call.token.as_deref(), Some("moshi_tok_51c9d0e7"));
    assert_eq!(
        note(&s),
        "On call token set, saved in .orqadence-local/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "••••d0e7").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "On call   on").is_some(), "{:#?}", rows(&buf));

    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(on_call_json(repo.path())["token"], Value::Null);
    assert_eq!(s.on_call.token, None);
    assert_eq!(
        note(&s),
        "On call token cleared: On call off, saved in .orqadence-local/config.json"
    );

    // MOSHI_WEBHOOK_TOKEN set wins: the Screen keeps it, and the foot says so.
    s.moshi_env = true;
    s.on_call.token = Some("env-tok".to_string());
    keys(&mut s, &[KeyCode::Enter]);
    type_in(&mut s, "file-tok");
    s.key(key(KeyCode::Enter));
    assert_eq!(on_call_json(repo.path())["token"], "file-tok");
    assert_eq!(s.on_call.token.as_deref(), Some("env-tok"));
    assert_eq!(
        note(&s),
        "On call token set; MOSHI_WEBHOOK_TOKEN in the environment still wins, saved in .orqadence-local/config.json"
    );
}

/// The minutes: 0 is refused, the text kept to mend; 10 saves; nothing puts
/// the default 5 back.
#[test]
fn on_call_minutes_refuse_0_save_10_and_empty_puts_back_5() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    on_call_page(&mut s);
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "0");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: 0 is not a whole number of at least 1. Nothing changed."
    );
    let typing = &s.settings.as_ref().unwrap().typing;
    assert!(matches!(typing, Some((_, text)) if text == "0"));
    assert!(!repo.path().join(".orqadence-local/config.json").exists());

    s.key(key(KeyCode::Backspace));
    type_in(&mut s, "10");
    s.key(key(KeyCode::Enter));
    assert_eq!(on_call_json(repo.path())["minutes"], 10);
    assert_eq!(s.on_call.minutes, 10);
    assert_eq!(
        note(&s),
        "On call after 10 minutes, saved in .orqadence-local/config.json"
    );

    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(on_call_json(repo.path())["minutes"], 5);
    assert_eq!(s.on_call.minutes, 5);
    assert_eq!(
        note(&s),
        "On call after 5 minutes, its default, saved in .orqadence-local/config.json"
    );
}

/// Send a test push rings the doorbell once off the draw loop and says
/// sent, or the error; with no token it is refused and rings nothing.
#[test]
fn a_test_push_rings_once_and_says_sent_or_the_error() {
    use crate::on_call::FakeDoorbell;
    use std::sync::atomic::Ordering;
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    let bell = Arc::new(FakeDoorbell::default());
    s.doorbell = bell.clone();
    on_call_page(&mut s);
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    assert_eq!(
        note(&s),
        "Refused: no Moshi token set: Enter on token types one."
    );
    assert!(bell.rings.lock().unwrap().is_empty());

    s.on_call.token = Some("tok".to_string());
    s.key(key(KeyCode::Enter));
    await_busy(&mut s);
    let title = format!("orqa · {}", s.folder);
    assert_eq!(
        *bell.rings.lock().unwrap(),
        [("tok".into(), title, "test push from Orqadence".into())]
    );
    assert_eq!(note(&s), "test push sent");

    bell.fail.store(true, Ordering::SeqCst);
    s.key(key(KeyCode::Enter));
    await_busy(&mut s);
    assert_eq!(bell.rings.lock().unwrap().len(), 2);
    assert_eq!(note(&s), "test push failed: http status: 500");
}

/// Minutes past u32::MAX are refused as a file's are at load, so On call's
/// wait never overflows.
#[test]
fn on_call_minutes_refuse_past_u32_max() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    on_call_page(&mut s);
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "4294967296");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: 4294967296 is not a whole number of at least 1. Nothing changed."
    );
    assert_eq!(s.on_call.minutes, 5);
}

/// Two labels in config.json: fe with a skill, guidance and a PR template,
/// codex-review a modifier pinning a row.
const LABELS: &str = r#"{"labels": {
  "fe": {"kind": "area", "skills": ["orqa-a11y"], "guidance": "Build the UI.", "pr_template": "fe.md"},
  "codex-review": {"kind": "modifier", "rows": {"review": {"app": "codex"}}}
}}"#;

/// /config open on the Labels page.
fn labels_page(tools: Arc<Fake>, repo: &Path) -> Screen {
    let mut s = screen_at(tools, repo);
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; LABELS_PAGE]);
    s.key(key(KeyCode::Enter));
    s
}

/// The Labels page lists each label of config.json as orqa:<name> with its
/// kind and what it carries; the left counts them.
#[test]
fn the_labels_page_lists_each_label_and_renders() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), LABELS);
    let s = labels_page(clones(), repo.path());
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    assert_eq!(text(&buf, 18, 69, 97), "▸ Labels    2 labels");
    let right: Vec<String> = (1..8).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Labels  2 labels",
            "A bd label orqa:<name> on a Ticket changes how it runs: the",
            "skills and guidance its code-editing Stages get, its rows,",
            "its PR template and an Extra review.",
            "",
            "▸ orqa:codex-review   modifier",
            "  orqa:fe             area     skills: orqa-a11y · guidance",
        ]
    );
    assert!(
        find(&buf, "a adds a label, e or Enter opens it").is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// a types a name: Enter writes orqa:mobile as an area label at once and
/// opens its fields; picking an installed skill and typing a guidance line
/// each save at once, uncommitted.
#[test]
fn adding_a_label_as_an_area_with_an_installed_skill_writes_the_entry() {
    let repo = TempDir::new();
    let tools = clones();
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    let mut s = labels_page(tools, repo.path());
    s.key(key(KeyCode::Char('a')));
    type_in(&mut s, "mobile");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path())["labels"],
        json!({"mobile": {"kind": "area"}})
    );
    assert_eq!(
        note(&s),
        "added orqa:mobile, an area label, saved uncommitted in .orqadence/config.json"
    );
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..6).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "orqa:mobile",
            "",
            "▸ kind        area",
            "  skills      none",
            "  guidance    none",
        ]
    );

    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "orqa:mobile skills").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "▸ orqa-tdd").is_some(), "{:#?}", rows(&buf));
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"]["skills"],
        json!(["orqa-tdd"])
    );
    assert_eq!(
        note(&s),
        "orqa:mobile takes orqa-tdd, saved uncommitted in .orqadence/config.json"
    );
    // the list stays open, the skill marked on
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "on ✓").is_some(), "{:#?}", rows(&buf));
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"]["skills"],
        json!([])
    );
    keys(&mut s, &[KeyCode::Esc, KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "Small screens first.");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"]["guidance"],
        json!("Small screens first.")
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ guidance    Small screens first.").is_some(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Up, KeyCode::Up, KeyCode::Char(' ')]);
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"]["kind"],
        json!("modifier")
    );
}

/// A source typed on a label's skills clones it, installs the skill into
/// the Skill manifest, and lists it on the entry.
#[test]
fn a_source_typed_on_a_labels_skills_clones_it_and_lists_it_on_the_entry() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), LABELS);
    let tools = clones();
    let mut s = labels_page(tools.clone(), repo.path());
    keys(
        &mut s,
        &[KeyCode::Down, KeyCode::Enter, KeyCode::Down, KeyCode::Enter],
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ from a source…").is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "mattpocock/skills/skills/engineering/tdd");
    s.key(key(KeyCode::Enter));
    await_busy(&mut s);
    assert!(
        tools.calls().iter().any(|c| c.contains("clone")),
        "{:#?}",
        tools.calls()
    );
    assert!(manifest(repo.path()).skills.contains_key("orqa-tdd"));
    assert_eq!(
        config_json(repo.path())["labels"]["fe"]["skills"],
        json!(["orqa-a11y", "orqa-tdd"])
    );
    assert_eq!(
        note(&s),
        "installed orqa-tdd from mattpocock/skills @ abc1234; orqa:fe takes it, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "skills      orqa-a11y, orqa-tdd").is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// r renames fe to web: the entry moves whole, its pr_template fe.md with
/// it, and the file is not touched.
#[test]
fn renaming_fe_to_web_moves_the_entry_and_keeps_its_pr_template() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), LABELS);
    let mut s = labels_page(clones(), repo.path());
    keys(&mut s, &[KeyCode::Down, KeyCode::Char('r')]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "rename orqa:fe › fe▏").is_some(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Backspace, KeyCode::Backspace]);
    type_in(&mut s, "web");
    s.key(key(KeyCode::Enter));
    let labels = config_json(repo.path())["labels"].clone();
    assert!(labels.get("fe").is_none(), "{labels}");
    assert_eq!(labels["web"]["pr_template"], json!("fe.md"));
    assert_eq!(labels["web"]["skills"], json!(["orqa-a11y"]));
    assert_eq!(
        note(&s),
        "renamed orqa:fe to orqa:web, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "▸ orqa:web").is_some(), "{:#?}", rows(&buf));
}

/// d asks first; n keeps the label, y removes its entry and leaves its PR
/// template file.
#[test]
fn deleting_a_label_asks_then_removes_the_entry_and_leaves_the_file() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), LABELS);
    let file = repo.path().join(".github/PULL_REQUEST_TEMPLATE/fe.md");
    write_file(&file, "## Screenshots\n");
    let mut s = labels_page(clones(), repo.path());
    keys(&mut s, &[KeyCode::Down, KeyCode::Char('d')]);
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "Delete orqa:fe? A Ticket still carrying it wakes its next Stage; its PR"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Char('n')));
    assert!(config_json(repo.path())["labels"].get("fe").is_some());
    keys(&mut s, &[KeyCode::Char('d'), KeyCode::Char('y')]);
    assert_eq!(
        config_json(repo.path())["labels"],
        json!({"codex-review": {"kind": "modifier", "rows": {"review": {"app": "codex"}}}})
    );
    assert!(file.is_file());
    assert_eq!(
        note(&s),
        "deleted orqa:fe, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ orqa:codex-review").is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// A name with a space is not a bd label, nothing after orqa: is no name,
/// and one config.json has clashes: each refused, the text kept to mend,
/// nothing written.
#[test]
fn a_name_that_is_not_a_valid_label_is_refused() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), LABELS);
    let mut s = labels_page(clones(), repo.path());
    s.key(key(KeyCode::Char('a')));
    type_in(&mut s, "my label");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: 'my label' is not a bd label: no spaces or commas. Nothing changed."
    );
    assert!(s.settings.as_ref().unwrap().typing.is_some());
    keys(&mut s, &[KeyCode::Backspace; 8]);
    type_in(&mut s, "orqa:");
    s.key(key(KeyCode::Enter));
    assert_eq!(note(&s), "Refused: no name after orqa:. Nothing changed.");
    type_in(&mut s, "fe");
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: orqa:fe is there already. Nothing changed."
    );
    assert!(s.settings.as_ref().unwrap().typing.is_some());
    assert_eq!(
        config_json(repo.path()),
        serde_json::from_str::<Value>(LABELS).unwrap()
    );
}

/// A label added to config.json since /config opened clashes as well: the
/// fresh read refuses it, the entry already there kept whole.
#[test]
fn a_label_added_meanwhile_is_not_overwritten() {
    let repo = TempDir::new();
    let path = repo.path().join(".orqadence/config.json");
    write_file(&path, LABELS);
    let mut s = labels_page(clones(), repo.path());
    let meanwhile = r#"{"labels": {"mobile": {"kind": "modifier", "guidance": "Mine."}}}"#;
    write_file(&path, meanwhile);
    s.key(key(KeyCode::Char('a')));
    type_in(&mut s, "mobile");
    s.key(key(KeyCode::Enter));
    assert!(
        note(&s).contains("orqa:mobile is there already"),
        "{}",
        note(&s)
    );
    assert_eq!(
        config_json(repo.path()),
        serde_json::from_str::<Value>(meanwhile).unwrap()
    );
}

/// A label a Stage could not read (skills not a list of strings) refuses a
/// pick: config.json is read afresh and its entry left as it was.
#[test]
fn a_malformed_skills_list_refuses_a_pick() {
    let repo = TempDir::new();
    let tools = clones();
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    let odd = r#"{"labels": {"fe": {"kind": "area", "skills": ["orqa-a11y", 7]}}}"#;
    write_file(&repo.path().join(".orqadence/config.json"), odd);
    let mut s = labels_page(tools, repo.path());
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);
    s.key(key(KeyCode::Enter));
    assert!(
        note(&s).contains("labels fe: invalid type: integer `7`, expected a string"),
        "{}",
        note(&s)
    );
    assert_eq!(
        config_json(repo.path())["labels"]["fe"]["skills"],
        json!(["orqa-a11y", 7])
    );
}

/// /config on a label's own page: the first label of the list opened.
fn open_label(tools: Arc<Fake>, repo: &Path) -> Screen {
    let mut s = labels_page(tools, repo);
    s.key(key(KeyCode::Enter));
    s
}

/// The open label's cursor on an item of its page.
fn goto(s: &mut Screen, item: LabelItem) {
    let st = s.settings.as_mut().unwrap();
    st.setting = st.label_items().iter().position(|i| *i == item).unwrap();
}

/// Enter opens the pick list on the item, the text filters it, Enter picks
/// the first match.
fn pick(s: &mut Screen, item: LabelItem, filter: &str) {
    goto(s, item);
    s.key(key(KeyCode::Enter));
    type_in(s, filter);
    s.key(key(KeyCode::Enter));
}

/// Setting orqa:be's implement model writes rows.implement.model, probed
/// first as on the Stage pages, and plans on one model; a row's "repo's row"
/// empties the field, which removes it, and its row and rows with it.
#[test]
fn a_labels_row_override_writes_its_field_and_emptying_it_removes_it() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"labels": {"be": {"kind": "area"}}}"#,
    );
    let mut s = open_label(apps(""), repo.path());
    let model = LabelItem::Row(0, Field::Model);
    pick(&mut s, model, "opus");
    await_probe(&mut s);
    assert_eq!(
        config_json(repo.path())["labels"]["be"],
        json!({"kind": "area", "rows": {"implement": {"model": "opus", "plan_model": "default"}}})
    );
    assert_eq!(
        note(&s),
        "orqa:be implement model opus, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸   model     opus").is_some(),
        "{:#?}",
        rows(&buf)
    );
    pick(&mut s, model, "repo");
    assert_eq!(
        config_json(repo.path())["labels"]["be"],
        json!({"kind": "area"})
    );
    assert_eq!(
        note(&s),
        "orqa:be implement model is the repo's, saved uncommitted in .orqadence/config.json"
    );
}

/// A label's implement App over the repo's split plans on one model: the
/// split runs on claude only, so codex would be refused with no split
/// control on the label's page to mend it.
#[test]
fn a_labels_implement_app_over_a_split_plans_on_one_model() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"implement": {"model": "claude-opus-5-5", "plan_model": "claude-fable-5-1"},
            "labels": {"be": {"kind": "area"}}}"#,
    );
    let mut s = open_label(apps(""), repo.path());
    pick(&mut s, LabelItem::Row(0, Field::App), "codex");
    type_in(&mut s, "gpt-6");
    s.key(key(KeyCode::Enter));
    await_probe(&mut s);
    assert_eq!(
        config_json(repo.path())["labels"]["be"]["rows"],
        json!({"implement": {"app": "codex", "model": "gpt-6-sol", "plan_model": "default"}}),
        "{}",
        note(&s)
    );
}

/// One label's broken Debate rule does not hide another label's: the
/// second label to break it is refused.
#[test]
fn a_rule_one_label_breaks_is_refused_on_another() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    let text = r#"{"side_a": {"model": "claude-opus-5-5"}, "labels": {
        "a": {"kind": "modifier", "rows": {"side_b": {"app": "claude", "model": "claude-opus-5-5"}}},
        "b": {"kind": "modifier"}}}"#;
    write_file(&file, text);
    let mut s = labels_page(apps(""), repo.path());
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    pick(&mut s, LabelItem::Row(5, Field::App), "claude");
    type_in(&mut s, "type");
    s.key(key(KeyCode::Enter));
    type_line(&mut s, "claude-opus-5-5");
    assert!(note(&s).starts_with("Refused: b side_b:"), "{}", note(&s));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

/// A label's Implement effort alone leaves its planning split: only a new
/// App or model plans on one model.
#[test]
fn a_labels_implement_effort_keeps_its_plan_model() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"labels": {"be": {"kind": "area", "rows": {"implement":
            {"app": "claude", "model": "claude-opus-5-5", "plan_model": "claude-fable-5-1"}}}}}"#,
    );
    let mut s = open_label(apps(""), repo.path());
    pick(&mut s, LabelItem::Row(0, Field::Effort), "high");
    assert_eq!(
        config_json(repo.path())["labels"]["be"]["rows"]["implement"],
        json!({"app": "claude", "model": "claude-opus-5-5",
               "plan_model": "claude-fable-5-1", "effort": "high"}),
        "{}",
        note(&s)
    );
}

/// config.json changed by hand while a label's model is probed is checked
/// again before it saves: a rule the change now breaks refuses it.
#[test]
fn a_label_override_is_checked_again_after_its_probe() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    write_file(
        &file,
        r#"{"side_a": {"app": "codex"}, "labels": {"b": {"kind": "modifier"}}}"#,
    );
    let mut s = open_label(apps(""), repo.path());
    pick(&mut s, LabelItem::Row(5, Field::App), "claude");
    type_in(&mut s, "type");
    s.key(key(KeyCode::Enter));
    type_line(&mut s, "claude-opus-5-5");
    assert!(s.settings.as_ref().unwrap().probe.is_some(), "{}", note(&s));
    let meanwhile =
        r#"{"side_a": {"model": "claude-opus-5-5"}, "labels": {"b": {"kind": "modifier"}}}"#;
    write_file(&file, meanwhile);
    await_probe(&mut s);
    assert!(
        note(&s).starts_with("b side_b: Both sides would be Anthropic"),
        "{}",
        note(&s)
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), meanwhile);
}

/// An Extra review model of none is refused, unprobed, as on the Stage
/// pages: the review row would run on no model.
#[test]
fn an_extra_review_model_of_none_is_refused_unprobed() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    write_file(&file, EXTRA);
    let mut s = labels_page(apps(""), repo.path());
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    pick(&mut s, LabelItem::Extra(Field::Model), "type");
    type_line(&mut s, "none");
    assert!(s.settings.as_ref().unwrap().probe.is_none());
    assert_eq!(
        note(&s),
        format!(
            "Refused: {}: review model none: only review_if_limited takes none. \
             Nothing changed.",
            file.display()
        )
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), EXTRA);
}

/// A label's review_if_limited model of none is the no-fallback sentinel:
/// saved at once, unprobed, as on the Stage pages.
#[test]
fn a_labels_if_limited_model_of_none_saves_unprobed() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), MOBILE);
    let mut s = open_label(apps(""), repo.path());
    pick(&mut s, LabelItem::Row(2, Field::Model), "none");
    assert!(s.settings.as_ref().unwrap().probe.is_none());
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"]["rows"],
        json!({"review_if_limited": {"model": "none"}}),
        "{}",
        note(&s)
    );
}

/// security with its Extra review, and codex-review, a Modifier.
const EXTRA: &str = r#"{"labels": {
  "codex-review": {"kind": "modifier"},
  "security": {"kind": "area", "extra_review": {"skill": "orqa-security-review", "position": "every", "debate": true}}
}}"#;

/// The right pane's lines, trimmed, from the top.
fn pane(s: &Screen, n: u16) -> Vec<String> {
    let buf = render(s, 160, 45);
    (1..=n)
        .map(|y| cols(&buf, y, 99, 158).trim_end().to_string())
        .collect()
}

/// Enter on position cycles it to the first Round, Space on Debate turns it
/// off; each saves at once, the rest of the entry as it was.
#[test]
fn an_extra_review_saves_its_position_and_debate() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), EXTRA);
    let mut s = labels_page(clones(), repo.path());
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    goto(&mut s, LabelItem::Position);
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "orqa:security Extra review runs first Round only, saved uncommitted in .orqadence/config.json"
    );
    goto(&mut s, LabelItem::Debate);
    s.key(key(KeyCode::Char(' ')));
    assert_eq!(
        config_json(repo.path())["labels"]["security"]["extra_review"],
        json!({"skill": "orqa-security-review", "position": "first", "debate": false})
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ debate      off  straight to the Fix").is_some(),
        "{:#?}",
        rows(&buf)
    );
    // the third press goes round: before the PR, then every Round again
    goto(&mut s, LabelItem::Position);
    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(
        config_json(repo.path())["labels"]["security"]["extra_review"]["position"],
        json!("every")
    );
}

/// A label without an Extra review gets the new-label defaults on its first
/// write; none clears the skill. The Extra review's model is probed and
/// saves as extra_review.model, "the Review's row" empties it.
#[test]
fn an_extra_review_takes_a_skill_and_its_own_row() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"labels": {"infra": {"kind": "area"}}}"#,
    );
    add(repo.path(), &*clones(), "mattpocock/skills", Some("tdd")).unwrap();
    let mut s = open_label(apps(""), repo.path());
    // Stage skills are not reviews: only the installed orqa-tdd is listed
    goto(&mut s, LabelItem::ExtraSkill);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "orqa-tdd").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "orqa-stage").is_none(), "{:#?}", rows(&buf));
    keys(&mut s, &[KeyCode::Esc]);
    pick(&mut s, LabelItem::ExtraSkill, "tdd");
    assert_eq!(
        config_json(repo.path())["labels"]["infra"]["extra_review"],
        json!({"skill": "orqa-tdd", "position": "every", "debate": true})
    );
    // the Review runs on codex until the label says otherwise
    pick(&mut s, LabelItem::Extra(Field::Model), "gpt-6");
    await_probe(&mut s);
    assert_eq!(
        config_json(repo.path())["labels"]["infra"]["extra_review"]["model"],
        json!("gpt-6-sol")
    );
    assert_eq!(
        note(&s),
        "orqa:infra extra review model gpt-6-sol, saved uncommitted in .orqadence/config.json"
    );
    pick(&mut s, LabelItem::Extra(Field::Model), "review");
    assert_eq!(
        config_json(repo.path())["labels"]["infra"]["extra_review"],
        json!({"skill": "orqa-tdd", "position": "every", "debate": true})
    );
    pick(&mut s, LabelItem::ExtraSkill, "none");
    assert_eq!(
        config_json(repo.path())["labels"]["infra"]["extra_review"],
        json!({"position": "every", "debate": true})
    );
}

/// A Modifier only changes rows: its page offers no Extra review.
#[test]
fn a_modifier_offers_no_extra_review() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), EXTRA);
    let s = open_label(clones(), repo.path());
    let items = s.settings.as_ref().unwrap().label_items();
    assert!(
        items.iter().all(|i| !matches!(
            i,
            LabelItem::ExtraSkill | LabelItem::Position | LabelItem::Debate | LabelItem::Extra(_)
        )),
        "{items:?}"
    );
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "EXTRA REVIEW").is_none(), "{:#?}", rows(&buf));
    assert!(find(&buf, "ROW OVERRIDES").is_some(), "{:#?}", rows(&buf));
}

/// A label pinning side_b to side A's family: the model list marks it, the
/// pick is refused with the check's text and nothing is written; an entry
/// written by hand shows the check on its page, in red.
#[test]
fn a_label_pinning_side_b_to_side_as_family_shows_the_check() {
    let repo = TempDir::new();
    let file = repo.path().join(".orqadence/config.json");
    write_file(&file, r#"{"labels": {"be": {"kind": "area"}}}"#);
    let mut s = open_label(apps(""), repo.path());
    pick(&mut s, LabelItem::Row(5, Field::App), "claude");
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "orqa:be side b model · claude (new App)").is_some(),
        "{:#?}",
        rows(&buf)
    );
    let (x, y) = find(&buf, "✗ side A's family").unwrap();
    assert_eq!(buf[(x, y)].fg, RED);
    s.key(key(KeyCode::Enter));
    assert_eq!(
        note(&s),
        "Refused: be side_b: Both sides would be Anthropic: the Debate needs two families. Nothing changed."
    );
    assert_eq!(
        config_json(repo.path()),
        json!({"labels": {"be": {"kind": "area"}}})
    );

    let odd = r#"{"labels": {"be": {"kind": "area", "rows": {"side_b": {"app": "claude"}}}}}"#;
    write_file(&file, odd);
    let s = open_label(clones(), repo.path());
    let buf = render(&s, 160, 45);
    let (x, y) = find(&buf, "✗ be side_b: Both sides would be Anthropic").unwrap();
    assert_eq!(buf[(x, y)].fg, RED);
    assert!(find(&buf, "CHECKS").is_some(), "{:#?}", rows(&buf));
}

const MOBILE: &str = r#"{"labels": {"mobile": {"kind": "area"}}}"#;

/// The PR template list offers default, each file of the template directory
/// and new; a file picked is the entry's pr_template, default takes it off.
#[test]
fn the_pr_template_picker_lists_default_the_files_and_new() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), MOBILE);
    let dir = repo.path().join(".github/PULL_REQUEST_TEMPLATE");
    write_file(&dir.join("a.md"), "## A\n");
    write_file(&dir.join("b.md"), "## B\n");
    write_file(&dir.join("notes.txt"), "not a template");
    let mut s = open_label(clones(), repo.path());
    goto(&mut s, LabelItem::Template);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    let (x, y) = find(&buf, "orqa:mobile PR template").unwrap();
    let listed: Vec<String> = (1..5)
        .map(|d| {
            cols(&buf, y + d, x as usize, x as usize + 12)
                .trim_end()
                .to_string()
        })
        .collect();
    assert_eq!(listed, ["▸ default", "  a.md", "  b.md", "  new…"]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"]["pr_template"],
        json!("b.md")
    );
    assert_eq!(
        note(&s),
        "orqa:mobile uses the PR template b.md, saved uncommitted in .orqadence/config.json"
    );
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "PR template b.md").is_some(),
        "{:#?}",
        rows(&buf)
    );
    pick(&mut s, LabelItem::Template, "default");
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"],
        json!({"kind": "area"})
    );
}

/// new for a shipped label writes <name>.md as the repo's default template
/// plus the label's shipped section, and maps it, asking nothing.
#[test]
fn a_new_template_for_a_shipped_label_is_the_default_plus_its_section() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"labels": {"fe": {"kind": "area"}}}"#,
    );
    write_file(
        &repo.path().join(".github/pull_request_template.md"),
        "## Mine\n<!-- x -->\n",
    );
    let mut s = open_label(clones(), repo.path());
    pick(&mut s, LabelItem::Template, "new");
    let written =
        std::fs::read_to_string(repo.path().join(".github/PULL_REQUEST_TEMPLATE/fe.md")).unwrap();
    assert!(
        written.starts_with("## Mine\n<!-- x -->\n\n## Screenshots\n<!-- every changed screen"),
        "{written}"
    );
    assert!(s.settings.as_ref().unwrap().typing.is_none());
    assert_eq!(
        config_json(repo.path())["labels"]["fe"]["pr_template"],
        json!("fe.md")
    );
    assert_eq!(
        note(&s),
        "wrote .github/PULL_REQUEST_TEMPLATE/fe.md; orqa:fe uses it, saved uncommitted in .orqadence/config.json"
    );
    // it is there now: a second new is refused, the file kept
    pick(&mut s, LabelItem::Template, "new");
    assert_eq!(
        note(&s),
        ".github/PULL_REQUEST_TEMPLATE/fe.md is there already: pick it from the list. Nothing changed."
    );
}

/// new for a label of the user's asks for the section's heading, then
/// writes Orqadence's default template (the repo has none) plus it.
#[test]
fn a_new_template_for_a_user_label_asks_the_heading() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), MOBILE);
    let mut s = open_label(clones(), repo.path());
    pick(&mut s, LabelItem::Template, "new");
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "section heading › ▏").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(!repo.path().join(".github").exists());
    type_in(&mut s, "Devices tried");
    s.key(key(KeyCode::Enter));
    let written =
        std::fs::read_to_string(repo.path().join(".github/PULL_REQUEST_TEMPLATE/mobile.md"))
            .unwrap();
    assert!(written.starts_with("## What\n"), "{written}");
    assert!(written.ends_with("<!-- the issue this closes -->\n\n## Devices tried\n"));
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"]["pr_template"],
        json!("mobile.md")
    );
}

/// A template file made while the heading is typed is kept: the write is
/// refused, not an overwrite.
#[test]
fn a_template_made_during_the_heading_prompt_is_not_overwritten() {
    let repo = TempDir::new();
    write_file(&repo.path().join(".orqadence/config.json"), MOBILE);
    let mut s = open_label(clones(), repo.path());
    pick(&mut s, LabelItem::Template, "new");
    let file = repo.path().join(".github/PULL_REQUEST_TEMPLATE/mobile.md");
    write_file(&file, "mine\n");
    type_in(&mut s, "Devices tried");
    s.key(key(KeyCode::Enter));
    assert!(
        note(&s).starts_with(".github/PULL_REQUEST_TEMPLATE/mobile.md: "),
        "{}",
        note(&s)
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine\n");
    assert_eq!(
        config_json(repo.path())["labels"]["mobile"],
        json!({"kind": "area"})
    );
}

/// A label's page: its fields, the Extra review, each Stage row with what it
/// falls through to or overrides, and the pick list's title.
#[test]
fn a_labels_page_renders_its_extra_review_and_rows() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"review": {"app": "claude", "model": "opus"},
            "labels": {"security": {"kind": "area", "pr_template": "security.md",
              "extra_review": {"skill": "orqa-security-review", "position": "before_pr", "debate": true, "model": "sonnet"},
              "rows": {"fix": {"effort": "high"}}}}}"#,
    );
    let mut s = open_label(clones(), repo.path());
    let page = pane(&s, 24);
    assert_eq!(
        page,
        [
            "orqa:security",
            "",
            "▸ kind        area",
            "  skills      none",
            "  guidance    none",
            "  PR template security.md  not found: the default is used",
            "",
            "EXTRA REVIEW",
            "  skill       orqa-security-review",
            "  position    before the PR",
            "  debate      on  joins the Debate",
            "  app         the Review's claude",
            "  model       sonnet",
            "  effort      the Review's default",
            "",
            "ROW OVERRIDES",
            "  Implement",
            "    app       repo's claude",
            "    model     repo's default",
            "    effort    repo's default",
            "  Review",
            "    app       repo's claude",
            "    model     repo's opus",
            "    effort    repo's default",
        ]
    );
    goto(&mut s, LabelItem::Row(6, Field::Effort));
    let page = pane(&s, 43);
    assert!(
        page.contains(&"▸   effort    high".to_string()),
        "{page:#?}"
    );
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "orqa:security fix effort").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "repo's row").is_some(), "{:#?}", rows(&buf));
}
