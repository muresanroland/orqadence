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
const STAGE_FIX: &str = ".agents/skills/orqa-stage-fix/SKILL.md";

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

/// Nothing to ask about a repo's own create-pr: the shipped one is
/// orqa-create-pr, beside it.
#[test]
fn install_skills_puts_orqa_create_pr_beside_the_repos_own_unasked() {
    let repo = repo_with_own_pr();
    let out = install(repo.path(), "");
    assert!(!out.contains("create-pr"), "init asked:\n{out}");
    assert_eq!(
        read(repo.path(), ".agents/skills/create-pr/SKILL.md"),
        "the repo's own"
    );
    let got = read(repo.path(), ".claude/skills/orqa-create-pr/SKILL.md");
    assert!(got.contains("name: orqa-create-pr"), "{got:?}");
}

#[test]
fn install_skills_force_skips_the_gate_and_keeps_the_repos_own_create_pr() {
    let repo = repo_with_own_pr();
    install(repo.path(), "");
    let fix = repo
        .path()
        .join(".orqadence/skills/orqa-stage-fix/SKILL.md");
    fs::write(&fix, "edited").unwrap();
    let mut out = Vec::new();
    let home = TempDir::new();
    install_skills(
        repo.path(),
        home.path(),
        true,
        &mut out,
        &mut "".as_bytes(),
        false,
    )
    .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(!out.contains("already"), "--force still asked:\n{out}");
    assert_eq!(
        read(repo.path(), ".agents/skills/create-pr/SKILL.md"),
        "the repo's own"
    );
    assert!(fs::read_to_string(fix)
        .unwrap()
        .contains("name: orqa-stage-fix"));
}

/// An install from before the prefix: each skill it named, shipped or
/// fetched, is renamed to orqa-<name>, its SKILL.md, links, manifest entry,
/// picks and record with it; the create-pr it put beside the repo's own
/// becomes orqa-create-pr. Its text, edited or not, is kept for the gate.
#[test]
fn an_install_from_before_the_prefix_is_renamed_to_it() {
    let repo = TempDir::new();
    let root = repo.path();
    for (name, body) in [
        ("tdd", "---\nname: tdd\n---\nred green\n"),
        ("stage-fix", "---\nname: stage-fix\n---\nedited\n"),
        (
            "orqadence-create-pr",
            "---\nname: orqadence-create-pr\n---\nold\n",
        ),
    ] {
        write_file(
            &root.join(format!(".orqadence/skills/{name}/SKILL.md")),
            body,
        );
        for dir in [".agents/skills", ".claude/skills"] {
            fs::create_dir_all(root.join(dir)).unwrap();
            std::os::unix::fs::symlink(
                format!("../../.orqadence/skills/{name}"),
                root.join(dir).join(name),
            )
            .unwrap();
        }
    }
    write_file(
        &root.join(".orqadence/skills.json"),
        r#"{"skills": {"tdd": {"repo": "https://github.com/mattpocock/skills"},
            "stage-fix": {"shipped": true}, "orqadence-create-pr": {"shipped": true}},
            "picks": {"test-first": "tdd", "prose": "mine"}}"#,
    );
    let record = BTreeMap::from([(
        ".agents/skills/orqadence-create-pr/SKILL.md",
        "---\nname: orqadence-create-pr\n---\nold\n",
    )]);
    write_file(&root.join(RECORD), &serde_json::to_string(&record).unwrap());

    let out = install(root, "\r"); // the gate: cancel, leave their text
    assert!(out.contains("init: renamed tdd to orqa-tdd"), "{out}");
    assert!(out.contains("already installed"), "no gate:\n{out}");

    assert_eq!(
        read(root, ".claude/skills/orqa-tdd/SKILL.md"),
        "---\nname: orqa-tdd\n---\nred green\n"
    );
    assert_eq!(
        read(root, ".agents/skills/orqa-stage-fix/SKILL.md"),
        "---\nname: orqa-stage-fix\n---\nedited\n"
    );
    assert!(
        read(root, ".orqadence/skills/orqa-create-pr/SKILL.md").contains("name: orqa-create-pr")
    );
    for old in ["tdd", "stage-fix", "orqadence-create-pr"] {
        for dir in [".orqadence/skills", ".agents/skills", ".claude/skills"] {
            let at = root.join(dir).join(old);
            assert!(fs::symlink_metadata(&at).is_err(), "{} left", at.display());
        }
    }
    let manifest = Manifest::load(root).unwrap();
    let names: Vec<&str> = manifest.skills.keys().map(String::as_str).collect();
    assert_eq!(names, ["orqa-create-pr", "orqa-stage-fix", "orqa-tdd"]);
    assert_eq!(manifest.pick("test-first"), "orqa-tdd");
    assert_eq!(
        manifest.pick("prose"),
        "mine",
        "a pick not installed is renamed"
    );
    let record: BTreeMap<String, String> = serde_json::from_str(&read(root, RECORD)).unwrap();
    assert_eq!(
        record[".agents/skills/orqa-create-pr/SKILL.md"],
        "---\nname: orqa-create-pr\n---\nold\n"
    );
}

