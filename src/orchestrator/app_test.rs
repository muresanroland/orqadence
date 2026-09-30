//! The App table and .orqadence/config.json: each Stage's App, model and
//! effort, read when the Stage starts.

use super::app::{
    app, canonical, checks, debate_inputs, floor_in, labels, row, ExtraReview, Floor, Label,
    IF_LIMITED,
};
use super::world::{new_world, spawn_ticket, succeed, BdTicket, World};
use super::write_file;
use crate::skills::manifest::{set_personal, Manifest, NONE};
use crate::skills::SKILLS;
use crate::tempdir::TempDir;
use serde_json::{json, Value};
use std::sync::Arc;

fn config(w: &World, body: &str) {
    write_file(&w.repo.join(".orqadence/config.json"), body);
}

/// The args after herdr's `--` that the Stage's session started with.
fn argv(w: &World, stage: &str) -> String {
    let start = &w.called(&format!("herdr agent start h-hx-1-{stage}"))[0];
    start.split_once(" -- ").unwrap().1.to_string()
}

#[test]
fn a_rows_model_and_effort_add_the_flags_on_claude_and_on_codex() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // Partial: the Review's app is left to its default, codex.
    config(
        &w,
        r#"{
  "implement": {"app": "claude", "model": "opus", "effort": "high"},
  "review": {"model": "gpt-6-sol", "effort": "low"},
  "fix": {"model": "sonnet"}
}"#,
    );
    o.run_ticket("hx-1");

    let run = o.run_dir("hx-1").display().to_string();
    assert!(
        argv(&w, "implement").ends_with(&format!("--add-dir {run} --model opus --effort high")),
        "{}",
        argv(&w, "implement")
    );
    assert_eq!(
        argv(&w, "review"),
        "--sandbox workspace-write -m gpt-6-sol -c model_reasoning_effort=low"
    );
    assert_eq!(
        argv(&w, "fix"),
        format!("--permission-mode auto --add-dir {run} --model sonnet")
    );
    assert!(w.called("herdr agent start h-hx-1-review")[0].contains("--kind codex"));
    w.await_line("hx-1 implement started: claude opus/high (pane 1-1)");
    w.await_line("hx-1 review 1 started: codex gpt-6-sol/low (pane 1-2)");
    w.await_line("hx-1 fix 1 started: claude sonnet (pane 1-4)");
}

/// A Review on claude starts in the Run directory with the worktree added
/// read-only, and waits on claude's trust, not on codex's, its default
/// App's.
#[test]
fn a_review_on_claude_runs_in_the_run_directory_and_trust_is_read_through_the_rows_app() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let home = TempDir::new(); // claude trusts the repo, codex nothing
    write_file(
        &home.path().join(".claude.json"),
        &format!(
            r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#,
            w.repo.display()
        ),
    );
    o.cfg.home = home.path().to_path_buf();
    config(&w, r#"{"review": {"app": "claude"}}"#);
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");

    let worktree = o.worktree("hx-1").display().to_string();
    w.await_line("hx-1 review 1 started: claude (pane 1-2)");
    assert_eq!(
        argv(&w, "review"),
        format!(
            "--permission-mode auto --add-dir {worktree} --settings \
             {{\"permissions\":{{\"deny\":[\"Edit(/{worktree}/**)\"]}},\"sandbox\":\
             {{\"allowUnsandboxedCommands\":false,\"enabled\":true,\"failIfUnavailable\":true}}}}"
        )
    );
    let split = &w.called("herdr pane split")[0];
    assert!(
        split.contains(&format!("--cwd {} ", o.run_dir("hx-1").display())),
        "{split}"
    );
}

/// A worktree path with a quote or backslash still makes valid settings JSON.
#[test]
fn a_worktree_path_is_escaped_in_the_review_settings_on_claude() {
    let worktree = r#"/tmp/a"b\c"#;
    let args = (app("claude").unwrap().run_dir_args)(worktree);
    let settings: serde_json::Value = serde_json::from_str(args.last().unwrap()).unwrap();
    assert_eq!(
        settings["permissions"]["deny"][0],
        format!("Edit(/{worktree}/**)")
    );
}

