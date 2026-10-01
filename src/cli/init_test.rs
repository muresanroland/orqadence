use super::run;
use crate::orchestrator::app::{self, Label};
use crate::orchestrator::write_file;
use crate::setup::setup_test::snapshot;
use crate::setup::{DEFAULT_TEMPLATE, LABELS, PR_TEMPLATE, TEMPLATE_DIR, TYPESAFE_SKILL};
use crate::skills::manifest::{Installed, Manifest, JOBS};
use crate::skills::SKILLS;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use crate::tools::Tools;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

/// A Target repo that passes every preflight check.
pub(super) fn prepared_repo() -> TempDir {
    let repo = TempDir::new();
    for dir in [".beads", ".agents/skills/create-pr"] {
        fs::create_dir_all(repo.path().join(dir)).unwrap();
    }
    fs::write(repo.path().join(".agents/skills/create-pr/SKILL.md"), "pr").unwrap();
    repo
}

/// Every prerequisite there, and a git whose every clone holds every job's
/// default, the typesafe-ai skill and every shipped label's skills at their
/// paths in the source, at commit abc123; terraform 1.9.0 is on PATH.
pub(super) fn ok_tools() -> Arc<Fake> {
    Fake::new(ok)
}

/// ok_tools' answers.
fn ok(_: &Path, argv: &[&str]) -> Result<String, String> {
    if argv.contains(&"clone") {
        let dest = Path::new(argv.last().unwrap());
        let defaults = JOBS.iter().map(|(_, suggestions)| suggestions[0]);
        let labels = LABELS.iter().flat_map(|label| label.sources());
        for (name, source) in defaults.chain([TYPESAFE_SKILL]).chain(labels) {
            if source.is_empty() {
                continue; // built in, or Shipped
            }
            // The skill's folder, the clone's root when the source is the repo alone.
            let path = source.splitn(3, '/').nth(2).unwrap_or("");
            // named upstream without the prefix
            let name = name.strip_prefix("orqa-").unwrap_or(name);
            write_file(
                &dest.join(path).join("SKILL.md"),
                &format!("---\nname: {name}\n---\n"),
            );
        }
    }
    Ok(match argv.join(" ").as_str() {
        "git remote" => "origin\n",
        "git rev-parse HEAD" => "abc123\n",
        "terraform version -json" => r#"{"terraform_version": "1.9.0"}"#,
        _ => "",
    }
    .to_string())
}

/// herdr_env with HOME at home, and TYPESAFE_API_KEY at key.
fn env_home(home: &Path, key: &str) -> impl Fn(&str) -> String {
    let (home, key) = (home.display().to_string(), key.to_string());
    move |name: &str| match name {
        "HOME" => home.clone(),
        "TYPESAFE_API_KEY" => key.clone(),
        _ => herdr_env(name),
    }
}

/// init with each key its own read, as a terminal delivers them.
fn init_keys(repo: &Path, home: &Path, keys: &[&str]) -> (i32, String) {
    init_with(repo, home, ok_tools(), keys, "")
}

/// init_keys with these tools and TYPESAFE_API_KEY at typesafe_key.
fn init_with(
    repo: &Path,
    home: &Path,
    tools: Arc<dyn Tools>,
    keys: &[&str],
    typesafe_key: &str,
) -> (i32, String) {
    let mut input: Box<dyn Read> = Box::new(std::io::empty());
    for key in keys.iter().rev() {
        input = Box::new(key.as_bytes().chain(input));
    }
    let mut out = Vec::new();
    let code = run(
        &["init".to_string()],
        &mut out,
        Some(&mut input),
        repo,
        tools,
        &env_home(home, typesafe_key),
    );
    (code, String::from_utf8(out).unwrap())
}

/// A skill's folder in .orqadence/skills holds it, and .agents/skills and
/// .claude/skills link that folder, relative.
fn assert_placed(repo: &Path, name: &str, text: &str) {
    let at = repo.join(".orqadence/skills").join(name).join("SKILL.md");
    let got = fs::read_to_string(&at).unwrap_or_else(|err| panic!("{name}: {err}"));
    assert!(got.contains(text), "{name}: {got}");
    for dir in [".agents/skills", ".claude/skills"] {
        let link = repo.join(dir).join(name);
        assert_eq!(
            fs::read_link(&link).ok(),
            Some(Path::new("../../.orqadence/skills").join(name)),
            "{link:?}"
        );
    }
}

/// Every Shipped skill and each job's default go in .orqadence/skills,
/// linked from .agents/skills and .claude/skills; nothing asks where.
#[test]
fn a_fresh_init_puts_every_skill_in_orqadence_skills_linked_from_both() {
    let (repo, home) = (bare_repo(), TempDir::new());
    let (code, out) = init_keys(repo.path(), home.path(), &[]);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    assert!(!out.contains("where should the skills go"), "{out}");
    for (name, _) in SKILLS {
        assert_placed(repo.path(), name, &format!("name: {name}"));
    }
    assert_placed(repo.path(), "orqa-tdd", "name: orqa-tdd");
    let manifest = Manifest::load(repo.path()).unwrap();
    assert_eq!(manifest.skills["orqa-tdd"].commit, "abc123");
    assert!(manifest.skills["orqa-stage-implement"].shipped);
}

/// A checkout an older init set up in the repo location: the skills it
/// installed in .agents/skills move to .orqadence/skills, edits and all,
/// renamed to orqa-<name>, and leave links behind; the repo's own skill
/// stays as it was.
#[test]
fn init_moves_the_skills_an_older_init_put_in_agents_skills() {
    let (repo, home) = (bare_repo(), TempDir::new());
    write_file(
        &repo.path().join(".orqadence/skills.json"),
        r#"{"location": "repo", "skills": {"stage-fix": {"shipped": true}, "tdd": {"repo": "https://github.com/mattpocock/skills", "path": "skills/engineering/tdd", "commit": "abc123"}}}"#,
    );
    let agents = repo.path().join(".agents/skills");
    write_file(&agents.join("stage-fix/SKILL.md"), "edited stage-fix");
    write_file(&agents.join("tdd/SKILL.md"), "---\nname: tdd\n---\nold");
    write_file(&agents.join("own/SKILL.md"), "the repo's own");
    fs::create_dir_all(repo.path().join(".claude/skills")).unwrap();
    for name in ["stage-fix", "tdd"] {
        std::os::unix::fs::symlink(
            Path::new("../../.agents/skills").join(name),
            repo.path().join(".claude/skills").join(name),
        )
        .unwrap();
    }
    let (code, out) = init_keys(repo.path(), home.path(), &[]);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    assert_placed(repo.path(), "orqa-stage-fix", "edited stage-fix");
    assert_placed(repo.path(), "orqa-tdd", "---\nname: orqa-tdd\n---\nold");
    for old in ["stage-fix", "tdd"] {
        for dir in [".agents/skills", ".claude/skills", ".orqadence/skills"] {
            let at = repo.path().join(dir).join(old);
            assert!(fs::symlink_metadata(&at).is_err(), "{at:?} left");
        }
    }
    let own = fs::symlink_metadata(agents.join("own")).unwrap();
    assert!(own.is_dir(), "the repo's own skill moved");
    assert_eq!(
        fs::read_to_string(agents.join("own/SKILL.md")).unwrap(),
        "the repo's own"
    );
    assert!(fs::symlink_metadata(repo.path().join(".claude/skills/own")).is_err());
    assert!(!repo.path().join(".orqadence/skills/own").exists());
}

