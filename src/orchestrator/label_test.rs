//! Ticket labels on the code-editing Stages: the label's skills and
//! guidance as Inputs, the Epic context in the Ticket file, and a label
//! skill missing on the base as a Question at Ticket start.

use super::stage::{Answer, Ask};
use super::world::{new_world, spawn_epic, spawn_ticket, BdTicket, World};
use super::write_file;
use crate::skills::SKILLS;
use serde_json::json;
use std::sync::Arc;

/// hx-1 with the bd labels given.
fn labelled_ticket(labels: &[&str]) -> BdTicket {
    BdTicket {
        labels: labels.iter().map(|l| l.to_string()).collect(),
        ..BdTicket::new("hx-1")
    }
}

/// config.json with orqa:db, an Area label with the skills and guidance
/// given, and orqa:codex-review, a Modifier.
fn config(w: &World, skills: &[&str], guidance: &str) {
    let doc = json!({"labels": {
        "db": {"kind": "area", "skills": skills, "guidance": guidance},
        "codex-review": {"kind": "modifier"},
    }});
    write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
}

/// The Label Inputs of the prompt writing `file`, in order.
fn label_inputs(w: &World, file: &str) -> Vec<String> {
    w.prompt(file)
        .lines()
        .filter(|l| l.starts_with("- Label"))
        .map(String::from)
        .collect()
}

/// A Ticket's labels reach the code-editing Stages, Implement, Fix and
/// address, as Inputs: the labels, Area first, the entries' installed
/// skills and their guidance, with the Ticket file. The Review and the
/// Debate get none of them.
#[test]
fn the_code_editing_stages_get_the_labels_skills_and_guidance_and_the_review_none() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:codex-review", "orqa:db"])]);
    config(&w, &["orqa-db-skill"], "Migrations are reversible.");
    w.installed("orqa-db-skill");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 opened");
    o.command("address-hx-1");
    w.await_line("hx-1 addressed PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();

    let want = [
        "- Label: orqa:db, orqa:codex-review",
        "- Label skills: orqa-db-skill",
        "- Label guidance: Migrations are reversible.",
    ];
    let ticket = format!(
        "- Ticket file: {}",
        o.run_dir("hx-1").join("ticket.md").display()
    );
    for file in ["implement.md", "fix-1.md", "address.md"] {
        assert_eq!(label_inputs(&w, file), want, "{file}");
        assert!(w.prompt(file).contains(&ticket), "{file}");
    }
    for file in ["review-1.md", "verdict-1.md"] {
        assert_eq!(label_inputs(&w, file), Vec::<String>::new(), "{file}");
        assert!(!w.prompt(file).contains("Ticket file"), "{file}");
    }
    assert!(!not_installed(&w).contains("orqa-db-skill"));
}

/// Implement's Not installed input, "" when it has none.
fn not_installed(w: &World) -> String {
    let implement = w.prompt("implement.md");
    let line = implement
        .lines()
        .find(|l| l.starts_with("- Not installed: "));
    line.unwrap_or_default().to_string()
}

/// A label skill the Stage's App cannot load, or that is missing, goes
/// under Not installed, named with its label, and not under Label skills.
#[test]
fn a_label_skill_not_installed_is_under_not_installed() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:db"])]);
    config(&w, &["orqa-db-skill"], "");
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(label_inputs(&w, "implement.md"), ["- Label: orqa:db"]);
    assert!(
        not_installed(&w).contains("orqa-db-skill (orqa:db)"),
        "{}",
        not_installed(&w)
    );
}

/// A personal ~/.claude/skills copy of a committed label skill shadows it:
/// a Question before any Stage, as a pick's is. Parked, no Stage starts.
#[test]
fn a_personal_copy_of_a_label_skill_is_a_question_at_ticket_start() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:db"])]);
    config(&w, &["orqa-db-skill"], "");
    w.installed("orqa-db-skill");
    write_file(
        &w.home.join(".claude/skills/orqa-db-skill/SKILL.md"),
        "yours",
    );
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let asked = w.await_event("shadows the committed");
    assert_eq!(
        asked.text,
        "your ~/.claude/skills/orqa-db-skill shadows the committed orqa-db-skill: claude runs yours"
    );
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");
    let Some(Ask::TicketStart { options }) = asked.ask else {
        panic!("no Ticket-start Question: {:?}", asked.ask);
    };
    o.answer("hx-1", "", Answer::Prompt(options[1].clone()));
    run.wait();
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");
}

