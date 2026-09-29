use super::{ask_typesafe, install_skills, preflight, typesafe_key, warnings};
use crate::orchestrator::write_file;
use crate::skills::manifest::{Manifest, JOBS, NONE};
use crate::skills::SKILLS;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const RECORD: &str = ".orqadence/installed-skills.json";
const STAGE_FIX: &str = ".agents/skills/stage-fix/SKILL.md";

/// A Target repo that already has a create-pr skill of its own.
fn repo_with_own_pr() -> TempDir {
    let repo = TempDir::new();
    let dir = repo.path().join(".agents/skills/create-pr");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), "the repo's own").unwrap();
    repo
}

/// init's skills step, `answer` on its input.
fn install(repo: &Path, answer: &str) -> String {
    let mut out = Vec::new();
    let home = TempDir::new();
    install_skills(
        repo,
        home.path(),
        false,
        false,
        &mut out,
        &mut answer.as_bytes(),
        false,
    )
    .unwrap();
    String::from_utf8(out).unwrap()
}

fn read(repo: &Path, path: &str) -> String {
    fs::read_to_string(repo.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
}

#[test]
fn install_skills_asks_before_touching_the_repos_own_create_pr() {
    for (answer, own, beside) in [
        ("\r", "the repo's own", ""), // enter on the first option keeps it
        ("", "the repo's own", ""),   // so does a closed stdin
        ("nonsense\r", "the repo's own", ""), // so do keys that mean nothing here
        ("\x1b[B\x1b[A\r", "the repo's own", ""), // down, then back up
        ("\x1b[B\r", "name: create-pr", ""), // down one: replace it
        (
            "\x1b[B\x1b[B\x1b[B\r",
            "the repo's own",
            "name: orqadence-create-pr",
        ), // down past the end
        ("1", "the repo's own", ""),  // a digit picks its option outright
        ("2", "name: create-pr", ""),
        ("3", "the repo's own", "name: orqadence-create-pr"),
    ] {
        let repo = repo_with_own_pr();
        let out = install(repo.path(), answer);
        assert!(
            out.contains("already has a create-pr skill"),
            "{answer:?}: init did not ask:\n{out}"
        );
        let got = read(repo.path(), ".agents/skills/create-pr/SKILL.md");
        assert!(
            got.contains(own),
            "{answer:?}: create-pr is {got:?}, want {own:?}"
        );
        let body = fs::read_to_string(
            repo.path()
                .join(".claude/skills/orqadence-create-pr/SKILL.md"),
        );
        if beside.is_empty() {
            assert!(
                body.is_err(),
                "{answer:?}: installed orqadence-create-pr anyway"
            );
            continue;
        }
        let body = body.unwrap_or_else(|err| {
            panic!("{answer:?}: orqadence-create-pr not installed through its link: {err}")
        });
        assert!(
            body.contains(beside),
            "{answer:?}: orqadence-create-pr not installed through its link: {body:?}"
        );
        assert!(
            !body.contains("name: create-pr"),
            "{answer:?}: orqadence-create-pr still calls itself create-pr"
        );
    }
}

#[test]
fn install_skills_does_not_ask_when_the_repo_has_no_create_pr() {
    let repo = TempDir::new();
    let out = install(repo.path(), "");
    assert!(
        !out.contains("already has"),
        "init asked about a create-pr the repo does not have:\n{out}"
    );
    let got = read(repo.path(), ".claude/skills/create-pr/SKILL.md");
    assert!(
        got.contains("name: create-pr"),
        "shipped create-pr not installed: {got:?}"
    );
}

#[test]
fn install_skills_force_skips_the_questions_and_keeps_the_repos_own_create_pr() {
    let repo = repo_with_own_pr();
    let mut out = Vec::new();
    let home = TempDir::new();
    install_skills(
        repo.path(),
        home.path(),
        true,
        false,
        &mut out,
        &mut "2\n".as_bytes(),
        false,
    )
    .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(!out.contains("already"), "--force still asked:\n{out}");
    assert_eq!(
        read(repo.path(), ".agents/skills/create-pr/SKILL.md"),
        "the repo's own"
    );
    assert!(read(repo.path(), ".orqadence/skills/stage-fix/SKILL.md").contains("name: stage-fix"));
}

#[test]
fn install_skills_overwrite_keeps_the_repos_own_create_pr_beside_the_recorded_one() {
    let repo = repo_with_own_pr();
    install(repo.path(), "3"); // beside it, as orqadence-create-pr
    let beside = repo
        .path()
        .join(".agents/skills/orqadence-create-pr/SKILL.md");
    fs::write(&beside, "edited").unwrap();
    let out = install(repo.path(), "3"); // the gate: overwrite everything
    assert!(out.contains("already installed"), "no gate:\n{out}");
    assert!(
        !out.contains("already has"),
        "asked about create-pr again:\n{out}"
    );
    assert_eq!(
        read(repo.path(), ".agents/skills/create-pr/SKILL.md"),
        "the repo's own"
    );
    assert!(
        fs::read_to_string(&beside)
            .unwrap()
            .contains("name: orqadence-create-pr"),
        "overwrite left the edited orqadence-create-pr"
    );
}

/// A checkout an older init set up at user level, run without HOME: its
/// skills cannot be copied, so init stops rather than install afresh.
#[test]
fn skills_at_user_level_without_home_stop_init() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/skills.json"),
        r#"{"location": "user", "skills": {"tdd": {"repo": "https://github.com/mattpocock/skills"}}}"#,
    );
    let err = install_skills(
        repo.path(),
        Path::new(""),
        false,
        false,
        &mut Vec::new(),
        &mut "".as_bytes(),
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("no HOME"), "{err}");
    assert!(!repo.path().join(".orqadence/skills").exists());
}