/// A checkout an older init set up at user level: each skill it installed
/// in ~/.agents/skills is copied in as it is, edits kept, and the ~ copy
/// stays for other checkouts.
#[test]
fn init_copies_the_skills_an_older_init_put_at_user_level() {
    let (repo, home) = (bare_repo(), TempDir::new());
    write_file(
        &repo.path().join(".orqadence/skills.json"),
        r#"{"location": "user", "skills": {"tdd": {"repo": "https://github.com/mattpocock/skills", "path": "skills/engineering/tdd", "commit": "abc123"}}}"#,
    );
    let mine = home.path().join(".agents/skills/tdd/SKILL.md");
    write_file(&mine, "---\nname: tdd\n---\nedited at user level");
    let (code, out) = init_keys(repo.path(), home.path(), &[]);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    assert_placed(repo.path(), "orqa-tdd", "edited at user level");
    assert_eq!(
        fs::read_to_string(&mine).unwrap(),
        "---\nname: tdd\n---\nedited at user level"
    );
    let saved = fs::read_to_string(repo.path().join(".orqadence/skills.json")).unwrap();
    assert!(!saved.contains("location"), "{saved}");
}

pub(super) fn herdr_env(key: &str) -> String {
    match key {
        "HERDR_ENV" => "1",
        "HERDR_PANE_ID" => "w1:p1",
        "HERDR_WORKSPACE_ID" => "w1",
        _ => "",
    }
    .to_string()
}

pub(super) fn run_with(
    args: &[&str],
    repo: &Path,
    tools: Arc<dyn Tools>,
    env: &dyn Fn(&str) -> String,
) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    let code = run(&args, &mut out, Some(&mut &b""[..]), repo, tools, env);
    (code, String::from_utf8(out).unwrap())
}

/// Init leaves the repo's own .gitignore alone: .orqadence-local ignores
/// itself.
#[test]
fn init_leaves_the_repos_gitignore_byte_for_byte() {
    let repo = prepared_repo();
    let ignore = "target/\n# ours, no newline at the end";
    fs::write(repo.path().join(".gitignore"), ignore).unwrap();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    assert_eq!(
        fs::read_to_string(repo.path().join(".gitignore")).unwrap(),
        ignore
    );
}

/// The .orqadence/ line an older init added goes, since .orqadence is
/// committed now; with no old run files there is nothing to ask.
#[test]
fn init_drops_the_old_gitignore_line_and_asks_nothing_without_old_run_files() {
    let repo = prepared_repo();
    let ignore = repo.path().join(".gitignore");
    fs::write(&ignore, "node_modules/\n.orqadence/\ntarget/\n").unwrap();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    assert_eq!(
        fs::read_to_string(&ignore).unwrap(),
        "node_modules/\ntarget/\n"
    );
    assert!(
        out.contains("init: removed .orqadence/ from .gitignore: its settings are committed now"),
        "not said:\n{out}"
    );
    assert!(!out.contains("Delete them?"), "asked:\n{out}");
}

/// The run files an init from before .orqadence-local left in .orqadence.
const OLD_RUN_FILES: [&str; 5] = [
    "state.json",
    "lock",
    "orchestrator.log",
    "runs",
    "worktrees",
];

/// A checkout from before .orqadence-local: the old .gitignore line, and
/// the run files of a saved run in .orqadence, two worktrees among them.
fn old_checkout() -> TempDir {
    let repo = prepared_repo();
    fs::write(repo.path().join(".gitignore"), ".orqadence/\n").unwrap();
    for file in [
        "state.json",
        "lock",
        "orchestrator.log",
        "runs/t1/implement.md",
        "worktrees/t1/f",
        "worktrees/t2/f",
    ] {
        write_file(&repo.path().join(".orqadence").join(file), "old");
    }
    repo
}

/// ok_tools, with uncommitted changes in the worktree of t1, and t2's one
/// git no longer knows.
fn old_tools() -> Arc<Fake> {
    Fake::new(|dir, argv| match argv {
        ["git", "status", "--porcelain", "--untracked-files=normal"] if dir.ends_with("t1") => {
            Ok(" M f\n".to_string())
        }
        ["git", "worktree", "remove", "--force", "--force", path] if path.ends_with("t2") => {
            Err("fatal: not a working tree".to_string())
        }
        _ => ok(dir, argv),
    })
}

#[test]
fn init_deletes_the_old_run_files_on_yes_keeping_the_branches() {
    let (repo, home) = (old_checkout(), TempDir::new());
    let tools = old_tools();
    let (code, out) = init_with(repo.path(), home.path(), tools.clone(), &["y\n"], "");
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    let old = repo.path().join(".orqadence");
    let question = &out[..out.find("Delete them?").expect("not asked")];
    assert!(
        question.contains(".orqadence/worktrees/t1 has uncommitted changes"),
        "t1 not named:\n{question}"
    );
    assert!(!question.contains("t2 has"), "clean t2 named:\n{question}");
    let calls = tools.calls();
    for ticket in ["t1", "t2"] {
        let remove = format!(
            "git worktree remove --force --force {}",
            old.join("worktrees").join(ticket).display()
        );
        assert!(calls.contains(&remove), "no {remove}: {calls:?}");
    }
    assert!(
        out.contains("not a working tree; deleting its folder anyway"),
        "t2's failure not said:\n{out}"
    );
    let t2 = format!(
        "git worktree remove --force --force {}",
        old.join("worktrees/t2").display()
    );
    let t2_removes = calls.iter().filter(|call| **call == t2).count();
    assert_eq!(t2_removes, 2, "t2's record not removed again: {calls:?}");
    assert!(
        !calls.iter().any(|call| call.contains("worktree prune")),
        "every missing worktree pruned: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.contains("branch -D")),
        "a branch deleted: {calls:?}"
    );
    for file in OLD_RUN_FILES {
        assert!(!old.join(file).exists(), "{file} left");
    }
}

/// No, a plain enter (the default is no) or nobody answering: init stops
/// before the skills step with nothing touched, and says why.
#[test]
fn init_keeps_the_old_run_files_and_fails_unless_told_yes() {
    for keys in [&["n\n"][..], &["\n"], &[]] {
        let (repo, home) = (old_checkout(), TempDir::new());
        let before = snapshot(repo.path());
        let tools = old_tools();
        let (code, out) = init_with(repo.path(), home.path(), tools.clone(), keys, "");
        assert_ne!(code, 0, "{keys:?}: init went on:\n{out}");
        assert!(
            out.contains("init: stopped: the old run files in .orqadence are kept"),
            "{keys:?}: no reason:\n{out}"
        );
        assert!(
            !out.contains("already has a create-pr skill"),
            "{keys:?}: reached the skills step:\n{out}"
        );
        assert_eq!(snapshot(repo.path()), before, "{keys:?}: touched");
        assert!(
            !tools
                .calls()
                .iter()
                .any(|call| call.contains("worktree remove")),
            "{keys:?}: {:?}",
            tools.calls()
        );
    }
}

