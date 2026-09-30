//! /config in the dock (draw/modal.rs's frame): the Pipeline's sections down
//! the left, the picked one's page, a pick list or a checklist on the right,
//! two lines of foot under a rule.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::modal::{divider, dock, joined, wrap_spans};
use super::{bold, cut, fg, SPINNER};
use crate::on_call;
use crate::orchestrator::app::{self, Check, APPS};
use crate::setup;
use crate::shell::brand::{BORDER, CYAN, GREEN, MUTED, ORANGE, PURPLE, RED, TEXT};
use crate::shell::config::{
    distinct, family_label, floor_name, job_name, job_said, label_line, position_name, short,
    short_commit, Field, LabelItem, Listing, Pick, Scope, Settings, Typing, APPS_PAGE, FLOORS,
    LABELS_PAGE, ON_CALL_PAGE, REVIEW_ROW, ROWS, RUN_PAGE, SECTIONS, SKILLS_PAGE, SKILL_ROWS,
    TYPESAFE_PAGE,
};
use crate::shell::Screen;
use crate::skills::manifest::NONE;

/// The ground of the row under the cursor, and of the section whose page has it.
const SEL_BG: Color = Color::Rgb(44, 36, 78);
const REST_BG: Color = Color::Rgb(28, 32, 50);

pub(super) fn config(f: &mut Frame, s: &Screen) {
    let st = s.settings.as_ref().unwrap();
    let (rect, block) = dock(f, s);
    let block = block
        .title(Span::styled(" /config ", bold(TEXT)))
        .title(Line::from(badges(s, st)).right_aligned())
        .title_bottom(Span::styled(format!(" {} ", hint(st)), fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [main, rule, foot] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas(inner);
    f.render_widget(divider(rule.width as usize), rule);
    f.render_widget(Paragraph::new(foot_lines(s, st, foot.width as usize)), foot);
    let [left, gap, right] = Layout::horizontal([
        Constraint::Length(if main.width < 80 { 22 } else { 28 }),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .areas(main);
    let bar = vec![Line::from(Span::styled("│", fg(BORDER))); gap.height as usize];
    f.render_widget(Paragraph::new(bar), gap);
    f.render_widget(
        Paragraph::new(pipeline(s, st, left.height, left.width as usize)),
        left,
    );
    let width = right.width as usize;
    let (lines, at) = match (&st.listing, &st.pick) {
        (Some(listing), _) => checklist(listing, width),
        (_, Some(pick)) => pick_lines(st, pick, width),
        _ if st.section == APPS_PAGE => apps_page(st, width),
        _ if st.section == SKILLS_PAGE => skills_page(st, width),
        _ if st.section == LABELS_PAGE => labels_page(st, width),
        _ if st.section == TYPESAFE_PAGE => typesafe_page(s, st, width),
        _ if st.section == RUN_PAGE => run_page(st, width),
        _ if st.section == ON_CALL_PAGE => on_call_page(s, st, width),
        _ => page(st, width),
    };
    // the cursor's line in view
    let top = (at + 2).saturating_sub(right.height as usize);
    f.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), right);
}

/// The Apps page: each App of the table, installed with its version or
/// greyed with its homepage; the experimental ones under their own heading.
fn apps_page(st: &Settings, width: usize) -> (Vec<Line<'static>>, usize) {
    let about = "The agent CLIs a Stage runs on, found on PATH when /config opened; orqa init installs herdr's integration for each.";
    let mut lines = head("Apps", st.apps_summary(), about, width);
    lines.push(Line::default());
    let mut at = 0;
    for (i, app) in APPS.iter().enumerate() {
        if i > 0 && app.experimental && !APPS[i - 1].experimental {
            lines.push(Line::default());
            let heading = "  experimental, unverified: from their docs, never run here";
            lines.push(Line::from(Span::styled(cut(heading, width), fg(ORANGE))));
        }
        let selected = st.open && i == st.setting;
        let on = st.installed[i].is_some();
        let name = match (on, selected) {
            (false, _) => fg(MUTED),
            (true, true) => bold(TEXT),
            (true, false) => fg(TEXT),
        };
        let (state, color, detail) = match &st.installed[i] {
            Some(version) => ("installed", GREEN, version.clone()),
            None => ("not installed", MUTED, app.home.to_string()),
        };
        let spans = vec![
            Span::styled(if selected { "▸ " } else { "  " }, fg(PURPLE)),
            Span::styled(pad(app.name, 12), name),
            Span::styled(pad(state, 15), fg(color)),
            Span::styled(cut(&detail, width.saturating_sub(29)), fg(MUTED)),
        ];
        if selected {
            at = lines.len();
            lines.push(filled(spans, width, SEL_BG));
        } else {
            lines.push(Line::from(spans));
        }
    }
    (lines, at)
}

/// A page's title and its description under it.
fn head(title: &str, summary: String, about: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(title.to_string(), bold(TEXT)),
        Span::styled(format!("  {summary}"), fg(MUTED)),
    ])];
    lines.extend(wrap_spans(
        vec![(about.to_string(), fg(MUTED))],
        width,
        "",
        "",
        fg(MUTED),
    ));
    lines
}

