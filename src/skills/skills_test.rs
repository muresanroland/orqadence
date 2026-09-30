use super::SKILLS;
use crate::orchestrator::write_file;
use crate::tempdir::TempDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A shipped skill's text by its name.
fn skill(name: &str) -> &'static str {
    SKILLS.iter().find(|(n, _)| *n == name).unwrap().1
}

/// infra-review names the Inputs and the section the Review Stage and the
/// Orchestrator rely on.
#[test]
fn infra_review_is_shipped_with_its_not_run_section() {
    let infra = skill("orqa-infra-review");
    for text in ["name: orqa-infra-review", "## Not run", "**Cache**"] {
        assert!(infra.contains(text), "infra-review lacks {text:?}");
    }
}

/// infra-review's fetch.sh, as it is shipped.
fn fetch_sh() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/orqa-infra-review/fetch.sh")
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
/// call (the tool, its cwd and its arguments); its terraform writes a module
/// manifest holding the folder init ran in. Returns the worktree, the cache
/// and the log. The real bash and git run: fetch.sh is outside the Tools seam.
fn fetch(files: &[(&str, &str)]) -> (TempDir, TempDir, String) {
    let (repo, cache, log, out) = run_fetch(files, "", 0);
    assert!(
        out.status.success(),
        "fetch.sh: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (repo, cache, log)
}

/// fetch.sh run as in `fetch`, with each stub tool printing `stub_out` and
/// exiting with `stub_status`; returns fetch.sh's output too.
fn run_fetch(
    files: &[(&str, &str)],
    stub_out: &str,
    stub_status: i32,
) -> (TempDir, TempDir, String, std::process::Output) {
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
            "#!/bin/sh\necho \"$(basename \"$0\") $(pwd -P) $*\" >> \"$STUB_LOG\"\n\
             if [ \"$(basename \"$0\") $1\" = 'terraform init' ]; then \
             mkdir -p .terraform/modules && pwd -P > .terraform/modules/modules.json; fi\n\
             printf '%s' \"$STUB_OUT\"\nexit \"$STUB_STATUS\"\n",
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
        .env("STUB_OUT", stub_out)
        .env("STUB_STATUS", stub_status.to_string())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GITHUB_TOKEN")
        .output()
        .unwrap();
    let log = fs::read_to_string(&log).unwrap_or_default();
    (repo, cache, log, out)
}

/// A touched Terraform root is initialised in a temp copy, never in the
/// worktree, and the plugins its .tflint.hcl names are fetched with gh's
/// token; the worktree is left clean and the cache's lock released.
#[test]
fn fetch_inits_a_touched_terraform_root_in_a_temp_copy() {
    let (repo, cache, log) = fetch(&[
        ("infra/main.tf", "resource \"null_resource\" \"a\" {}\n"),
        (
            ".tflint.hcl",
            "plugin \"aws\" {\n  enabled = true\n  source  = \"github.com/terraform-linters/tflint-ruleset-aws\"\n}\n",
        ),
    ]);
    // the stubs log the cwd with links resolved (/var is /private/var on macOS)
    let worktree = fs::canonicalize(repo.path()).unwrap();
    let worktree = worktree.to_str().unwrap();
    let init = log
        .lines()
        .find(|line| line.starts_with("terraform ") && line.contains(" init -backend=false"))
        .unwrap_or_else(|| panic!("no terraform init: {log}"));
    assert!(init.split(' ').nth(1).unwrap().ends_with("/infra"), "{log}");
    let tflint = log
        .lines()
        .find(|line| line.starts_with("tflint ") && line.ends_with(" --init"))
        .unwrap_or_else(|| panic!("no tflint --init: {log}"));
    assert!(
        log.lines()
            .any(|line| line.starts_with("gh ") && line.ends_with(" auth token")),
        "{log}"
    );
    for call in [init, tflint] {
        let cwd = call.split(' ').nth(1).unwrap();
        assert!(!cwd.starts_with(worktree), "ran in the worktree: {call}");
    }
    assert_eq!(git(repo.path(), &["status", "--porcelain"]), "");
    assert!(
        !cache.path().join("providers.lock").exists(),
        "lock left behind"
    );
}

/// A diff that touches nothing fetch.sh fetches for calls no tool.
#[test]
fn fetch_calls_no_tool_for_a_readme() {
    let (_, _, log) = fetch(&[("docs/README.md", "how\n")]);
    assert_eq!(log, "");
}

/// The modules init installed in a touched root's temp copy (registry and
/// git modules, which the offline review cannot fetch) are kept in the
/// cache by commit: modules/<HEAD>/<root>/.terraform/modules.
#[test]
fn fetch_keeps_a_roots_modules_in_the_cache() {
    let (repo, cache, log) = fetch(&[("infra/main.tf", "module \"m\" { source = \"x/y/z\" }\n")]);
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let manifest = cache.path().join(format!(
        "modules/{}/infra/.terraform/modules/modules.json",
        head.trim()
    ));
    assert!(manifest.exists(), "no {}\n{log}", manifest.display());
}

/// A manifest fetch.sh validates with kubeconform, which prints `out` and
/// exits with `status`; returns whether fetch.sh succeeded, and its stderr.
fn fetch_kube(out: &str, status: i32) -> (bool, String) {
    let (_, _, log, run) = run_fetch(
        &[("k8s/app.yaml", "kind: Deployment\nmetadata:\n  name: web\n")],
        out,
        status,
    );
    assert!(log.starts_with("kubeconform "), "{log}");
    (
        run.status.success(),
        String::from_utf8_lossy(&run.stderr).into_owned(),
    )
}

/// A schema that fails to download, whatever the HTTP status, or to reach
/// the cache, and a kubeconform that stops before any resource, fail the
/// fetch and say why on stderr.
#[test]
fn fetch_fails_when_kubeconform_cannot_fetch_a_schema() {
    for out in [
        "k8s/app.yaml - Deployment web failed validation: failed downloading schema at https://x: dial tcp: no such host\n",
        "k8s/app.yaml - Deployment web failed validation: error while downloading schema at https://x - received HTTP status 403\n",
        "k8s/app.yaml - Deployment web failed validation: failed to write cache to disk: read-only file system\n",
        "failed opening cache folder /c: permission denied\n",
    ] {
        let (ok, stderr) = fetch_kube(out, 1);
        assert!(!ok, "fetch.sh passed on {out:?}");
        assert!(stderr.contains(out.trim_end()), "{stderr}");
    }
}

/// An invalid manifest, or a kind with no schema (a CRD, skipped), is the
/// review's to report: the fetch still succeeds.
#[test]
fn fetch_leaves_invalid_manifests_and_missing_schemas_to_the_review() {
    for (out, status) in [
        (
            "k8s/app.yaml - Deployment web is invalid: problem validating schema\n",
            1,
        ),
        ("", 0),
    ] {
        let (ok, stderr) = fetch_kube(out, status);
        assert!(ok, "fetch.sh failed on {out:?}: {stderr}");
    }
}

/// stage-moderate takes the Extra review's Findings from its second Input,
/// and stage-fix knows the not-debated fix items and the Unreviewed wording.
#[test]
fn the_stage_skills_know_the_extra_review() {
    assert!(skill("orqa-stage-moderate").contains(
        "Take every Finding from the **Review file**, and from the **Extra review file** when Inputs carry one"
    ));
    let fix = skill("orqa-stage-fix");
    assert!(fix.contains("| not debated | extra review`"));
    assert!(fix.contains("`the extra review skipped too`"));
    assert!(skill("orqa-stage-review").contains("`extra-review-*.md`"));
}

/// create-pr fills a template it is given before any it finds itself, and
/// replaces each section's comment with content.
#[test]
fn create_pr_prefers_a_given_template() {
    let create_pr = skill("orqa-create-pr");
    assert!(create_pr.contains("A template you are given comes before any you find yourself"));
    assert!(create_pr.contains("replace each section's HTML comment"));
}

/// create-pr passes the files it is told to attach on its gh pr create call.
#[test]
fn create_pr_passes_attachments() {
    assert!(skill("orqa-create-pr").contains("`--attach <file>`"));
}

/// stage-fix takes screenshots when the PR template has a Screenshots
/// section, before create-pr, into the Run directory's pr/ folder, and hands
/// them to --attach; they are never committed.
#[test]
fn stage_fix_takes_screenshots_for_a_screenshots_section() {
    let fix = skill("orqa-stage-fix");
    for text in [
        "has a `## Screenshots` section",
        "`<Run directory>/pr/`",
        "`--attach`",
        "never committed",
        "2.99",
    ] {
        assert!(fix.contains(text), "stage-fix lacks {text:?}");
    }
    let at = |part: &str| fix.find(part).unwrap_or_else(|| panic!("no {part:?}"));
    assert!(at("**Screenshots**") < at("Run the orqa-create-pr skill"));
}

/// stage-fix fills the PR template, then puts Orqadence's run parts under
/// one heading after the template's own, in order; Unreviewed stays first.
#[test]
fn stage_fix_names_the_orqadence_run_heading_and_its_parts_in_order() {
    let fix = skill("orqa-stage-fix");
    assert!(fix.contains("**PR template**"));
    assert!(fix.contains("**Extra review files**"));
    let at = |part: &str| fix.find(part).unwrap_or_else(|| panic!("no {part:?}"));
    let parts = [
        "**Unreviewed**",
        "`## Orqadence run`",
        "**Verdict history**",
        "**Leftovers never re-checked**",
        "**Extra review skipped**",
        "**Extra review Findings still open at the cap**",
        "`## Not run`",
    ];
    for pair in parts.windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{} before {}", pair[0], pair[1]);
    }
}