/// A Shell from before .orqadence-local still holds .orqadence/lock: init
/// refuses, even told yes, and touches nothing.
#[test]
fn init_refuses_while_a_shell_holds_the_old_lock() {
    let (repo, home) = (old_checkout(), TempDir::new());
    let lock = repo.path().join(".orqadence/lock");
    fs::write(&lock, std::process::id().to_string()).unwrap();
    let held = fs::File::open(&lock).unwrap();
    held.lock().unwrap();
    let before = snapshot(repo.path());
    let tools = old_tools();
    let (code, out) = init_with(repo.path(), home.path(), tools.clone(), &["y\n"], "");
    assert_ne!(code, 0, "init went on:\n{out}");
    assert!(
        out.contains(
            "init: a Shell is running in this checkout; close it, then run orqa init again"
        ),
        "no refusal:\n{out}"
    );
    assert_eq!(snapshot(repo.path()), before, "touched");
    assert_eq!(tools.calls(), Vec::<String>::new());
}

/// A checkout from before .orqadence-local, its skills installed: an init
/// that cancels at the skills gate still makes the folder orqa opens on,
/// and moves the TypeSafe key kept in .orqadence into it.
#[test]
fn init_cancelled_at_the_skills_gate_still_makes_the_local_folder() {
    let repo = prepared_repo();
    run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    fs::remove_dir_all(repo.path().join(".orqadence-local")).unwrap();
    let old_key = repo.path().join(".orqadence/typesafe-key");
    fs::write(&old_key, "sk-old").unwrap();
    fs::set_permissions(&old_key, fs::Permissions::from_mode(0o644)).unwrap();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    assert!(out.contains("already installed"), "no gate:\n{out}");
    assert_eq!(
        fs::read_to_string(repo.path().join(".orqadence-local/.gitignore")).unwrap(),
        "*\n"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".orqadence-local/typesafe-key")).unwrap(),
        "sk-old"
    );
    let key = fs::metadata(repo.path().join(".orqadence-local/typesafe-key")).unwrap();
    assert_eq!(
        key.permissions().mode() & 0o777,
        0o600,
        "readable by others"
    );
    assert!(!old_key.exists());

    // An old key beside the kept one: the kept one wins, the old one goes.
    fs::write(&old_key, "sk-stale").unwrap();
    run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(
        fs::read_to_string(repo.path().join(".orqadence-local/typesafe-key")).unwrap(),
        "sk-old"
    );
    assert!(!old_key.exists(), "a stale key left to be committed");

    // A key an earlier init moved but left readable by others is narrowed.
    let kept = repo.path().join(".orqadence-local/typesafe-key");
    fs::set_permissions(&kept, fs::Permissions::from_mode(0o644)).unwrap();
    run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(
        fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
        0o600,
        "a moved key left readable by others"
    );
}

#[test]
fn init_keeps_edited_skill_unless_forced() {
    let repo = prepared_repo();
    run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    let skill = repo
        .path()
        .join(".orqadence/skills/orqa-stage-fix/SKILL.md");
    let shipped = fs::read_to_string(&skill).unwrap();
    fs::write(&skill, "edited in the Target repo").unwrap();

    let (code, _) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "second init exit {code}");
    assert_eq!(
        fs::read_to_string(&skill).unwrap(),
        "edited in the Target repo",
        "rerun overwrote an edited skill"
    );

    run_with(&["init", "--force"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(
        fs::read_to_string(&skill).unwrap(),
        shipped,
        "--force did not restore the shipped skill"
    );
}

#[test]
fn preflight_names_each_missing_prerequisite() {
    let repo = TempDir::new(); // no bd workspace, no create-pr skill
    let tools: Arc<dyn Tools> = Fake::new(|_, argv| {
        if argv.join(" ") == "gh auth status" {
            return Err("not logged in".to_string());
        }
        Ok(String::new()) // git remote prints nothing
    });
    let no_env = |_: &str| String::new();

    let (code, out) = run_with(&["init"], repo.path(), tools.clone(), &no_env);
    assert_ne!(code, 0, "init: exit 0 with nothing prepared");
    for want in [
        "bd workspace",
        "gh is not authenticated",
        "git remote",
        "HERDR_ENV",
    ] {
        assert!(out.contains(want), "init: output lacks {want:?}:\n{out}");
    }
}

/// orqa-create-pr goes beside a repo's own create-pr, unasked.
#[test]
fn init_installs_orqa_create_pr_and_keeps_the_repos_own() {
    let own = prepared_repo();
    let (code, out) = run_with(&["init"], own.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    let got = fs::read_to_string(own.path().join(".orqadence/skills/orqa-create-pr/SKILL.md"));
    assert!(
        got.as_ref()
            .is_ok_and(|got| got.contains("name: orqa-create-pr")),
        "shipped orqa-create-pr not installed: {got:?}"
    );
    let got = fs::read_to_string(own.path().join(".agents/skills/create-pr/SKILL.md")).unwrap();
    assert_eq!(got, "pr", "init replaced the repo's own create-pr: {got:?}");
    assert!(!out.contains("create-pr skill"), "init asked:\n{out}");
}

#[test]
fn init_asks_for_typesafe_and_preflight_warns_when_off() {
    let repo = prepared_repo();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "TypeSafe off failed preflight:\n{out}");
    assert!(
        out.contains("preflight: TypeSafe is off: every Wake will be a Question"),
        "no warning with TypeSafe off:\n{out}"
    );
    assert!(out.contains("Use TypeSafe"), "init did not ask:\n{out}");

    let with_key = |key: &str| {
        let key = key.to_string();
        move |name: &str| {
            if name == "TYPESAFE_API_KEY" {
                key.clone()
            } else {
                herdr_env(name)
            }
        }
    };
    let (_, out) = run_with(&["init"], repo.path(), ok_tools(), &with_key("sk-env"));
    assert!(
        !out.contains("Use TypeSafe"),
        "asked with the variable set:\n{out}"
    );
    assert!(
        !out.contains("preflight: TypeSafe") && !out.contains("no TypeSafe key"),
        "warned with the variable set:\n{out}"
    );

    // Typed on init's stdin after the answer to the gate (its own
    // keystroke, as a terminal delivers it), yes and the key are kept in the
    // repo.
    let args = ["init".to_string()];
    let mut out = Vec::new();
    let mut keys = b"2".chain(&b"\r"[..]).chain(&b"sk-typed\n"[..]);
    let code = run(
        &args,
        &mut out,
        Some(&mut keys),
        repo.path(),
        ok_tools(),
        &herdr_env,
    );
    let out = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(repo.path().join(".orqadence-local/typesafe-key"))
            .unwrap()
            .trim(),
        "sk-typed"
    );
    assert!(
        !out.contains("preflight: TypeSafe"),
        "warned with the key kept:\n{out}"
    );
}

/// A Target repo that passes every preflight check and has no skill of its
/// own: init asks the docs/agents setup, then TypeSafe.
fn bare_repo() -> TempDir {
    let repo = prepared_repo();
    fs::remove_dir_all(repo.path().join(".agents")).unwrap();
    repo
}