/// A page's row: its label, padded, and its value; the cursor's filled.
/// Moves at to it when it has the cursor.
fn item(
    lines: &mut Vec<Line<'static>>,
    at: &mut usize,
    selected: bool,
    label: String,
    value: Vec<Span<'static>>,
    width: usize,
) {
    let mut spans = vec![
        Span::styled(if selected { "▸ " } else { "  " }, fg(PURPLE)),
        Span::styled(label, if selected { bold(TEXT) } else { fg(TEXT) }),
    ];
    spans.extend(value);
    if selected {
        *at = lines.len();
        lines.push(filled(spans, width, SEL_BG));
    } else {
        lines.push(Line::from(spans));
    }
}

/// The Skills page: where the skills live, read-only, your personal
/// skills' switch, then each skill Orqadence installed with its source @
/// commit and the jobs using it.
fn skills_page(st: &Settings, width: usize) -> (Vec<Line<'static>>, usize) {
    let about = "The skills Orqadence installed, from their sources; the Skill manifest is .orqadence/skills.json.";
    let summary = format!("{} installed", st.skills());
    let mut lines = head("Skills", summary, about, width);
    lines.push(Line::default());
    let value = vec![
        Span::styled(".orqadence/skills", fg(TEXT)),
        Span::styled(", committed", fg(MUTED)),
    ];
    let mut at = 0;
    let selected = |i: usize| st.open && st.setting == i;
    item(
        &mut lines,
        &mut at,
        selected(0),
        pad("location", 12),
        value,
        width,
    );
    let linked = vec![(
        "linked from .agents/skills and .claude/skills".to_string(),
        fg(MUTED),
    )];
    lines.extend(wrap_spans(linked, width, "    ", "      ", fg(MUTED)));
    let personal = match st.personal {
        true => vec![
            Span::styled("on", fg(TEXT)),
            Span::styled(", for you alone", fg(MUTED)),
        ],
        false => vec![
            Span::styled("off", fg(TEXT)),
            Span::styled(": Orqadence's and the repo's only", fg(MUTED)),
        ],
    };
    item(
        &mut lines,
        &mut at,
        selected(1),
        pad("yours", 12),
        personal,
        width,
    );
    let yours = vec![(
        "~/.claude/skills, ~/.agents/skills and Claude Code plugins".to_string(),
        fg(MUTED),
    )];
    lines.extend(wrap_spans(yours, width, "    ", "      ", fg(MUTED)));
    lines.push(Line::default());
    for (i, name) in st.skill_names().into_iter().enumerate() {
        let skill = &st.manifest.skills[&name];
        let value = match skill.shipped {
            true => vec![Span::styled("shipped with Orqadence", fg(MUTED))],
            false => vec![
                Span::styled(short(&skill.repo).to_string(), fg(TEXT)),
                Span::styled(format!(" @ {}", short_commit(&skill.commit)), fg(MUTED)),
            ],
        };
        item(
            &mut lines,
            &mut at,
            selected(i + SKILL_ROWS),
            pad(&name, 24),
            value,
            width,
        );
        // the jobs using it on a line of their own, as the pane is narrow
        let jobs = st.jobs_using(&name);
        if !jobs.is_empty() {
            let used = vec![(format!("← {}", jobs.join(", ")), fg(CYAN))];
            lines.extend(wrap_spans(used, width, "    ", "      ", fg(CYAN)));
        }
    }
    (lines, at)
}

/// The Labels page: each label of config.json as orqa:<name>, area or
/// modifier, and what it carries, an unreadable entry's error in red; or
/// the open label's own page.
fn labels_page(st: &Settings, width: usize) -> (Vec<Line<'static>>, usize) {
    if let Some(name) = &st.label {
        return label_page(st, name, width);
    }
    let about = "A bd label orqa:<name> on a Ticket changes how it runs: the skills and guidance its code-editing Stages get, its rows, its PR template and an Extra review.";
    let mut lines = head("Labels", st.labels_summary(), about, width);
    lines.push(Line::default());
    let mut at = 0;
    let labels = st.labels();
    if labels.is_empty() {
        lines.push(Line::from(Span::styled("  none: a adds one", fg(MUTED))));
    }
    for (i, (name, label)) in labels.iter().enumerate() {
        let value = match label {
            Ok(label) => {
                vec![
                    Span::styled(pad(&label.kind, 9), fg(TEXT)),
                    Span::styled(cut(&label_line(label), width.saturating_sub(31)), fg(MUTED)),
                ]
            }
            Err(err) => vec![Span::styled(cut(err, width.saturating_sub(22)), fg(RED))],
        };
        let label = pad(&format!("orqa:{name}"), 20);
        item(
            &mut lines,
            &mut at,
            st.open && i == st.setting,
            label,
            value,
            width,
        );
    }
    (lines, at)
}

