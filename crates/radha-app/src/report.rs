//! Result v2 derivations (PRD 17.6, DESIGN 20.8) as pure functions.
//!
//! Every figure comes from the one scoring module (`radha_core`, RULES 36):
//! the minute curve uses `radha_core::wpm` per minute, the mistake split uses
//! `radha_core::word_mistakes`. Exam-rule formulas are not here (P09); no
//! verdict ever claims an official qualification (RULES: honest
//! certification).

use radha_core::{Severity, WordMistake};

/// How one committed word compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Correct,
    /// A half mistake (case, spacing, spelling per `radha_core::mistake`).
    Half,
    /// A full mistake.
    Full,
    /// The word was left out (a full mistake, drawn dashed).
    Omitted,
}

impl DiffKind {
    /// The number the `.slint` side switches on.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            DiffKind::Correct => 0,
            DiffKind::Half => 1,
            DiffKind::Full => 2,
            DiffKind::Omitted => 3,
        }
    }
}

/// One row of the diff inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordDiff {
    pub expected: String,
    pub typed: String,
    pub kind: DiffKind,
    /// "Spelling (½)", "Omission", "" when correct.
    pub label: &'static str,
}

/// Pair committed words (target text per word span) with what was typed
/// (the matcher is word-aligned, so word `i` typed is compared with word
/// `i`) and the core's classification of each committed word.
#[must_use]
pub fn word_diff(expected: &[String], typed: &[&str], kinds: &[WordMistake]) -> Vec<WordDiff> {
    kinds
        .iter()
        .enumerate()
        .map(|(i, &k)| {
            let kind = match (k, k.severity()) {
                (WordMistake::None, _) => DiffKind::Correct,
                (WordMistake::Omission, _) => DiffKind::Omitted,
                (_, Severity::Full) => DiffKind::Full,
                _ => DiffKind::Half,
            };
            let label = match k {
                WordMistake::None => "",
                WordMistake::Omission => "Omission",
                WordMistake::Spacing => "Spacing (½)",
                WordMistake::Case => "Case (½)",
                WordMistake::Spelling => "Spelling (½)",
            };
            WordDiff {
                expected: expected.get(i).cloned().unwrap_or_default(),
                typed: if kind == DiffKind::Omitted {
                    String::new()
                } else {
                    typed.get(i).map(|s| (*s).to_owned()).unwrap_or_default()
                },
                kind,
                label,
            }
        })
        .collect()
}

/// Gross WPM per minute of active typing, from `(active seconds, code
/// points typed)` after each input. The last, partial minute is scaled by
/// its own length (at least 5 s, so a final keystroke does not spike).
#[must_use]
pub fn minute_wpm(points: &[(f64, usize)]) -> Vec<f64> {
    minute_rates(points, radha_core::wpm)
}

/// KDPH per minute of active typing (the curve's second view, RS-02), on
/// the same minute spans as [`minute_wpm`].
#[must_use]
pub fn minute_kdph(points: &[(f64, usize)]) -> Vec<f64> {
    minute_rates(points, radha_core::kdph)
}

fn minute_rates(points: &[(f64, usize)], rate: fn(usize, f64) -> f64) -> Vec<f64> {
    let Some(&(end, _)) = points.last() else {
        return Vec::new();
    };
    let minutes = (end / 60.0).floor() as usize + 1;
    let mut out = Vec::with_capacity(minutes);
    let mut before = 0usize;
    let mut idx = 0usize;
    for m in 0..minutes {
        let limit = (m as f64 + 1.0) * 60.0;
        let mut at_end = before;
        while let Some(&(t, n)) = points.get(idx) {
            if t >= limit && m + 1 < minutes {
                break;
            }
            at_end = n;
            idx += 1;
        }
        let span = if m + 1 < minutes {
            60.0
        } else {
            (end - m as f64 * 60.0).max(5.0)
        };
        out.push(rate(at_end.saturating_sub(before), span));
        before = at_end;
    }
    out
}

/// A chart in the `SparklineChart` viewbox (0 0 360 72).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartPath {
    pub line: String,
    pub area: String,
    pub peak_x: f32,
    pub peak_y: f32,
}