#[test]
fn typesafe_is_its_own_opt_in_kept_in_config_json() {
    // (answers after the docs/agents setup, TYPESAFE_API_KEY, on)
    for (keys, env_key, on) in [
        (&["\n", "sk-typed\n"][..], "", true),
        (&["y\n", "\n"][..], "", false), // an empty key is no
        (&["n\n"][..], "", false),
        (&[][..], "sk-env", true), // not asked
        (&[][..], "", false),      // non-interactive
    ] {
        let (repo, home) = (bare_repo(), TempDir::new());
        let keys: Vec<&str> = ["\n"].iter().chain(keys).copied().collect();
        let (code, out) = init_with(repo.path(), home.path(), ok_tools(), &keys, env_key);
        let case = format!("{keys:?} {env_key:?}");
        assert_eq!(code, 0, "{case}: init exit {code}:\n{out}");
        assert_eq!(
            out.contains("Use TypeSafe"),
            env_key.is_empty(),
            "{case}: asked or not:\n{out}"
        );
        let config: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(repo.path().join(".orqadence/config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["typesafe"], on, "{case}:\n{out}");
        let manifest = Manifest::load(repo.path()).unwrap();
        assert_eq!(
            manifest.skills.contains_key("orqa-typesafe-ai"),
            on,
            "{case}:\n{out}"
        );
        assert_eq!(
            out.contains("preflight: TypeSafe is off"),
            !on,
            "{case}:\n{out}"
        );
    }
    // The typed key is kept where it always was.
    let (repo, home) = (bare_repo(), TempDir::new());
    init_keys(repo.path(), home.path(), &["\n", "\n", "sk-typed\n"]);
    assert_eq!(
        fs::read_to_string(repo.path().join(".orqadence-local/typesafe-key"))
            .unwrap()
            .trim(),
        "sk-typed"
    );
    // Nobody answering later keeps the choice made; Ctrl-C is no.
    let (_, out) = init_keys(repo.path(), home.path(), &["2"]);
    assert!(out.contains("init: TypeSafe on"), "{out}");
    let (_, out) = init_keys(repo.path(), home.path(), &["2", "\x03"]);
    assert!(out.contains("init: TypeSafe off"), "{out}");
}

#[test]
fn with_no_beads_yes_runs_bd_init_and_non_interactive_skips() {
    let (repo, home) = (bare_repo(), TempDir::new());
    fs::remove_dir_all(repo.path().join(".beads")).unwrap();
    let tools = ok_tools();
    let (_, out) = init_with(repo.path(), home.path(), tools.clone(), &["\n"], "");
    assert!(
        out.contains("Run bd init now?") && out.contains("› 1. Yes"),
        "{out}"
    );
    assert!(
        tools
            .calls()
            .contains(&"bd init --non-interactive".to_string()),
        "{out}"
    );

    let repo = bare_repo();
    fs::remove_dir_all(repo.path().join(".beads")).unwrap();
    let tools = ok_tools();
    let (code, out) = run_with(&["init"], repo.path(), tools.clone(), &herdr_env);
    assert!(
        !tools.calls().iter().any(|c| c.starts_with("bd init")),
        "{out}"
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("preflight: no bd workspace here"), "{out}");
}

#[test]
fn docs_agents_writes_only_what_is_missing_and_the_block_into_claude_md_else_agents_md() {
    let model = |name: &str| fs::read_to_string(Path::new("docs/agents").join(name)).unwrap();
    let repo = prepared_repo();
    write_file(&repo.path().join("docs/agents/domain.md"), "ours");
    write_file(&repo.path().join("CLAUDE.md"), "# Ours\n");
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "{out}");
    assert_eq!(read(repo.path(), "docs/agents/domain.md"), "ours");
    for name in ["issue-tracker.md", "triage-labels.md"] {
        assert_eq!(
            read(repo.path(), &format!("docs/agents/{name}")),
            model(name)
        );
    }
    let claude = read(repo.path(), "CLAUDE.md");
    assert!(
        claude.starts_with("# Ours\n\n## Agent skills\n")
            && claude.contains("docs/agents/domain.md"),
        "{claude}"
    );
    assert!(!repo.path().join("AGENTS.md").exists());
    // Ctrl-C is no: nothing written.
    let repo_c = bare_repo();
    init_keys(repo_c.path(), TempDir::new().path(), &["\x03"]);
    assert!(!repo_c.path().join("docs").exists());
    assert!(!repo_c.path().join("AGENTS.md").exists());
    // Everything there: asked nothing.
    let (_, out) = init_keys(repo.path(), TempDir::new().path(), &["2"]);
    assert!(!out.contains("docs/agents setup"), "{out}");

    // No CLAUDE.md: the block goes into AGENTS.md.
    let repo = prepared_repo();
    write_file(&repo.path().join("AGENTS.md"), "# Agents");
    run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert!(
        read(repo.path(), "AGENTS.md").starts_with("# Agents\n\n## Agent skills\n"),
        "{}",
        read(repo.path(), "AGENTS.md")
    );
    assert_eq!(
        read(repo.path(), "docs/agents/domain.md"),
        model("domain.md")
    );
    assert!(!repo.path().join("CLAUDE.md").exists());
}

fn read(repo: &Path, path: &str) -> String {
    fs::read_to_string(repo.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
}

#[test]
fn an_outdated_app_is_offered_once_and_yes_installs_it() {
    const STATUS: &str = "pi: not installed (/h/.pi/agent/extensions/herdr-agent-state.ts)
claude: current (v10) (/h/.claude/hooks/herdr-agent-state.sh)
codex: outdated (v7) (/h/.codex/herdr-agent-state.sh)
";
    let herdr = |codex_on_path: bool| {
        Fake::new(move |dir, argv| match argv.join(" ").as_str() {
            "herdr integration status" => Ok(STATUS.to_string()),
            "which codex" if !codex_on_path => Err("codex not found".to_string()),
            // pi, in the App table, is not on PATH: not offered.
            "which pi" => Err("pi not found".to_string()),
            _ => ok(dir, argv),
        })
    };
    let installs = |tools: &Fake| -> Vec<String> {
        let calls = tools.calls();
        calls
            .into_iter()
            .filter(|c| c.starts_with("herdr integration install"))
            .collect()
    };
    // The docs/agents setup, TypeSafe no, the labels, both switches no, On
    // call no, then the integrations: yes.
    let (repo, home) = (bare_repo(), TempDir::new());
    let tools = herdr(true);
    let (_, out) = init_with(
        repo.path(),
        home.path(),
        tools.clone(),
        &["\n", "n\n", "\n", "\n", "\n", "\n", "\n"],
        "",
    );
    assert_eq!(
        out.matches("Install herdr's integration").count(),
        1,
        "{out}"
    );
    assert!(
        out.contains(" ⬡ Codex ")
            && out.contains("  outdated (v7) (/h/.codex/herdr-agent-state.sh)"),
        "{out}"
    );
    assert!(
        !out.contains("/h/.pi/") && !out.contains("/h/.claude/"),
        "{out}"
    );
    assert_eq!(installs(&tools), ["herdr integration install codex"]);

    // Nobody answering skips it and says what that costs.
    let tools = herdr(true);
    let (_, out) = run_with(&["init"], bare_repo().path(), tools.clone(), &herdr_env);
    assert!(out.contains("/continue starts them fresh"), "{out}");
    assert_eq!(installs(&tools), Vec::<String>::new());

    // An App not on PATH is not offered.
    let tools = herdr(false);
    let (_, out) = run_with(&["init"], bare_repo().path(), tools.clone(), &herdr_env);
    assert!(!out.contains("Install herdr's integration"), "{out}");

    // A failed status is said, not taken for nothing to offer.
    let tools = Fake::new(|dir, argv| match argv.join(" ").as_str() {
        "herdr integration status" => Err("unknown command integration".to_string()),
        _ => ok(dir, argv),
    });
    let (_, out) = run_with(&["init"], bare_repo().path(), tools, &herdr_env);
    assert!(
        out.contains("herdr integration status failed")
            && out.contains("unknown command integration"),
        "{out}"
    );
}

const COMMITTED: &str =
    "init: this repo's Orqadence settings are committed; asking only this machine's questions";

/// ok_tools in a checkout whose git tracks .orqadence/config.json, with
/// codex's herdr integration outdated.
fn committed_tools() -> Arc<Fake> {
    Fake::new(|dir, argv| match argv.join(" ").as_str() {
        "git ls-files .orqadence/config.json" => Ok(".orqadence/config.json\n".to_string()),
        "herdr integration status" => {
            Ok("codex: outdated (v7) (/h/.codex/herdr-agent-state.sh)\n".to_string())
        }
        _ => ok(dir, argv),
    })
}

/// A fresh clone of a repo whose settings are committed: the team's
/// questions are not asked again and no job's default is installed; this
/// machine's steps still run.
#[test]
fn a_committed_checkout_asks_only_this_machines_questions() {
    let (repo, home) = (prepared_repo(), TempDir::new());
    let config = r#"{"typesafe": false}"#;
    write_file(&repo.path().join(".orqadence/config.json"), config);
    let tools = committed_tools();
    let (_, out) = init_with(repo.path(), home.path(), tools.clone(), &[], "");
    assert_eq!(out.matches(COMMITTED).count(), 1, "{out}");
    assert!(!out.contains("Use TypeSafe"), "{out}");
    assert!(
        !tools.calls().iter().any(|call| call.contains("clone")),
        "a default installed: {:?}",
        tools.calls()
    );
    assert!(out.contains("Install herdr's integration"), "{out}");
    assert!(repo.path().join(".orqadence-local").is_dir());
    assert_eq!(read(repo.path(), ".agents/skills/create-pr/SKILL.md"), "pr");
    assert_eq!(read(repo.path(), ".orqadence/config.json"), config);
}

/// TypeSafe committed on: the key is asked, and kept, unless
/// TYPESAFE_API_KEY is set or a key is kept already.
#[test]
fn a_committed_checkout_with_typesafe_on_asks_for_the_key_unless_set() {
    // (TYPESAFE_API_KEY, a key kept, asked)
    for (env_key, kept_key, asked) in [
        ("", false, true),
        ("sk-env", false, false),
        ("", true, false),
    ] {
        let (repo, home) = (prepared_repo(), TempDir::new());
        write_file(
            &repo.path().join(".orqadence/config.json"),
            r#"{"typesafe": true}"#,
        );
        if kept_key {
            write_file(
                &repo.path().join(".orqadence-local/typesafe-key"),
                "sk-kept\n",
            );
        }
        // The docs/agents setup, then the key.
        let keys = ["\n", "sk-typed\n"];
        let (_, out) = init_with(repo.path(), home.path(), committed_tools(), &keys, env_key);
        assert_eq!(
            out.contains("TypeSafe API key"),
            asked,
            "{env_key:?}:\n{out}"
        );
        assert!(!out.contains("Use TypeSafe"), "{env_key:?}:\n{out}");
        let kept = fs::read_to_string(repo.path().join(".orqadence-local/typesafe-key"));
        assert_eq!(kept.is_ok_and(|key| key.trim() == "sk-typed"), asked);
    }
}

/// Cancel at the Shipped skills gate leaves the skills as they are and, on
/// a committed checkout, goes on to bd init's offer and herdr's integrations.
#[test]
fn cancel_at_the_gate_on_a_committed_checkout_goes_on_to_this_machines_steps() {
    let (repo, home) = (bare_repo(), TempDir::new());
    init_keys(repo.path(), home.path(), &[]);
    let skill = repo
        .path()
        .join(".orqadence/skills/orqa-stage-fix/SKILL.md");
    fs::write(&skill, "edited").unwrap();
    fs::remove_dir_all(repo.path().join(".beads")).unwrap();
    let (_, out) = init_with(repo.path(), home.path(), committed_tools(), &["1"], "");
    assert!(
        out.contains("✓ cancel, leave their text as it is"),
        "not cancelled:\n{out}"
    );
    assert!(out.contains("Run bd init now?"), "{out}");
    assert!(out.contains("Install herdr's integration"), "{out}");
    assert_eq!(fs::read_to_string(&skill).unwrap(), "edited");
}

/// A config.json git does not track: every question, as before.
#[test]
fn a_repo_whose_config_json_is_untracked_asks_every_question() {
    let (repo, home) = (prepared_repo(), TempDir::new());
    write_file(&repo.path().join(".orqadence/config.json"), "{}");
    let tools = ok_tools();
    let (_, out) = init_with(repo.path(), home.path(), tools.clone(), &[], "");
    assert!(!out.contains(COMMITTED), "{out}");
    for question in ["docs/agents setup", "Use TypeSafe"] {
        assert!(out.contains(question), "{question:?} not asked:\n{out}");
    }
    assert!(
        tools.calls().iter().any(|call| call.contains("clone")),
        "no default installed: {:?}",
        tools.calls()
    );
}

const ON_CALL: &str = "Ring your phone through Moshi when a Question waits?";

/// The On call object init kept in .orqadence-local/config.json, if any.
fn on_call(repo: &Path) -> Option<serde_json::Value> {
    let text = fs::read_to_string(repo.join(".orqadence-local/config.json")).ok()?;
    Some(serde_json::from_str::<serde_json::Value>(&text).unwrap()["on_call"].clone())
}

/// On call is asked after TypeSafe, default no: y and a token keep it,
/// readable only by you; enter alone, or nobody answering, keeps none.
#[test]
fn init_asks_on_call_and_keeps_the_moshi_token_readable_only_by_you() {
    // The docs/agents setup, TypeSafe no, the labels, both switches no, On
    // call yes, the token.
    let (repo, home) = (bare_repo(), TempDir::new());
    let keys = ["\n", "n\n", "\n", "\n", "\n", "y\n", "moshi-tok\n"];
    let (code, out) = init_keys(repo.path(), home.path(), &keys);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(ON_CALL) && out.contains("› 2. No"), "{out}");
    assert!(
        out.contains("see https://github.com/muresanroland/orqadence/blob/main/docs/on-call.md"),
        "{out}"
    );
    assert!(!out.contains("moshi-tok"), "the token echoed:\n{out}");
    assert!(
        out.contains("Moshi token kept in .orqadence-local/config.json"),
        "{out}"
    );
    assert_eq!(on_call(repo.path()).unwrap()["token"], "moshi-tok");
    let path = repo.path().join(".orqadence-local/config.json");
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // Enter alone is no; nobody answering changes nothing.
    for keys in [&["\n", "n\n", "\n", "\n", "\n", "\n"][..], &[][..]] {
        let (repo, home) = (bare_repo(), TempDir::new());
        let (code, out) = init_keys(repo.path(), home.path(), keys);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains(ON_CALL), "{keys:?}: not asked:\n{out}");
        assert_eq!(on_call(repo.path()), None, "{keys:?}:\n{out}");
    }
}

