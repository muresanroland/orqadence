//! The modal a plan Question docks in (layout C of harness-0sx.5): the live
//! Shell keeps the left 42% and the plan takes the right 58%, its markdown
//! styled; under 110 columns it folds to a box over the dimmed Shell. A Wake
//! and a Stage's own question dock in the same frame (harness-crk), and so
//! do the approval modal, /manual-work, /brainstorm's idea modal and the
//! Tickets and start-Map modals charting opens.

use std::cell::Cell;
use std::fs;

use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Wrap,
};
use ratatui::Frame;

use super::{bold, cut, fg, shell};
use crate::orchestrator::judgment::{Action, WAITS};
use crate::orchestrator::manual;
use crate::orchestrator::stage::{plural, Ask};
use crate::shell::brand::{lerp, BLUE, BORDER, CYAN, GREEN, INK, MUTED, ORANGE, PURPLE, RED, TEXT};
use crate::shell::{About, NoticeKind, Question, Screen};

/// Under this many columns the dock folds over the Shell.
const FOLD: u16 = 110;
/// What the Shell's colors fade toward behind the fold.
const DIM_TO: Color = Color::Rgb(8, 12, 20);
/// The ground of code, and of a diff's added and removed lines.
const CODE_BG: Color = Color::Rgb(24, 31, 48);
const ADD_BG: Color = Color::Rgb(16, 46, 28);
const DEL_BG: Color = Color::Rgb(56, 20, 26);
/// The ground of a docked Wake's or Stage question's band.
const BAND: Color = Color::Rgb(34, 28, 58);

/// The frame: from 110 columns the Shell in the left 42% and a thick box in
/// the right 58%; under, the Shell dimmed behind a rounded box that leaves
/// it the input line (and a notice or the / or @ list, while one shows),
/// with margins from 100x30 up. The box and its border, which the caller
/// titles and renders.
pub(super) fn dock(f: &mut Frame, s: &Screen) -> (Rect, Block<'static>) {
    let area = f.area();
    let (rect, border) = if area.width >= FOLD {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
                .areas(area);
        shell(f, left, s);
        (right, BorderType::Thick)
    } else {
        let keep = shell(f, area, s);
        let above = Rect {
            height: keep - area.y,
            ..area
        };
        let buf = f.buffer_mut();
        for y in above.top()..above.bottom() {
            for x in above.left()..above.right() {
                let cell = &mut buf[(x, y)];
                cell.fg = lerp((cell.fg, DIM_TO), 0.7);
            }
        }
        let rect = if area.width < 100 || area.height < 30 {
            above
        } else {
            let rect = area.inner(Margin::new(area.width / 12, 2));
            Rect {
                height: rect.height.min(keep.saturating_sub(rect.y)),
                ..rect
            }
        };
        (rect, BorderType::Rounded)
    };
    s.dock_area.set(rect);
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(border)
        .border_style(fg(PURPLE))
        .padding(Padding::horizontal(1));
    (rect, block)
}

/// A Notice modal is this many columns wide, wider only for a longer word
/// (a URL) so it stays on one row: it fits 80.
const NOTICE_W: u16 = 60;

/// The front Notice modal over whatever else shows: a box centred over a
/// Clear, its border and title red and ' ERROR ' or green and ' NOTICE ';
/// the message wrapped whole to the box, which is as tall as it up to the
/// screen, a longer one from its scroll row; one [ OK ], focused, or
/// [ Yes ] and [ No ], the cursor's focused; at its foot the time left
/// while it closes by itself.
pub(super) fn notice(f: &mut Frame, s: &Screen) {
    let n = &s.notices[0];
    let (title, c) = match n.kind {
        NoticeKind::Error => (" ERROR ", RED),
        NoticeKind::Info => (" NOTICE ", GREEN),
    };
    let area = f.area();
    let longest = n.text.split_whitespace().map(|w| w.chars().count()).max();
    let width = NOTICE_W
        .max(longest.unwrap_or(0) as u16 + 4)
        .min(area.width);
    // The border and padding take 4 columns; the border, a blank row and
    // the button 4 rows.
    let rows: Vec<Line> = n
        .text
        .lines()
        .flat_map(|line| {
            wrap_spans(
                vec![(line.to_string(), fg(TEXT))],
                width.saturating_sub(4) as usize,
                "",
                "",
                fg(TEXT),
            )
        })
        .collect();
    let height = (rows.len() as u16 + 4).min(area.height);
    let shown = height.saturating_sub(4) as usize;
    let mut foot = Vec::new();
    if rows.len() > shown {
        foot.push("↑↓ scrolls".to_string());
    }
    if let Some(closes) = n.closes {
        let ms = (closes - (s.cfg.clock)()).num_milliseconds().max(0);
        foot.push(format!("closes in {}s", (ms + 999) / 1000));
    }
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(c))
        .padding(Padding::horizontal(1))
        .title(Span::styled(title, bold(c)));
    if !foot.is_empty() {
        let foot = format!(" {} ", foot.join(" · "));
        block = block.title_bottom(Span::styled(foot, fg(MUTED)));
    }
    let rect = area.centered(Constraint::Length(width), Constraint::Length(height));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    let [text, _, button] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let from = n.scroll.get().min(rows.len().saturating_sub(shown));
    n.scroll.set(from);
    let rows: Vec<Line> = rows.into_iter().skip(from).take(shown).collect();
    f.render_widget(Paragraph::new(rows), text);
    let button_at = |label: &'static str, on: bool| match on {
        true => Span::styled(label, bold(Color::Black).bg(c)),
        false => Span::styled(label, fg(TEXT)),
    };
    let buttons = match n.yes {
        Some(_) => vec![
            button_at("[ Yes ]", n.cursor == 0),
            Span::raw("  "),
            button_at("[ No ]", n.cursor == 1),
        ],
        None => vec![button_at("[ OK ]", true)],
    };
    f.render_widget(Line::from(buttons).centered(), button);
}

