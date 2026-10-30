//! Passage layout for the typing screen: word wrap, colour runs and the
//! visible window.
//!
//! The target can be 20 000 characters long, so the screen never receives it
//! whole. The text is wrapped once into lines of target-unit ranges; on every
//! key only the few lines around the cursor are turned into colour runs and
//! sent to Slint. Auto-scroll falls out of that: the window always starts one
//! line above the cursor's line.

use std::ops::Range;

use radha_core::UnitState;

use crate::typing_rules::HighlightMode;

/// Lines shown at once.
pub const VISIBLE_LINES: usize = 5;

/// How a run of units is drawn. The numbers are what the `.slint` side
/// switches on (`PassageSegment.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Pending = 0,
    Correct = 1,
    Wrong = 2,
    Cursor = 3,
    /// The rest of the word being typed (saffron wash, DESIGN 20.7).
    Active = 4,
    /// A committed word none of whose units was typed (omitted).
    Skipped = 5,
}

/// One colour run of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub state: RunState,
}

/// Greedy word wrap of `units` into lines of at most `cols` units. A space
/// stays at the end of the line it follows; a word longer than a line is
/// split. Every unit belongs to exactly one line.
#[must_use]
pub fn wrap(units: &[String], cols: usize) -> Vec<Range<usize>> {
    let cols = cols.max(1);
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let is_space = |u: &String| u.chars().all(char::is_whitespace);
    while i < units.len() {
        // The next word (non-space run) and the spaces after it.
        let word_start = i;
        while i < units.len() && !is_space(&units[i]) {
            i += 1;
        }
        let word_end = i;
        while i < units.len() && is_space(&units[i]) {
            i += 1;
        }
        let word_len = word_end - word_start;
        if word_start > start && word_end - start > cols {
            // The word does not fit after what the line already holds.
            lines.push(start..word_start);
            start = word_start;
        }
        // Hard-split a word longer than a whole line.
        while word_end - start > cols && word_len > cols {
            lines.push(start..start + cols);
            start += cols;
        }
    }
    if start < units.len() || lines.is_empty() {
        lines.push(start..units.len());
    }
    lines
}

/// The line holding unit `cursor` (the last line past the end).
#[must_use]
pub fn line_of(lines: &[Range<usize>], cursor: usize) -> usize {
    lines
        .iter()
        .position(|l| cursor < l.end)
        .unwrap_or(lines.len().saturating_sub(1))
}

/// Lines to show for `cursor`: one line of context above it.
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
pub fn window(lines: &[Range<usize>], cursor: usize) -> Range<usize> {
    let current = line_of(lines, cursor);
    let start = current
        .saturating_sub(1)
        .min(lines.len().saturating_sub(VISIBLE_LINES));
    start..(start + VISIBLE_LINES).min(lines.len())
}

/// [`window`] for a pane that shows `rows` lines.
#[must_use]
pub fn window_n(lines: &[Range<usize>], cursor: usize, rows: usize) -> Range<usize> {
    let rows = rows.max(1);
    let current = line_of(lines, cursor);
    let start = current
        .saturating_sub(1)
        .min(lines.len().saturating_sub(rows));
    start..(start + rows).min(lines.len())
}

/// `rows` lines from `first` (a manual scroll), kept inside the passage.
#[must_use]
pub fn window_from(lines: &[Range<usize>], first: usize, rows: usize) -> Range<usize> {
    let rows = rows.max(1);
    let start = first.min(lines.len().saturating_sub(rows));
    start..(start + rows).min(lines.len())
}

/// The lines to draw for a visible `window` of a passage of `total` lines:
/// one buffer line above and one below, so a one-line scroll can glide
/// (the outgoing line slides out, the incoming one slides in) instead of
/// jumping. The screen clips the block to the visible rows.
#[must_use]
pub fn with_buffer_lines(window: &Range<usize>, total: usize) -> Range<usize> {
    window.start.saturating_sub(1)..(window.end + 1).min(total.max(window.end))
}