/// Line + area commands for `values` (at least one point; a single point
/// draws a flat line). The y axis runs from 0 to the peak (+10 %).
#[must_use]
pub fn chart_path(values: &[f64]) -> Option<ChartPath> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let peak = values.iter().copied().fold(0.0_f64, f64::max).max(1.0) * 1.1;
    let n = values.len();
    let point = |i: usize, v: f64| -> (f32, f32) {
        let x = if n == 1 {
            0.0
        } else {
            i as f64 * 360.0 / (n - 1) as f64
        };
        let y = 72.0 - v / peak * 64.0 - 4.0;
        (x as f32, y as f32)
    };
    let mut pts: Vec<(f32, f32)> = values
        .iter()
        .enumerate()
        .map(|(i, &v)| point(i, v))
        .collect();
    if n == 1 {
        pts.push((360.0, pts[0].1));
    }
    let mut line = String::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        line.push_str(&format!(
            "{} {x:.1} {y:.1} ",
            if i == 0 { "M" } else { "L" }
        ));
    }
    let area = format!("{line}L 360 72 L 0 72 Z");
    let (pi, _) =
        values.iter().enumerate().fold(
            (0, f64::MIN),
            |best, (i, &v)| if v > best.1 { (i, v) } else { best },
        );
    let (peak_x, peak_y) = pts.get(pi).copied().unwrap_or((0.0, 72.0));
    Some(ChartPath {
        line: line.trim_end().to_owned(),
        area,
        peak_x,
        peak_y,
    })
}

/// Heatmap class of a key (DESIGN 20.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatClass {
    /// Not typed in this session.
    Unused,
    /// Accuracy >= 95 %.
    Optimal,
    /// 85–95 %.
    Warning,
    /// Below 85 %.
    Critical,
}

impl HeatClass {
    /// The number the `.slint` side switches on.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            HeatClass::Unused => 0,
            HeatClass::Optimal => 1,
            HeatClass::Warning => 2,
            HeatClass::Critical => 3,
        }
    }
}

/// Accuracy % and class of a key with `attempts` and `errors`.
#[must_use]
pub fn heat(attempts: u32, errors: u32) -> (f64, HeatClass) {
    if attempts == 0 {
        return (0.0, HeatClass::Unused);
    }
    let acc = f64::from(attempts.saturating_sub(errors)) * 100.0 / f64::from(attempts);
    let class = if acc >= 95.0 {
        HeatClass::Optimal
    } else if acc >= 85.0 {
        HeatClass::Warning
    } else {
        HeatClass::Critical
    };
    (acc, class)
}

/// Running per-key latency for the heatmap (RS-06): the sum of
/// `mean ms × attempts` and the attempts it covers, so the mean is weighted by
/// how often each unit was typed. Missing, negative or non-finite means are
/// skipped.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KeyLatency {
    sum_ms: f64,
    attempts: u32,
}

impl KeyLatency {
    /// Add a unit's mean latency over `attempts` presses.
    pub fn add(&mut self, mean_ms: Option<f64>, attempts: u32) {
        if let Some(ms) = mean_ms.filter(|ms| ms.is_finite() && *ms >= 0.0) {
            if attempts > 0 {
                self.sum_ms += ms * f64::from(attempts);
                self.attempts = self.attempts.saturating_add(attempts);
            }
        }
    }

    /// Weighted mean in ms, `None` when nothing was measured.
    #[must_use]
    pub fn mean_ms(&self) -> Option<f64> {
        (self.attempts > 0).then(|| self.sum_ms / f64::from(self.attempts))
    }
}

/// The smart recovery drill (RS-07): missed words and weak units, each
/// twice, about two minutes of text at most. `None` when nothing was missed.
#[must_use]
pub fn recovery_drill(diff: &[WordDiff], weak_units: &[String]) -> Option<String> {
    let mut words: Vec<&str> = Vec::new();
    for d in diff.iter().filter(|d| d.kind != DiffKind::Correct) {
        let w = d.expected.as_str();
        if !w.trim().is_empty() && !words.contains(&w) {
            words.push(w);
        }
    }
    for u in weak_units {
        if !u.trim().is_empty() && !words.contains(&u.as_str()) {
            words.push(u);
        }
    }
    words.truncate(16);
    if words.is_empty() {
        return None;
    }
    let mut out: Vec<&str> = Vec::with_capacity(words.len() * 2);
    for pass in 0..2 {
        if pass == 1 {
            words.reverse();
        }
        out.extend(words.iter().copied());
    }
    Some(out.join(" "))
}