/// A label's own page: area or modifier, its skills, guidance and PR
/// template; an Area label's Extra review; the Stage rows it overrides, and
/// the rules those keep, above them. One that cannot be read shows why
/// instead.
fn label_page(st: &Settings, name: &str, width: usize) -> (Vec<Line<'static>>, usize) {
    let mut lines = vec![Line::from(Span::styled(format!("orqa:{name}"), bold(CYAN)))];
    let label = match st.label_of(name) {
        Ok(label) => label,
        Err(err) => {
            let err = vec![(format!("{err}: mend it in config.json"), fg(RED))];
            lines.push(Line::default());
            lines.extend(wrap_spans(err, width, "  ", "  ", fg(RED)));
            return (lines, 0);
        }
    };
    // above the rows, where a long page shows them
    check_lines(&mut lines, app::label_checks_of(&st.doc, name), width);
    lines.push(Line::default());
    let room = width.saturating_sub(15);
    let or_none = |text: String| match text.is_empty() {
        true => vec![Span::styled("none", fg(MUTED))],
        false => vec![Span::styled(cut(&text, room), fg(TEXT))],
    };
    let heading = |text: String| Line::from(Span::styled(text, bold(MUTED)));
    let mut at = 0;
    for (i, it) in st.label_items().into_iter().enumerate() {
        match it {
            LabelItem::ExtraSkill => {
                lines.extend([Line::default(), heading("EXTRA REVIEW".into())]);
            }
            LabelItem::Row(0, Field::App) => lines.extend([
                Line::default(),
                heading("ROW OVERRIDES".into()),
                heading(format!("  {}", ROWS[0].name)),
            ]),
            LabelItem::Row(row, Field::App) => lines.push(heading(format!("  {}", ROWS[row].name))),
            _ => {}
        }
        let (field, value) = match it {
            LabelItem::Kind => (
                "kind".to_string(),
                vec![Span::styled(label.kind.clone(), bold(TEXT))],
            ),
            LabelItem::Skills => ("skills".to_string(), or_none(label.skills.join(", "))),
            LabelItem::Guidance => ("guidance".to_string(), or_none(label.guidance.clone())),
            LabelItem::Template => (
                "PR template".to_string(),
                match label.pr_template.as_str() {
                    "" => vec![
                        Span::styled("default", fg(MUTED)),
                        Span::styled("  the repo's default template", fg(MUTED)),
                    ],
                    file if st.templates.iter().any(|f| f == file) => {
                        vec![Span::styled(file.to_string(), bold(TEXT))]
                    }
                    file => vec![
                        Span::styled(file.to_string(), bold(TEXT)),
                        Span::styled("  not found: the default is used", fg(RED)),
                    ],
                },
            ),
            LabelItem::ExtraSkill => (
                "skill".to_string(),
                or_none(label.extra_review.skill.clone()),
            ),
            LabelItem::Position => (
                "position".to_string(),
                vec![Span::styled(
                    position_name(&label.extra_review.position),
                    fg(TEXT),
                )],
            ),
            LabelItem::Debate => {
                let on = label.extra_review.debate;
                let detail = match on {
                    true => "  joins the Debate",
                    false => "  straight to the Fix",
                };
                (
                    "debate".to_string(),
                    vec![
                        Span::styled(on_off(on), fg(TEXT)),
                        Span::styled(detail, fg(MUTED)),
                    ],
                )
            }
            LabelItem::Extra(field) => (
                field.name().to_string(),
                own_value(st, Scope::Extra, REVIEW_ROW, field),
            ),
            LabelItem::Row(row, field) => (
                format!("  {}", field.name()),
                own_value(st, Scope::Label, row, field),
            ),
        };
        item(
            &mut lines,
            &mut at,
            i == st.setting,
            pad(&field, 12),
            value,
            width,
        );
    }
    (lines, at)
}

/// What a label sets on a row field or its Extra review's, else what the
/// empty field falls through to.
fn own_value(st: &Settings, scope: Scope, row: usize, field: Field) -> Vec<Span<'static>> {
    match st.own(scope, row, field) {
        own if own.is_empty() => {
            let whose = if scope == Scope::Extra {
                "the Review's"
            } else {
                "repo's"
            };
            let fell = st.fallback(scope, row, field);
            vec![Span::styled(format!("{whose} {fell}"), fg(MUTED))]
        }
        own => vec![Span::styled(own, bold(TEXT))],
    }
}