/// A token kept is not asked again: init says On call is on.
#[test]
fn a_kept_moshi_token_is_not_asked_again() {
    let (repo, home) = (bare_repo(), TempDir::new());
    write_file(
        &repo.path().join(".orqadence-local/config.json"),
        r#"{"on_call": {"token": "kept", "minutes": 5}}"#,
    );
    let (_, out) = init_keys(repo.path(), home.path(), &["\n", "n\n"]);
    assert!(!out.contains(ON_CALL), "{out}");
    assert!(out.contains("init: On call on"), "{out}");
    assert_eq!(on_call(repo.path()).unwrap()["token"], "kept");
}

/// On call is per person: a checkout whose settings are committed asks it.
#[test]
fn a_committed_checkout_asks_on_call() {
    let (repo, home) = (prepared_repo(), TempDir::new());
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"typesafe": false}"#,
    );
    // The docs/agents setup, On call yes, the token.
    let keys = ["\n", "y\n", "moshi-tok\n"];
    let (_, out) = init_with(repo.path(), home.path(), committed_tools(), &keys, "");
    assert!(out.contains(COMMITTED), "{out}");
    assert!(out.contains(ON_CALL), "{out}");
    assert_eq!(on_call(repo.path()).unwrap()["token"], "moshi-tok");
}