/// Whether moving the first visible line from `from` to `to` glides: a
/// one-line step does (typing onto the next line, Backspace, one wheel
/// notch); a page jump or a fresh passage snaps.
#[must_use]
pub fn glides(from: Option<usize>, to: usize) -> bool {
    from.is_some_and(|f| f.abs_diff(to) == 1)
}

fn is_space(u: &str) -> bool {
    !u.is_empty() && u.chars().all(char::is_whitespace)
}

/// The word highlighter (TY-03): colour runs of one line by WORD, not by
/// key. The word being typed (`active`) is one saffron block; committed
/// words are green or red, an omitted word (`skipped`) is struck through;
/// spaces carry no colour unless typed wrong. Word-only mode keeps just
/// the current word; Blind mode colours nothing until the Result. With
/// Rules → Key Guide on, `key_cursor` names the unit to mark as the next key.
#[must_use]
pub fn word_runs(
    units: &[String],
    states: &[UnitState],
    line: Range<usize>,
    key_cursor: Option<usize>,
    active: Range<usize>,
    skipped: &[Range<usize>],
    highlight: HighlightMode,
) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for i in line {
        let Some(text) = units.get(i) else { break };
        let colour = match states.get(i) {
            Some(UnitState::Correct) => RunState::Correct,
            Some(UnitState::Wrong) => RunState::Wrong,
            Some(UnitState::Pending) | None => RunState::Pending,
        };
        let state = match highlight {
            HighlightMode::Blind => RunState::Pending,
            _ if key_cursor == Some(i) => RunState::Cursor,
            _ if active.contains(&i) => RunState::Active,
            HighlightMode::WordOnly => RunState::Pending,
            HighlightMode::WordError if is_space(text) => {
                if colour == RunState::Wrong {
                    RunState::Wrong
                } else {
                    RunState::Pending
                }
            }
            HighlightMode::WordError if skipped.iter().any(|r| r.contains(&i)) => RunState::Skipped,
            HighlightMode::WordError => colour,
        };
        match out.last_mut() {
            Some(run) if run.state == state && state != RunState::Cursor => run.text.push_str(text),
            _ => out.push(Run {
                text: text.clone(),
                state,
            }),
        }
    }
    out
}

/// Every word's 1-based number at each unit (spaces count with the word
/// before them), and the word total: the "Word 6 of 400" footer in O(1)
/// per key.
#[must_use]
pub fn word_numbers(units: &[String]) -> (Vec<u32>, u32) {
    let mut numbers = Vec::with_capacity(units.len());
    let mut n = 0u32;
    let mut in_word = false;
    for u in units {
        if is_space(u) {
            in_word = false;
        } else if !in_word {
            in_word = true;
            n = n.saturating_add(1);
        }
        numbers.push(n.max(1));
    }
    (numbers, n)
}

/// `text` cut to its first `limit` words at a word boundary (0 = all).
#[must_use]
pub fn truncate_words(text: &str, limit: u32) -> &str {
    if limit == 0 {
        return text;
    }
    let mut words = 0u32;
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if in_word && words == limit {
                return text.get(..i).unwrap_or(text);
            }
            in_word = false;
        } else if !in_word {
            in_word = true;
            words += 1;
        }
    }
    text
}

/// Colour runs of one line. The unit at `cursor` gets its own
/// [`RunState::Cursor`] run.
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
pub fn runs(units: &[String], states: &[UnitState], line: Range<usize>, cursor: usize) -> Vec<Run> {
    runs_with(units, states, line, cursor, 0..0, HighlightMode::WordError)
}