#[test]
fn the_moderators_inputs_carry_each_sides_command() {
    let skill = SKILLS
        .iter()
        .find(|(name, _)| *name == "orqa-stage-moderate")
        .unwrap()
        .1;
    for own in ["claude -p", "codex exec"] {
        assert!(
            !skill.contains(own),
            "orqa-stage-moderate still runs {own:?}"
        );
    }
    // Started in the worktree, a claude side is granted the Run directory,
    // its sibling, which holds the diff; the model and effort go before -p.
    let claude = "'claude' '--tools' 'Read,Grep,Glob,Skill' '--add-dir' '{run}'";
    let codex = "'codex' 'exec' '--sandbox' 'read-only'";
    for (body, side_a, side_b) in [
        ("", format!("{claude} '-p'"), codex.to_string()),
        (
            r#"{
  "side_a": {"app": "codex", "model": "gpt-6-sol", "effort": "low"},
  "side_b": {"app": "claude", "model": "claude-opus-5-5[1m]", "effort": "high"}
}"#,
            format!("{codex} '-m' 'gpt-6-sol' '-c' 'model_reasoning_effort=low'"),
            format!("{claude} '--model' 'claude-opus-5-5[1m]' '--effort' 'high' '-p'"),
        ),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        if !body.is_empty() {
            config(&w, body);
        }
        o.run_ticket("hx-1");

        // The Moderator's prompt: the Stage skill's body, then its Inputs.
        let prompt = w
            .called("herdr agent prompt")
            .into_iter()
            .find(|call| call.contains("verdict-1.md"))
            .unwrap();
        let inputs = prompt.split_once("## Inputs").unwrap().1;
        let run = o.run_dir("hx-1").display().to_string();
        for want in [
            format!("- Side A command: {side_a}\n").replace("{run}", &run),
            format!("- Side B command: {side_b}\n").replace("{run}", &run),
        ] {
            assert!(inputs.contains(&want), "{want:?} not in:{inputs}");
        }
    }
}

/// Each experimental App on every Stage it can take: a worktree Stage (Fix)
/// and the Review start with its unattended args and the row's model and
/// effort, under its herdr kind; its Debate side runs read-only with them,
/// before a closing -p, which copilot's takes the brief as its value.
/// opencode and cursor have no effort flag: cursor's comes typed on the
/// model.
#[test]
fn the_experimental_apps_start_panes_and_sides_with_their_args() {
    // (App, model, a pane in the worktree, the Review, side A's command)
    let cases = [
        (
            "pi",
            "anthropic/claude-opus-5-5",
            "--model anthropic/claude-opus-5-5 --thinking high",
            "--model anthropic/claude-opus-5-5 --thinking high",
            "'pi' '--tools' 'read,grep,find,ls' '--model' 'anthropic/claude-opus-5-5' '--thinking' 'high' '-p'",
        ),
        (
            "opencode",
            "anthropic/claude-opus-5-5",
            "--auto -m anthropic/claude-opus-5-5",
            "--auto -m anthropic/claude-opus-5-5",
            r#"'env' 'OPENCODE_PERMISSION={"edit":"deny","bash":"deny","external_directory":"allow"}' 'opencode' 'run' '-m' 'anthropic/claude-opus-5-5'"#,
        ),
        (
            "copilot",
            "claude-opus-5.5",
            "--allow-all --add-dir {run} --model claude-opus-5.5 --effort high",
            "--allow-all --add-dir {worktree} --model claude-opus-5.5 --effort high",
            "'copilot' '--deny-tool=write' '--deny-tool=shell' '--add-dir' '{run}' '--model' 'claude-opus-5.5' '--effort' 'high' '-p'",
        ),
        (
            "cursor",
            "opus-5.5[effort=high]",
            "--force --add-dir {run} --model opus-5.5[effort=high]",
            "--force --add-dir {worktree} --model opus-5.5[effort=high]",
            "'cursor-agent' '--mode' 'ask' '--trust' '--add-dir' '{run}' '--model' 'opus-5.5[effort=high]' '-p'",
        ),
    ];
    for (name, model, pane, review, side) in cases {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        let row = json!({"app": name, "model": model, "effort": "high"});
        let doc = json!({"fix": row, "review": row, "side_a": row, "moderator": {"app": name}});
        config(&w, &doc.to_string());
        o.run_ticket("hx-1");

        let (run, worktree) = (o.run_dir("hx-1"), o.worktree("hx-1"));
        let put = |s: &str| {
            s.replace("{run}", &run.display().to_string())
                .replace("{worktree}", &worktree.display().to_string())
        };
        assert_eq!(argv(&w, "fix"), put(pane), "{name}");
        assert_eq!(argv(&w, "review"), put(review), "{name}");
        for stage in ["fix", "review", "debate"] {
            let start = &w.called(&format!("herdr agent start h-hx-1-{stage}"))[0];
            assert!(start.contains(&format!(" --kind {name} ")), "{start}");
        }
        let inputs = w.prompt("verdict-1.md");
        let want = format!("- Side A command: {}\n", put(side));
        assert!(inputs.contains(&want), "{want:?} not in:{inputs}");
    }
}

