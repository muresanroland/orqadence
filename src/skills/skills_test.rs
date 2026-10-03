use super::manifest::Manifest;
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

/// stage-release names the Inputs it reads, the VERSION: line of its
/// result, the repo that keeps its version only in tags, and never merges.
#[test]
fn stage_release_names_its_inputs_the_version_line_and_the_tags_only_case() {
    let release = skill("orqa-stage-release");
    for text in [
        "**Bump**",
        "**Tickets**",
        "**Result file**",
        "VERSION: v",
        "only in tags",
        "never merges",
    ] {
        assert!(release.contains(text), "stage-release lacks {text:?}");
    }
}

/// Address PR comments never rebases.
#[test]
fn stage_address_pr_comments_holds_no_rebase() {
    let comments = skill("orqa-stage-address-pr-comments");
    assert!(comments.contains("gh run view") && comments.contains("--log-failed"));
    assert!(!comments.to_lowercase().contains("rebase"));
    assert!(!comments.contains("{{merge-conflicts}}"));
}

/// address-pr-comments ends its replies with the marker threads.sh:28 and
/// pr.rs:346 match exactly.
#[test]
fn address_pr_comments_writes_the_reply_marker() {
    assert!(skill("orqa-address-pr-comments").contains("<!-- address-pr-comments -->"));
}

/// A PR comment asking for work another open Ticket does is answered as
/// covered by it, with the marker, and nothing is changed: the Stage hands
/// the Tickets on and says it for its own steps, the shipped skill for its.
#[test]
fn a_pr_comment_another_open_ticket_covers_is_answered_not_fixed() {
    let stage = skill("orqa-stage-address-pr-comments");
    let shipped = skill("orqa-address-pr-comments");
    for text in [
        "the **Other open Tickets**, and each failing check",
        "`Covered by <id>: <title>. <!-- address-pr-comments -->`",
        "resolveReviewThread",
        "Never delete or revert what an open Ticket there depends on",
    ] {
        assert!(
            stage.contains(text),
            "stage-address-pr-comments lacks {text:?}"
        );
    }
    for text in [
        "**other open Tickets**",
        "| covered by another open Ticket | `Covered by <id>: <title>.` | yes |",
        "Never delete or revert what one of these open Tickets depends on",
    ] {
        assert!(shipped.contains(text), "address-pr-comments lacks {text:?}");
    }
}

/// threads.sh leaves out PR #78's two threads answered Covered by
/// harness-we9.3, resolved or not, as pr.rs's items do.
#[test]
fn threads_sh_leaves_out_a_thread_answered_as_covered() {
    let mut pr: serde_json::Value =
        serde_json::from_str(include_str!("../orchestrator/testdata/pr/78.json")).unwrap();
    for thread in pr["reviewThreads"]["nodes"].as_array_mut().unwrap() {
        thread["isResolved"] = serde_json::json!(false);
    }
    let reply = serde_json::json!({"data": {"repository": {"pullRequest": pr}}});
    let Some(found) = threads(&reply) else {
        return;
    };
    assert_eq!(found, serde_json::json!([]));
}

/// threads.sh run with a stub gh first on PATH: gh's --jq filter applied
/// by jq to `reply`, a GraphQL reply. The real bash runs.
fn threads(reply: &serde_json::Value) -> Option<serde_json::Value> {
    for tool in ["bash", "jq"] {
        if Command::new(tool).arg("--version").output().is_err() {
            eprintln!("skipped: threads.sh's test needs {tool}");
            return None;
        }
    }
    let stubs = TempDir::new();
    let fixture = stubs.path().join("reply.json");
    write_file(&fixture, &reply.to_string());
    let gh = stubs.path().join("gh");
    write_file(
        &gh,
        "#!/bin/sh\nwhile [ $# -gt 0 ] && [ \"$1\" != --jq ]; do shift; done\n\
         exec jq \"$2\" \"$FIXTURE\"\n",
    );
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        stubs.path().display(),
        std::env::var("PATH").unwrap()
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("skills/orqa-address-pr-comments/scripts/threads.sh");
    let out = Command::new("bash")
        .arg(script)
        .arg("7")
        .current_dir(stubs.path())
        .env("PATH", path)
        .env("FIXTURE", &fixture)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "threads.sh: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(serde_json::from_slice(&out.stdout).unwrap())
}