/// A skill in .orqadence/skills, linked from both links folders.
fn put_linked(root: &Path, name: &str, body: &str) {
    write_file(
        &root.join(format!(".orqadence/skills/{name}/SKILL.md")),
        body,
    );
    for dir in [".agents/skills", ".claude/skills"] {
        fs::create_dir_all(root.join(dir)).unwrap();
        std::os::unix::fs::symlink(
            format!("../../.orqadence/skills/{name}"),
            root.join(dir).join(name),
        )
        .unwrap();
    }
}

/// A skill installed under both names (a merge of a branch from before the
/// prefix): the unprefixed one is stale, and its folder, links and entry
/// go; its pick turns to the prefixed one, which is kept as it is.
#[test]
fn a_skill_installed_under_both_names_loses_the_unprefixed_one() {
    let repo = TempDir::new();
    let root = repo.path();
    put_linked(root, "tdd", "---\nname: tdd\n---\nold\n");
    put_linked(root, "orqa-tdd", "---\nname: orqa-tdd\n---\nkept\n");
    write_file(
        &root.join(".orqadence/skills.json"),
        r#"{"skills": {"tdd": {"repo": "https://github.com/mattpocock/skills"},
            "orqa-tdd": {"repo": "https://github.com/mattpocock/skills"}},
            "picks": {"test-first": "tdd"}}"#,
    );

    install(root, "");

    for dir in [".orqadence/skills", ".agents/skills", ".claude/skills"] {
        let at = root.join(dir).join("tdd");
        assert!(fs::symlink_metadata(&at).is_err(), "{} left", at.display());
    }
    assert_eq!(
        read(root, ".claude/skills/orqa-tdd/SKILL.md"),
        "---\nname: orqa-tdd\n---\nkept\n"
    );
    let manifest = Manifest::load(root).unwrap();
    assert!(!manifest.skills.contains_key("tdd"));
    assert_eq!(manifest.pick("test-first"), "orqa-tdd");
}