/// The Run page: the Tickets a run takes at once, a bad value in red with
/// its check.
fn run_page(st: &Settings, width: usize) -> (Vec<Line<'static>>, usize) {
    let about = "How many Tickets an Epic run or a Ticket run has in the Pipeline at once.";
    let mut lines = head("Run", run_summary(st), about, width);
    lines.push(Line::default());
    let value = match st.max_tickets() {
        Ok((n, true)) => vec![
            Span::styled(n.to_string(), fg(TEXT)),
            Span::styled("  default", fg(MUTED)),
        ],
        Ok((n, false)) => vec![Span::styled(n.to_string(), fg(TEXT))],
        Err(written) => vec![Span::styled(written, fg(RED))],
    };
    let mut at = 0;
    let label = pad("tickets at once", 22);
    item(&mut lines, &mut at, st.open, label, value, width);
    check_lines(&mut lines, st.checks(Some(RUN_PAGE)), width);
    (lines, at)
}

/// The Run page's line on the left: "3 at once".
fn run_summary(st: &Settings) -> String {
    match st.max_tickets() {
        Ok((n, _)) => format!("{n} at once"),
        Err(written) => written,
    }
}

/// The TypeSafe page: on or off, its key, masked, and the floors, a bad one
/// in red with its check.
fn typesafe_page(s: &Screen, st: &Settings, width: usize) -> (Vec<Line<'static>>, usize) {
    let key = &s.cfg.api_key;
    let on = st.typesafe(key);
    let about =
        "Judgments: a Wake's next step, a Plan's approval, a Finding the Debate still disputes.";
    let mut lines = head("TypeSafe", on_off(on).to_string(), about, width);
    lines.push(Line::default());
    let state = match on {
        true => vec![Span::styled("on", bold(GREEN))],
        false => vec![
            Span::styled("off", bold(ORANGE)),
            Span::styled("  every Wake and Plan is a Question", fg(MUTED)),
        ],
    };
    let shown = masked(key, "none");
    let mut at = 0;
    let selected = |i: usize| st.open && st.setting == i;
    item(
        &mut lines,
        &mut at,
        selected(0),
        pad("TypeSafe", 22),
        state,
        width,
    );
    item(
        &mut lines,
        &mut at,
        selected(1),
        pad("key", 22),
        vec![shown],
        width,
    );
    for (i, floor) in FLOORS.iter().enumerate() {
        let value = match st.floor(floor) {
            Ok((value, true)) => vec![
                Span::styled(format!("{value:.2}"), fg(TEXT)),
                Span::styled("  default", fg(MUTED)),
            ],
            Ok((value, false)) => vec![Span::styled(format!("{value:.2}"), fg(TEXT))],
            Err(written) => vec![Span::styled(written, fg(RED))],
        };
        let label = pad(&floor_name(floor), 22);
        item(&mut lines, &mut at, selected(2 + i), label, value, width);
    }
    check_lines(&mut lines, st.checks(Some(TYPESAFE_PAGE)), width);
    (lines, at)
}