/// CodeRabbit's review body yields only the entries with a severity header,
/// not its 🔇 LGTM ones, and its logins count with [bot] or without. A
/// deleted account's review, its author null, is no bot's.
#[test]
fn threads_sh_keeps_coderabbits_rated_entries_and_its_thread() {
    let bot = serde_json::json!({"login": "coderabbitai[bot]", "__typename": "Bot"});
    let body = "**Actionable comments posted: 0**\n\n<details>\n\
        <summary>🧹 Nitpick comments (1)</summary><blockquote>\n\n<details>\n\
        <summary>src/a.rs (1)</summary><blockquote>\n\n\
        `10-12`: _🎯 Functional Correctness_ | _🟠 Major_ | _⚡ Quick win_\n\n\
        **Guard the empty list.**\n\nThe loop indexes [0] unchecked.\n\n\
        <!-- cr-comment:v1:abc123 -->\n\n</blockquote></details>\n\n</blockquote></details>\n\
        <details>\n<summary>🔇 Additional comments (1)</summary><blockquote>\n\n<details>\n\
        <summary>src/b.rs (1)</summary><blockquote>\n\n`3-3`: LGTM!\n\n\
        <!-- cr-comment:v1:def456 -->\n\n</blockquote></details>\n\n</blockquote></details>";
    let reply = serde_json::json!({"data": {"repository": {"pullRequest": {
        "reviewThreads": {"nodes": [{
            "id": "T1", "isResolved": false, "isOutdated": false, "path": "src/a.rs", "line": 5,
            "comments": {"nodes": [{"databaseId": 11, "author": bot,
                "body": "_🎯 Functional Correctness_ | _🟡 Minor_ | _⚡ Quick win_\n\n**Rename x.**"}]},
        }]},
        "reviews": {"nodes": [
            {"databaseId": 21, "url": "https://x/r21", "author": bot, "body": body},
            {"databaseId": 22, "url": "https://x/r22", "author": null, "body": ""},
        ]},
        "comments": {"nodes": []},
    }}}});
    let Some(found) = threads(&reply) else {
        return;
    };
    let got: Vec<(&str, &str)> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["kind"].as_str().unwrap(), c["title"].as_str().unwrap()))
        .collect();
    assert_eq!(
        got,
        [
            ("thread", "Rename x."),
            ("outside", "Guard the empty list.")
        ]
    );
    assert_eq!(found[0]["thread_id"], "T1");
    assert_eq!(found[1]["path"], "src/a.rs");
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
        "skip `gh pr edit --attach`",
        "not retried if it fails",
    ] {
        assert!(fix.contains(text), "stage-fix lacks {text:?}");
    }
    let at = |part: &str| fix.find(part).unwrap_or_else(|| panic!("no {part:?}"));
    assert!(at("**Screenshots**") < at("Run the orqa-create-pr skill"));
    // The "not attached and why" note follows the --attach recheck, so a
    // retry that succeeds cannot leave it false; create-pr writes none.
    assert!(at("`gh pr edit --attach`") < at("names each file still missing"));
    assert!(!skill("orqa-create-pr").contains("did not attach"));
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
        "**Manual work**: each open item from the **Manual work** Input",
    ];
    for pair in parts.windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{} before {}", pair[0], pair[1]);
    }
}

/// stage-address-pr-comments, once it has pushed, captures an orqa:fe
/// Ticket's changed screens again into the Run directory's pr/ and attaches
/// them, or says why not, and rewrites each shipped label section of the PR
/// body its changes made stale, leaving the rest as it is.
#[test]
fn stage_address_pr_comments_recaptures_screenshots_and_refreshes_label_sections() {
    let stage = skill("orqa-stage-address-pr-comments");
    for text in ["`orqa:fe`", "`<Run directory>/pr/`", "**PR template**"] {
        assert!(
            stage.contains(text),
            "stage-address-pr-comments lacks {text:?}"
        );
    }
    for label in crate::setup::LABELS
        .iter()
        .filter(|l| !l.pr_section.is_empty())
    {
        let heading = format!("`{}`", label.pr_section.lines().next().unwrap());
        assert!(
            stage.contains(&heading),
            "stage-address-pr-comments lacks {heading}"
        );
    }
}