/// Certification level of a session (RULES: Official / Provisional /
/// PracticeOnly). An exam session takes its profile's derived level
/// (`exam_mode::certification`); practice modes are plain practice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Certification {
    /// Not an exam.
    Practice,
    PracticeOnly,
    /// Thresholds known, other required rules not verified (P09).
    Provisional,
    /// Every required rule verified (P09).
    Official,
}

impl Certification {
    /// Pill text.
    #[must_use]
    pub fn pill(self) -> &'static str {
        match self {
            Certification::Practice => "Practice result",
            Certification::PracticeOnly => "Practice only · no pass/fail",
            Certification::Provisional => "Provisional rules",
            Certification::Official => "Official rules",
        }
    }

    /// `VerificationBadge.level` id.
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Certification::Practice | Certification::PracticeOnly => "practice",
            Certification::Provisional => "provisional",
            Certification::Official => "official",
        }
    }
}

/// Stars for a completed lesson (Golu `StarRules`: 1 = completed,
/// 2 = accuracy >= 90 %, 3 = accuracy >= 95 % and net WPM >= target).
#[must_use]
pub fn stars(accuracy: f64, net_wpm: f64, target_wpm: f64) -> u8 {
    let mut s = 1;
    if accuracy >= 90.0 {
        s = 2;
    }
    if accuracy >= 95.0 && net_wpm >= target_wpm {
        s = 3;
    }
    s
}

/// The verdict card's headline and detail. LEARN: lesson + stars. TEST:
/// against the suggested target. EXAM: no verdict (practice only).
#[must_use]
pub fn verdict(
    mode: crate::exercise::PracticeMode,
    cert: Certification,
    net_wpm: f64,
    accuracy: f64,
    target: crate::exercise::Targets,
) -> (String, String) {
    use crate::exercise::PracticeMode as M;
    let target_wpm = f64::from(target.wpm);
    let target_acc = f64::from(target.accuracy);
    match mode {
        M::Exam => (
            "Practice attempt recorded in RAM".to_owned(),
            format!(
                "{} — exam rules are not verified yet, so no pass or fail is shown.",
                cert.pill()
            ),
        ),
        M::Learn | M::Beginner | M::Classic => {
            let s = stars(accuracy, net_wpm, target_wpm);
            (
                format!("Lesson complete · {}", "★".repeat(usize::from(s))),
                format!(
                    "{s} of 3 stars. 2 stars at 90% accuracy; 3 stars at 95% and {target_wpm:.0} net WPM."
                ),
            )
        }
        M::Test | M::Flow => {
            let met = net_wpm >= target_wpm && accuracy >= target_acc;
            (
                if met {
                    "Suggested target met".to_owned()
                } else {
                    "Below the suggested target".to_owned()
                },
                format!(
                    "Suggested target {target_wpm:.0} net WPM at {target_acc:.0}% (a practice guide, not an exam rule)."
                ),
            )
        }
    }
}

/// Error % of all target units reached.
#[must_use]
pub fn error_percent(accuracy: f64) -> f64 {
    (100.0 - accuracy).clamp(0.0, 100.0)
}

/// The scoring policy a session was scored under (RS-03). The engine has
/// exactly one today, PRD 8.2 (every uncorrected error costs one word per
/// minute); exam profiles add theirs in P09. The audit only ever shows the
/// policy that produced the numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringPolicy {
    /// `radha_core::net_wpm`, PRD 8.2, unweighted.
    Practice82,
}

impl ScoringPolicy {
    /// The formula, as written in the policy.
    #[must_use]
    pub fn formula(self) -> &'static str {
        match self {
            ScoringPolicy::Practice82 => {
                "Net WPM = max(0, Gross WPM − uncorrected errors ÷ minutes)"
            }
        }
    }
}

/// The penalty audit card (RS-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PenaltyAudit {
    pub formula: String,
    /// The formula with this session's numbers.
    pub worked: String,
    /// Uncorrected errors left in the text.
    pub raw: usize,
    /// Errors the policy exempts from the penalty.
    pub exempted: usize,
    /// Errors the penalty counted.
    pub penalised: usize,
    pub note: String,
}

