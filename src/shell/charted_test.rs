//! What charting came out as: the Tickets modal starts the checked
//! Tickets, the start-Map modal starts the Map or leaves it for /continue,
//! and its Continue form keeps the answer given last time.

use crossterm::event::KeyCode;

use std::sync::Arc;

use super::chart_test::{map, saved, started, world, writes};
use super::shell_test::{await_line, key, line, render, rows, shell};
use super::Screen;
use crate::brainstorm::{Brainstorm, IDEA, RESEARCH};
use crate::orchestrator::world::{new_world, BdTicket, World};

/// Open Ticket `id`, on its own, with a two-line description.
fn ticket(id: &str) -> BdTicket {
    BdTicket {
        no_epic: true,
        description: format!("\nWhat {id} does.\nMore of it."),
        ..BdTicket::new(id)
    }
}

#[test]
fn a_tickets_result_opens_the_tickets_modal_every_row_checked() {
    let w = world(
        vec![ticket("hx-1"), ticket("hx-2")],
        writes("STATUS: done\nTICKETS: hx-1 hx-2\n"),
    );
    let mut s = started(&w);

    await_line(&mut s, "hx-7 charting done: Tickets hx-1, hx-2");

    let t = s.tickets.as_ref().expect("no Tickets modal");
    assert_eq!((t.idea.as_str(), t.title.as_str()), ("hx-7", "Ticket hx-7"));
    let rows: Vec<_> = t
        .rows
        .iter()
        .map(|r| (r.id.as_str(), r.about.as_str(), r.on))
        .collect();
    assert_eq!(
        rows,
        [
            ("hx-1", "What hx-1 does.", true),
            ("hx-2", "What hx-2 does.", true)
        ]
    );
    s.close();
}

/// Map hx-m as chart_test has it, two of its grilling Waypoints research,
/// with a Destination.
fn charted_map() -> Vec<BdTicket> {
    let mut issues = map(1);
    issues[0].description =
        "The why.\n\n## Destination\nAn Epic that\nworks offline.\n\n## Notes\nnone".to_string();
    issues[1].labels = vec![RESEARCH.to_string()];
    issues[2].labels = vec![RESEARCH.to_string()];
    issues
}

#[test]
fn a_map_result_opens_the_start_map_modal_with_its_destination_and_counts() {
    let w = world(charted_map(), writes("STATUS: done\nMAP: hx-m\n"));
    let mut s = started(&w);

    await_line(&mut s, "hx-7 charting done: Map hx-m");

    let m = s.start_map.as_ref().expect("no start-Map modal");
    assert_eq!(
        (m.idea.as_str(), m.map.as_str(), m.title.as_str()),
        ("hx-7", "hx-m", "Ticket hx-m")
    );
    assert_eq!(m.destination, "An Epic that works offline.");
    assert_eq!(
        m.counts,
        "6 Waypoints: 3 with you, 2 research, 1 writes the Epic · branch brainstorm/hx-7"
    );
    assert!(m.background && !m.again);
    s.close();
}

/// The Shell over hx-1, hx-2 and hx-3 of Epic hx, hx-9 on its own, and
/// Idea hx-7 closed by charting.
fn idle() -> (Arc<World>, Screen) {
    let mut issues: Vec<BdTicket> = ["hx-1", "hx-2", "hx-3"].map(BdTicket::new).into();
    issues.push(ticket("hx-9"));
    issues.push(BdTicket {
        status: "closed".to_string(),
        labels: vec![IDEA.to_string()],
        ..ticket("hx-7")
    });
    let (w, _) = new_world(issues);
    let s = shell(&w);
    (w, s)
}

/// The Tickets modal on `ids`, as charting Idea hx-7 opens it.
fn charted(s: &mut Screen, ids: &[&str]) {
    let b = Brainstorm {
        idea: "hx-7".to_string(),
        tickets: ids.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    };
    s.open_tickets(&b);
}

#[test]
fn start_tickets_on_an_idle_shell_starts_a_ticket_run_on_the_checked_ones() {
    let (_w, mut s) = idle();
    charted(&mut s, &["hx-1", "hx-2", "hx-3"]);

    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Char(' ')));
    s.key(key(KeyCode::Tab));
    s.key(key(KeyCode::Enter));

    assert!(s.tickets.is_none());
    assert!(s.run.is_some(), "{:?}", s.notice);
    assert_eq!(s.state.queue, ["hx-1", "hx-3"]);
    s.close();
}

#[test]
fn start_tickets_beside_a_live_ticket_run_adds_them() {
    let (_w, mut s) = idle();
    s.command("/start-ticket hx-1");
    charted(&mut s, &["hx-2", "hx-3"]);

    s.key(key(KeyCode::Tab));
    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-3 added to the run");
    assert_eq!(s.state.queue, ["hx-1", "hx-2", "hx-3"]);
    s.close();
}

#[test]
fn beside_a_live_epic_run_start_tickets_is_greyed_and_enter_does_nothing() {
    let (_w, mut s) = idle();
    s.command("/start-epic hx");
    let queue = s.state.queue.clone();
    charted(&mut s, &["hx-9"]);

    s.key(key(KeyCode::Tab));
    s.key(key(KeyCode::Enter));

    assert!(s.tickets.is_some(), "the modal stays");
    assert_eq!(s.state.queue, queue);
    let shown = rows(&render(&s, 140, 40)).join("\n");
    assert!(
        shown.contains(
            "An Epic run is live: Start tickets is refused, the Tickets stay open in bd."
        ),
        "{shown}"
    );
    s.close();
}