/// The Orchestrator merges, never a session (ADR 0007): the skills whose
/// sessions push to a PR say so, and no shipped skill names `gh pr merge`
/// except to forbid it.
#[test]
fn no_skill_merges_the_orchestrator_does() {
    for name in [
        "orqa-create-pr",
        "orqa-stage-fix",
        "orqa-stage-rebase",
        "orqa-stage-address-pr-comments",
    ] {
        assert!(
            skill(name).contains("The Orchestrator merges, never a session"),
            "{name}"
        );
    }
    for (name, text) in SKILLS {
        for line in text.lines().filter(|line| line.contains("gh pr merge")) {
            assert!(line.contains("No `gh pr merge`"), "{name}: {line}");
        }
    }
}

/// manual-work names the folder a session files, its first line and
/// headings, the forms it takes, the blocking result and every command a
/// session never runs.
#[test]
fn manual_work_names_its_folder_status_and_never_run_list() {
    let manual = skill("orqa-manual-work");
    for text in [
        "name: orqa-manual-work",
        "<Run directory>/manual-work/<n>/",
        "`manual-work.md`",
        "Ticket: <id> · Stage: <stage> · Blocks: yes|no",
        "## What",
        "## Why",
        "## How",
        "## Report back",
        "`wizard.sh`",
        "`prompt.md`",
        "`STATUS: manual` as its first line and the folder path as its second",
        "Manual work <n> done: <facts>",
        "`gh secret`",
        "`gh variable`",
        "`gh workflow run`",
        "`gh api` writes to repo settings",
        "cloud CLIs acting on real accounts",
        "`terraform plan`",
        "`terraform apply`",
        "`terraform init`",
        "mattpocock/skills",
    ] {
        assert!(manual.contains(text), "manual-work lacks {text:?}");
    }
}

/// manual-work's template.sh ships as vendored, with the MIT notice of
/// mattpocock/skills, and still parses.
#[test]
fn manual_work_template_is_vendored_with_its_licence() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/orqa-manual-work/template.sh");
    let shipped = fs::read_to_string(&path).unwrap();
    assert!(shipped.starts_with("#!/usr/bin/env bash\n"));
    for text in [
        "github.com/mattpocock/skills",
        "MIT License",
        "Copyright (c) 2026 Matt Pocock",
        "The above copyright notice and this permission notice shall be included",
        "# STAGES: author this section.",
    ] {
        assert!(shipped.contains(text), "template.sh lacks {text:?}");
    }
    let out = Command::new("bash").arg("-n").arg(&path).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The code-editing Stages load manual-work by name and know STATUS: manual
/// and the prompt that answers it; the Review and the Debate never file
/// Manual work.
#[test]
fn the_code_editing_stages_load_manual_work_and_the_review_and_debate_do_not() {
    for name in [
        "orqa-stage-implement",
        "orqa-stage-fix",
        "orqa-stage-rebase",
        "orqa-stage-address-pr-comments",
    ] {
        for text in [
            "load the orqa-manual-work skill by name",
            "`gh secret`",
            "`STATUS: manual` as its first line and the item's folder path as its second",
            "`Manual work <n> done: <facts>`",
        ] {
            assert!(skill(name).contains(text), "{name} lacks {text:?}");
        }
    }
    for name in [
        "orqa-stage-fix",
        "orqa-stage-rebase",
        "orqa-stage-address-pr-comments",
    ] {
        assert!(
            skill(name).contains("never file one of them again"),
            "{name}"
        );
    }
    for name in ["orqa-stage-review", "orqa-stage-moderate"] {
        for text in ["manual-work", "STATUS: manual"] {
            assert!(!skill(name).contains(text), "{name} names {text:?}");
        }
    }
}

