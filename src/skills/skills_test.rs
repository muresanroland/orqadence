use super::SKILLS;
use crate::orchestrator::write_file;
use crate::tempdir::TempDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A Shipped skill's text by name.
fn shipped(name: &str) -> &'static str {
    SKILLS
        .iter()
        .find(|(skill, _)| *skill == name)
        .unwrap_or_else(|| panic!("{name} is not shipped"))
        .1
}

/// infra-review names every check it runs, how each result maps to a
/// severity, the Not run section and the Inputs fetch.sh leaves it.
#[test]
fn infra_review_is_shipped_and_names_its_checks() {
    let skill = shipped("orqa-infra-review");
    assert!(skill.contains("name: orqa-infra-review"), "{skill}");
    for text in [
        // the checks
        "terraform fmt",
        "terraform validate",
        "terraform test",
        "mock_provider",
        "tflint",
        "trivy config --skip-check-update",
        "hadolint",
        "helm lint",
        "helm template",
        "kubeconform",
        "actionlint",
        "shellcheck",
        // offline, from the cache
        "-backend=false",
        "-plugin-dir",
        "TFLINT_PLUGIN_DIR",
        "-ignore-missing-schemas",
        // the severities
        "CRITICAL",
        "UNKNOWN",
        "style",
        "notice",
        "[expression]",
        "[syntax-check]",
        "[shellcheck]",
        "[ERROR]",
        "[WARNING]",
        "run terraform fmt",
        ".tftest.hcl",
        // what did not run, and why
        "## Not run",
        "**Fetch**",
        "**Cache**",
    ] {
        assert!(skill.contains(text), "infra-review lacks {text:?}");
    }
    assert!(!skill.contains("checkov"), "infra-review runs checkov");
}

/// infra-review's fetch.sh, as it is shipped.
fn fetch_sh() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/orqa-infra-review/fetch.sh")
}

#[test]
fn fetch_sh_parses() {
    let out = Command::new("bash")
        .arg("-n")
        .arg(fetch_sh())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// git in `repo`, cut off from the user's own config.
fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// A worktree whose branch t, off main, commits `files`; fetch.sh run in it
/// as the Orchestrator runs it, with stub tools first on PATH that log each
/// call (the tool, its cwd and its arguments). Returns the worktree and the
/// log. The real bash and git run: fetch.sh is outside the Tools seam.
fn fetch(files: &[(&str, &str)]) -> (TempDir, String) {
    let (repo, cache, stubs) = (TempDir::new(), TempDir::new(), TempDir::new());
    git(repo.path(), &["init", "-q", "-b", "main"]);
    write_file(&repo.path().join("README.md"), "infra\n");
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "base"]);
    git(repo.path(), &["checkout", "-qb", "t"]);
    for (path, text) in files {
        write_file(&repo.path().join(path), text);
    }
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "change"]);

    let log = stubs.path().join("log");
    for tool in ["terraform", "tflint", "gh", "kubeconform", "helm"] {
        let stub = stubs.path().join(tool);
        write_file(
            &stub,
            "#!/bin/sh\necho \"$(basename \"$0\") $PWD $*\" >> \"$STUB_LOG\"\n",
        );
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        stubs.path().display(),
        std::env::var("PATH").unwrap()
    );
    let out = Command::new("bash")
        .arg(fetch_sh())
        .current_dir(repo.path())
        .env("PATH", path)
        .env("STUB_LOG", &log)
        .env("ORQA_CACHE", cache.path())
        .env("ORQA_BASE", "main")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GITHUB_TOKEN")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fetch.sh: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (repo, fs::read_to_string(&log).unwrap_or_default())
}

/// A touched Terraform root is initialised in a temp copy, never in the
/// worktree, and the plugins its .tflint.hcl names are fetched with gh's
/// token; the worktree is left clean.
#[test]
fn fetch_inits_a_touched_terraform_root_in_a_temp_copy() {
    let (repo, log) = fetch(&[
        ("infra/main.tf", "resource \"null_resource\" \"a\" {}\n"),
        (
            ".tflint.hcl",
            "plugin \"aws\" {\n  enabled = true\n  source  = \"github.com/terraform-linters/tflint-ruleset-aws\"\n}\n",
        ),
    ]);
    let calls: Vec<(&str, &str, &str)> = log
        .lines()
        .map(|line| {
            let mut words = line.splitn(3, ' ');
            let tool = words.next().unwrap();
            (tool, words.next().unwrap(), words.next().unwrap_or(""))
        })
        .collect();
    let worktree = repo.path().to_str().unwrap();
    let init = calls
        .iter()
        .find(|(tool, _, args)| *tool == "terraform" && args.starts_with("init -backend=false"))
        .unwrap_or_else(|| panic!("no terraform init: {log}"));
    assert!(init.1.ends_with("/infra"), "{log}");
    let tflint = calls
        .iter()
        .find(|(tool, _, args)| *tool == "tflint" && *args == "--init")
        .unwrap_or_else(|| panic!("no tflint --init: {log}"));
    assert!(
        calls
            .iter()
            .any(|call| call.0 == "gh" && call.2 == "auth token"),
        "{log}"
    );
    for (tool, cwd, _) in [init, tflint] {
        assert!(
            !cwd.starts_with(worktree),
            "{tool} ran in the worktree: {log}"
        );
    }
    assert_eq!(git(repo.path(), &["status", "--porcelain"]), "");
}

/// A diff that touches nothing fetch.sh fetches for calls no tool.
#[test]
fn fetch_calls_no_tool_for_a_readme() {
    let (_, log) = fetch(&[("docs/README.md", "how\n")]);
    assert_eq!(log, "");
}