#[test]
fn cancel_or_esc_leaves_the_tickets_open_and_starts_nothing() {
    let (w, mut s) = idle();
    charted(&mut s, &["hx-1"]);
    s.key(key(KeyCode::Esc));
    assert!(s.tickets.is_none());

    charted(&mut s, &["hx-1"]);
    s.key(key(KeyCode::BackTab));
    s.key(key(KeyCode::Enter));

    assert!(s.tickets.is_none());
    assert!(s.run.is_none());
    assert!(w.called("bd close").is_empty());
}

/// The Shell with the start-Map modal open on Map hx-m, as charting left it.
fn map_modal() -> (Arc<World>, Screen) {
    let w = world(charted_map(), writes("STATUS: done\nMAP: hx-m\n"));
    let mut s = started(&w);
    await_line(&mut s, "hx-7 charting done: Map hx-m");
    (w, s)
}

fn shown(s: &Screen) -> String {
    rows(&render(s, 200, 40)).join("\n")
}

#[test]
fn space_flips_the_checkbox_and_its_line() {
    let (_w, mut s) = map_modal();
    let on = "2 Research Waypoints start in tab research-hx-m, 2 at once at most (max_research)";
    assert!(shown(&s).contains(on), "{}", shown(&s));

    s.key(key(KeyCode::Char(' ')));

    assert!(!s.start_map.as_ref().unwrap().background);
    let off = "the sessions with you take research as it reaches the frontier";
    assert!(
        shown(&s).contains(off) && !shown(&s).contains(on),
        "{}",
        shown(&s)
    );
    s.close();
}

#[test]
fn start_map_saves_the_answer_and_makes_the_map_live() {
    let (w, mut s) = map_modal();

    s.key(key(KeyCode::Char(' ')));
    s.key(key(KeyCode::Tab));
    s.key(key(KeyCode::Enter));

    assert!(s.start_map.is_none());
    assert_eq!(s.live.as_deref(), Some("hx-7"));
    assert!(!saved(&w).background);
    assert!(!s.brainstorms[0].background);
    s.close();

    let (w, mut s) = map_modal();
    s.key(key(KeyCode::Enter));
    assert!(saved(&w).background, "Enter on the checkbox starts it too");
    assert!(s.brainstorms[0].background);
    s.close();
}

#[test]
fn cancel_keeps_the_map_and_suggests_continue() {
    let (w, mut s) = map_modal();

    s.key(key(KeyCode::BackTab));
    s.key(key(KeyCode::Enter));

    assert!(s.start_map.is_none());
    assert!(s.live.is_none());
    assert_eq!(s.suggestion.as_deref(), Some("/continue @hx-m"));
    let last = line(s.events.last().unwrap());
    assert_eq!(last, "hx-m not started: /continue @hx-m starts it");
    assert_eq!(saved(&w).map, "hx-m");
    s.close();
}

#[test]
fn the_continue_form_shows_the_saved_answer() {
    let (_w, mut s) = map_modal();
    s.key(key(KeyCode::Esc));
    let mut b = s.brainstorms[0].clone();
    b.background = false;

    s.open_start_map(&b, true);

    let m = s.start_map.as_ref().unwrap();
    assert!(m.again && !m.background);
    assert_eq!(
        m.counts,
        "0/6 Waypoints closed · 3 with you open · 2 research open"
    );
    let text = shown(&s);
    for want in [
        " CONTINUE · hx-m ",
        "[ ] start the research in the background   your answer last time",
        "Continue",
        " Enter keeps it · Space flips it · Esc cancels ",
    ] {
        assert!(text.contains(want), "{want:?} not in:\n{text}");
    }
    s.key(key(KeyCode::Char(' ')));
    assert!(!shown(&s).contains("your answer last time"), "flipped");

    s.key(key(KeyCode::Char(' ')));
    s.key(key(KeyCode::Enter));
    assert_eq!(s.live.as_deref(), Some("hx-7"));
    s.close();
}

#[test]
fn the_tickets_modal_renders_and_unchecking_changes_the_runs_line() {
    let (_w, mut s) = idle();
    charted(&mut s, &["hx-1", "hx-9"]);
    let text = shown(&s);
    for want in [
        " CHARTED · 2 Tickets from hx-7 ",
        "hx-7 Ticket hx-7 came out small: 2 Tickets, no Map.",
        "Check the ones to start.",
        "› [x] hx-1  Ticket hx-1",
        "  [x] hx-9  Ticket hx-9",
        "      What hx-9 does.",
        "runs /start-ticket @hx-1 @hx-9",
        "Start tickets",
        " ↑↓ move · Space checks · Tab the buttons · Enter · Esc cancels ",
    ] {
        assert!(text.contains(want), "{want:?} not in:\n{text}");
    }
    assert!(!text.contains("An Epic run is live"), "{text}");

    s.key(key(KeyCode::Char(' ')));

    let text = shown(&s);
    assert!(text.contains("› [ ] hx-1  Ticket hx-1"), "{text}");
    assert!(text.contains("runs /start-ticket @hx-9"), "{text}");
    assert!(!text.contains("@hx-1"), "{text}");
}

#[test]
fn the_start_map_modal_renders_the_map_its_destination_and_counts() {
    let (_w, s) = map_modal();
    let text = shown(&s);
    for want in [
        " CHARTED · a Map from hx-7 ",
        "hx-7 came out as a Map.",
        "hx-m  Ticket hx-m",
        "Destination: An Epic that works offline.",
        "6 Waypoints: 3 with you, 2 research, 1 writes the Epic · branch brainstorm/hx-7",
        "› [x] start the research in the background",
        "Start Map",
        "Cancel",
        "Start runs brainstorm-waypoint on the next Waypoint in a pane beside the Shell.",
        " Space checks · Tab moves · Enter · Esc cancels ",
    ] {
        assert!(text.contains(want), "{want:?} not in:\n{text}");
    }
    let mut s = s;
    s.close();
}
