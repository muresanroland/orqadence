use ratatui::style::Style;

use super::modal::wrap_spans;

/// A word of emoji sequences is cut by the renderer's measure: 40 × ❤️
/// (❤ plus U+FE0F, two columns) rows no wider than the box, every heart
/// kept whole.
#[test]
fn a_word_of_emoji_sequences_wraps_within_the_row() {
    let hearts = "❤\u{FE0F}".repeat(40);
    let lines = wrap_spans(
        vec![(hearts, Style::default())],
        56,
        "",
        "",
        Style::default(),
    );
    for line in &lines {
        assert!(line.width() <= 56, "{} columns: {line:?}", line.width());
    }
    let text: String = lines
        .iter()
        .flat_map(|l| &l.spans)
        .map(|s| &*s.content)
        .collect();
    assert_eq!(text, "❤\u{FE0F}".repeat(40));
    for span in lines.iter().flat_map(|l| &l.spans) {
        assert!(!span.content.starts_with('\u{FE0F}'), "{lines:#?}");
    }
}