/// TypeSafe off: the Moderator is told so under Inputs, and its pane gets no
/// key; on, neither changes.
#[test]
fn with_typesafe_off_the_moderator_gets_the_input_and_no_key() {
    for (body, off) in [("", false), (r#"{"typesafe": false}"#, true)] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        if !body.is_empty() {
            config(&w, body);
        }
        o.run_ticket("hx-1");

        let prompt = w
            .called("herdr agent prompt")
            .into_iter()
            .find(|call| call.contains("verdict-1.md"))
            .unwrap();
        let inputs = prompt.split_once("## Inputs").unwrap().1;
        assert_eq!(inputs.contains("- TypeSafe: off\n"), off, "{inputs}");
        let keyed = w
            .called("herdr")
            .iter()
            .any(|c| c.contains("TYPESAFE_API_KEY=sk-test"));
        assert_eq!(keyed, !off, "off {off}: the key reached a pane or not");
    }
}

/// Running Stages keep theirs; the Stages that start after a change use it.
#[test]
fn config_changed_between_two_stages_reaches_the_second() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let file = w.repo.join(".orqadence/config.json");
    w.session(move |p| {
        if p.stage == "implement" {
            write_file(&file, r#"{"review": {"model": "gpt-6-sol"}}"#);
        }
        succeed(p)
    });
    o.run_ticket("hx-1");

    assert!(!argv(&w, "implement").contains("gpt-6-sol"));
    assert_eq!(argv(&w, "review"), "--sandbox workspace-write -m gpt-6-sol");
}