/// A checkout install from before ADR 0006: its skills are in
/// .orqadence/skills already and gain their links, and the lines that hid
/// the worktrees' links from git go, since they would hide these too.
#[test]
fn a_checkout_install_is_linked_and_its_exclude_lines_go() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/skills/stage-fix/SKILL.md"),
        "edited",
    );
    write_file(
        &repo.path().join(".orqadence/skills.json"),
        r#"{"location": "checkout", "skills": {"stage-fix": {"shipped": true}}}"#,
    );
    let exclude = repo.path().join(".git/info/exclude");
    write_file(
        &exclude,
        "/.claude/skills/stage-fix\n*.tmp\n/.agents/skills/stage-fix\n",
    );
    install(repo.path(), ""); // the gate, unanswered: cancel
    assert_eq!(fs::read_to_string(&exclude).unwrap(), "*.tmp\n");
    for dir in [".agents/skills", ".claude/skills"] {
        assert_eq!(
            fs::read_link(repo.path().join(dir).join("stage-fix")).unwrap(),
            Path::new("../../.orqadence/skills/stage-fix"),
            "{dir}"
        );
    }
    assert_eq!(read(repo.path(), STAGE_FIX), "edited");
}

fn record(repo: &Path) -> BTreeMap<String, String> {
    serde_json::from_str(&read(repo, RECORD)).unwrap()
}