/// A secret as a page shows it: the tail of a long one only, to tell them
/// apart; `none` when empty.
fn masked(secret: &str, none: &'static str) -> Span<'static> {
    match secret.chars().count() {
        0 => Span::styled(none, fg(MUTED)),
        n if n < 12 => Span::styled("••••", fg(TEXT)),
        n => {
            let tail: String = secret.chars().skip(n - 4).collect();
            Span::styled(format!("••••{tail}"), fg(TEXT))
        }
    }
}

/// The On call page: the Moshi token, masked, the minutes, the test push,
/// and docs/on-call.md's URL at its foot.
fn on_call_page(s: &Screen, st: &Settings, width: usize) -> (Vec<Line<'static>>, usize) {
    let token = s.on_call.token.as_deref().unwrap_or_default();
    let about = "Rings your phone through Moshi when a Question waits and nobody answers.";
    let mut lines = head(
        "On call",
        on_off(!token.is_empty()).to_string(),
        about,
        width,
    );
    lines.push(Line::default());
    let rows = [
        ("token", vec![masked(token, "not set")]),
        (
            "minutes",
            vec![Span::styled(s.on_call.minutes.to_string(), fg(TEXT))],
        ),
        ("send a test push", Vec::new()),
    ];
    let mut at = 0;
    for (i, (label, value)) in rows.into_iter().enumerate() {
        let selected = st.open && st.setting == i;
        item(&mut lines, &mut at, selected, pad(label, 22), value, width);
    }
    lines.push(Line::default());
    // on a line of its own, whole where the pane is wide enough to click it
    let doc = vec![(on_call::DOC.to_string(), fg(CYAN))];
    lines.extend(wrap_spans(doc, width, "", "", fg(MUTED)));
    (lines, at)
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// A source's skills to tick, the installed ones ticked for good.
fn checklist(listing: &Listing, width: usize) -> (Vec<Line<'static>>, usize) {
    let title = format!("Skills in {}", listing.source);
    let mut lines = vec![Line::from(Span::styled(title, bold(CYAN)))];
    let mut at = 0;
    for (i, (name, installed, ticked)) in listing.names.iter().enumerate() {
        let tick = if *ticked { "[x] " } else { "[ ] " };
        let value = match installed {
            true => vec![Span::styled("installed", fg(GREEN))],
            false => Vec::new(),
        };
        let label = format!("{tick}{}", pad(name, 24));
        item(
            &mut lines,
            &mut at,
            i == listing.cursor,
            label,
            value,
            width,
        );
    }
    (lines, at)
}

/// `text` cut and padded to `width`.
fn pad(text: &str, width: usize) -> String {
    format!("{:<width$}", cut(text, width.saturating_sub(1)))
}

/// A row filled to `width` on `bg`: the cursor's.
fn filled(mut spans: Vec<Span<'static>>, width: usize, bg: Color) -> Line<'static> {
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    Line::from(spans).style(Style::default().bg(bg))
}

/// The title's right end: the Questions waiting behind it, a live run, the
/// last save.
fn badges(s: &Screen, st: &Settings) -> Vec<Span<'static>> {
    let mut badges = Vec::new();
    let broken = st.checks(None).iter().filter(|c| !c.holds).count();
    if broken > 0 {
        let s = if broken == 1 { "" } else { "s" };
        badges.push(Span::styled(format!("✗ {broken} check{s}"), bold(RED)));
    }
    if !s.questions.is_empty() {
        badges.push(Span::styled(
            format!("{} waiting", s.questions.len()),
            bold(ORANGE),
        ));
    }
    if s.running {
        badges.push(Span::styled("run live", fg(CYAN)));
    }
    if let Some(time) = &st.saved {
        badges.push(Span::styled(format!("saved {time}"), fg(GREEN)));
    }
    if badges.is_empty() {
        return Vec::new();
    }
    let mut spans = vec![Span::raw(" ")];
    spans.extend(joined(badges));
    spans.push(Span::raw(" "));
    spans
}

fn hint(st: &Settings) -> &'static str {
    match st {
        _ if st.probe.is_some() => "probing… · Esc drops it",
        _ if st.busy.is_some() => "working…",
        _ if st.confirm.is_some() => "y yes · n no",
        Settings {
            typing: Some((Typing::Model(_), _)),
            ..
        } => "Enter probes and saves · Esc cancels",
        _ if st.typing.is_some() => "Enter saves · Esc cancels",
        _ if st.listing.is_some() => "↑↓ move · Space ticks · Enter installs · Esc back",
        _ if st.pick.as_ref().is_some_and(|p| p.field == Field::Skills) => {
            "↑↓ move · type to filter · Enter toggles · Esc back"
        }
        _ if st.pick.is_some() => "↑↓ move · type to filter · Enter picks · Esc back",
        _ if st.open && st.section == APPS_PAGE => "↑↓ App · ← or Esc back",
        _ if st.open && st.section == SKILLS_PAGE => {
            "↑↓ skill · Enter toggles yours · a add · u update · U update all · d remove · ← back"
        }
        _ if st.open && st.section == LABELS_PAGE && st.label.is_some() => {
            "↑↓ field · Enter changes · Space toggles · ← or Esc back"
        }
        _ if st.open && st.section == LABELS_PAGE => {
            "↑↓ label · a add · e edit · r rename · d delete · ← or Esc back"
        }
        _ if st.open && st.section == 0 => {
            "↑↓ setting · Enter changes · Space toggles · ← or Esc back"
        }
        _ if st.open => "↑↓ setting · Enter changes · ← or Esc back",
        _ => "↑↓ Stage · Enter or → opens · Esc closes",
    }
}

/// The left: PIPELINE, each section with its summary joined by │ where
/// there is room, a rule, then the lines whose pages come later.
fn pipeline(s: &Screen, st: &Settings, height: u16, width: usize) -> Vec<Line<'static>> {
    let row = |name: &str, summary: String, mark: bool, selected: bool| {
        let (name_style, bg) = match (selected, st.open) {
            (true, false) => (bold(PURPLE), Some(SEL_BG)),
            (true, true) => (bold(TEXT), Some(REST_BG)),
            (false, _) => (fg(TEXT), None),
        };
        let room = width.saturating_sub(if mark { 14 } else { 12 });
        let mut spans = vec![
            Span::styled(if selected { "▸ " } else { "  " }, fg(PURPLE)),
            Span::styled(pad(name, 10), name_style),
            Span::styled(cut(&summary, room), fg(MUTED)),
        ];
        if mark {
            spans.push(Span::styled(" ✗", bold(RED)));
        }
        match bg {
            Some(bg) => filled(spans, width, bg),
            None => Line::from(spans),
        }
    };
    let mut lines = vec![Line::from(Span::styled("PIPELINE", bold(MUTED)))];
    for (i, (_, short, _)) in SECTIONS.iter().enumerate() {
        if i > 0 && height >= 17 {
            lines.push(Line::from(Span::styled("  │", fg(BORDER))));
        }
        lines.push(row(
            short,
            st.summary(i),
            st.checks(Some(i)).iter().any(|c| !c.holds),
            i == st.section,
        ));
    }
    lines.push(divider(width));
    let typesafe = on_off(st.typesafe(&s.cfg.api_key)).to_string();
    lines.push(row(
        "Apps",
        st.apps_summary(),
        false,
        st.section == APPS_PAGE,
    ));
    lines.push(row(
        "Skills",
        format!("{} installed", st.skills()),
        false,
        st.section == SKILLS_PAGE,
    ));
    lines.push(row(
        "Labels",
        st.labels_summary(),
        false,
        st.section == LABELS_PAGE,
    ));
    lines.push(row(
        "TypeSafe",
        typesafe,
        st.checks(Some(TYPESAFE_PAGE)).iter().any(|c| !c.holds),
        st.section == TYPESAFE_PAGE,
    ));
    lines.push(row(
        "Run",
        run_summary(st),
        st.checks(Some(RUN_PAGE)).iter().any(|c| !c.holds),
        st.section == RUN_PAGE,
    ));
    lines.push(row(
        "On call",
        on_off(s.on_call.token.is_some()).to_string(),
        false,
        st.section == ON_CALL_PAGE,
    ));
    lines
}