/// A config.json no Stage can start on, read as a Stage starts, is a Wake
/// before its session starts: unreadable, naming the file, a field not a
/// string, a Stage other than Implement, the Review or a Debate side on
/// codex, a plan model split from Implement's model where either is not a
/// full claude- id, a split off claude, an App the table does not have,
/// none off the Review's fallback, both Debate sides on one family, or a
/// side whose family cannot be told.
#[test]
fn an_unreadable_config_or_a_stage_codex_cannot_run_wakes_the_stage_that_reads_it() {
    const SPLIT_NOT_FULL: &str = "{file}: implement plan_model splits from model: \
         the split needs a full claude- model id for each half";
    for (body, label, reason) in [
        ("{ not json", "implement", "{file}: "),
        (
            r#"{"implement": {"model": 5}}"#,
            "implement",
            "{file}: implement model is not a string",
        ),
        (
            r#"{"moderator": {"app": "codex"}}"#,
            "debate 1",
            "moderator does not run on codex",
        ),
        (
            r#"{"implement": {"plan_model": "claude-fable-5-1"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
        (
            r#"{"implement": {"model": "opus", "plan_model": "claude-fable-5-1"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
        (
            r#"{"implement": {"model": "claude-opus-5-5", "plan_model": "fable"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
        (
            r#"{"implement": {"app": "codex", "plan_model": "claude-fable-5-1"}}"#,
            "implement",
            "{file}: implement plan_model splits from model: the split runs on claude only",
        ),
        (
            r#"{"side_b": {"app": "gemini"}}"#,
            "debate 1",
            r#"{file}: no App named "gemini" for side_b"#,
        ),
        (
            r#"{"implement": {"model": "none"}}"#,
            "implement",
            "{file}: implement model none: only review_if_limited takes none",
        ),
        (
            r#"{"side_b": {"app": "claude"}}"#,
            "debate 1",
            "Both sides would be Anthropic: the Debate needs two families",
        ),
        (
            r#"{"side_b": {"app": "pi"}}"#,
            "debate 1",
            "Side B's family cannot be told from its model: name one",
        ),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        config(&w, body);
        let o = Arc::new(o);
        let _run = spawn_ticket(o.clone(), "hx-1");

        let file = w.repo.join(".orqadence/config.json");
        let reason = reason.replace("{file}", &file.display().to_string());
        w.await_line(&format!("hx-1 stuck in {label}: {reason}"));
        let stage = label.split(' ').next().unwrap();
        assert!(
            w.called(&format!("herdr agent start h-hx-1-{stage}"))
                .is_empty(),
            "{body}"
        );
    }
}

/// Records the picks in the world's Skill manifest.
fn pick(w: &World, picks: &[(&str, &str)]) {
    let mut manifest = Manifest::load(&w.repo).unwrap();
    for (job, pick) in picks {
        manifest.picks.insert(job.to_string(), pick.to_string());
    }
    manifest.save(&w.repo).unwrap();
}

/// Each job's line names its pick in the mention form of the App it runs
/// on: in words on claude, plugin-qualified for a plugin's skill, $name on
/// codex. A none pick drops the line, and so does a pick not installed,
/// which the Inputs tell the Stage to note; the generic line stays.
#[test]
fn a_jobs_line_names_its_pick_in_the_apps_mention_form_or_is_dropped() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    set_personal(&w.repo, true).unwrap();
    write_file(&w.home.join(".claude/skills/tdd/SKILL.md"), "tdd");
    let plugin = TempDir::new();
    write_file(&plugin.path().join("skills/ponytail/SKILL.md"), "lazy");
    let plugins = serde_json::json!([
        {"id": "ponytail@market", "enabled": true, "installPath": plugin.path()},
    ])
    .to_string();
    w.hook(move |_, argv| {
        (argv.join(" ") == "claude plugin list --json").then(|| Ok(plugins.clone()))
    });
    pick(
        &w,
        &[
            ("test-first", "tdd"),
            ("working-mode", "ponytail:ponytail"),
            ("prose", NONE),
            ("review", "review-agent"),
        ],
    );
    o.run_ticket("hx-1");

    let implement = w.prompt("implement.md");
    for want in [
        "   Use the tdd skill for it.\n",
        "   Use the ponytail:ponytail skill for all your work",
        "test-first: a failing test, then the code.",
        "read the diff against the acceptance criteria and fix what is missing or wrong.",
        "- Not installed: orqa-code-review (self-review): their lines are left out; say so in the result file\n",
    ] {
        assert!(implement.contains(want), "{want:?} not in:\n{implement}");
    }
    for gone in [
        "{{",
        "code-review skill",
        "for your commits and result file",
    ] {
        assert!(!implement.contains(gone), "{gone:?} in:\n{implement}");
    }
    let review = w.prompt("review-1.md");
    assert!(
        review.contains("   Use the $review-agent skill for this review"),
        "{review}"
    );
    assert!(!review.contains("Not installed"), "{review}");
}

/// A personal skill, at home or a plugin's, is not installed until the
/// user turns their personal skills on: its line is left out.
#[test]
fn a_personal_pick_is_not_installed_until_personal_skills_are_on() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    write_file(&w.home.join(".claude/skills/tdd/SKILL.md"), "tdd");
    pick(&w, &[("test-first", "tdd")]);
    o.run_ticket("hx-1");

    let implement = w.prompt("implement.md");
    assert!(!implement.contains("Use the tdd skill"), "{implement}");
    assert!(implement.contains(" tdd (test-first),"), "{implement}");
}

/// A pick counts as installed only where the App running its line loads
/// it: codex, the Review's default, reads .agents/skills, never Claude's
/// .claude/skills or its plugins.
#[test]
fn a_pick_only_another_app_loads_is_not_installed() {
    for (dir, picked, installed) in [
        (".agents/skills", "requesting-code-review", true),
        (".claude/skills", "requesting-code-review", false),
        ("plugin/skills", "sp:requesting-code-review", false),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        set_personal(&w.repo, true).unwrap();
        write_file(
            &w.home.join(dir).join("requesting-code-review/SKILL.md"),
            "review",
        );
        let plugins = serde_json::json!([
            {"id": "sp@market", "enabled": true, "installPath": w.home.join("plugin")},
        ])
        .to_string();
        w.hook(move |_, argv| {
            (argv.join(" ") == "claude plugin list --json").then(|| Ok(plugins.clone()))
        });
        pick(&w, &[("review", picked)]);
        o.run_ticket("hx-1");

        let review = w.prompt("review-1.md");
        let line = review.contains("Use the $requesting-code-review skill");
        let noted = review.contains(&format!("- Not installed: {picked} (review)"));
        assert_eq!((line, noted), (installed, !installed), "{dir}:\n{review}");
    }
}

/// A pick built into codex is not installed on claude.
#[test]
fn a_built_in_pick_is_not_installed_on_another_app() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    config(&w, r#"{"review": {"app": "claude"}}"#);
    pick(&w, &[("review", "review-agent")]);
    o.run_ticket("hx-1");

    let review = w.prompt("review-1.md");
    assert!(!review.contains("review-agent skill"), "{review}");
    assert!(
        review.contains("- Not installed: review-agent (review)"),
        "{review}"
    );
}

/// The audit at none: no audit line, so the Moderator skips it and notes it.
#[test]
fn the_audit_at_none_is_skipped_and_noted() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.picked("audit", "orqa-ponytail-review");
    o.run_ticket("hx-1");
    let audit = "Use the orqa-ponytail-review skill on the diff";
    assert!(w.prompt("verdict-1.md").contains(audit));

    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    pick(&w, &[("audit", NONE)]);
    o.run_ticket("hx-1");
    let debate = w.prompt("verdict-1.md");
    assert!(!debate.contains("Use the "), "{debate}");
    assert!(
        debate.contains("no over-engineering audit (none picked)"),
        "{debate}"
    );
    assert!(
        !debate.contains("audit)"),
        "noted as not installed:\n{debate}"
    );
    // The Review's own default, none: its step 3 stands alone.
    let review = w.prompt("review-1.md");
    assert!(!review.contains("Use the "), "{review}");
    assert!(
        review.contains("3. Look for real problems only"),
        "{review}"
    );
}