const REBASE_QUESTION: &str = "Rebase PRs that conflict with main by themselves?";
const ADDRESS_PR_COMMENTS_QUESTION: &str =
    "Open PR comments and failing checks for approval by themselves?";

/// config.json's two switches: (rebase_auto, address_pr_comments_auto),
/// each on only when true.
fn switches(repo: &Path) -> (bool, bool) {
    (
        app::switch(repo, &app::REBASE_AUTO),
        app::switch(repo, &app::ADDRESS_PR_COMMENTS_AUTO),
    )
}

/// After the Ticket labels init asks the two switches, default no: y and n
/// set each, enter alone is no, and nobody answering leaves both off.
#[test]
fn init_asks_both_switches_and_enter_is_no() {
    // (the answers after the docs/agents setup, TypeSafe no and the labels)
    for (keys, want) in [
        (&["y\n", "n\n"][..], (true, false)),
        (&["n\n", "y\n"][..], (false, true)),
        (&["\n", "\n"][..], (false, false)),
    ] {
        let (repo, home) = (bare_repo(), TempDir::new());
        let keys: Vec<&str> = ["\n", "n\n", "\n"].iter().chain(keys).copied().collect();
        let (code, out) = init_keys(repo.path(), home.path(), &keys);
        assert_eq!(code, 0, "{keys:?}:\n{out}");
        assert!(
            out.contains(REBASE_QUESTION) && out.contains(ADDRESS_PR_COMMENTS_QUESTION),
            "{out}"
        );
        assert_eq!(switches(repo.path()), want, "{keys:?}:\n{out}");
    }
    let repo = bare_repo();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(REBASE_QUESTION) && out.contains(ADDRESS_PR_COMMENTS_QUESTION),
        "{out}"
    );
    assert_eq!(switches(repo.path()), (false, false), "{out}");
}

/// A re-run keeps a switch that is on: nobody answering, or enter alone,
/// changes nothing.
#[test]
fn a_rerun_of_init_keeps_a_switch_that_is_on() {
    let (repo, home) = (bare_repo(), TempDir::new());
    init_keys(repo.path(), home.path(), &["\n", "n\n", "\n", "y\n", "y\n"]);
    assert_eq!(switches(repo.path()), (true, true));
    // The gate, then nobody answering.
    let (_, out) = init_keys(repo.path(), home.path(), &["2"]);
    assert!(
        out.contains(ADDRESS_PR_COMMENTS_QUESTION),
        "not asked:\n{out}"
    );
    assert_eq!(switches(repo.path()), (true, true), "{out}");
    // The gate, TypeSafe no, the labels, then enter on each switch.
    let (_, out) = init_keys(repo.path(), home.path(), &["2", "n\n", "\r", "\n", "\n"]);
    let asked = &out[out.find(REBASE_QUESTION).expect("not asked")..];
    assert_eq!(
        asked.matches("✓ Yes").count(),
        2,
        "not both answered:\n{asked}"
    );
    assert_eq!(switches(repo.path()), (true, true), "{out}");
}

/// The switches are team settings: a checkout whose settings are committed
/// is not asked them.
#[test]
fn a_committed_checkout_is_not_asked_the_switches() {
    let (repo, home) = (prepared_repo(), TempDir::new());
    let config = r#"{"typesafe": false}"#;
    write_file(&repo.path().join(".orqadence/config.json"), config);
    let (_, out) = init_with(repo.path(), home.path(), committed_tools(), &["\n"], "");
    assert!(out.contains(COMMITTED), "{out}");
    assert!(
        !out.contains(REBASE_QUESTION) && !out.contains(ADDRESS_PR_COMMENTS_QUESTION),
        "{out}"
    );
    assert_eq!(read(repo.path(), ".orqadence/config.json"), config);
}

/// config.json's labels, each entry read as the Orchestrator reads it.
fn labels_in(repo: &Path) -> BTreeMap<String, Label> {
    let (_, doc) = app::read(repo).unwrap();
    app::labels(&doc)
        .into_iter()
        .map(|(name, label)| (name, label.unwrap()))
        .collect()
}

/// The rule that opens init's labels step.
const LABELS_STEP: &str = "TICKET LABELS";

/// init lists the seven shipped labels checked, and nobody answering keeps
/// them all: each gets its entry, kind, guidance, rows and Extra review as
/// shipped, and its skills are installed through the manifest with their
/// sources; the Shipped orqa-infra-review is not fetched.
#[test]
fn init_lists_the_shipped_labels_checked_and_writes_all_seven_unanswered() {
    let repo = bare_repo();
    let tools = ok_tools();
    let (code, out) = run_with(&["init"], repo.path(), tools.clone(), &herdr_env);
    assert_eq!(code, 0, "{out}");
    let list = &out[out.find(LABELS_STEP).expect("no labels step")..];
    for (i, label) in LABELS.iter().enumerate() {
        let row = format!("[x] {}. {}", i + 1, label.name);
        assert!(list.contains(&row), "{row:?} not listed:\n{list}");
    }
    let labels = labels_in(repo.path());
    // The Ticket's list: each label's kind and its skills' sources, which
    // the entry names as installed and the manifest holds pinned.
    let manifest = Manifest::load(repo.path()).unwrap();
    let shipped: [(&str, &str, &[&str]); 7] = [
        ("fe", "area", &["anthropics/skills/skills/frontend-design"]),
        (
            "be",
            "area",
            &["addyosmani/agent-skills/skills/api-and-interface-design"],
        ),
        (
            "db",
            "area",
            &[
                "supabase/agent-skills/skills/supabase-postgres-best-practices",
                "addyosmani/agent-skills/skills/deprecation-and-migration",
            ],
        ),
        (
            "security",
            "area",
            &["addyosmani/agent-skills/skills/security-and-hardening"],
        ),
        (
            "architecture",
            "area",
            &[
                "mattpocock/skills/skills/engineering/codebase-design",
                "mattpocock/skills/skills/engineering/domain-modeling",
            ],
        ),
        (
            "infra",
            "area",
            &[
                "hashicorp/agent-skills/plugins/terraform/skills/terraform-style-guide",
                "hashicorp/agent-skills/plugins/terraform/skills/terraform-test",
                "docker/skills/skills/docker-build-strategies",
                "lukasniessen/kubernetes-skill",
                "addyosmani/agent-skills/skills/ci-cd-and-automation",
                "github/awesome-copilot/skills/github-actions-hardening",
            ],
        ),
        ("codex-review", "modifier", &[]),
    ];
    for (name, kind, sources) in shipped {
        assert_eq!(labels[name].kind, kind, "{name}");
        let installed: Vec<String> = labels[name]
            .skills
            .iter()
            .map(|skill| {
                let skill = &manifest.skills[skill];
                assert_eq!(skill.commit, "abc123", "{name}");
                let repo = skill.repo.strip_prefix("https://github.com/").unwrap();
                format!("{repo}/{}", skill.path)
                    .trim_end_matches('/')
                    .to_string()
            })
            .collect();
        assert_eq!(installed, sources, "{name}");
    }
    assert_eq!(labels.len(), shipped.len());
    for (name, said) in [
        ("fe", "screenshots of every changed screen"),
        ("be", "status codes"),
        ("db", "rollback"),
        ("security", "Open question"),
        ("architecture", "ADR"),
        ("infra", "mock_provider"),
    ] {
        assert!(
            labels[name].guidance.contains(said),
            "{name}: {}",
            labels[name].guidance
        );
    }
    assert_eq!(labels["fe"].skills, ["orqa-frontend-design"]);
    assert_eq!(labels["db"].extra_review, app::ExtraReview::default());
    let security = &labels["security"].extra_review;
    assert_eq!(
        (
            security.skill.as_str(),
            security.position.as_str(),
            security.debate
        ),
        ("orqa-security-review", "every", true)
    );
    assert_eq!(
        manifest.skills["orqa-security-review"],
        Installed {
            repo: "https://github.com/getsentry/skills".to_string(),
            path: "skills/security-review".to_string(),
            commit: "abc123".to_string(),
            ..Installed::default()
        }
    );
    let infra = &labels["infra"].extra_review;
    assert_eq!(
        (infra.skill.as_str(), infra.position.as_str(), infra.debate),
        ("orqa-infra-review", "every", false)
    );
    let codex = &labels["codex-review"];
    assert_eq!(codex.rows["review"]["app"], "codex");
    assert!(codex.skills.is_empty() && codex.guidance.is_empty());
    for (name, _, _) in shipped.iter().filter(|(name, ..)| *name != "codex-review") {
        assert!(labels[*name].rows.is_empty(), "{name}");
    }
    for label in LABELS {
        for (name, _) in label.skills {
            assert!(
                out.contains(&format!("init: installed {name}, for orqa:{}", label.name)),
                "{out}"
            );
        }
    }
    assert!(manifest.skills["orqa-infra-review"].shipped);
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| call.contains("clone") && call.contains("infra-review")),
        "{:?}",
        tools.calls()
    );
}