/// A setting's label on its section's page, padded; the toggle a checkbox.
fn label(st: &Settings, row: usize, field: Field) -> String {
    let split = st.split().is_some();
    match (field, ROWS[row].lead) {
        (Field::Same, _) => {
            let tick = if split { ' ' } else { 'x' };
            format!("[{tick}] Same model for plan and implementation")
        }
        (Field::Model, _) if row == 0 && split => pad("implement model", 22),
        (Field::Job(j), _) => pad(&job_name(j), 24),
        (_, "") => pad(field.name(), 22),
        (_, lead) => pad(&format!("{lead} {}", field.name()), 22),
    }
}

/// A setting's value: the App; the model and its family, or none; the
/// effort, or that the App has none.
fn value(st: &Settings, row: usize, field: Field) -> Vec<Span<'static>> {
    let v = st.value(row, field);
    let muted = |text: String| Span::styled(text, fg(MUTED));
    let shown = match v.as_str() {
        "default" | "none" => muted(v.clone()),
        _ => Span::styled(v.clone(), bold(TEXT)),
    };
    let app = st.app(row);
    match field {
        Field::Model if v == "none" => {
            vec![shown, muted("  no fallback".into())]
        }
        Field::Model | Field::Plan => match app {
            Some(app) => vec![shown, muted(format!("  {}", family_label(app, &v)))],
            None => vec![shown],
        },
        Field::Effort => match app {
            Some(app) if app.effort.is_empty() => {
                vec![muted(format!("— {} has no effort flag", app.name))]
            }
            _ => vec![shown],
        },
        Field::App => vec![shown],
        Field::Same | Field::Skills | Field::Template | Field::ExtraSkill => vec![],
        Field::Job(_) if v == NONE => {
            vec![shown, muted("  the Stage skill's own instructions".into())]
        }
        Field::Job(_) => match app.and_then(|app| st.have(app, &v)) {
            Some((_, _, detail)) => vec![shown, muted(format!("  {detail}"))],
            None => vec![shown, Span::styled("  not installed", fg(RED))],
        },
    }
}

/// The section's page: its title and Apps, its description, its rows'
/// settings a blank line apart; and the cursor's line.
fn page(st: &Settings, width: usize) -> (Vec<Line<'static>>, usize) {
    let (title, _, about) = SECTIONS[st.section];
    let items = st.items();
    let apps = distinct(items.iter().map(|&(row, _)| st.value(row, Field::App)));
    let mut lines = head(title, apps.join(", "), about, width);
    let mut at = 0;
    for (i, &(row, field)) in items.iter().enumerate() {
        if field == Field::App {
            lines.push(Line::default());
        }
        if matches!(field, Field::Job(_)) && !matches!(items[i - 1].1, Field::Job(_)) {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled("DELEGATE SKILLS", bold(MUTED))));
        }
        let selected = st.open && i == st.setting;
        item(
            &mut lines,
            &mut at,
            selected,
            label(st, row, field),
            value(st, row, field),
            width,
        );
    }
    check_lines(&mut lines, st.checks(Some(st.section)), width);
    (lines, at)
}

/// A page's CHECKS, each held or broken.
fn check_lines(lines: &mut Vec<Line<'static>>, checks: Vec<Check>, width: usize) {
    if !checks.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("CHECKS", bold(MUTED))));
    }
    for check in checks {
        let (mark, color, text) = if check.holds {
            ("✓", GREEN, MUTED)
        } else {
            ("✗", RED, RED)
        };
        let text = vec![(format!("{}.", check.text), fg(text))];
        let mark = format!("  {mark} ");
        lines.extend(wrap_spans(text, width, &mark, "    ", bold(color)));
    }
}