/// The Review's fallback: unset while config.json has no row for it or its
/// model is none; a row with no model runs its App's default.
#[test]
fn the_fallback_is_unset_at_none_and_runs_a_row_with_no_model() {
    let (w, _o) = new_world(vec![BdTicket::new("hx-1")]);
    let fallback = || {
        super::app::fallback_row(&w.repo, &[])
            .unwrap()
            .map(|r| r.said())
    };
    assert_eq!(fallback(), None);
    config(
        &w,
        r#"{"review_if_limited": {"app": "claude", "model": "none"}}"#,
    );
    assert_eq!(fallback(), None);
    config(&w, r#"{"review_if_limited": {"app": "claude"}}"#);
    assert_eq!(fallback().as_deref(), Some("claude"));
}

/// One name per model whichever App runs it: an alias, a dotted version, a
/// provider prefix, a [1m] suffix and a dated id all name one model.
#[test]
fn a_model_has_one_name_across_apps() {
    for (model, want) in [
        ("opus", "claude-opus-5-5"),
        ("opus-5.5", "claude-opus-5-5"),
        ("anthropic/claude-opus-5-5", "claude-opus-5-5"),
        ("claude-opus-5-5[1m]", "claude-opus-5-5"),
        ("fable", "claude-fable-5-1"),
        ("sonnet-5", "claude-sonnet-5"),
        ("claude-haiku-4-5-20251001", "claude-haiku-4-5"),
        ("haiku", "claude-haiku-4-5"),
        ("openai/gpt-5.5", "gpt-5-5"),
        ("anthropic/claude-opus-5-5:high", "claude-opus-5-5"),
        // Only pi's thinking level goes: a tag names another model.
        ("qwen3:32b", "qwen3:32b"),
        ("gpt-6-sol", "gpt-6-sol"),
    ] {
        assert_eq!(canonical(model), want, "{model}");
    }
}

/// Each rule over a config.json: whether it holds, and what it says.
fn checks_of(doc: Value) -> Vec<(bool, String)> {
    checks(&doc)
        .into_iter()
        .map(|c| (c.holds, c.text))
        .collect()
}

#[test]
fn the_rules_over_config_json() {
    let holds = |text: &str| (true, text.to_string());
    let broken = |text: &str| (false, text.to_string());
    // The defaults hold; the fallback at none is not checked.
    assert_eq!(
        checks_of(json!({})),
        [
            holds("The Review runs on codex's default, not Implement's claude's default"),
            holds("The sides come from two families: Anthropic and OpenAI"),
        ]
    );
    // One model across Apps and spellings.
    assert_eq!(
        checks_of(json!({
            "implement": {"model": "opus"},
            "review": {"model": "anthropic/claude-opus-5-5"},
            IF_LIMITED: {"model": "opus-5.5"},
        }))[..2],
        [
            broken("The Review would run on Implement's model, claude-opus-5-5: it must not review its own work"),
            broken("The Review if limited would run on Implement's model, claude-opus-5-5: it must not review its own work"),
        ]
    );
    // One App's default is one model.
    assert_eq!(
        checks_of(json!({"review": {"app": "claude"}}))[0],
        broken("The Review would run on Implement's model, claude's default: it must not review its own work")
    );
    // The Debate: two families, each its App's.
    assert_eq!(
        checks_of(json!({"side_b": {"app": "claude", "model": "sonnet"}}))[1],
        broken("Both sides would be Anthropic: the Debate needs two families")
    );
    assert_eq!(
        checks_of(json!({
            "side_a": {"app": "codex", "model": "gpt-6-sol"},
            "side_b": {"app": "claude", "model": "opus"},
        }))[1],
        holds("The sides come from two families: OpenAI and Anthropic")
    );
    // An App that runs several: the family is its model's, and its default
    // could be any model, so a row a rule reads names one.
    assert_eq!(
        checks_of(json!({
            "side_a": {"app": "pi", "model": "anthropic/claude-opus-5-5"},
            "side_b": {"app": "opencode", "model": "google/gemini-3-pro"},
        }))[1],
        holds("The sides come from two families: Anthropic and Google")
    );
    assert_eq!(
        checks_of(json!({"side_a": {"app": "cursor", "model": "gpt-6"}}))[1],
        broken("Both sides would be OpenAI: the Debate needs two families")
    );
    assert_eq!(
        checks_of(json!({"side_a": {"app": "pi", "model": "moonshot/kimi-k3"}}))[1],
        broken("Side A's family cannot be told from its model: name one")
    );
    assert_eq!(
        checks_of(json!({"review": {"app": "copilot"}}))[0],
        broken("The Review cannot be checked: copilot's default model is unknown, name one")
    );
    assert_eq!(
        checks_of(json!({"implement": {"app": "pi"}}))[0],
        broken("The Review cannot be checked: pi's default model is unknown, name one")
    );
    assert_eq!(
        checks_of(json!({"review": {"app": "pi", "model": "openai/gpt-6"}}))[0],
        holds("The Review runs on gpt-6, not Implement's claude's default")
    );
}

/// The model lists /config offers on the experimental Apps, from each one's
/// listing as the research describes it (shapes unverified): pi's table with
/// its thinking levels, opencode's provider/model lines, cursor's "<id> -
/// <name>" lines; copilot lists none, so default and 'type an id…' stay.
#[test]
fn the_experimental_apps_list_their_models() {
    let tools = crate::tools::fake::Fake::new(|_, argv| match argv.join(" ").as_str() {
        "pi --list-models" => Ok(
            "provider   model            context  max-out  thinking  images\n\
             anthropic  claude-opus-5-5  200K     32K      yes       yes\n\
             openai     gpt-6            400K     128K     yes       yes\n"
                .to_string(),
        ),
        "opencode models" => Ok("anthropic/claude-opus-5-5\nopenai/gpt-6\n".to_string()),
        "cursor-agent models" => Ok(
            "Available models\n\nauto - Auto\nopus-5.5 - Claude Opus 5.5\n\
             Tip: use --model <id> to switch.\n"
                .to_string(),
        ),
        _ => Ok(String::new()),
    });
    let dir = std::path::Path::new("/repo");
    let ids = |name: &str| -> Vec<String> {
        (app(name).unwrap().models)(&*tools, dir)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    };
    assert_eq!(ids("pi"), ["anthropic/claude-opus-5-5", "openai/gpt-6"]);
    let (_, levels) = &(app("pi").unwrap().models)(&*tools, dir).unwrap()[0];
    assert_eq!(
        levels,
        &["off", "minimal", "low", "medium", "high", "xhigh", "max"]
    );
    assert_eq!(
        ids("opencode"),
        ["anthropic/claude-opus-5-5", "openai/gpt-6"]
    );
    assert_eq!(ids("cursor"), ["auto", "opus-5.5"]);
    assert!(ids("copilot").is_empty());
}

/// A floor is config.json's number from 0 to 1; missing or empty is its
/// default; anything else, null too, refuses, naming the key.
#[test]
fn a_floor_is_a_number_from_0_to_1_and_missing_or_empty_is_the_default() {
    const FLOOR: Floor = Floor {
        key: "plan_floor",
        default: 0.65,
    };
    let refused = Err("plan_floor is not a number from 0 to 1".to_string());
    for (doc, want) in [
        (json!(null), Ok(0.65)),
        (json!({}), Ok(0.65)),
        (json!({"plan_floor": ""}), Ok(0.65)),
        (json!({"plan_floor": 0.6}), Ok(0.6)),
        (json!({"plan_floor": 0}), Ok(0.0)),
        (json!({"plan_floor": 1}), Ok(1.0)),
        (json!({"plan_floor": 1.5}), refused.clone()),
        (json!({"plan_floor": -0.1}), refused.clone()),
        (json!({"plan_floor": "0.6"}), refused.clone()),
        (json!({"plan_floor": true}), refused.clone()),
        (json!({"plan_floor": null}), refused.clone()),
    ] {
        assert_eq!(floor_in(&doc, &FLOOR), want, "{doc}");
    }
}

/// config.json's labels, each entry by its name: every field read, kind
/// alone enough; a kind that is neither area nor modifier, missing too, a
/// row config.json has not, or a field of the wrong type refuses that entry.
#[test]
fn a_labels_object_parses_and_an_entry_needs_only_kind() {
    let doc = json!({"labels": {
        "be": {
            "kind": "area",
            "skills": ["api-design"],
            "guidance": "You build backends.",
            "rows": {"implement": {"model": "opus", "effort": ""}},
            "pr_template": "be.md",
            "extra_review": {"skill": "security-review", "position": "first", "debate": true,
                "app": "codex", "model": "gpt-6-sol", "effort": "high"},
        },
        "codex-review": {"kind": "modifier"},
        "loose": {"skills": []},
        "odd": {"kind": "persona"},
        "typo": {"kind": "area", "rows": {"reveiw": {"app": "codex"}}},
        "bad": {"kind": "area", "guidance": 5},
    }});
    let labels = labels(&doc);
    let be = Label {
        kind: "area".to_string(),
        skills: vec!["api-design".to_string()],
        guidance: "You build backends.".to_string(),
        rows: [(
            "implement".to_string(),
            [("model", "opus"), ("effort", "")]
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .into(),
        )]
        .into(),
        pr_template: "be.md".to_string(),
        extra_review: ExtraReview {
            skill: "security-review".to_string(),
            position: "first".to_string(),
            debate: true,
            app: "codex".to_string(),
            model: "gpt-6-sol".to_string(),
            effort: "high".to_string(),
        },
    };
    assert_eq!(labels["be"], Ok(be));
    let modifier = Label {
        kind: "modifier".to_string(),
        ..Default::default()
    };
    assert_eq!(labels["codex-review"], Ok(modifier));
    for (name, err) in [
        ("loose", "labels loose kind is not area or modifier"),
        ("odd", "labels odd kind is not area or modifier"),
        ("typo", "labels typo rows has no row reveiw"),
    ] {
        assert_eq!(labels[name], Err(err.to_string()));
    }
    assert!(
        labels["bad"]
            .as_ref()
            .is_err_and(|err| err.starts_with("labels bad: invalid type")),
        "{:?}",
        labels["bad"]
    );
    assert!(super::app::labels(&json!({})).is_empty());
}

/// A repo whose config.json is body.
fn repo_with(body: &Value) -> TempDir {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".orqadence/config.json"),
        &body.to_string(),
    );
    repo
}