/// The two skills every Brainstorm skill loads ship, ported from
/// mattpocock/skills: grilling asks one question at a time through the
/// App's question tool, never in rounds; domain modeling writes CONTEXT.md
/// and docs/adr/ and commits only its own paths.
#[test]
fn the_brainstorm_grilling_and_domain_modeling_skills_ship_with_their_rules() {
    let grilling = skill("orqa-brainstorm-grilling");
    for text in [
        "name: orqa-brainstorm-grilling",
        "one question at a time",
        "your App's own question tool (AskUserQuestion on claude)",
        "`(Recommended)`",
        "Never ask in rounds",
    ] {
        assert!(
            grilling.contains(text),
            "brainstorm-grilling lacks {text:?}"
        );
    }
    let domain = skill("orqa-brainstorm-domain-modeling");
    for text in [
        "name: orqa-brainstorm-domain-modeling",
        "`CONTEXT.md`",
        "`docs/adr/`",
        "git commit -m '<message>' -- <paths>",
        "never `git add -A`",
    ] {
        assert!(
            domain.contains(text),
            "brainstorm-domain-modeling lacks {text:?}"
        );
    }
}

/// brainstorm-chart and brainstorm-epic name their Inputs, the brainstorm:*
/// labels and the keys of their result files, and commit only their own
/// paths.
#[test]
fn the_brainstorm_chart_and_epic_skills_name_their_inputs_labels_and_results() {
    let chart = skill("orqa-brainstorm-chart");
    for text in [
        "name: orqa-brainstorm-chart",
        "**IDEA**",
        "**TICKET LABELS**",
        "**RESULT FILE**",
        "--no-inherit-labels",
        "brainstorm:map",
        "brainstorm:epic",
        "STATUS: done",
        "MAP: <id>",
        "TICKETS: <id> <id>",
        "PR: <url>",
        "LABEL: <name> | <guidance> | skills: <a>, <b> | tickets: <ids>",
    ] {
        assert!(chart.contains(text), "brainstorm-chart lacks {text:?}");
    }
    let epic = skill("orqa-brainstorm-epic");
    for text in [
        "name: orqa-brainstorm-epic",
        "**MAP**",
        "**WAYPOINT**",
        "**TICKET LABELS**",
        "**RELEASES**",
        "**RESULT FILE**",
        "which Epics carry `orqa:release`",
        "--no-inherit-labels",
        "STATUS: done",
        "EPICS: <id> <id>",
        "--status=closed --limit 0",
        "PR: <url>",
        "LABEL: <name> | <guidance> | skills: <a>, <b> | tickets: <ids>",
    ] {
        assert!(epic.contains(text), "brainstorm-epic lacks {text:?}");
    }
    for (name, body) in [("chart", chart), ("epic", epic)] {
        for text in [
            "orqa-brainstorm-grilling",
            "orqa-brainstorm-domain-modeling",
        ] {
            assert!(
                body.contains(text),
                "brainstorm-{name} does not load {text}"
            );
        }
        assert!(
            !body.contains("git add -A"),
            "brainstorm-{name} stages every path"
        );
    }
}

/// brainstorm-chart and brainstorm-epic each carry the {{prose}} line, which
/// fill_jobs fills with the job's pick as it does a Stage skill's.
#[test]
fn the_brainstorm_chart_and_epic_skills_fill_the_prose_line() {
    let have = ["orqa-caveman".to_string()];
    for name in ["orqa-brainstorm-chart", "orqa-brainstorm-epic"] {
        let body = skill(name);
        assert_eq!(body.matches("{{prose}}").count(), 1, "{name}");
        let (filled, lacking) =
            crate::skills::manifest::Manifest::default().fill_jobs(body, &have, &[], "");
        assert!(lacking.is_empty(), "{name}: {lacking:?}");
        assert!(!filled.contains("{{"), "{name} keeps a placeholder");
        assert!(
            filled.contains("Use the orqa-caveman skill for your commits"),
            "{name}"
        );
    }
}