/// Every file under the repo with its content, to prove a run touched nothing.
pub(crate) fn snapshot(repo: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.symlink_metadata().unwrap().is_dir() {
                walk(&path, into);
            } else {
                into.insert(
                    path.display().to_string(),
                    fs::read(&path).unwrap_or_default(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(repo, &mut files);
    files
}

#[test]
fn install_skills_records_every_file_it_writes_without_a_gate() {
    let repo = TempDir::new();
    let out = install(repo.path(), "");
    assert!(
        !out.contains("already installed"),
        "a fresh repo hit the gate:\n{out}"
    );
    let record = record(repo.path());
    assert_eq!(record.len(), 6, "record: {record:?}");
    for (rel, wrote) in &record {
        assert_eq!(
            fs::read_to_string(repo.path().join(rel)).unwrap(),
            *wrote,
            "{rel}: record does not match the file"
        );
    }
    assert!(record.contains_key(STAGE_FIX), "record: {record:?}");
}

#[test]
fn install_skills_refresh_rewrites_only_files_unedited_since_install() {
    let repo = TempDir::new();
    install(repo.path(), "");
    let shipped = read(repo.path(), STAGE_FIX);
    // stage-fix as an older release wrote it, still unedited: the record agrees.
    let stale = repo.path().join(STAGE_FIX);
    fs::write(&stale, "older shipped text").unwrap();
    let mut rec = record(repo.path());
    rec.insert(STAGE_FIX.to_string(), "older shipped text".to_string());
    fs::write(
        repo.path().join(RECORD),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();
    // stage-review edited in the Target repo: the record disagrees.
    let edited = repo.path().join(".agents/skills/stage-review/SKILL.md");
    fs::write(&edited, "edited in the Target repo").unwrap();

    let out = install(repo.path(), "2");
    assert!(out.contains("already installed"), "no gate:\n{out}");
    assert_eq!(
        read(repo.path(), STAGE_FIX),
        shipped,
        "refresh left the stale skill"
    );
    assert_eq!(
        record(repo.path())[STAGE_FIX],
        shipped,
        "refresh did not update the record"
    );
    assert_eq!(
        fs::read_to_string(&edited).unwrap(),
        "edited in the Target repo",
        "refresh overwrote an edited skill"
    );

    install(repo.path(), "3");
    assert!(
        fs::read_to_string(&edited)
            .unwrap()
            .contains("name: stage-review"),
        "overwrite left the edited skill"
    );
}

#[test]
fn install_skills_refresh_treats_a_file_without_a_record_as_edited() {
    // Installed by the Go binary: the files are there, the record is not.
    let repo = TempDir::new();
    install(repo.path(), "");
    fs::remove_file(repo.path().join(RECORD)).unwrap();
    fs::write(repo.path().join(STAGE_FIX), "from the Go binary").unwrap();
    install(repo.path(), "2");
    assert_eq!(read(repo.path(), STAGE_FIX), "from the Go binary");
}

#[test]
fn install_skills_cancel_and_a_closed_stdin_touch_nothing() {
    for answer in ["1", "\r", "", "\x1b[B\x1b[A\r"] {
        let repo = TempDir::new();
        install(repo.path(), "");
        fs::write(repo.path().join(STAGE_FIX), "edited").unwrap();
        let before = snapshot(repo.path());
        let out = install(repo.path(), answer);
        assert!(
            out.contains("already installed"),
            "{answer:?}: no gate:\n{out}"
        );
        assert_eq!(
            snapshot(repo.path()),
            before,
            "{answer:?}: cancel touched a file"
        );
    }
}

fn ask_key(repo: &Path, env: &str, typed: &str) -> String {
    let mut out = Vec::new();
    let typed = format!("y\n{typed}"); // yes to TypeSafe, then the key
    ask_typesafe(repo, env, &mut out, &mut typed.as_bytes(), false).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn ask_typesafe_key_stores_the_typed_key_read_only_to_the_user() {
    let repo = TempDir::new();
    let out = ask_key(repo.path(), "", "sk-typed\n");
    assert!(out.contains("TypeSafe API key"), "not asked:\n{out}");
    let path = repo.path().join(".orqadence-local/typesafe-key");
    assert_eq!(fs::read_to_string(&path).unwrap().trim(), "sk-typed");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        typesafe_key(repo.path(), &|_| String::new()).as_deref(),
        Some("sk-typed")
    );

    // Asked again, the stored key stands and no question is put.
    let out = ask_key(repo.path(), "", "sk-other\n");
    assert!(
        !out.contains("TypeSafe API key"),
        "asked with a key stored:\n{out}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap().trim(), "sk-typed");
}

#[test]
fn ask_typesafe_key_skips_when_the_variable_is_set_or_stdin_is_silent() {
    let repo = TempDir::new();
    let out = ask_key(repo.path(), "sk-env", "sk-typed\n");
    assert!(
        !out.contains("TypeSafe API key"),
        "asked with the variable set:\n{out}"
    );
    assert!(!repo.path().join(".orqadence-local/typesafe-key").exists());

    for typed in ["", "\n", "sk-a\x03", "sk-b\x04sk-c\n", "\x1b[A\t\n"] {
        let out = ask_key(repo.path(), "", typed);
        assert!(
            out.contains("TypeSafe API key"),
            "{typed:?}: not asked:\n{out}"
        );
        assert!(
            !repo.path().join(".orqadence-local/typesafe-key").exists(),
            "{typed:?}: stored anyway"
        );
    }
    assert_eq!(typesafe_key(repo.path(), &|_| String::new()), None);
    // Control bytes and an arrow key never reach the key.
    ask_key(repo.path(), "", "\x1b[Ask-\x01d\x7f\n");
    assert_eq!(
        read(repo.path(), ".orqadence-local/typesafe-key").trim(),
        "sk-"
    );
}

#[test]
fn typesafe_key_prefers_the_variable_over_the_file() {
    let repo = TempDir::new();
    ask_key(repo.path(), "", "sk-file\n");
    let env = |key: &str| {
        if key == "TYPESAFE_API_KEY" {
            " sk-env ".to_string()
        } else {
            String::new()
        }
    };
    assert_eq!(typesafe_key(repo.path(), &env).as_deref(), Some("sk-env"));
}

/// The environment with HOME at home, nothing else set.
fn home_env(home: &Path) -> impl Fn(&str) -> String {
    let home = home.display().to_string();
    move |key: &str| {
        if key == "HOME" {
            home.clone()
        } else {
            String::new()
        }
    }
}

fn picks_missing(repo: &Path, home: &Path) -> Vec<String> {
    preflight(repo, &*Fake::quiet(), &home_env(home))
        .into_iter()
        .filter(|m| m.contains("is missing"))
        .collect()
}

/// A pick counts only where the App running its line loads it: the Review,
/// on codex by default, never reads .claude/skills.
#[test]
fn preflight_counts_a_pick_only_where_its_app_loads_it() {
    let (repo, home) = (TempDir::new(), TempDir::new());
    let mut manifest = Manifest::default();
    for (job, _) in JOBS {
        manifest.picks.insert(job.to_string(), NONE.to_string());
    }
    manifest.picks.insert("review".into(), "rcr".into());
    manifest.save(repo.path()).unwrap();
    write_file(&repo.path().join(".claude/skills/rcr/SKILL.md"), "claude's");
    assert_eq!(
        picks_missing(repo.path(), home.path()),
        ["the review skill rcr is missing: /config installs it, or picks another"]
    );
    write_file(&repo.path().join(".agents/skills/rcr/SKILL.md"), "codex's");
    assert_eq!(
        picks_missing(repo.path(), home.path()),
        Vec::<String>::new()
    );
}

#[test]
fn preflight_fails_on_a_missing_pick_naming_its_job_and_none_opts_out() {
    let (repo, home) = (TempDir::new(), TempDir::new());
    let missing = picks_missing(repo.path(), home.path());
    assert!(
        missing.contains(
            &"the self review skill code-review is missing: orqa init installs it, or /config picks another"
                .to_string()
        ),
        "{missing:?}"
    );
    // The review job's default is none: nothing to miss.
    assert_eq!(missing.len(), JOBS.len() - 1, "{missing:?}");

    let mut manifest = Manifest::default();
    for (job, _) in JOBS {
        manifest.picks.insert(job.to_string(), NONE.to_string());
    }
    manifest
        .picks
        .insert("review".into(), "review-agent".into()); // codex's own
    manifest.save(repo.path()).unwrap();
    assert_eq!(
        picks_missing(repo.path(), home.path()),
        Vec::<String>::new()
    );
    // Only codex has it: a Review on claude lacks it.
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"review": {"app": "claude"}}"#,
    );
    assert_eq!(
        picks_missing(repo.path(), home.path()),
        ["the review skill review-agent is missing: /config installs it, or picks another"]
    );
    fs::remove_file(repo.path().join(".orqadence/config.json")).unwrap();

    // A pick you have anywhere, at user level here, is there. One not a
    // job's default is /config's to install.
    manifest.picks.insert("test-first".into(), "tdd".into());
    manifest
        .picks
        .insert("prose".into(), "caveman-commit".into());
    manifest.save(repo.path()).unwrap();
    assert_eq!(
        picks_missing(repo.path(), home.path()),
        [
            "the test first skill tdd is missing: orqa init installs it, or /config picks another",
            "the prose skill caveman-commit is missing: /config installs it, or picks another",
        ]
    );
    manifest.picks.insert("prose".into(), NONE.into());
    manifest.save(repo.path()).unwrap();
    write_file(&home.path().join(".claude/skills/tdd/SKILL.md"), "yours");
    assert_eq!(
        picks_missing(repo.path(), home.path()),
        Vec::<String>::new()
    );
}

#[test]
fn preflight_warns_of_superpowers() {
    let repo = TempDir::new();
    assert_eq!(warnings(repo.path(), &*Fake::quiet()), Vec::<String>::new());

    let plugins = |enabled: bool| {
        let reply = serde_json::json!([
            {"id": "superpowers@claude-plugins-official", "enabled": enabled, "installPath": "/nowhere"},
        ])
        .to_string();
        Fake::new(move |_, argv| match argv.join(" ").as_str() {
            "claude plugin list --json" => Ok(reply.clone()),
            other => Err(format!("unexpected: {other}")),
        })
    };
    let got = warnings(repo.path(), &*plugins(true));
    assert!(
        got.len() == 1 && got[0].contains("superpowers") && got[0].contains("SessionStart"),
        "{got:?}"
    );
    assert_eq!(
        warnings(repo.path(), &*plugins(false)),
        Vec::<String>::new()
    );
}

#[test]
fn preflight_names_each_row_whose_app_is_not_on_path() {
    let (repo, home) = (TempDir::new(), TempDir::new());
    let no_codex = Fake::new(|_, argv| match argv.join(" ").as_str() {
        "which codex" => Err("codex not found".to_string()),
        _ => Ok(String::new()),
    });
    let got: Vec<String> = preflight(repo.path(), &*no_codex, &home_env(home.path()))
        .into_iter()
        .filter(|m| m.contains("PATH"))
        .collect();
    // The defaults: the Review and Debate side B on codex, the rest on claude.
    assert_eq!(
        got,
        [
            "review runs on codex, which is not on PATH",
            "side_b runs on codex, which is not on PATH",
        ]
    );
    // The Review's fallback once its key is set, no model needed; unset, it
    // runs nothing.
    let config = repo.path().join(".orqadence/config.json");
    crate::orchestrator::write_file(&config, r#"{"review_if_limited": {"app": "codex"}}"#);
    let got = preflight(repo.path(), &*no_codex, &home_env(home.path()));
    assert!(
        got.contains(&"review_if_limited runs on codex, which is not on PATH".to_string()),
        "{got:?}"
    );
}

/// An installed Stage skill edited to lose a job's placeholder never runs
/// that job's pick: the preflight warns, naming both.
#[test]
fn preflight_warns_of_a_stage_skill_that_lost_a_placeholder() {
    let repo = TempDir::new();
    let shipped = SKILLS
        .iter()
        .find(|(name, _)| *name == "stage-implement")
        .unwrap()
        .1;
    let at = repo.path().join(".agents/skills/stage-implement/SKILL.md");
    write_file(&at, shipped);
    assert_eq!(warnings(repo.path(), &*Fake::quiet()), Vec::<String>::new());

    let edited: Vec<&str> = shipped
        .lines()
        .filter(|line| !line.contains("{{test-first}}"))
        .collect();
    write_file(&at, &edited.join("\n"));
    assert_eq!(
        warnings(repo.path(), &*Fake::quiet()),
        ["the installed stage-implement lacks {{test-first}}: the test first skill you pick never runs there; put the line back, or refresh it with orqa init"]
    );
}