/// The Ticket's labels, by name.
fn names(labels: &[&str]) -> Vec<String> {
    labels.iter().map(|l| l.to_string()).collect()
}

/// A Ticket label's row field wins over config.json's row; an empty one
/// falls through to it.
#[test]
fn an_orqa_be_implement_model_wins_over_the_repos_and_an_empty_field_falls_through() {
    let repo = repo_with(&json!({
        "implement": {"model": "sonnet", "effort": "high"},
        "labels": {"be": {"kind": "area", "rows": {"implement": {"model": "opus", "effort": ""}}}},
    }));
    let said = |labels: &[&str]| row(repo.path(), "implement", &names(labels)).map(|r| r.said());
    assert_eq!(said(&["be"]), Ok("claude opus/high".to_string()));
    assert_eq!(said(&[]), Ok("claude sonnet/high".to_string()));
}

/// Labels that clash are not guessed at: two Area labels, two Modifiers
/// setting one field, or an orqa: label with no entry refuses the row. An
/// Area and a Modifier on one field is no clash: the Modifier's wins.
#[test]
fn labels_that_clash_or_have_no_entry_refuse_the_row() {
    let review = |app: &str| json!({"review": {"app": app}});
    let repo = repo_with(&json!({"labels": {
        "be": {"kind": "area", "rows": {"review": {"app": "claude", "model": "opus"}}},
        "fe": {"kind": "area"},
        "codex-review": {"kind": "modifier", "rows": review("codex")},
        "pi-review": {"kind": "modifier", "rows": review("pi")},
        "fast": {"kind": "modifier", "rows": {"review": {"effort": "low"}}},
    }}));
    let said = |labels: &[&str]| row(repo.path(), "review", &names(labels)).map(|r| r.said());
    assert_eq!(
        said(&["be", "fe"]),
        Err("Area labels orqa:be and orqa:fe: a Ticket takes one".to_string())
    );
    assert_eq!(
        said(&["codex-review", "pi-review"]),
        Err("orqa:codex-review and orqa:pi-review both set review app".to_string())
    );
    assert_eq!(
        said(&["typo"]),
        Err("orqa:typo has no entry in config.json's labels".to_string())
    );
    assert_eq!(
        said(&["codex-review", "be", "fast"]),
        Ok("codex opus/low".to_string())
    );
}