/// The Ticket file is written as each code-editing Stage starts, the parent
/// Epic's description under ## Epic context, and Fix gets it as an Input
/// too. A Ticket without a parent has no such heading.
#[test]
fn the_ticket_file_ends_with_the_epics_description_and_fix_gets_it() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().epic_description = "Both sides expect a JSON API.".to_string();
    w.hook(|_, argv| (argv == ["bd", "show", "hx-1"]).then(|| Ok("hx-1 · the Ticket\n".into())));
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    let ticket = o.run_dir("hx-1").join("ticket.md");
    assert_eq!(
        std::fs::read_to_string(&ticket).unwrap(),
        "hx-1 · the Ticket\n\n## Epic context\n\nBoth sides expect a JSON API.\n"
    );
    let input = format!("\n- Ticket file: {}\n", ticket.display());
    assert!(
        w.prompt("fix-1.md").contains(&input),
        "{}",
        w.prompt("fix-1.md")
    );
    assert!(!w.prompt("review-1.md").contains(&input));

    let alone = BdTicket {
        no_epic: true,
        ..BdTicket::new("hx-1")
    };
    let (w, o) = new_world(vec![alone]);
    w.lock().epic_description = "Not this Ticket's.".to_string();
    w.hook(|_, argv| (argv == ["bd", "show", "hx-1"]).then(|| Ok("hx-1 · the Ticket\n".into())));
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    let ticket = o.run_dir("hx-1").join("ticket.md");
    assert_eq!(
        std::fs::read_to_string(&ticket).unwrap(),
        "hx-1 · the Ticket\n"
    );
}

/// A label skill, or a label's Extra review skill, the checkout's Skill
/// manifest records but the Ticket's base lacks is a Question before any
/// Stage. Run without it starts Implement without it: under Not installed,
/// not under Label skills, for this Ticket's run.
#[test]
fn a_label_skill_missing_from_the_base_is_a_question_at_ticket_start() {
    for (skills, extra) in [(vec!["orqa-db-skill"], ""), (vec![], "orqa-db-skill")] {
        let (w, o) = new_world(vec![labelled_ticket(&["orqa:db"])]);
        let doc = json!({"labels": {"db": {"kind": "area", "skills": skills,
            "extra_review": {"skill": extra}}}});
        write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
        w.unmerged_skill("orqa-db-skill");
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");

        let text = "orqa-db-skill is not committed on hx-1's base: commit and merge it first, \
                    or run without it";
        let asked = w.await_event("is not committed");
        assert_eq!(asked.text, text);
        assert!(w.called("herdr agent start").is_empty(), "a Stage started");
        let Some(Ask::TicketStart { options }) = asked.ask else {
            panic!("no Ticket-start Question: {:?}", asked.ask);
        };
        assert_eq!(
            options,
            [
                "park: commit and merge orqa-db-skill, then /continue @hx-1",
                "run without it"
            ]
        );
        o.answer("hx-1", "", Answer::Prompt(options[1].clone()));
        run.wait();

        w.await_line("hx-1 running without orqa-db-skill");
        w.await_line("hx-1 PR #hx-1 opened");
        assert_eq!(label_inputs(&w, "implement.md"), ["- Label: orqa:db"]);
        assert_eq!(
            not_installed(&w).contains("orqa-db-skill (orqa:db)"),
            extra.is_empty(),
            "{}",
            not_installed(&w)
        );
    }
}

/// Each code-editing Stage skill says to load the Label skills and follow
/// the Label guidance, and that the Epic context is context, not scope.
#[test]
fn the_code_editing_stage_skills_load_the_label_skills_and_bound_the_epic_context() {
    let line = "Load every skill under **Label skills** by name in this session, and follow \
                **Label guidance**.";
    for name in [
        "orqa-stage-implement",
        "orqa-stage-fix",
        "orqa-stage-address",
    ] {
        let skill = SKILLS.iter().find(|(n, _)| *n == name).unwrap().1;
        assert!(skill.contains(line), "{name} lacks the label line");
        assert!(
            skill.contains("**Ticket file**") && skill.contains("context, not scope"),
            "{name} does not bound the Epic context"
        );
    }
}