/// A pick list: its title and filter, its entries with the current one
/// marked; and the cursor's line.
fn pick_lines(st: &Settings, pick: &Pick, width: usize) -> (Vec<Line<'static>>, usize) {
    let row = &ROWS[pick.row];
    let on = match (pick.field, st.pick_app(pick)) {
        (Field::App | Field::Skills | Field::Template | Field::ExtraSkill, _) | (_, None) => {
            String::new()
        }
        (_, Some(app)) if pick.app.is_some() => format!(" · {} (new App)", app.name),
        (_, Some(app)) => format!(" · {}", app.name),
    };
    let label = st.label.as_deref().unwrap_or_default();
    let title = match (pick.field, pick.scope) {
        (Field::Job(j), _) => format!("{}{on}", job_said(j)),
        (Field::Skills, _) => format!("orqa:{label} skills"),
        (Field::Template, _) => format!("orqa:{label} PR template"),
        (Field::ExtraSkill, _) => format!("orqa:{label} extra review skill"),
        (field, Scope::Label) => format!(
            "orqa:{label} {} {}{on}",
            row.key.replace('_', " "),
            field.name()
        ),
        (field, Scope::Extra) => format!("orqa:{label} extra review {}{on}", field.name()),
        (field, Scope::Repo) => format!("{} {}{on}", row.name, field.name()),
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(title, bold(CYAN)),
        Span::styled("   filter › ", fg(MUTED)),
        Span::styled(pick.filter.clone(), fg(TEXT)),
        Span::styled("▏", fg(PURPLE)),
    ])];
    let entries = st.entries(pick);
    // a job's current pick is marked ' ✓' after its mark
    let (name_w, tick) = match pick.field {
        Field::App => (12, 1),
        Field::Job(_) | Field::Skills | Field::Template | Field::ExtraSkill => (24, 3),
        _ => (18, 1),
    };
    let marks = entries.iter().filter_map(|e| e.mark);
    let mark_w = marks.map(|m| m.0.chars().count() + tick).max().unwrap_or(0);
    let detail_w = width.saturating_sub(2 + name_w + mark_w.max(10)).min(44);
    let (mut at, mut n) = (0, 0);
    for e in entries {
        if e.picks.is_none() {
            lines.push(Line::from(Span::styled(
                format!("  {}", e.name),
                bold(MUTED),
            )));
            continue;
        }
        let selected = n == pick.cursor;
        n += 1;
        let spans = vec![
            Span::styled(if selected { "▸ " } else { "  " }, fg(PURPLE)),
            Span::styled(
                pad(&e.name, name_w),
                match (e.dim, selected) {
                    (true, _) => fg(MUTED),
                    (false, true) => bold(TEXT),
                    (false, false) => fg(TEXT),
                },
            ),
            Span::styled(pad(&e.detail, detail_w), fg(MUTED)),
            match e.mark {
                Some((mark, color)) if e.current => Span::styled(format!("{mark} ✓"), fg(color)),
                Some((mark, color)) => Span::styled(mark, fg(color)),
                None if e.current => Span::styled("✓ current", fg(GREEN)),
                None => Span::raw(""),
            },
        ];
        if selected {
            at = lines.len();
            lines.push(filled(spans, width, SEL_BG));
        } else {
            lines.push(Line::from(spans));
        }
    }
    if n == 0 {
        lines.push(Line::from(Span::styled("  nothing matches", fg(MUTED))));
    }
    (lines, at)
}