/// Unchecking db (its digit toggles it) writes the six others and installs
/// no db skill.
#[test]
fn unchecking_db_writes_six_entries_and_installs_no_db_skill() {
    let (repo, home) = (bare_repo(), TempDir::new());
    let tools = ok_tools();
    // The docs/agents setup, TypeSafe no, then db off and enter.
    let keys = ["\n", "n\n", "3", "\r"];
    let (code, out) = init_with(repo.path(), home.path(), tools.clone(), &keys, "");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("[ ] 3. db"), "{out}");
    let labels = labels_in(repo.path());
    assert_eq!(labels.len(), 6, "{:?}", labels.keys());
    assert!(!labels.contains_key("db"));
    let manifest = Manifest::load(repo.path()).unwrap();
    for name in [
        "orqa-supabase-postgres-best-practices",
        "orqa-deprecation-and-migration",
    ] {
        assert!(
            !manifest.skills.contains_key(name),
            "{name} installed:\n{out}"
        );
    }
    assert!(
        manifest.skills.contains_key("orqa-frontend-design"),
        "{out}"
    );
    assert!(
        !tools.calls().iter().any(|call| call.contains("supabase")),
        "{:?}",
        tools.calls()
    );
    assert!(!repo.path().join(TEMPLATE_DIR).join("db.md").exists());
    assert!(repo.path().join(TEMPLATE_DIR).join("fe.md").exists());
}

/// A second init keeps an entry the user edited, and fetches nothing again.
#[test]
fn a_second_init_keeps_an_entry_the_user_edited() {
    let (repo, home) = (bare_repo(), TempDir::new());
    init_keys(repo.path(), home.path(), &[]);
    let config = repo.path().join(".orqadence/config.json");
    let mut doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
    doc["labels"]["fe"]["guidance"] = "ours".into();
    doc["labels"]["fe"]["skills"] = serde_json::json!([]);
    fs::write(&config, doc.to_string()).unwrap();
    let tools = ok_tools();
    // The gate: refresh; TypeSafe no; the labels all checked.
    let keys = ["2", "n\n", "\r"];
    let (code, out) = init_with(repo.path(), home.path(), tools.clone(), &keys, "");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(LABELS_STEP), "{out}");
    let labels = labels_in(repo.path());
    assert_eq!(labels.len(), 7);
    assert_eq!(labels["fe"].guidance, "ours");
    assert!(labels["fe"].skills.is_empty());
    assert!(
        !tools.calls().iter().any(|call| call.contains("clone")),
        "{:?}",
        tools.calls()
    );
}

/// A checkout whose settings are committed is not asked about the labels.
#[test]
fn a_committed_checkout_does_not_ask_the_labels() {
    let (repo, home) = (prepared_repo(), TempDir::new());
    write_file(
        &repo.path().join(".orqadence/config.json"),
        r#"{"typesafe": false}"#,
    );
    let (_, out) = init_with(
        repo.path(),
        home.path(),
        committed_tools(),
        &["\n", "\r"],
        "",
    );
    assert!(!out.contains(LABELS_STEP), "{out}");
    assert!(labels_in(repo.path()).is_empty());
}

/// A label skill whose fetch fails leaves init unready: the preflight names
/// the missing Extra review skill, as it does a label's own skill.
#[test]
fn a_failed_label_skill_fetch_fails_the_preflight() {
    let repo = bare_repo();
    let tools = Fake::new(|dir, argv| {
        if argv.contains(&"clone") && argv.iter().any(|arg| arg.contains("getsentry")) {
            return Err("clone failed".to_string());
        }
        ok(dir, argv)
    });
    let (code, out) = run_with(&["init"], repo.path(), tools, &herdr_env);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("init: orqa-security-review not installed"),
        "{out}"
    );
    assert!(
        out.contains("preflight: orqa:security's skill orqa-security-review is missing"),
        "{out}"
    );
    assert!(!out.contains("ready"), "{out}");
}

/// The checklist draws with auto-wrap off, so a row wider than the terminal
/// stays one row, and turns it back on when it folds, even unanswered.
#[test]
fn the_labels_checklist_turns_auto_wrap_off_and_back_on() {
    let repo = bare_repo();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "{out}");
    let list = &out[out.find(LABELS_STEP).expect("no labels step")..];
    let (off, row, on) = (
        list.find("\x1b[?7l").unwrap(),
        list.find("[x] 1. fe").unwrap(),
        list.find("\x1b[?7h").unwrap(),
    );
    assert!(off < row && row < on, "{list:?}");
    assert!(list.contains("✓ fe, be, db"), "{list:?}");
}

/// The six Area labels and each one's section heading.
const AREA_SECTIONS: [(&str, &str); 6] = [
    ("fe", "## Screenshots"),
    ("be", "## Contract"),
    ("db", "## Schema"),
    ("security", "## Threat note"),
    ("architecture", "## Structure"),
    ("infra", "## Infra"),
];