/// A label's rows keep the Debate's rule: a label pinning side_b to side
/// A's family is refused, by /config's checks and as the Debate starts.
#[test]
fn a_label_pinning_side_b_to_side_as_family_is_refused() {
    let doc = json!({"labels": {"be": {"kind": "area", "rows": {"side_b": {"app": "claude"}}}}});
    let broken = (
        false,
        "be side_b: Both sides would be Anthropic: the Debate needs two families".to_string(),
    );
    assert!(
        checks_of(doc.clone()).contains(&broken),
        "{:?}",
        checks_of(doc)
    );
    let repo = repo_with(&doc);
    let debate = |labels: &[&str]| debate_inputs(repo.path(), "", &names(labels), |_| None);
    assert_eq!(
        debate(&["be"]).err().as_deref(),
        Some("Both sides would be Anthropic: the Debate needs two families")
    );
    assert!(debate(&[]).is_ok());
}

/// A label's rows keep the Review's rule: a label putting review_if_limited
/// on Implement's model is refused by /config's checks.
#[test]
fn a_label_putting_the_reviews_fallback_on_implements_model_is_refused() {
    let doc = json!({"implement": {"app": "claude", "model": "opus"},
        "labels": {"be": {"kind": "area",
            "rows": {"review_if_limited": {"app": "claude", "model": "opus"}}}}});
    let broken: Vec<_> = checks(&doc).into_iter().filter(|c| !c.holds).collect();
    assert_eq!(broken.len(), 1);
    assert!(
        broken[0].text.starts_with(
            "be review_if_limited: The Review if limited would run on Implement's model"
        ),
        "{}",
        broken[0].text
    );
}