/// Audit how `net` came from `gross` under `policy`.
#[must_use]
pub fn penalty_audit(
    policy: ScoringPolicy,
    cert: Certification,
    gross: f64,
    net: f64,
    errors: usize,
    backspaces: u32,
    active_seconds: f64,
) -> PenaltyAudit {
    let minutes = active_seconds / 60.0;
    let worked = if minutes > 0.0 && minutes.is_finite() {
        format!("max(0, {gross:.1} − {errors} ÷ {minutes:.2}) = {net:.1}")
    } else {
        "No active typing time, so no speed was scored.".to_owned()
    };
    let mut note = format!(
        "Every uncorrected error counts once; none are exempted. {backspaces} backspace{} \
         fixed mistakes before they counted.",
        if backspaces == 1 { "" } else { "s" }
    );
    if cert == Certification::PracticeOnly {
        note.push_str(
            " This exam's own penalty rules are not verified, so the practice formula above \
             was used and no pass or fail is given.",
        );
    }
    PenaltyAudit {
        formula: policy.formula().to_owned(),
        worked,
        raw: errors,
        exempted: 0,
        penalised: errors,
        note,
    }
}

/// One finger's share of the work (RS-06 finger balance).
#[derive(Debug, Clone, PartialEq)]
pub struct FingerStat {
    pub label: &'static str,
    pub presses: u32,
    pub errors: u32,
    /// Share of all presses, 0..=1.
    pub share: f64,
}

/// Finger labels, left little finger to right little finger, then thumbs.
pub const FINGER_LABELS: [&str; 9] = [
    "L-Pin", "L-Rng", "L-Mid", "L-Idx", "R-Idx", "R-Mid", "R-Rng", "R-Pin", "Thumb",
];

/// The finger slot of a key: `finger` 0 little … 4 thumb, on the `left`
/// hand or not.
#[must_use]
pub fn finger_slot(finger: u8, left: bool) -> usize {
    match (finger.min(4), left) {
        (4, _) => 8,
        (f, true) => usize::from(f),
        (f, false) => 7 - usize::from(f),
    }
}

/// Sum `(slot, presses, errors)` into the nine fingers.
#[must_use]
pub fn finger_balance(keys: &[(usize, u32, u32)]) -> Vec<FingerStat> {
    let mut sums = [(0u32, 0u32); 9];
    for &(slot, a, e) in keys {
        if let Some(s) = sums.get_mut(slot) {
            s.0 = s.0.saturating_add(a);
            s.1 = s.1.saturating_add(e);
        }
    }
    let total: u64 = sums.iter().map(|s| u64::from(s.0)).sum();
    FINGER_LABELS
        .iter()
        .zip(sums)
        .map(|(&label, (presses, errors))| FingerStat {
            label,
            presses,
            errors,
            share: if total == 0 {
                0.0
            } else {
                f64::from(presses) / total as f64
            },
        })
        .collect()
}

/// Everything the exported scorecard prints (RS-09).
#[derive(Debug, Clone, PartialEq)]
pub struct Scorecard {
    pub cert: Certification,
    pub product: String,
    pub exercise: String,
    pub mode: String,
    pub layout: String,
    /// `YYYY-MM-DD HH:MM UTC`.
    pub date: String,
    pub figures: Vec<(&'static str, String)>,
    pub audit: PenaltyAudit,
    /// Mistakes only, capped.
    pub mistakes: Vec<WordDiff>,
    /// (name, roll number) when the candidate entered them (RS-10).
    pub candidate: Option<(String, String)>,
}

impl Scorecard {
    /// The title: "Practice scorecard" unless the rules are Official.
    #[must_use]
    pub fn title(&self) -> &'static str {
        match self.cert {
            Certification::Official => "Scorecard · official rules",
            _ => "Practice scorecard",
        }
    }
}

/// Most mistakes printed on a scorecard.
pub const SCORECARD_MISTAKES: usize = 200;

/// HTML-escape text for an element body or a quoted attribute.
#[must_use]
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() && c != '\n' => {}
            c => out.push(c),
        }
    }
    out
}