/// An empty repo gets the generic default, create-pr's five sections with
/// a comment each, and one file per checked Area label: the default plus
/// the label's section. codex-review gets none. Each Area entry's
/// pr_template names its file.
#[test]
fn init_writes_the_default_template_and_one_per_checked_area_label() {
    let repo = bare_repo();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "{out}");
    let default = read(repo.path(), DEFAULT_TEMPLATE);
    assert_eq!(default, PR_TEMPLATE);
    let labels = labels_in(repo.path());
    for (name, heading) in AREA_SECTIONS {
        let path = format!("{TEMPLATE_DIR}/{name}.md");
        let text = read(repo.path(), &path);
        assert!(text.starts_with(&default), "{path}:\n{text}");
        let section = &text[default.len()..];
        assert!(section.trim_start().starts_with(heading), "{path}:\n{text}");
        assert_eq!(section.matches("<!--").count(), 1, "{path}:\n{text}");
        assert_eq!(labels[name].pr_template, format!("{name}.md"), "{name}");
        assert!(out.contains(&format!("init: wrote {path}")), "{out}");
    }
    assert!(!repo
        .path()
        .join(TEMPLATE_DIR)
        .join("codex-review.md")
        .exists());
    assert_eq!(labels["codex-review"].pr_template, "");
    assert!(out.contains("PR TEMPLATES"), "{out}");
}

/// init over a repo with a default template of its own, answered `answer`
/// to the PR TEMPLATES question: the docs/agents setup, TypeSafe no, the
/// labels all checked, then the answer.
fn init_own_default(repo: &Path, answer: &str) -> String {
    let home = TempDir::new();
    let keys = ["\n", "n\n", "\r", answer];
    let (code, out) = init_with(repo, home.path(), ok_tools(), &keys, "");
    assert_eq!(code, 0, "{out}");
    out
}

/// A repo's own default template, answered yes, frames the label
/// templates and stays where it is.
#[test]
fn a_repos_own_default_template_answered_yes_frames_the_label_templates() {
    let repo = bare_repo();
    write_file(&repo.path().join(DEFAULT_TEMPLATE), "ours\n");
    let out = init_own_default(repo.path(), "y\n");
    assert!(
        out.contains("Make .github/pull_request_template.md the default"),
        "{out}"
    );
    assert_eq!(read(repo.path(), DEFAULT_TEMPLATE), "ours\n");
    let labels = labels_in(repo.path());
    for (name, heading) in AREA_SECTIONS {
        let text = read(repo.path(), &format!("{TEMPLATE_DIR}/{name}.md"));
        assert_eq!(text.lines().next(), Some("ours"), "{name}:\n{text}");
        assert!(text.contains(heading), "{name}:\n{text}");
        assert_eq!(labels[name].pr_template, format!("{name}.md"), "{name}");
    }
}

/// Answered no, nothing is written and every label uses the repo's
/// template: pr_template stays empty. A default at the root is found too.
#[test]
fn a_repos_own_default_template_answered_no_writes_nothing() {
    let repo = bare_repo();
    write_file(&repo.path().join("PULL_REQUEST_TEMPLATE.md"), "ours\n");
    let out = init_own_default(repo.path(), "n\n");
    assert!(
        out.contains("Make PULL_REQUEST_TEMPLATE.md the default"),
        "{out}"
    );
    assert!(!repo.path().join(TEMPLATE_DIR).exists());
    assert!(!repo.path().join(DEFAULT_TEMPLATE).exists());
    let labels = labels_in(repo.path());
    for (name, _) in AREA_SECTIONS {
        assert_eq!(labels[name].pr_template, "", "{name}");
    }
}

/// With only a template directory, init asks which of its files is the
/// default: the one picked is copied to the default's path and frames the
/// label templates; the directory's own files are untouched.
#[test]
fn a_template_directory_alone_asks_which_is_the_default() {
    let repo = bare_repo();
    let dir = repo.path().join(TEMPLATE_DIR);
    write_file(&dir.join("bug.md"), "bug\n");
    write_file(&dir.join("feature.md"), "feature\n");
    let out = init_own_default(repo.path(), "2");
    assert!(
        out.contains("Which of these is the default template?"),
        "{out}"
    );
    for option in ["1. bug.md", "2. feature.md", "3. none"] {
        assert!(out.contains(option), "{option}:\n{out}");
    }
    assert_eq!(read(repo.path(), DEFAULT_TEMPLATE), "feature\n");
    assert_eq!(
        read(repo.path(), &format!("{TEMPLATE_DIR}/bug.md")),
        "bug\n"
    );
    assert_eq!(
        read(repo.path(), &format!("{TEMPLATE_DIR}/feature.md")),
        "feature\n"
    );
    let labels = labels_in(repo.path());
    for (name, heading) in AREA_SECTIONS {
        let text = read(repo.path(), &format!("{TEMPLATE_DIR}/{name}.md"));
        assert!(
            text.starts_with("feature\n") && text.contains(heading),
            "{name}:\n{text}"
        );
        assert_eq!(labels[name].pr_template, format!("{name}.md"), "{name}");
    }
    assert!(
        out.contains(&format!("init: wrote {DEFAULT_TEMPLATE}")),
        "{out}"
    );
}

/// none picked writes the generic default, as an empty repo gets.
#[test]
fn a_template_directory_alone_answered_none_gets_the_generic_default() {
    let repo = bare_repo();
    write_file(&repo.path().join(TEMPLATE_DIR).join("bug.md"), "bug\n");
    init_own_default(repo.path(), "3");
    let default = read(repo.path(), DEFAULT_TEMPLATE);
    assert!(default.starts_with("## What\n"), "{default}");
    let fe = read(repo.path(), &format!("{TEMPLATE_DIR}/fe.md"));
    assert!(
        fe.starts_with(&default) && fe.contains("## Screenshots"),
        "{fe}"
    );
}

/// An existing <name>.md in the directory is kept and mapped, and offered
/// as the default; nobody answering picks none, the generic one. The rest
/// are built.
#[test]
fn an_existing_label_template_is_kept_and_mapped() {
    let repo = bare_repo();
    write_file(&repo.path().join(TEMPLATE_DIR).join("fe.md"), "mine\n");
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Which of these"), "{out}");
    assert_eq!(read(repo.path(), DEFAULT_TEMPLATE), PR_TEMPLATE);
    assert_eq!(
        read(repo.path(), &format!("{TEMPLATE_DIR}/fe.md")),
        "mine\n"
    );
    let labels = labels_in(repo.path());
    assert_eq!(labels["fe"].pr_template, "fe.md");
    for (name, heading) in AREA_SECTIONS.iter().filter(|(name, _)| *name != "fe") {
        let text = read(repo.path(), &format!("{TEMPLATE_DIR}/{name}.md"));
        assert!(text.contains(heading), "{name}:\n{text}");
        assert_eq!(labels[*name].pr_template, format!("{name}.md"), "{name}");
    }
}

/// A second init overwrites no template and asks nothing about them.
#[test]
fn a_second_init_overwrites_no_template() {
    let (repo, home) = (bare_repo(), TempDir::new());
    init_keys(repo.path(), home.path(), &[]);
    let github = repo.path().join(".github");
    write_file(&github.join("pull_request_template.md"), "edited\n");
    write_file(&github.join("PULL_REQUEST_TEMPLATE/fe.md"), "edited fe\n");
    let before = snapshot(&github);
    // The gate: refresh; TypeSafe no; the labels all checked.
    let (code, out) = init_keys(repo.path(), home.path(), &["2", "n\n", "\r"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("PR TEMPLATES"), "{out}");
    assert_eq!(snapshot(&github), before);
    assert_eq!(labels_in(repo.path())["fe"].pr_template, "fe.md");
}