/// [`runs`] under a highlight rule (TY-05), with the active word's
/// untyped units marked [`RunState::Active`] (not in Blind mode). Word-only
/// and Blind modes never colour correctness.
#[must_use]
pub fn runs_with(
    units: &[String],
    states: &[UnitState],
    line: Range<usize>,
    cursor: usize,
    active: Range<usize>,
    highlight: HighlightMode,
) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for i in line {
        let Some(text) = units.get(i) else { break };
        let state = if i == cursor {
            RunState::Cursor
        } else {
            let coloured = match states.get(i) {
                Some(UnitState::Correct) => RunState::Correct,
                Some(UnitState::Wrong) => RunState::Wrong,
                Some(UnitState::Pending) | None => RunState::Pending,
            };
            let in_word = active.contains(&i);
            match highlight {
                HighlightMode::WordError if in_word && coloured == RunState::Pending => {
                    RunState::Active
                }
                HighlightMode::WordError => coloured,
                HighlightMode::WordOnly if in_word => RunState::Active,
                HighlightMode::WordOnly | HighlightMode::Blind => RunState::Pending,
            }
        };
        match out.last_mut() {
            Some(run) if run.state == state && state != RunState::Cursor => run.text.push_str(text),
            _ => out.push(Run {
                text: text.clone(),
                state,
            }),
        }
    }
    out
}

/// Columns that fit in `width_px` for a script: monospace Latin is 0.6 em
/// per unit; a Devanagari akshar is wider and proportional, so it is
/// budgeted at 0.95 em to keep lines inside the pane.
#[must_use]
pub fn columns(width_px: f32, font_px: f32, devanagari: bool) -> usize {
    let per_unit = font_px * if devanagari { 0.95 } else { 0.6 };
    if per_unit <= 0.0 || !width_px.is_finite() {
        return 40;
    }
    ((width_px / per_unit).floor() as usize).clamp(10, 200)
}

/// The wrapped passage, rewrapped only when the column count changes (a
/// resize, a font size step, focus mode). Per key nothing is rewrapped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wrapped {
    pub cols: usize,
    pub lines: Vec<Range<usize>>,
}