/// A self-contained, print-safe HTML scorecard: light surface, no
/// gradients, no scripts, no external resources, no certificate number.
#[must_use]
pub fn scorecard_html(card: &Scorecard) -> String {
    let e = escape_html;
    let mut h = String::with_capacity(8 * 1024);
    h.push_str("<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">");
    h.push_str(&format!("<title>{}</title>", e(card.title())));
    h.push_str(
        "<style>body{font-family:'Noto Sans Devanagari','Inter',sans-serif;background:#FFFDF8;\
         color:#352B30;margin:32px;}h1{color:#C84B73;margin:0 0 4px}h2{color:#6B3A5B;\
         font-size:16px;margin:24px 0 8px}.meta{color:#6A5A62}table{border-collapse:collapse;\
         width:100%}td,th{border:1px solid #DCC9B8;padding:6px 8px;text-align:left}\
         th{background:#F8F1E8}.note{color:#6A5A62;font-size:13px}.formula{font-family:\
         'JetBrains Mono',monospace}</style></head><body>",
    );
    h.push_str(&format!("<h1>{}</h1>", e(card.title())));
    h.push_str(&format!(
        "<p class=\"meta\">{} · {} · {} · {} · {}</p>",
        e(&card.product),
        e(&card.exercise),
        e(&card.mode),
        e(&card.layout),
        e(&card.date)
    ));
    if let Some((name, roll)) = &card.candidate {
        h.push_str("<p>Candidate: <strong>");
        h.push_str(&e(name));
        h.push_str("</strong>");
        if !roll.is_empty() {
            h.push_str(&format!(" · Roll number {}", e(roll)));
        }
        h.push_str("</p>");
    }
    h.push_str(&format!("<p class=\"note\">{}</p>", e(card.cert.pill())));
    h.push_str("<h2>Figures</h2><table>");
    for (label, value) in &card.figures {
        h.push_str(&format!(
            "<tr><th>{}</th><td>{}</td></tr>",
            e(label),
            e(value)
        ));
    }
    h.push_str("</table><h2>Penalty audit</h2>");
    h.push_str(&format!(
        "<p class=\"formula\">{}</p><p class=\"formula\">{}</p>",
        e(&card.audit.formula),
        e(&card.audit.worked)
    ));
    h.push_str(&format!(
        "<p>Errors: {} raw · {} exempted · {} penalised</p><p class=\"note\">{}</p>",
        card.audit.raw,
        card.audit.exempted,
        card.audit.penalised,
        e(&card.audit.note)
    ));
    h.push_str("<h2>Mistakes</h2>");
    if card.mistakes.is_empty() {
        h.push_str("<p>No word mistakes.</p>");
    } else {
        h.push_str("<table><tr><th>Expected</th><th>Typed</th><th>Type</th><th>Weight</th></tr>");
        for m in card.mistakes.iter().take(SCORECARD_MISTAKES) {
            let weight = match m.kind {
                DiffKind::Correct => "0",
                DiffKind::Half => "½",
                DiffKind::Full | DiffKind::Omitted => "1",
            };
            let label = if m.label.is_empty() { "Full" } else { m.label };
            h.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{weight}</td></tr>",
                e(&m.expected),
                if m.typed.is_empty() {
                    "—".to_owned()
                } else {
                    e(&m.typed)
                },
                e(label)
            ));
        }
        h.push_str("</table>");
        if card.mistakes.len() > SCORECARD_MISTAKES {
            h.push_str(&format!(
                "<p class=\"note\">{} more not shown.</p>",
                card.mistakes.len() - SCORECARD_MISTAKES
            ));
        }
    }
    h.push_str(
        "<p class=\"note\">Generated by Radha Typing Tutor on this computer. This is a record \
         of a typing session, not a certificate; it carries no certificate number.</p>",
    );
    h.push_str("</body></html>\n");
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::{PracticeMode, Targets};

    #[test]
    fn key_latency_is_weighted_by_attempts_and_skips_bad_values() {
        let mut l = KeyLatency::default();
        assert_eq!(l.mean_ms(), None);
        l.add(Some(100.0), 3);
        l.add(Some(200.0), 1);
        l.add(None, 5);
        l.add(Some(f64::NAN), 5);
        l.add(Some(-4.0), 5);
        l.add(Some(900.0), 0);
        let mean = l.mean_ms().unwrap();
        assert!((mean - 125.0).abs() < 1e-9, "{mean}");
    }

    #[test]
    fn word_diff_classifies_with_the_core_vocabulary() {
        let expected: Vec<String> = ["The", "cat", "sat", "down"].map(String::from).to_vec();
        let typed = ["the", "cxt", "down"];
        let kinds = [
            WordMistake::Case,
            WordMistake::Spelling,
            WordMistake::Omission,
            WordMistake::None,
        ];
        let d = word_diff(&expected, &typed, &kinds);
        let k: Vec<DiffKind> = d.iter().map(|w| w.kind).collect();
        assert_eq!(
            k,
            [
                DiffKind::Half,
                DiffKind::Half,
                DiffKind::Omitted,
                DiffKind::Correct
            ]
        );
        assert_eq!(d[0].label, "Case (½)");
        assert_eq!(d[2].typed, "", "an omitted word shows nothing typed");
        assert_eq!(d[2].label, "Omission");
    }

    #[test]
    fn minute_curve_uses_the_core_wpm_formula() {
        // 300 code points in the first minute, 150 in a 30 s second minute.
        let pts = [(1.0, 10), (59.0, 300), (61.0, 310), (90.0, 450)];
        let m = minute_wpm(&pts);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0], radha_core::wpm(300, 60.0));
        assert_eq!(m[1], radha_core::wpm(150, 30.0));
        assert!((m[0] - 60.0).abs() < 1e-9, "300 chars / 5 / 1 min");
        assert!(minute_wpm(&[]).is_empty());
        let short = minute_wpm(&[(1.0, 5)]);
        assert_eq!(short, [radha_core::wpm(5, 5.0)], "never a 1 s spike");
    }

    #[test]
    fn chart_paths_fit_the_viewbox() {
        let c = chart_path(&[10.0, 40.0, 20.0]).unwrap();
        assert!(c.line.starts_with("M 0.0 "));
        assert!(c.area.ends_with("L 360 72 L 0 72 Z"));
        assert_eq!(c.peak_x, 180.0, "peak is the middle point");
        assert!(c.peak_y > 0.0 && c.peak_y < 72.0);
        let one = chart_path(&[30.0]).unwrap();
        assert!(
            one.line.contains("L 360.0"),
            "a single minute is a flat line"
        );
        assert!(chart_path(&[]).is_none());
        assert!(chart_path(&[f64::NAN]).is_none());
    }

    #[test]
    fn heat_classes_and_recovery_drill() {
        assert_eq!(heat(0, 0).1, HeatClass::Unused);
        assert_eq!(heat(20, 1).1, HeatClass::Optimal);
        assert_eq!(heat(10, 1).1, HeatClass::Warning);
        assert_eq!(heat(10, 2).1, HeatClass::Critical);
        assert_eq!(heat(4, 1).0, 75.0);

        let diff = word_diff(
            &["क".into(), "कार्य".into(), "नमस्ते".into()],
            &["क", "काय", "नमस्ते"],
            &[WordMistake::None, WordMistake::Spelling, WordMistake::None],
        );
        let drill = recovery_drill(&diff, &["र्य".into()]).unwrap();
        assert_eq!(drill, "कार्य र्य र्य कार्य");
        let clean = word_diff(&["a".into()], &["a"], &[WordMistake::None]);
        assert_eq!(recovery_drill(&clean, &[]), None);
    }

    #[test]
    fn verdicts_are_practice_honest() {
        let t = Targets {
            wpm: 25,
            accuracy: 93,
        };
        let (h, d) = verdict(
            PracticeMode::Exam,
            Certification::PracticeOnly,
            60.0,
            99.0,
            t,
        );
        assert!(!h.to_lowercase().contains("qualified") && !h.contains("Pass"));
        assert!(d.contains("no pass or fail"));
        let (h, _) = verdict(PracticeMode::Test, Certification::Practice, 30.0, 95.0, t);
        assert_eq!(h, "Suggested target met");
        let (h, _) = verdict(PracticeMode::Test, Certification::Practice, 20.0, 95.0, t);
        assert_eq!(h, "Below the suggested target");
        let (h, _) = verdict(
            PracticeMode::Beginner,
            Certification::Practice,
            30.0,
            96.0,
            t,
        );
        assert!(h.ends_with("★★★"));
        assert_eq!(stars(80.0, 99.0, 10.0), 1);
        assert_eq!(stars(92.0, 99.0, 10.0), 2);
        assert_eq!(stars(97.0, 5.0, 10.0), 2, "3 stars need the target speed");
        assert_eq!(Certification::PracticeOnly.badge(), "practice");
        assert_eq!(error_percent(97.5), 2.5);
    }

    #[test]
    fn kdph_curve_shares_the_minute_spans() {
        let pts = [(1.0, 10), (59.0, 300), (61.0, 310), (90.0, 450)];
        let k = minute_kdph(&pts);
        assert_eq!(
            k,
            [radha_core::kdph(300, 60.0), radha_core::kdph(150, 30.0)]
        );
        assert_eq!(k.len(), minute_wpm(&pts).len());
        assert!(minute_kdph(&[]).is_empty());
    }

    #[test]
    fn the_penalty_audit_shows_the_formula_that_produced_net() {
        let gross = 60.0;
        let net = radha_core::net_wpm(gross, 6, 1.0);
        let a = penalty_audit(
            ScoringPolicy::Practice82,
            Certification::Practice,
            gross,
            net,
            6,
            3,
            60.0,
        );
        assert_eq!(a.worked, "max(0, 60.0 − 6 ÷ 1.00) = 54.0");
        assert_eq!((a.raw, a.exempted, a.penalised), (6, 0, 6));
        assert!(a.formula.contains("uncorrected errors"));
        assert!(!a.note.contains("pass or fail"));
        let exam = penalty_audit(
            ScoringPolicy::Practice82,
            Certification::PracticeOnly,
            gross,
            net,
            6,
            1,
            60.0,
        );
        assert!(exam.note.contains("not verified") && exam.note.contains("1 backspace "));
        let idle = penalty_audit(
            ScoringPolicy::Practice82,
            Certification::Practice,
            0.0,
            0.0,
            0,
            0,
            0.0,
        );
        assert!(idle.worked.starts_with("No active typing"));
    }

    #[test]
    fn finger_balance_sums_slots_and_shares() {
        assert_eq!(finger_slot(0, true), 0);
        assert_eq!(finger_slot(3, true), 3);
        assert_eq!(finger_slot(3, false), 4);
        assert_eq!(finger_slot(0, false), 7);
        assert_eq!(finger_slot(4, true), 8);
        let b = finger_balance(&[(0, 10, 1), (0, 10, 0), (7, 20, 5), (99, 5, 5)]);
        assert_eq!(b.len(), 9);
        assert_eq!((b[0].presses, b[0].errors), (20, 1));
        assert_eq!(b[0].share, 0.5);
        assert_eq!(b[7].share, 0.5);
        assert!(finger_balance(&[]).iter().all(|f| f.share == 0.0));
    }

    fn card(cert: Certification) -> Scorecard {
        Scorecard {
            cert,
            product: "English <Typing>".into(),
            exercise: "Exercise 1 · \"Home\"".into(),
            mode: "TEST".into(),
            layout: "QWERTY".into(),
            date: "2026-10-25 10:00 UTC".into(),
            figures: vec![("Net WPM", "54".into())],
            audit: penalty_audit(ScoringPolicy::Practice82, cert, 60.0, 54.0, 6, 0, 60.0),
            mistakes: word_diff(
                &["cat".into(), "<b>".into()],
                &["cxt", "x"],
                &[WordMistake::Spelling, WordMistake::Omission],
            ),
            candidate: Some(("Asha <script>".into(), "R-1".into())),
        }
    }

    #[test]
    fn the_scorecard_is_honest_escaped_and_offline() {
        let html = scorecard_html(&card(Certification::PracticeOnly));
        assert!(html.contains("<h1>Practice scorecard</h1>"));
        assert!(
            html.contains("Asha &lt;script&gt;"),
            "candidate text is escaped"
        );
        assert!(!html.contains("<script"), "no script ever");
        assert!(html.contains("English &lt;Typing&gt;"));
        assert!(html.contains("&lt;b&gt;"));
        assert!(html.contains("max(0, 60.0 − 6 ÷ 1.00) = 54.0"));
        let lower = html.to_lowercase();
        assert!(!lower.contains("http"), "no external resource");
        assert!(!lower.contains("cert #") && !lower.contains("certificate no"));
        assert!(!lower.contains("qualified"));
        assert!(!lower.contains("gradient"), "print-safe");
        assert_eq!(card(Certification::Practice).title(), "Practice scorecard");
        assert_eq!(
            card(Certification::Provisional).title(),
            "Practice scorecard"
        );
        assert_eq!(
            card(Certification::Official).title(),
            "Scorecard · official rules"
        );
    }
}