/// The front approval modal in the dock: its PR, the other PRs waiting,
/// one row per item (the cursor's marked, scrolled into sight) with its
/// checkbox, summary and rating, then [ Fix comments ] and [ Cancel ]; at
/// its foot the keys and, while it runs, the countdown.
pub(super) fn approval(f: &mut Frame, s: &Screen) {
    let a = &s.approvals[0];
    let (rect, block) = dock(f, s);
    let mut foot = "↑↓ Space toggles · Enter fixes · Esc cancels".to_string();
    if let Some(at) = a.approves {
        let secs = ((at - (s.cfg.clock)()).num_milliseconds().max(0) + 999) / 1000;
        foot += &format!(" · approves in {}:{:02}", secs / 60, secs % 60);
    }
    let title = format!(" PR COMMENTS · {} ", s.name(&a.ticket));
    let block = block
        .title(Span::styled(title, bold(TEXT)))
        .title_bottom(Span::styled(format!(" {foot} "), fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let [head, _, body, rule, buttons] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let mut badges = vec![
        Span::styled(a.pr.clone(), fg(TEXT)),
        Span::styled(plural(a.rows.len(), "PR comment"), fg(TEXT)),
    ];
    if s.approvals.len() > 1 {
        let waiting = format!("{} waiting", plural(s.approvals.len() - 1, "more PR"));
        badges.push(Span::styled(waiting, bold(ORANGE)));
    }
    f.render_widget(Line::from(joined(badges)), head);

    let w = body.width as usize;
    let from = a
        .cursor
        .saturating_sub((body.height as usize).saturating_sub(1));
    let rows: Vec<Line> = (a.rows.iter().enumerate().skip(from))
        .map(|(i, (item, on))| {
            let (mark, style) = match i == a.cursor {
                true => ("›", bold(PURPLE)),
                false => (" ", fg(TEXT)),
            };
            let check = if *on { "[x]" } else { "[ ]" };
            let rating = format!("  {}", item.rating);
            let room = w.saturating_sub(rating.chars().count());
            let text = cut(&format!("{mark} {check} {}", item.summary), room);
            Line::from(vec![
                Span::styled(format!("{text:<room$}"), style),
                Span::styled(rating, fg(MUTED)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(rows), body);
    f.render_widget(divider(rule.width as usize), rule);
    let line = Line::from(vec![
        Span::styled("[ Fix comments ]", bold(Color::Black).bg(GREEN)),
        Span::raw("  "),
        Span::styled("[ Cancel ]", bold(Color::Black).bg(RED)),
    ]);
    f.render_widget(line, buttons);
}

/// The /manual-work modal in the dock: one row per open item (the
/// cursor's marked, scrolled into sight) with its checkbox, Ticket, What and
/// folder, a blocking one's box [-] and "blocking" after it; the cursor
/// row's whole folder, cut at the width; then [ Mark done ] and [ Close ]; the keys
/// at its foot.
pub(super) fn manual_work(f: &mut Frame, s: &Screen) {
    let Some(m) = &s.manual_work else {
        return;
    };
    let (rect, block) = dock(f, s);
    let foot = " ↑↓ Space checks · Enter marks done · Esc closes ";
    let block = block
        .title(Span::styled(" MANUAL WORK ", bold(TEXT)))
        .title_bottom(Span::styled(foot, fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let shown = |folder: &std::path::Path| {
        let folder = folder.strip_prefix(&s.cfg.repo).unwrap_or(folder);
        folder.display().to_string()
    };
    // the row cuts its folder first, so the cursor's shows whole under them
    let detail: Vec<char> = format!("Folder: {}", shown(&m.rows[m.cursor].1.folder))
        .chars()
        .collect();
    // cut at the width, not on words: a path has no spaces to wrap at
    let lines: Vec<Line> = (detail.chunks(inner.width.max(1) as usize))
        .map(|piece| Line::from(Span::styled(String::from_iter(piece), fg(MUTED))))
        .collect();
    let [head, _, body, folder, rule, buttons] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(lines.len() as u16),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let open = Span::styled(format!("{} open", m.rows.len()), fg(TEXT));
    f.render_widget(Line::from(open), head);

    let w = body.width as usize;
    let from = m
        .cursor
        .saturating_sub((body.height as usize).saturating_sub(1));
    let rows: Vec<Line> = (m.rows.iter().enumerate().skip(from))
        .map(|(i, (id, item, on))| {
            let (mark, style) = match i == m.cursor {
                true => ("›", bold(PURPLE)),
                false => (" ", fg(TEXT)),
            };
            let (check, blocking) = match (item.blocks, on) {
                (true, _) => ("[-]", "  blocking"),
                (false, true) => ("[x]", ""),
                (false, false) => ("[ ]", ""),
            };
            let room = w.saturating_sub(blocking.chars().count());
            let text = format!(
                "{mark} {check} {id}  {}  {}",
                item.what,
                shown(&item.folder)
            );
            Line::from(vec![
                Span::styled(cut(&text, room), style),
                Span::styled(blocking, fg(ORANGE)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(rows), body);
    f.render_widget(Paragraph::new(lines), folder);
    f.render_widget(divider(rule.width as usize), rule);
    let line = Line::from(vec![
        Span::styled("[ Mark done ]", bold(Color::Black).bg(GREEN)),
        Span::raw("  "),
        Span::styled("[ Close ]", bold(Color::Black).bg(RED)),
    ]);
    f.render_widget(line, buttons);
}

/// A dock's badges joined by a muted " · ".
pub(super) fn joined(badges: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, badge) in badges.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", fg(MUTED)));
        }
        spans.push(badge);
    }
    spans
}

/// A dock's rule across `width`.
pub(super) fn divider(width: usize) -> Line<'static> {
    Line::from(Span::styled("─".repeat(width), fg(BORDER)))
}

/// The plan Question in the dock: its badges (the other Questions, the
/// lines since it opened), its text, the Judgment's scores, the plan from its
/// scroll row with a scrollbar, and its options at the foot (docked a list,
/// folded one row), or the feedback being typed in that option's place.
pub(super) fn plan(f: &mut Frame, s: &Screen) {
    let q = &s.questions[0];
    let About::Asked(Ask::Plan { plan, judged, .. }) = &q.about else {
        return;
    };
    let width = f.area().width;
    let (rect, block) = dock(f, s);
    let options = s.options();
    let n = options.len();
    let hint = match (s.composing, rect.width < 90) {
        (true, _) => "Enter sends the feedback, Esc goes back".to_string(),
        (false, false) => format!(
            "↑↓ PgUp PgDn scroll · Tab heading · ←→ or 1-{n} pick · Enter answers · Esc hides"
        ),
        (false, true) => format!("↑↓ PgUp PgDn · Tab · ←→ 1-{n} · Enter · Esc"),
    };
    let title = format!(
        " PLAN · {} ",
        s.name(q.ticket.as_deref().unwrap_or_default())
    );
    let block = block
        .title(Span::styled(title, bold(TEXT)))
        .title_bottom(Span::styled(format!(" {hint} "), fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let w = inner.width as usize;
    let judged = judged.map_or(Vec::new(), |judged| {
        judged_rows(judged.said(), judged.short(), w)
    });
    let folded = width < FOLD;
    let foot = if folded { 1 } else { n as u16 };
    let [head, body, rule, foot] = Layout::vertical([
        Constraint::Length(2 + judged.len().max(1) as u16),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(foot),
    ])
    .areas(inner);
    let lines: Vec<Line> = heading(s, q, w).into_iter().chain(judged).collect();
    f.render_widget(Paragraph::new(lines), head);

    // The plan from its scroll row, kept inside; the last column is the
    // scrollbar's.
    let (rows, heads) = md(plan, body.width.saturating_sub(1) as usize);
    let (total, h) = (rows.len(), body.height as usize);
    let text = Rect {
        width: body.width.saturating_sub(1),
        ..body
    };
    let from = scrolled(f, s, rows, heads, &q.scroll, text);
    if total > h {
        let mut state = ScrollbarState::new(total - h).position(from);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_style(fg(MUTED))
            .track_style(fg(BORDER));
        f.render_stateful_widget(bar, body, &mut state);
    }
    f.render_widget(divider(rule.width as usize), rule);

    // The feedback being typed shows its tail up to the cursor.
    let w = foot.width as usize;
    let composed = || {
        let room = w.saturating_sub(12);
        let (before, after) = s.input.split_at(s.at());
        let typed = before
            .chars()
            .skip(before.chars().count().saturating_sub(room));
        Line::from(vec![
            Span::styled("feedback › ", bold(PURPLE)),
            Span::styled(typed.collect::<String>(), fg(TEXT)),
            Span::styled("▌", fg(TEXT)),
            Span::styled(after, fg(TEXT)),
        ])
    };
    // Folded, one row: where it does not fit, each option's first word.
    let row = |short: bool| {
        let mut spans = Vec::new();
        for (i, option) in options.iter().enumerate() {
            let option = match short {
                true => option.split(' ').next().unwrap_or_default(),
                false => option.as_str(),
            };
            let style = match i == q.cursor {
                true => bold(Color::Black).bg(PURPLE),
                false => fg(TEXT),
            };
            spans.push(Span::styled(format!(" {} {option} ", i + 1), style));
            spans.push(Span::raw(" "));
        }
        Line::from(spans)
    };
    let lines: Vec<Line> = if folded && s.composing {
        vec![composed()]
    } else if folded {
        match row(false) {
            line if line.width() > w => vec![row(true)],
            line => vec![line],
        }
    } else {
        options
            .iter()
            .enumerate()
            .map(|(i, option)| match s.composing && i == 1 {
                true => composed(),
                false => option_row(i, option, i == q.cursor, w),
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines), foot);
}

/// A docked Question's first two lines: its badges (the other Questions,
/// the lines since it first showed), shortened where the long ones do not
/// fit in `w`, and its text.
fn heading(s: &Screen, q: &Question, w: usize) -> [Line<'static>; 2] {
    let opened = *q.opened.get_or_init(chrono::Local::now);
    let new = s.events.iter().filter(|e| e.time >= opened).count();
    let badges = |short: bool| {
        let mut badges = Vec::new();
        if s.questions.len() > 1 {
            let text = format!("{} more waiting", s.questions.len() - 1);
            badges.push(Span::styled(text, bold(ORANGE)));
        }
        if new > 0 {
            let text = match short {
                true => format!("{new} new"),
                false => format!("{new} new on RECENT"),
            };
            badges.push(Span::styled(text, fg(CYAN)));
        }
        Line::from(joined(badges))
    };
    let badge_line = match badges(false) {
        line if line.width() > w => badges(true),
        line => line,
    };
    [
        badge_line,
        Line::from(Span::styled(cut(&q.text, w), fg(MUTED))),
    ]
}

/// A Judgment's lines under the text, each score named: `said` whole where
/// it fits in `w`, else `short`, wrapped at a score where that does not fit
/// either.
fn judged_rows(said: String, short: String, w: usize) -> Vec<Line<'static>> {
    let rows = match format!("judged: {said}") {
        text if text.chars().count() <= w => vec![text],
        _ => short.split(", ").fold(vec![], |mut rows, score| {
            match rows.last_mut() {
                None => rows.push(format!("judged: {score}")),
                Some(row) if row.len() + 2 + score.len() <= w => *row = format!("{row}, {score}"),
                Some(_) => rows.push(score.to_string()),
            }
            rows
        }),
    };
    rows.into_iter()
        .map(|text| Line::from(Span::styled(text, fg(TEXT))))
        .collect()
}

/// A docked option's row, cut to `w`: "› 1. approve" in bold purple under
/// the cursor, "  2. park" otherwise.
fn option_row(i: usize, option: &str, on: bool, w: usize) -> Line<'static> {
    let (mark, style) = if on {
        ("›", bold(PURPLE))
    } else {
        (" ", fg(TEXT))
    };
    Line::from(Span::styled(
        cut(&format!("{mark} {}. {option}", i + 1), w),
        style,
    ))
}

/// A Wake, a Stage's own question or Manual work in the dock (the A+C mix
/// of harness-crk): its badges, text, the Judgment's scores and the facts;
/// the pane's last lines, or the question or the item's What, Why, How and
/// folder from its start, in a box that gives up
/// its rows first; the band of what the option under the cursor sends word
/// for word or does, or the prompt being typed; then the options.
pub(super) fn asked(f: &mut Frame, s: &Screen) {
    let q = &s.questions[0];
    let item: String;
    let (kind, pane, text, wake) = match &q.about {
        About::Asked(Ask::Wake { pane, tail, .. }) => ("WAKE", pane.as_str(), tail, true),
        About::Asked(Ask::StageQuestion { pane, question, .. }) => {
            ("QUESTION", pane.as_str(), question, false)
        }
        // the item's own sections, under their headings
        About::Asked(Ask::Manual {
            pane, item: work, ..
        }) => {
            let folder = work
                .folder
                .strip_prefix(&s.cfg.repo)
                .unwrap_or(&work.folder);
            item = format!(
                "What\n{}\n\nWhy\n{}\n\nHow\n{}\n\nFolder: {}",
                work.what,
                work.why,
                work.how,
                folder.display()
            );
            ("MANUAL WORK", pane.as_str(), &item, false)
        }
        About::Asked(Ask::Labels { .. }) => ("QUESTION", "", &q.text, false),
        About::Asked(Ask::Merge { open, .. }) => ("QUESTION", "", open, false),
        _ => return,
    };
    // where the session is, as the Question's line names it: "(pane 2-1)"
    let at = q
        .text
        .rsplit_once("(pane ")
        .and_then(|(_, at)| at.strip_suffix(')'))
        .unwrap_or(pane);
    let (rect, block) = dock(f, s);
    let options = s.options();
    let n = options.len();
    let long = format!("↑↓ or 1-{n} pick · PgUp PgDn scroll · Enter answers · Esc hides");
    let hint = match (s.composing, long.chars().count() + 4 > rect.width as usize) {
        (true, _) => "Enter sends it, Esc goes back".to_string(),
        (false, false) => long,
        (false, true) => format!("↑↓ 1-{n} · PgUp PgDn · Enter · Esc"),
    };
    let name = s.name(q.ticket.as_deref().unwrap_or_default());
    let title = match &q.about {
        About::Asked(Ask::Manual { stage, .. }) => format!(" {kind} · {name} {stage} · blocks "),
        _ => format!(" {kind} · {name} "),
    };
    let block = block
        .title(Span::styled(title, bold(TEXT)))
        .title_bottom(Span::styled(format!(" {hint} "), fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let w = inner.width as usize;
    // no badges, no row for them: rows are few here
    let mut head: Vec<Line> = heading(s, q, w).into();
    head.retain(|line| line.width() > 0);
    if let About::Asked(Ask::Wake {
        judged: Some(judged),
        ..
    }) = &q.about
    {
        head.extend(judged_rows(judged.said(), judged.said(), w));
    }
    head.push(Line::from(Span::styled(
        cut(&facts(s, q, at), w),
        fg(MUTED),
    )));
    let (sends, said) = sends(s, q, at);
    let band = wrap_spans(
        vec![(format!("{sends}: "), fg(MUTED)), (said, fg(TEXT))],
        w,
        "› ",
        "  ",
        bold(PURPLE),
    );
    let [head_a, body, band_a, rule, foot] = Layout::vertical([
        Constraint::Length(head.len() as u16),
        // the box gives up its rows first, the band and the options never
        Constraint::Min(0),
        Constraint::Length(band.len() as u16),
        Constraint::Length(1),
        Constraint::Length(n as u16),
    ])
    .areas(inner);
    f.render_widget(Paragraph::new(head), head_a);

    let title = match &q.about {
        _ if wake => format!(" pane {at}, its last lines "),
        About::Asked(Ask::Labels { .. }) => " the Ticket's labels ".to_string(),
        About::Asked(Ask::Merge { .. }) => " still open ".to_string(),
        About::Asked(Ask::Manual { .. }) => " what to do ".to_string(),
        _ => " the session asks ".to_string(),
    };
    let frame = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(BORDER))
        .padding(Padding::horizontal(1))
        .title(Span::styled(title, fg(MUTED)));
    let inside = frame.inner(body);
    f.render_widget(frame, body);
    let rows: Vec<Line> = text
        .trim_end()
        .lines()
        .flat_map(|line| {
            let body = line.trim_start();
            let pad = &line[..line.len() - body.len()];
            wrap_spans(
                vec![(body.to_string(), fg(TEXT))],
                inside.width as usize,
                pad,
                pad,
                fg(TEXT),
            )
        })
        .collect();
    // A Wake's tail from its last line, `scroll` rows up; a Stage's
    // question from its first, `scroll` rows down.
    let h = inside.height as usize;
    s.page.set(h.saturating_sub(2).max(1));
    let max = rows.len().saturating_sub(h);
    let row = q.scroll.get().min(max);
    q.scroll.set(row);
    let from = if wake { max - row } else { row };
    let shown: Vec<Line> = rows.into_iter().skip(from).take(h).collect();
    f.render_widget(Paragraph::new(shown), inside);

    f.render_widget(
        Paragraph::new(band).style(Style::default().bg(BAND)),
        band_a,
    );
    f.render_widget(divider(w), rule);
    let rows: Vec<Line> = options
        .iter()
        .enumerate()
        .map(|(i, option)| option_row(i, option, i == q.cursor, w))
        .collect();
    f.render_widget(Paragraph::new(rows), foot);
}

/// The facts strip, from what the Orchestrator already knows: the Stage and
/// Round and when it asked; for a Wake the result file and its state, and
/// what is left for the session; for a Stage's question that its session
/// waits in pane `at`.
fn facts(s: &Screen, q: &Question, at: &str) -> String {
    let id = q.ticket.as_deref().unwrap_or_default();
    let ts = s.state.tickets.get(id).cloned().unwrap_or_default();
    let stage = match ts.round {
        0 => ts.stage.clone(),
        round => format!("{}, round {round}", ts.stage),
    };
    let mut facts = vec![stage, format!("asked {}", q.asked.format("%H:%M:%S"))];
    match &q.about {
        About::Asked(Ask::Wake { file, actions, .. }) => {
            let path = file.strip_prefix(&s.cfg.repo).unwrap_or(file).display();
            let state = match fs::read_to_string(file) {
                Err(_) => "is missing".to_string(),
                Ok(body) => match body.lines().next().map(str::trim) {
                    Some(first) if first.starts_with("STATUS:") => format!("says {first}"),
                    _ => "was written without the STATUS line first".to_string(),
                },
            };
            facts.push(format!("result file {path} {state}"));
            let mut left = vec![];
            if actions.iter().any(|a| a.is_nudge()) {
                left.push("a nudge".to_string());
            }
            if actions.contains(&Action::Retry) {
                left.push("a retry".to_string());
            }
            if actions.contains(&Action::Wait) {
                left.push(plural(WAITS.saturating_sub(ts.waits), "wait"));
            }
            if !left.is_empty() {
                facts.push(format!("left for this session: {}", left.join(", ")));
            }
        }
        About::Asked(Ask::Labels { .. }) => {
            facts.push("no session yet: the Ticket waits for your answer".to_string())
        }
        About::Asked(Ask::Merge { .. }) => {
            facts.push("no session: its PR waits for your answer".to_string())
        }
        _ => facts.push(format!("the session in pane {at} waits for your answer")),
    }
    facts.retain(|fact| !fact.is_empty());
    facts.join(" · ")
}

/// What the option under the cursor sends to the session in pane `at`, word
/// for word, or does: its title and its text; your own, as typed so far.
fn sends(s: &Screen, q: &Question, at: &str) -> (String, String) {
    let id = q.ticket.as_deref().unwrap_or_default();
    let word = format!("sends to pane {at}, word for word");
    let does = |text: String| ("does".to_string(), text);
    let open = || {
        does(format!(
            "focuses pane {at} in herdr; this Question stays open"
        ))
    };
    let park = || {
        does(format!(
            "parks the Ticket where it is, pane {at} left open; /continue @{id} picks it up again"
        ))
    };
    let own = || {
        let text = match s.composing {
            true => {
                let (before, after) = s.input.split_at(s.at());
                format!("{before}▌{after}")
            }
            false => "what you type next; Enter starts typing".to_string(),
        };
        (format!("sends to pane {at}"), text)
    };
    match &q.about {
        About::Asked(Ask::Wake { actions, file, .. }) => match actions.get(q.cursor) {
            Some(Action::Retry) => does(format!(
                "closes pane {at} and starts a fresh session on the Stage's prompt; spends the Stage's one retry"
            )),
            Some(Action::Park) => park(),
            Some(Action::Wait) => {
                does("leaves the session alone ten minutes, then looks again".to_string())
            }
            Some(nudge) => (word, nudge.nudge(file).unwrap_or_default()),
            None if q.cursor == actions.len() => open(),
            None => own(),
        },
        About::Asked(Ask::StageQuestion { options, .. }) => {
            match (options.get(q.cursor), q.cursor.saturating_sub(options.len())) {
                (Some(option), _) => (word, option.clone()),
                (None, 0) => own(),
                (None, 1) => open(),
                (None, _) => park(),
            }
        }
        About::Asked(Ask::Manual { item, .. }) => {
            let done = manual::done_prompt(&item.folder, "");
            match q.cursor {
                0 => (word, done),
                1 if s.composing => {
                    let (sends, typed) = own();
                    (sends, format!("{done}{typed}"))
                }
                1 => own(),
                _ => park(),
            }
        }
        About::Asked(Ask::Labels { options }) => match options.get(q.cursor) {
            Some(_) if q.cursor + 1 == options.len() => does(format!(
                "parks the Ticket where it is; /continue @{id} asks again"
            )),
            Some(option) => match option.strip_prefix("remove ") {
                Some(label) => does(format!("runs bd label remove {id} {label}")),
                None => does(format!(
                    "keeps {option}: runs bd label remove {id} for each other label asked about"
                )),
            },
            None => (String::new(), String::new()),
        },
        About::Asked(Ask::Merge { options, .. }) => {
            match options.get(q.cursor).map(String::as_str) {
                Some("merge") => does(
                    "merges the PR with these still open, once its checks are green and it is mergeable"
                        .to_string(),
                ),
                Some("park") => does(format!(
                    "parks the Ticket, its PR left open; /continue @{id} asks again"
                )),
                Some(_) => {
                    does("waits another bot_wait for the review, then asks again".to_string())
                }
                None => (String::new(), String::new()),
            }
        }
        _ => (String::new(), String::new()),
    }
}

/// The rows in `area` from the scroll row, kept inside; the page and the
/// heads left for Screen::scroll_rows. The first row shown.
pub(super) fn scrolled(
    f: &mut Frame,
    s: &Screen,
    rows: Vec<Line>,
    heads: Vec<usize>,
    scroll: &Cell<usize>,
    area: Rect,
) -> usize {
    let h = area.height as usize;
    s.page.set(h.saturating_sub(2).max(1));
    *s.heads.borrow_mut() = heads;
    let from = scroll.get().min(rows.len().saturating_sub(h));
    scroll.set(from);
    let shown: Vec<Line> = rows.into_iter().skip(from).take(h).collect();
    f.render_widget(Paragraph::new(shown), area);
    from
}

/// The plan's markdown as rows `width` wide, and the rows its headings
/// start on: # purple and underlined, ## cyan, ### bold; - and * bullets
/// (• and, nested, ◦) and numbered items hanging under their text; >
/// quotes muted italic; `code` orange on a tint and **bold** inline; fenced
/// code on a tinted ground, a diff's (or an unlabelled fence's) + lines
/// green, - lines red, @@ cyan.
// ponytail: the subset plans use; no tables, links, ~~~ fences or italics.
fn md(text: &str, width: usize) -> (Vec<Line<'static>>, Vec<usize>) {
    let width = width.max(12);
    let (mut rows, mut heads) = (Vec::new(), Vec::new());
    // Inside a fence: whether it colors a diff.
    let mut fence = None;
    for line in text.lines() {
        let line = line.trim_end();
        let body = line.trim_start();
        if let Some(label) = body.strip_prefix("```") {
            fence = match fence {
                None => Some(label.is_empty() || label == "diff"),
                Some(_) => None,
            };
            let label = if fence.is_some() { label } else { "" };
            rows.push(Line::from(Span::styled(
                format!(" {label:<w$}", w = width - 1),
                fg(MUTED).bg(CODE_BG),
            )));
            continue;
        }
        if let Some(diff) = fence {
            let (c, bg) = match line {
                _ if diff && line.starts_with("@@") => (CYAN, CODE_BG),
                _ if diff && line.starts_with('+') => (GREEN, ADD_BG),
                _ if diff && line.starts_with('-') => (RED, DEL_BG),
                _ => (TEXT, CODE_BG),
            };
            let chars: Vec<char> = line.chars().collect();
            let pieces = chars.chunks(width - 1).map(String::from_iter);
            for piece in pieces.chain(chars.is_empty().then(String::new)) {
                rows.push(Line::from(Span::styled(
                    format!(" {piece:<w$}", w = width - 1),
                    fg(c).bg(bg),
                )));
            }
            continue;
        }
        if body.is_empty() {
            rows.push(Line::default());
            continue;
        }
        let level = body.chars().take_while(|c| *c == '#').count();
        if (1..=3).contains(&level) && body[level..].starts_with(' ') {
            heads.push(rows.len());
            let style = [
                bold(PURPLE).add_modifier(Modifier::UNDERLINED),
                bold(CYAN),
                bold(TEXT),
            ][level - 1];
            let text = inline(&body[level + 1..], style);
            rows.extend(wrap_spans(text, width, "", "", style));
            continue;
        }
        let pad = " ".repeat(line.len() - body.len());
        let numbered = body
            .split_once(". ")
            .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        let (first, hang, text, base, lead) =
            if let Some(item) = body.strip_prefix("- ").or(body.strip_prefix("* ")) {
                let bullet = if pad.is_empty() { "• " } else { "◦ " };
                let hang = format!("{pad}  ");
                (format!("{pad}{bullet}"), hang, item, fg(TEXT), fg(MUTED))
            } else if let Some((n, item)) = numbered {
                let first = format!("{pad}{n}. ");
                let hang = " ".repeat(first.chars().count());
                (first, hang, item, fg(TEXT), fg(MUTED))
            } else if let Some(quote) = body.strip_prefix("> ") {
                let italic = fg(MUTED).add_modifier(Modifier::ITALIC);
                (
                    "│ ".to_string(),
                    "│ ".to_string(),
                    quote,
                    italic,
                    fg(ORANGE),
                )
            } else {
                (pad.clone(), pad, body, fg(TEXT), fg(TEXT))
            };
        rows.extend(wrap_spans(inline(text, base), width, &first, &hang, lead));
    }
    (rows, heads)
}

/// A line's `code` and **bold** as styled pieces over `base`.
fn inline(text: &str, base: Style) -> Vec<(String, Style)> {
    let style = |code: bool, strong: bool| match (code, strong) {
        (true, _) => fg(ORANGE).bg(CODE_BG),
        (false, true) => base.add_modifier(Modifier::BOLD),
        (false, false) => base,
    };
    let mut pieces = Vec::new();
    let (mut piece, mut code, mut strong) = (String::new(), false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' || (c == '*' && !code && chars.next_if_eq(&'*').is_some()) {
            pieces.push((std::mem::take(&mut piece), style(code, strong)));
            if c == '`' {
                code = !code;
            } else {
                strong = !strong;
            }
        } else {
            piece.push(c);
        }
    }
    pieces.push((piece, style(code, strong)));
    pieces.retain(|(text, _)| !text.is_empty());
    pieces
}

/// Word-wraps styled pieces to `width`, `first` leading the first row and
/// `hang` the rest, both in `lead`; a word longer than the row is cut at
/// the row's end and goes on under it, as fenced code does. Widths are
/// terminal columns, so a wide character takes two.
pub(super) fn wrap_spans(
    pieces: Vec<(String, Style)>,
    width: usize,
    first: &str,
    hang: &str,
    lead: Style,
) -> Vec<Line<'static>> {
    let cols = |s: &str| Span::raw(s).width();
    let mut lines = Vec::new();
    let mut row = vec![Span::styled(first.to_string(), lead)];
    let (mut used, mut fresh) = (cols(first), true);
    for (text, style) in pieces {
        for mut word in text.split_inclusive(' ') {
            loop {
                if !fresh && used + cols(word.trim_end()) > width {
                    let next = vec![Span::styled(hang.to_string(), lead)];
                    lines.push(Line::from(std::mem::replace(&mut row, next)));
                    used = cols(hang);
                    fresh = true;
                }
                if fresh {
                    word = word.trim_start();
                }
                if word.is_empty() {
                    break;
                }
                let room = width.saturating_sub(used).max(1);
                let at = match cols(word.trim_end()) > room {
                    true => {
                        // Cut before the character that overflows the row,
                        // measuring the whole prefix as the renderer does (❤
                        // takes one column, ❤️ two), and back over zero-width
                        // marks so ❤ keeps its U+FE0F. Take at least one
                        // character so a narrow row still moves on.
                        let one = word.chars().next().map_or(0, char::len_utf8);
                        let mut at = word
                            .char_indices()
                            .find(|&(i, c)| cols(&word[..i + c.len_utf8()]) > room)
                            .map_or(word.len(), |(i, _)| i);
                        while word[at..]
                            .chars()
                            .next()
                            .is_some_and(|c| at > 0 && cols(c.encode_utf8(&mut [0; 4])) == 0)
                        {
                            at = word[..at].char_indices().last().map_or(0, |(i, _)| i);
                        }
                        at.max(one)
                    }
                    false => word.len(),
                };
                let (piece, rest) = word.split_at(at);
                used += cols(piece);
                row.push(Span::styled(piece.to_string(), style));
                fresh = false;
                word = rest;
            }
        }
    }
    lines.push(Line::from(row));
    lines
}

/// /brainstorm's idea modal in the dock: the question, the text in a
/// rounded box ten rows tall (wrapped, each newline a row, its last rows
/// shown, the cursor at its end while focused), the Ctrl+G line, Start
/// and Cancel filled, the focused one bright with ›, and the foot line; the
/// keys at its foot.
pub(super) fn idea(f: &mut Frame, s: &Screen) {
    let Some(idea) = &s.idea else {
        return;
    };
    let (rect, block) = dock(f, s);
    let foot = " Enter starts · Tab moves · Ctrl+J a new line · Esc cancels ";
    let block = block
        .title(Span::styled(" BRAINSTORM · a new idea ", bold(PURPLE)))
        .title_bottom(Span::styled(foot, fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [_, ask, _, input, keys, _, buttons, _, about] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(10),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);
    let ask_line = Span::styled("What is the idea? Paste or write it here", bold(TEXT));
    f.render_widget(Line::from(ask_line), ask);

    let edit = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(if idea.focus == 0 { PURPLE } else { BORDER }))
        .padding(Padding::horizontal(1));
    let room = edit.inner(input);
    let cursor = if idea.focus == 0 { "▌" } else { "" };
    let text = format!("{}{cursor}", idea.text);
    let rows: Vec<Line> = text
        .split('\n')
        .flat_map(|l| {
            let piece = vec![(l.to_string(), fg(TEXT))];
            wrap_spans(piece, room.width as usize, "", "", fg(TEXT))
        })
        .collect();
    let from = rows.len().saturating_sub(room.height as usize);
    let rows: Vec<Line> = rows.into_iter().skip(from).collect();
    f.render_widget(Paragraph::new(rows).block(edit), input);

    let muted = |t: &str| Line::from(Span::styled(t.to_string(), fg(MUTED)));
    let editor = "Ctrl+G opens it in your editor ($VISUAL, $EDITOR, else code -w, vi, nano)";
    f.render_widget(muted(editor), keys);
    let line = buttons_line("Start", true, idea.focus.checked_sub(1));
    f.render_widget(line, buttons);
    let said = "Start creates the Idea in bd and its worktree on brainstorm/<idea>.";
    f.render_widget(
        Paragraph::new(muted(said)).wrap(Wrap { trim: false }),
        about,
    );
}

/// A brainstorm modal's buttons: `green` and Cancel, filled, the focused
/// one (0 or 1) bright with ›; `green` greyed when it cannot be pressed.
fn buttons_line(green: &str, enabled: bool, focus: Option<usize>) -> Line<'static> {
    let button = |text: &str, c: Color, enabled: bool, focused: bool| {
        let style = match (enabled, focused) {
            (false, true) => bold(TEXT).bg(BORDER),
            (false, false) => fg(MUTED).bg(Color::Rgb(40, 46, 60)),
            (true, true) => bold(INK).bg(c),
            (true, false) => fg(c).bg(lerp((c, INK), 0.8)),
        };
        let mark = if focused { "›" } else { " " };
        Span::styled(format!(" {mark} {text}   "), style)
    };
    Line::from(vec![
        button(green, GREEN, enabled, focus == Some(0)),
        Span::raw("   "),
        button("Cancel", RED, true, focus == Some(1)),
    ])
}

/// A checkbox, the cursor's marked with ›.
fn check(on: bool, focused: bool) -> Line<'static> {
    let mark = if focused { "› " } else { "  " };
    let boxed = if on { "[x] " } else { "[ ] " };
    let c = if focused { PURPLE } else { TEXT };
    Line::from(vec![
        Span::styled(mark, bold(PURPLE)),
        Span::styled(boxed, bold(c)),
    ])
}

/// The Tickets modal in the dock: what charting came out as, a checkbox
/// row per Ticket with its description's first line under it, the
/// /start-ticket line the checks make, the warning beside a live Epic run,
/// then Start tickets (greyed when it cannot start) and Cancel. Unwrapped,
/// scrolled to keep the focused row or the buttons in view.
pub(super) fn tickets(f: &mut Frame, s: &Screen) {
    let Some(t) = &s.tickets else {
        return;
    };
    let (rect, block) = dock(f, s);
    let n = t.rows.len();
    let title = format!(" CHARTED · {} from {} ", plural(n, "Ticket"), t.idea);
    let foot = " ↑↓ move · Space checks · Tab the buttons · Enter · Esc cancels ";
    let block = block
        .title(Span::styled(title, bold(PURPLE)))
        .title_bottom(Span::styled(foot, fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let muted = |t: String| Line::from(Span::styled(t, fg(MUTED)));
    let small = format!(" came out small: {}, no Map.", plural(n, "Ticket"));
    let mut lines = vec![
        Line::default(),
        Line::from(vec![
            Span::styled(format!("{} ", t.idea), bold(BLUE)),
            Span::styled(t.title.clone(), bold(TEXT)),
            Span::styled(small, fg(TEXT)),
        ]),
        muted("Check the ones to start.".to_string()),
        Line::default(),
    ];
    let mut at = 0; // the last line to keep in view: the focused row's about line
    for (i, r) in t.rows.iter().enumerate() {
        let focused = t.focus == i;
        let mut row = check(r.on, focused);
        row.spans
            .push(Span::styled(format!("{}  ", r.id), fg(BLUE)));
        let style = if focused { bold(TEXT) } else { fg(TEXT) };
        row.spans.push(Span::styled(r.title.clone(), style));
        lines.push(row);
        lines.push(muted(format!("      {}", r.about)));
        if focused {
            at = lines.len() - 1;
        }
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("runs ", fg(MUTED)),
        Span::styled(t.command(), fg(PURPLE)),
    ]));
    if s.epic_live() {
        let warn = "An Epic run is live: Start tickets is refused, the Tickets stay open in bd.";
        lines.push(Line::from(Span::styled(warn, fg(ORANGE))));
    }
    lines.push(Line::default());
    let focus = t.focus.checked_sub(n);
    lines.push(buttons_line("Start tickets", s.tickets_start(), focus));
    if focus.is_some() {
        at = lines.len() - 1;
    }
    let top = (at + 1).saturating_sub(inner.height as usize);
    f.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), inner);
}

/// The start-Map modal in the dock, or its Continue form: the Map, its
/// Destination and counts (the Continue form's counts and its rebase
/// line), the background checkbox and what it does, Start Map (Continue)
/// and Cancel, and the start form's foot line. Wrapped; the form from the
/// checkbox down keeps its rows, a long Destination cut short above it.
pub(super) fn start_map(f: &mut Frame, s: &Screen) {
    let Some(m) = &s.start_map else {
        return;
    };
    let (rect, block) = dock(f, s);
    let (title, foot) = match m.again {
        true => (
            format!(" CONTINUE · {} ", m.map),
            " Enter keeps it · Space flips it · Esc cancels ",
        ),
        false => (
            format!(" CHARTED · a Map from {} ", m.idea),
            " Space checks · Tab moves · Enter · Esc cancels ",
        ),
    };
    let block = block
        .title(Span::styled(title, bold(PURPLE)))
        .title_bottom(Span::styled(foot, fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let muted = |t: String| Line::from(Span::styled(t, fg(MUTED)));
    let named = Line::from(vec![
        Span::styled(format!("{}  ", m.map), bold(BLUE)),
        Span::styled(m.title.clone(), bold(TEXT)),
    ]);
    let mut lines = vec![Line::default()];
    if m.again {
        lines.extend([named, muted(m.counts.clone())]);
        if !m.rebase.is_empty() {
            lines.push(muted(m.rebase.clone()));
        }
    } else {
        let came = format!("{} came out as a Map.", m.idea);
        lines.extend([
            Line::from(Span::styled(came, bold(TEXT))),
            Line::default(),
            named,
        ]);
        if !m.destination.is_empty() {
            let text = format!("Destination: {}", m.destination);
            lines.push(Line::from(Span::styled(text, fg(TEXT))));
        }
        lines.push(muted(m.counts.clone()));
    }
    let mut form = vec![Line::default()];
    let focused = m.focus == 0;
    let mut row = check(m.background, focused);
    let style = if focused { bold(TEXT) } else { fg(TEXT) };
    row.spans
        .push(Span::styled("start the research in the background", style));
    // only while the box still holds the saved answer
    let saved = s.brainstorms.iter().find(|b| b.idea == m.idea);
    if m.again && saved.is_some_and(|b| b.background == m.background) {
        row.spans
            .push(Span::styled("   your answer last time", fg(MUTED)));
    }
    form.push(row);
    form.push(muted(match m.background {
        true => format!(
            "      {} start in tab research-{}, {} at once at most (max_research)",
            plural(m.research, "Research Waypoint"),
            m.map,
            m.max_research
        ),
        false => "      the sessions with you take research as it reaches the frontier".to_string(),
    }));
    form.push(Line::default());
    let green = if m.again { "Continue" } else { "Start Map" };
    form.push(buttons_line(green, true, m.focus.checked_sub(1)));
    if !m.again {
        form.push(Line::default());
        form.push(muted(format!(
            "Start runs brainstorm-waypoint on the next Waypoint in a pane beside the Shell. \
             Cancel keeps the Map: /continue @{} starts it later.",
            m.map
        )));
    }
    let wrap = Wrap { trim: false };
    let (head, form) = (
        Paragraph::new(lines).wrap(wrap),
        Paragraph::new(form).wrap(wrap),
    );
    let form_h = (form.line_count(inner.width) as u16).min(inner.height);
    let head_h = (head.line_count(inner.width) as u16).min(inner.height - form_h);
    let [head_at, form_at] =
        Layout::vertical([Constraint::Length(head_h), Constraint::Length(form_h)]).areas(inner);
    f.render_widget(head, head_at);
    f.render_widget(form, form_at);
}