/// The foot's two lines: a probe going, the id being typed, the last
/// note, else the note on the setting under the cursor.
fn foot_lines(s: &Screen, st: &Settings, width: usize) -> Vec<Line<'static>> {
    if let Some(probe) = &st.probe {
        let spin = SPINNER[(s.ticks / 2) as usize % SPINNER.len()];
        let text = format!(
            "probing {} on {} with a one-line prompt…",
            probe.model, probe.app
        );
        return vec![Line::from(vec![
            Span::styled(format!("{spin} "), bold(ORANGE)),
            Span::styled(cut(&text, width.saturating_sub(2)), fg(ORANGE)),
        ])];
    }
    if let Some(busy) = &st.busy {
        let spin = SPINNER[(s.ticks / 2) as usize % SPINNER.len()];
        return vec![Line::from(vec![
            Span::styled(format!("{spin} "), bold(ORANGE)),
            Span::styled(cut(&busy.text, width.saturating_sub(2)), fg(ORANGE)),
        ])];
    }
    if let Some((text, _)) = &st.confirm {
        let mut lines = wrap_spans(
            vec![(text.clone(), bold(TEXT))],
            width.saturating_sub(5),
            "",
            "",
            bold(TEXT),
        );
        lines.truncate(2);
        if let Some(last) = lines.last_mut() {
            last.push_span(Span::styled("  y/n", bold(ORANGE)));
        }
        return lines;
    }
    if let Some((typing, text)) = &st.typing {
        let (prompt, shown, help) = match typing {
            Typing::Model(pick) => {
                let app = st.pick_app(pick).map_or("", |a| a.name);
                (
                    format!("{} model id › ", ROWS[pick.row].name),
                    text.clone(),
                    format!("A {app} model id: probed with a one-line prompt before it saves."),
                )
            }
            Typing::Source => (
                "source › ".to_string(),
                text.clone(),
                "owner/repo, owner/repo/path, or a git or GitHub URL (…/tree/<ref>/<path>): Orqadence clones it.".to_string(),
            ),
            Typing::Key => (
                "TypeSafe key › ".to_string(),
                "•".repeat(text.chars().count()),
                format!("Shown as dots; kept in {}, readable only by you.", setup::KEY_FILE),
            ),
            Typing::Floor(floor) => (
                format!("{} › ", floor_name(floor)),
                text.clone(),
                "A number from 0 to 1, or nothing for the default, saved at once, uncommitted, to .orqadence/config.json; the next Judgment reads it.".to_string(),
            ),
            Typing::MaxTickets => (
                "tickets at once › ".to_string(),
                text.clone(),
                "A whole number of at least 1, or nothing for the default, saved at once, uncommitted, to .orqadence/config.json; the live run's next pass reads it.".to_string(),
            ),
            Typing::Token => (
                "Moshi token › ".to_string(),
                "•".repeat(text.chars().count()),
                format!("Shown as dots; kept in {}, readable only by you; nothing clears it.", on_call::CONFIG),
            ),
            Typing::Minutes => (
                "minutes › ".to_string(),
                text.clone(),
                format!("A whole number of at least 1, or nothing for the default {}, saved at once; the next tick reads it.", on_call::DEFAULT_MINUTES),
            ),
            Typing::LabelName => (
                "label › ".to_string(),
                text.clone(),
                "The part after orqa:, as bd takes a label: no spaces or commas. Written at once as an area label, uncommitted, to .orqadence/config.json.".to_string(),
            ),
            Typing::Rename(old) => (
                format!("rename orqa:{old} › "),
                text.clone(),
                "The new name after orqa:. The entry moves with its PR template mapping; the template file keeps its name.".to_string(),
            ),
            Typing::Heading => (
                "section heading › ".to_string(),
                text.clone(),
                "The heading of this label's section, added after the default template in a new file; saved at once, uncommitted.".to_string(),
            ),
            Typing::Guidance => (
                "guidance › ".to_string(),
                text.clone(),
                "One line for its code-editing Stages; nothing clears it. Saved at once, uncommitted, to .orqadence/config.json.".to_string(),
            ),
        };
        let (help, color) = st.note.clone().unwrap_or((help, MUTED));
        return vec![
            Line::from(vec![
                Span::styled(prompt, bold(PURPLE)),
                Span::styled(shown, fg(TEXT)),
                Span::styled("▏", fg(PURPLE)),
            ]),
            Line::from(Span::styled(cut(&help, width), fg(color))),
        ];
    }
    let (text, color) = match (&st.note, &st.pick) {
        (Some((note, color)), _) => (note.clone(), *color),
        (None, _) if st.listing.is_some() => (
            "Space ticks a skill; Enter installs the ticked ones; Esc installs none.".to_string(),
            MUTED,
        ),
        (None, Some(pick)) => (st.note_of(pick.row, pick.field), MUTED),
        (None, None) if st.open && st.section == APPS_PAGE => (st.app_note(st.setting), MUTED),
        (None, None) if st.open && st.section == SKILLS_PAGE => (st.skills_note(st.setting), MUTED),
        (None, None) if st.open && st.section == LABELS_PAGE => (st.labels_note(), MUTED),
        (None, None) if st.open && st.section == TYPESAFE_PAGE => {
            (st.typesafe_note(st.setting), MUTED)
        }
        (None, None) if st.open && st.section == RUN_PAGE => (st.run_note(), MUTED),
        (None, None) if st.open && st.section == ON_CALL_PAGE => {
            (st.on_call_note(st.setting), MUTED)
        }
        (None, None) if st.open => {
            let (row, field) = st.items()[st.setting];
            (st.note_of(row, field), MUTED)
        }
        (None, None) => (
            "↑↓ picks a Stage; Enter opens it. Every change saves at once to .orqadence/config.json."
                .to_string(),
            MUTED,
        ),
    };
    let mut lines = wrap_spans(vec![(text, fg(color))], width, "", "", fg(color));
    lines.truncate(2);
    lines
}