/// A folder at a skill's new name the manifest does not name is not taken:
/// the skill is left as it is and the user told. One whose old folder is
/// gone, renamed by an init that failed after, takes it.
#[test]
fn a_folder_at_the_new_name_is_taken_only_when_the_old_one_is_gone() {
    let repo = TempDir::new();
    let root = repo.path();
    put_linked(root, "tdd", "---\nname: tdd\n---\nmine\n");
    write_file(
        &root.join(".orqadence/skills/orqa-tdd/SKILL.md"),
        "someone else's",
    );
    write_file(
        &root.join(".orqadence/skills/orqa-caveman/SKILL.md"),
        "---\nname: orqa-caveman\n---\n",
    );
    write_file(
        &root.join(".orqadence/skills.json"),
        r#"{"skills": {"tdd": {"repo": "https://github.com/mattpocock/skills"},
            "caveman": {"repo": "https://github.com/JuliusBrussee/caveman"}}}"#,
    );

    let out = install(root, "");

    assert!(out.contains("init: tdd is not renamed"), "{out}");
    assert!(
        out.contains("init: renamed caveman to orqa-caveman"),
        "{out}"
    );
    assert_eq!(
        read(root, ".claude/skills/tdd/SKILL.md"),
        "---\nname: tdd\n---\nmine\n"
    );
    assert_eq!(
        read(root, ".orqadence/skills/orqa-tdd/SKILL.md"),
        "someone else's"
    );
    assert!(fs::symlink_metadata(root.join(".claude/skills/orqa-tdd")).is_err());
    let manifest = Manifest::load(root).unwrap();
    let names: Vec<&str> = manifest.skills.keys().map(String::as_str).collect();
    assert!(
        names.contains(&"tdd") && !names.contains(&"orqa-tdd"),
        "{names:?}"
    );
    assert!(names.contains(&"orqa-caveman"), "{names:?}");
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
        &repo
            .path()
            .join(".orqadence/skills/orqa-stage-fix/SKILL.md"),
        "edited",
    );
    write_file(
        &repo.path().join(".orqadence/skills.json"),
        r#"{"location": "checkout", "skills": {"orqa-stage-fix": {"shipped": true}}}"#,
    );
    let exclude = repo.path().join(".git/info/exclude");
    write_file(
        &exclude,
        "/.claude/skills/orqa-stage-fix\n*.tmp\n/.agents/skills/orqa-stage-fix\n",
    );
    install(repo.path(), ""); // the gate, unanswered: cancel
    assert_eq!(fs::read_to_string(&exclude).unwrap(), "*.tmp\n");
    for dir in [".agents/skills", ".claude/skills"] {
        assert_eq!(
            fs::read_link(repo.path().join(dir).join("orqa-stage-fix")).unwrap(),
            Path::new("../../.orqadence/skills/orqa-stage-fix"),
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
    // orqa-stage-fix as an older release wrote it, still unedited: the record agrees.
    let stale = repo.path().join(STAGE_FIX);
    fs::write(&stale, "older shipped text").unwrap();
    let mut rec = record(repo.path());
    rec.insert(STAGE_FIX.to_string(), "older shipped text".to_string());
    fs::write(
        repo.path().join(RECORD),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();
    // orqa-stage-review edited in the Target repo: the record disagrees.
    let edited = repo
        .path()
        .join(".agents/skills/orqa-stage-review/SKILL.md");
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
            .contains("name: orqa-stage-review"),
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

/// A personal pick counts only once the user turns their personal skills on.
#[test]
fn preflight_counts_a_personal_pick_only_when_personal_skills_are_on() {
    let (repo, home) = (TempDir::new(), TempDir::new());
    let mut manifest = Manifest::default();
    for (job, _) in JOBS {
        manifest.picks.insert(job.to_string(), NONE.to_string());
    }
    manifest.picks.insert("prose".into(), "mine".into());
    manifest.save(repo.path()).unwrap();
    write_file(&home.path().join(".claude/skills/mine/SKILL.md"), "mine");
    assert_eq!(
        picks_missing(repo.path(), home.path()),
        ["the prose skill mine is missing: /config installs it, or picks another"]
    );
    crate::skills::manifest::set_personal(repo.path(), true).unwrap();
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
            &"the self review skill orqa-code-review is missing: orqa init installs it, or /config picks another"
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

    // A job's default is init's to install; one not a job's default is
    // /config's.
    manifest
        .picks
        .insert("test-first".into(), "orqa-tdd".into());
    manifest
        .picks
        .insert("prose".into(), "caveman-commit".into());
    manifest.save(repo.path()).unwrap();
    assert_eq!(
        picks_missing(repo.path(), home.path()),
        [
            "the test first skill orqa-tdd is missing: orqa init installs it, or /config picks another",
            "the prose skill caveman-commit is missing: /config installs it, or picks another",
        ]
    );
    manifest.picks.insert("prose".into(), NONE.into());
    manifest.save(repo.path()).unwrap();
    write_file(
        &repo.path().join(".orqadence/skills/orqa-tdd/SKILL.md"),
        "installed",
    );
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
        .find(|(name, _)| *name == "orqa-stage-implement")
        .unwrap()
        .1;
    let at = repo
        .path()
        .join(".agents/skills/orqa-stage-implement/SKILL.md");
    write_file(&at, shipped);
    assert_eq!(warnings(repo.path(), &*Fake::quiet()), Vec::<String>::new());

    let edited: Vec<&str> = shipped
        .lines()
        .filter(|line| !line.contains("{{test-first}}"))
        .collect();
    write_file(&at, &edited.join("\n"));
    assert_eq!(
        warnings(repo.path(), &*Fake::quiet()),
        ["the installed orqa-stage-implement lacks {{test-first}}: the test first skill you pick never runs there; put the line back, or refresh it with orqa init"]
    );
}