impl Wrapped {
    /// Wrap `units` for a pane `width_px` wide at `font_px`. Returns `true`
    /// when the lines changed.
    pub fn fit(&mut self, units: &[String], width_px: f32, font_px: f32, devanagari: bool) -> bool {
        let cols = columns(width_px, font_px, devanagari);
        if cols == self.cols && !self.lines.is_empty() {
            return false;
        }
        self.cols = cols;
        self.lines = wrap(units, cols);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(s: &str) -> Vec<String> {
        s.chars().map(String::from).collect()
    }

    fn text(u: &[String], r: &Range<usize>) -> String {
        u[r.clone()].concat()
    }

    #[test]
    fn wrap_breaks_between_words_and_keeps_spaces_on_the_line() {
        let u = units("the cat sat on the mat");
        let lines = wrap(&u, 8);
        let shown: Vec<String> = lines.iter().map(|r| text(&u, r)).collect();
        assert_eq!(shown, ["the cat ", "sat on ", "the mat"]);
        // Every unit exactly once, in order.
        assert_eq!(lines.first().map(|r| r.start), Some(0));
        assert!(lines.windows(2).all(|w| w[0].end == w[1].start));
        assert_eq!(lines.last().map(|r| r.end), Some(u.len()));
    }

    #[test]
    fn wrap_splits_an_overlong_word() {
        let u = units("abcdefghij xy");
        let shown: Vec<String> = wrap(&u, 4).iter().map(|r| text(&u, r)).collect();
        assert_eq!(shown, ["abcd", "efgh", "ij ", "xy"]);
        let empty = wrap(&[], 10);
        assert_eq!(empty.len(), 1, "an empty text is one empty line");
        assert!(empty[0].is_empty());
    }

    #[test]
    fn the_window_follows_the_cursor_with_one_line_of_context() {
        let lines: Vec<Range<usize>> = (0..20).map(|i| i * 10..i * 10 + 10).collect();
        assert_eq!(window(&lines, 0), 0..5);
        assert_eq!(window(&lines, 15), 0..5);
        assert_eq!(window(&lines, 35), 2..7, "auto-scroll");
        assert_eq!(window(&lines, 199), 15..20, "the end stays full");
        assert_eq!(window(&lines, 500), 15..20);
        assert_eq!(window(&lines[..2], 5), 0..2);
    }

    #[test]
    fn runs_group_states_and_isolate_the_cursor() {
        let u = units("abcdef");
        let s = [
            UnitState::Correct,
            UnitState::Correct,
            UnitState::Wrong,
            UnitState::Pending,
            UnitState::Pending,
            UnitState::Pending,
        ];
        let r = runs(&u, &s, 0..6, 3);
        let flat: Vec<(&str, RunState)> = r.iter().map(|r| (r.text.as_str(), r.state)).collect();
        assert_eq!(
            flat,
            [
                ("ab", RunState::Correct),
                ("c", RunState::Wrong),
                ("d", RunState::Cursor),
                ("ef", RunState::Pending)
            ]
        );
    }

    #[test]
    fn rewrap_happens_only_when_the_columns_change() {
        let u = units("the quick brown fox jumps over the lazy dog again and again");
        let mut w = Wrapped::default();
        assert!(w.fit(&u, 300.0, 22.0, false), "first wrap");
        let narrow = w.lines.clone();
        assert!(!w.fit(&u, 301.0, 22.0, false), "same columns: no rewrap");
        assert!(w.fit(&u, 900.0, 22.0, false), "wider window rewraps");
        assert!(w.lines.len() < narrow.len());
        assert!(w.fit(&u, 900.0, 34.0, false), "a bigger font rewraps");
        // Every unit is still on exactly one line, and the cursor's line is
        // in the visible window after the rewrap.
        assert_eq!(w.lines.first().map(|r| r.start), Some(0));
        assert!(w.lines.windows(2).all(|p| p[0].end == p[1].start));
        assert_eq!(w.lines.last().map(|r| r.end), Some(u.len()));
        let cursor = 40;
        let win = window(&w.lines, cursor);
        assert!(win.contains(&line_of(&w.lines, cursor)));
    }

    #[test]
    fn highlight_modes_and_the_active_word() {
        let u = units("ab cd");
        let s = [
            UnitState::Correct,
            UnitState::Wrong,
            UnitState::Correct,
            UnitState::Pending,
            UnitState::Pending,
        ];
        let flat = |h| {
            runs_with(&u, &s, 0..5, 3, 3..5, h)
                .into_iter()
                .map(|r| (r.text, r.state))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            flat(HighlightMode::WordError),
            [
                ("a".into(), RunState::Correct),
                ("b".into(), RunState::Wrong),
                (" ".into(), RunState::Correct),
                ("c".into(), RunState::Cursor),
                ("d".into(), RunState::Active),
            ]
        );
        assert_eq!(
            flat(HighlightMode::WordOnly),
            [
                ("ab ".into(), RunState::Pending),
                ("c".into(), RunState::Cursor),
                ("d".into(), RunState::Active),
            ],
            "no correctness colours"
        );
        assert_eq!(
            flat(HighlightMode::Blind),
            [
                ("ab ".into(), RunState::Pending),
                ("c".into(), RunState::Cursor),
                ("d".into(), RunState::Pending),
            ]
        );
    }

    #[test]
    fn the_highlighter_is_word_based_and_marks_keys_only_with_the_key_guide() {
        // "ab cd ef": "ab" committed wrong, "cd" being typed, "ef" ahead.
        let u = units("ab cd ef");
        let s = [
            UnitState::Wrong,
            UnitState::Wrong,
            UnitState::Correct,
            UnitState::Correct,
            UnitState::Pending,
            UnitState::Pending,
            UnitState::Pending,
            UnitState::Pending,
        ];
        let flat = |h, key: Option<usize>| {
            word_runs(&u, &s, 0..8, key, 3..5, &[], h)
                .into_iter()
                .map(|r| (r.text, r.state))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            flat(HighlightMode::WordError, None),
            [
                ("ab".into(), RunState::Wrong),
                (" ".into(), RunState::Pending),
                ("cd".into(), RunState::Active),
                (" ef".into(), RunState::Pending),
            ],
            "no per-key cursor by default: the whole word is one block"
        );
        assert_eq!(
            flat(HighlightMode::WordError, Some(4)),
            [
                ("ab".into(), RunState::Wrong),
                (" ".into(), RunState::Pending),
                ("c".into(), RunState::Active),
                ("d".into(), RunState::Cursor),
                (" ef".into(), RunState::Pending),
            ],
            "Key Guide on: the next key is marked"
        );
        assert_eq!(
            flat(HighlightMode::WordOnly, None),
            [
                ("ab ".into(), RunState::Pending),
                ("cd".into(), RunState::Active),
                (" ef".into(), RunState::Pending),
            ]
        );
        assert_eq!(
            flat(HighlightMode::Blind, Some(4)),
            [("ab cd ef".into(), RunState::Pending)],
            "blind: no colouring at all, not even the key guide"
        );
        let omitted: Vec<Range<usize>> = std::iter::once(0..2).collect();
        let skipped = word_runs(&u, &s, 0..3, None, 3..5, &omitted, HighlightMode::WordError);
        assert_eq!(skipped[0].state, RunState::Skipped);
    }

    #[test]
    fn word_numbers_windows_and_word_limits() {
        let u = units("ab  cd e");
        let (n, total) = word_numbers(&u);
        assert_eq!(total, 3);
        assert_eq!(n, [1, 1, 1, 1, 2, 2, 2, 3]);
        assert_eq!(word_numbers(&[]).1, 0);
        let lines: Vec<Range<usize>> = (0..20).map(|i| i * 10..i * 10 + 10).collect();
        assert_eq!(window_n(&lines, 35, 8), 2..10);
        assert_eq!(window_n(&lines, 199, 8), 12..20);
        assert_eq!(window_n(&lines, 35, 5), window(&lines, 35));
        assert_eq!(window_from(&lines, 3, 8), 3..11);
        assert_eq!(window_from(&lines, 50, 8), 12..20, "clamped to the end");
        assert_eq!(truncate_words("one two  three four", 2), "one two");
        assert_eq!(truncate_words("one two", 5), "one two");
        assert_eq!(truncate_words("one two", 0), "one two");
        assert_eq!(truncate_words("  lead one two", 1), "  lead");
        assert_eq!(truncate_words("राम श्याम सीता", 2), "राम श्याम");
    }

    #[test]
    fn columns_scale_with_width_and_script() {
        assert_eq!(columns(664.0, 22.0, false), 50, "13.2 px per mono cell");
        assert!(columns(664.0, 22.0, true) < 50);
        assert_eq!(columns(10.0, 22.0, false), 10, "never below 10");
        assert_eq!(columns(f32::NAN, 22.0, false), 40);
    }

    #[test]
    fn a_one_line_scroll_has_both_buffer_lines_drawn_and_glides() {
        // 20 lines, 5 shown from line 6: lines 5 and 11 are drawn too.
        assert_eq!(with_buffer_lines(&(6..11), 20), 5..12);
        // At the top and the bottom there is nothing to buffer.
        assert_eq!(with_buffer_lines(&(0..5), 20), 0..6);
        assert_eq!(with_buffer_lines(&(15..20), 20), 14..20);
        assert_eq!(with_buffer_lines(&(0..1), 1), 0..1);
        // Auto-scroll moves the window one line per wrapped line typed, so
        // the block glides from the line it showed to the next.
        let u = units(&"word ".repeat(200));
        let lines = wrap(&u, 20);
        let mut first = None;
        let mut moves = 0;
        for cursor in 0..u.len() {
            let w = window_n(&lines, cursor, 5);
            if first.is_some_and(|f| f != w.start) {
                assert!(glides(first, w.start), "{first:?} -> {}", w.start);
                moves += 1;
            }
            // The previous first line is always in the drawn block.
            let drawn = with_buffer_lines(&w, lines.len());
            if let Some(f) = first {
                assert!(drawn.contains(&f), "{f} outside {drawn:?}");
            }
            first = Some(w.start);
        }
        assert!(moves > 10);
        // Jumps and a fresh passage snap.
        assert!(!glides(None, 0));
        assert!(!glides(Some(3), 3));
        assert!(!glides(Some(3), 8), "a page");
        assert!(glides(Some(4), 3), "Backspace or a wheel notch up");
    }
}