/// The two Waypoint skills ship: brainstorm-waypoint works one Waypoint
/// with the user, prototypes in a worktree of its own, and leaves research
/// and the build-Epic Waypoint to Orqadence; brainstorm-research works one
/// Research Waypoint alone and never touches the Map. Both file Manual work
/// and name the Waypoint they took; neither commits more than its paths.
#[test]
fn the_brainstorm_waypoint_and_research_skills_ship_with_their_rules() {
    let waypoint = skill("orqa-brainstorm-waypoint");
    for text in [
        "name: orqa-brainstorm-waypoint",
        "- **MAP**",
        "- **BACKGROUND**",
        "- **PROMPT**",
        "- **WAYPOINT**",
        "- **RESULT FILE**",
        "bd ready --parent <MAP> --unassigned --json",
        "skip `brainstorm:research` when BACKGROUND is on",
        "never take `brainstorm:epic`",
        "`.orqadence-local/worktrees/<idea>-proto-<name>`",
        "`prototype/<name>`",
        "--no-inherit-labels",
        "load the orqa-brainstorm-research skill by name",
        "[LOGIC.md](LOGIC.md)",
        "[UI.md](UI.md)",
    ] {
        assert!(
            waypoint.contains(text),
            "brainstorm-waypoint lacks {text:?}"
        );
    }
    assert_eq!(waypoint.matches("{{working-mode}}").count(), 1);
    let research = skill("orqa-brainstorm-research");
    for text in [
        "name: orqa-brainstorm-research",
        "- **MAP**",
        "- **WAYPOINT**",
        "- **RESULT FILE**",
        "Never edit the Map's description",
        "git commit -m '<message>' -- docs/research/<name>.md",
        "--no-inherit-labels",
        "never hand the research to a background agent",
    ] {
        assert!(
            research.contains(text),
            "brainstorm-research lacks {text:?}"
        );
    }
    assert!(!research.contains("{{working-mode}}"));
    for (name, text) in [
        ("orqa-brainstorm-waypoint", waypoint),
        ("orqa-brainstorm-research", research),
    ] {
        for want in [
            "load the orqa-manual-work skill by name",
            "`STATUS: manual`",
            "WAYPOINT: <id>",
            "{{prose}}",
            "index.lock",
        ] {
            assert!(text.contains(want), "{name} lacks {want:?}");
        }
        assert!(!text.contains("git add -A"), "{name} names git add -A");
    }
}

/// Every file ported from mattpocock/skills keeps the attribution and the
/// licence, and only the two Waypoint skills carry job lines. LOGIC.md and
/// UI.md keep only the attribution: they ship with SKILL.md's notice.
#[test]
fn the_mattpocock_derived_files_keep_their_licence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills");
    for file in [
        "orqa-brainstorm-grilling/SKILL.md",
        "orqa-brainstorm-domain-modeling/SKILL.md",
        "orqa-brainstorm-domain-modeling/ADR-FORMAT.md",
        "orqa-brainstorm-domain-modeling/CONTEXT-FORMAT.md",
        "orqa-brainstorm-waypoint/SKILL.md",
        "orqa-brainstorm-research/SKILL.md",
    ] {
        let body = fs::read_to_string(root.join(file)).unwrap();
        for text in [
            "of [mattpocock/skills](https://github.com/mattpocock/skills) (MIT License",
            "The above copyright notice and this permission notice shall be included",
        ] {
            assert!(body.contains(text), "{file} lacks {text:?}");
        }
        if !matches!(
            file,
            "orqa-brainstorm-waypoint/SKILL.md" | "orqa-brainstorm-research/SKILL.md"
        ) {
            assert!(!body.contains("{{"), "{file} carries a job line");
        }
    }
}

/// fill_jobs fills brainstorm-waypoint's working-mode line and both
/// skills' prose lines with the jobs' defaults.
#[test]
fn fill_jobs_fills_the_brainstorm_waypoint_and_research_job_lines() {
    let have = ["orqa-ponytail".to_string(), "orqa-caveman".to_string()];
    let manifest = Manifest::default();
    let (waypoint, lacking) = manifest.fill_jobs(skill("orqa-brainstorm-waypoint"), &have, &[], "");
    assert!(lacking.is_empty(), "{lacking:?}");
    assert!(!waypoint.contains("{{"), "{waypoint}");
    for text in ["Use the orqa-ponytail skill", "Use the orqa-caveman skill"] {
        assert!(
            waypoint.contains(text),
            "brainstorm-waypoint lacks {text:?}"
        );
    }
    let (research, lacking) = manifest.fill_jobs(skill("orqa-brainstorm-research"), &have, &[], "");
    assert!(lacking.is_empty(), "{lacking:?}");
    assert!(!research.contains("{{"), "{research}");
    assert!(
        research.contains("Use the orqa-caveman skill"),
        "{research}"
    );
}