/// A label's rows keep runs_on: a label row naming codex for fix is
/// refused, by /config's checks, on the Fix's page, and as the Fix starts.
#[test]
fn a_label_row_naming_codex_for_fix_is_refused_by_runs_on() {
    let doc = json!({"labels": {"be": {"kind": "area", "rows": {"fix": {"app": "codex"}}}}});
    let broken: Vec<_> = checks(&doc).into_iter().filter(|c| !c.holds).collect();
    assert_eq!(broken.len(), 1);
    assert_eq!(broken[0].rows, ["fix"]);
    assert_eq!(broken[0].text, "be fix does not run on codex");
    // config.json's own fix on codex is not the label's to answer for
    let own = json!({"fix": {"app": "codex"},
        "labels": {"be": {"kind": "area", "rows": {"fix": {"effort": "low"}}}}});
    assert!(checks(&own).iter().all(|c| c.holds));
    let repo = repo_with(&doc);
    assert_eq!(
        row(repo.path(), "fix", &names(&["be"])).err().as_deref(),
        Some("fix does not run on codex")
    );
}

/// hx-1 with the bd labels given.
fn labelled_ticket(labels: &[&str]) -> BdTicket {
    BdTicket {
        labels: names(labels),
        ..BdTicket::new("hx-1")
    }
}

/// Only the Ticket's orqa: labels count, read from bd as each Stage
/// starts: its Implement starts on orqa:be's model, and a triage label
/// changes nothing, though config.json has an entry by its name.
#[test]
fn a_tickets_orqa_label_pins_its_implement_and_a_triage_label_changes_nothing() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:be", "ready-for-agent"])]);
    let area = |model: &str| json!({"kind": "area", "rows": {"implement": {"model": model}}});
    let doc = json!({
        "implement": {"model": "sonnet", "effort": "high"},
        "labels": {"be": area("opus"), "ready-for-agent": area("haiku")},
    });
    config(&w, &doc.to_string());
    o.run_ticket("hx-1");

    w.await_line("hx-1 implement started: claude opus/high (pane 1-1)");
    assert!(argv(&w, "implement").ends_with("--model opus --effort high"));
}

/// Two Area labels on one Ticket are not guessed at: its Stage Wakes with a
/// reason naming both, before any session starts.
#[test]
fn a_ticket_with_two_area_labels_wakes_naming_both() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:be", "orqa:fe"])]);
    let area = json!({"kind": "area"});
    config(&w, &json!({"labels": {"be": area, "fe": area}}).to_string());
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 stuck in implement: Area labels orqa:be and orqa:fe: a Ticket takes one");
    assert!(w.called("herdr agent start").is_empty());
}
