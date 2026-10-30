//! Typing Flow presentation rules (PRD 17.8, DESIGN 8): how the flow
//! engine's figures are worded and drawn. Plain functions over plain data
//! (`RULES.MD` Rule 16); `ui/flow.rs` applies them to the Slint globals.

use radha_core::flow::tracker::{slot_char, KeyTally, KEY_SLOTS};
use radha_core::flow::{CadenceBar, CorridorPoint, DrillStage, FlowReport, FlowTracker, WeakKey};
use radha_core::TimerMode;
use radha_db::FlowRhythmRecord;

use crate::exercise::mm_ss;
use crate::report::ChartPath;

/// The Flow Exercise START text (DESIGN 8.3 "BEGIN FLOW").
#[must_use]
pub fn begin_label(timer: TimerMode) -> String {
    match timer {
        TimerMode::Countdown(l) => format!("Begin Flow · {}", mm_ss(l.seconds())),
        TimerMode::Stopwatch => "Begin Flow · stopwatch".to_owned(),
        TimerMode::Off => "Begin Flow · untimed".to_owned(),
    }
}

/// The duration dial's highlighted preset for `timer`: the preset with the
/// same length, `None` for a custom length, a stopwatch or no timer.
#[must_use]
pub fn dial_index(timer: TimerMode) -> Option<usize> {
    let seconds = timer.limit()?.seconds();
    radha_core::TimerPreset::all()
        .iter()
        .position(|p| p.seconds() == seconds)
}

/// The `flow_rhythm` row of a session's report (saved in the one
/// `save_session` transaction).
#[must_use]
pub fn record(report: &FlowReport) -> FlowRhythmRecord {
    FlowRhythmRecord {
        engine_schema: report.schema,
        smoothness: report.smoothness,
        cadence_cv: report.cadence_cv,
        mean_interval_ms: report.mean_ms,
        scored_intervals: report.counts.scored,
        hesitations: report.counts.hesitations,
        pauses: report.counts.pauses,
        corrections: report.counts.corrections,
        weak_keys: report.weak_keys.iter().map(|k| k.key).collect(),
    }
}

/// Bars in the live cadence wave.
pub const WAVE_BARS: usize = 40;

/// The Home card's smoothness preview: the last real figure, or "—" when
/// no Flow session has been recorded (never a made-up number).
#[must_use]
pub fn home_smoothness(last: Option<f64>) -> String {
    match last {
        Some(v) if v.is_finite() => format!("{:.0}%", v.clamp(0.0, 100.0)),
        _ => "—".to_owned(),
    }
}

/// A percentage figure, "—" when unknown.
#[must_use]
pub fn percent(v: Option<f64>) -> String {
    match v {
        Some(v) if v.is_finite() => format!("{v:.0}%"),
        _ => "—".to_owned(),
    }
}

/// Cadence variance as a percentage (coefficient of variation × 100).
#[must_use]
pub fn variance(cv: Option<f64>) -> String {
    percent(cv.map(|c| c * 100.0))
}

/// The y of a smoothness value in the 0 0 360 72 viewbox (0 % at the
/// bottom, 100 % near the top).
#[must_use]
pub fn corridor_y(smoothness: f64) -> f32 {
    let v = if smoothness.is_finite() {
        smoothness.clamp(0.0, 100.0)
    } else {
        0.0
    };
    (72.0 - v / 100.0 * 64.0 - 4.0) as f32
}

/// The smoothness corridor in the `SparklineChart` viewbox. `None` with no
/// point (the screen then says so instead of drawing a fake line).
#[must_use]
pub fn corridor_path(points: &[CorridorPoint]) -> Option<ChartPath> {
    if points.is_empty() {
        return None;
    }
    let n = points.len();
    let mut pts: Vec<(f32, f32)> = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = if n == 1 {
                0.0
            } else {
                i as f32 * 360.0 / (n - 1) as f32
            };
            (x, corridor_y(p.smoothness))
        })
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
    let line = line.trim_end().to_owned();
    let area = format!("{line} L 360 72 L 0 72 Z");
    let (peak_x, peak_y) = pts
        .iter()
        .copied()
        .fold((0.0, 72.0), |b, p| if p.1 < b.1 { p } else { b });
    Some(ChartPath {
        line,
        area,
        peak_x,
        peak_y,
    })
}

/// The caption under the corridor.
#[must_use]
pub fn corridor_footer(report: &FlowReport) -> String {
    let c = report.counts;
    let tempo = report
        .mean_ms
        .map(|m| format!(" · average beat {m:.0} ms"))
        .unwrap_or_default();
    format!(
        "{} scored keystrokes{tempo} · {} hesitation{} · {} pause{} · {} correction{}",
        c.scored,
        c.hesitations,
        if c.hesitations == 1 { "" } else { "s" },
        c.pauses,
        if c.pauses == 1 { "" } else { "s" },
        c.corrections,
        if c.corrections == 1 { "" } else { "s" }
    )
}

/// The key cap text of a weak key.
#[must_use]
pub fn key_label(key: char) -> String {
    match key {
        ' ' => "Space".to_owned(),
        c => c.to_string(),
    }
}

/// One weak key's line, about rhythm first.
#[must_use]
pub fn weak_detail(k: &WeakKey, baseline_ms: Option<f64>) -> String {
    let slow = match (k.mean_latency_ms, baseline_ms) {
        (Some(m), Some(b)) if b > 0.0 && m > b * 1.15 => {
            format!(" · {:.0}% slower than your beat", (m / b - 1.0) * 100.0)
        }
        _ => String::new(),
    };
    format!(
        "missed {} of {} ({:.0}% clean){slow}",
        k.errors,
        k.attempts,
        k.accuracy()
    )
}

/// The drill's stage line.
#[must_use]
pub fn drill_stage(stage: DrillStage) -> &'static str {
    match stage {
        DrillStage::Key => "Isolate the key, then real words",
        DrillStage::Word => "Real words from this passage",
    }
}

// --- Flow Exercise chooser ---------------------------------------------------

/// A Flow card's rhythm line: "Smooth 94% · best 97%", "—" before any
/// scored rhythm (never an invented figure).
#[must_use]
pub fn card_rhythm(flow: Option<(f64, f64)>) -> String {
    match flow {
        Some((last, best)) if last.is_finite() && best.is_finite() => format!(
            "Smooth {:.0}% · best {:.0}%",
            last.clamp(0.0, 100.0),
            best.clamp(0.0, 100.0)
        ),
        _ => "—".to_owned(),
    }
}

// --- Flow Typing (live) --------------------------------------------------------

/// Hesitation marks kept per session (the buffer is allocated once).
pub const HESITATION_MARKS: usize = 64;

/// The live Flow feedback beside the tracker: the run of smooth beats,
/// where hesitations landed, and the keys missed most. Updated once per
/// key in O(1) (the key table is only scanned after a miss); no allocation
/// after [`LiveFlow::new`].
#[derive(Debug, Clone, PartialEq)]
pub struct LiveFlow {
    /// Smooth scored beats in a row right now.
    pub streak: u32,
    /// The longest run this session.
    pub best_streak: u32,
    /// Word numbers (1-based) a hesitation landed on, in typing order.
    pub marks: Vec<u32>,
    /// The three keys missed most (at least twice), worst first.
    pub weak: [Option<(char, u32)>; 3],
    scored: u64,
    hesitations: u32,
    pauses: u32,
}

impl Default for LiveFlow {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveFlow {
    #[must_use]
    pub fn new() -> Self {
        LiveFlow {
            streak: 0,
            best_streak: 0,
            marks: Vec::with_capacity(HESITATION_MARKS),
            weak: [None; 3],
            scored: 0,
            hesitations: 0,
            pauses: 0,
        }
    }

    /// Forget the session (keeps the buffer).
    pub fn reset(&mut self) {
        self.streak = 0;
        self.best_streak = 0;
        self.marks.clear();
        self.weak = [None; 3];
        self.scored = 0;
        self.hesitations = 0;
        self.pauses = 0;
    }

    /// Hesitations seen so far.
    #[must_use]
    pub fn hesitations(&self) -> u32 {
        self.hesitations
    }

    /// Read the tracker after one key aimed at word number `word`;
    /// `missed` when that key was wrong. Returns whether the weak keys
    /// changed (the only part worth a model rewrite).
    pub fn after_key(&mut self, t: &FlowTracker, word: u32, missed: bool) -> bool {
        let c = t.counts();
        if c.pauses != self.pauses {
            self.pauses = c.pauses;
            self.streak = 0;
        }
        if c.scored != self.scored {
            self.scored = c.scored;
            let mut newest = [CadenceBar::default()];
            t.fill_bars(&mut newest);
            if newest[0].smooth {
                self.streak = self.streak.saturating_add(1);
                self.best_streak = self.best_streak.max(self.streak);
            } else {
                self.streak = 0;
            }
        }
        if c.hesitations != self.hesitations {
            let new = c.hesitations.saturating_sub(self.hesitations);
            self.hesitations = c.hesitations;
            for _ in 0..new {
                if self.marks.len() < HESITATION_MARKS {
                    self.marks.push(word);
                }
            }
        }
        if !missed {
            return false;
        }
        let weak = top_missed(t.keys());
        let changed = weak != self.weak;
        self.weak = weak;
        changed
    }
}

/// The three keys of `keys` missed most (2+ misses; Space excluded, as in
/// the report), worst first, ties in key order. No allocation.
#[must_use]
pub fn top_missed(keys: &[KeyTally; KEY_SLOTS]) -> [Option<(char, u32)>; 3] {
    let mut top: [Option<(char, u32)>; 3] = [None; 3];
    for (slot, k) in keys.iter().enumerate() {
        let Some(c) = slot_char(slot) else { continue };
        if c == ' ' || k.errors < 2 {
            continue;
        }
        let mut item = Some((c, k.errors));
        for t in &mut top {
            let better = match (item, *t) {
                (Some((_, e)), Some((_, te))) => e > te,
                (Some(_), None) => true,
                _ => false,
            };
            if better {
                std::mem::swap(t, &mut item);
            }
        }
    }
    top
}

/// The live tempo line: "212 ms a key · ≈ 57 WPM pace"; "—" before the
/// first interval. (5 keys a word: WPM = 12 000 / ms per key.)
#[must_use]
pub fn pace_text(tempo_ms: Option<f64>) -> String {
    match tempo_ms {
        Some(ms) if ms.is_finite() && ms >= 1.0 => {
            format!("{ms:.0} ms a key · ≈ {:.0} WPM pace", 12_000.0 / ms)
        }
        _ => "—".to_owned(),
    }
}

// --- Flow Result -----------------------------------------------------------------

/// One bar of the Result's cadence chart: a slice of the session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CadenceSlice {
    /// The slice's mean interval against the session mean: 0.5 = on the
    /// session's beat, higher = slower.
    pub level: f32,
    /// The slice's smoothness reached the steady threshold.
    pub steady: bool,
}

/// The cadence chart of a report: one bar per corridor slice. Empty when
/// nothing was scored.
#[must_use]
pub fn cadence_chart(
    points: &[CorridorPoint],
    mean_ms: Option<f64>,
    steady: f64,
) -> Vec<CadenceSlice> {
    let Some(mean) = mean_ms.filter(|m| m.is_finite() && *m > 0.0) else {
        return Vec::new();
    };
    points
        .iter()
        .map(|p| CadenceSlice {
            level: if p.mean_ms.is_finite() {
                (p.mean_ms / (2.0 * mean)).clamp(0.04, 1.0) as f32
            } else {
                0.04
            },
            steady: p.smoothness.is_finite() && p.smoothness >= steady,
        })
        .collect()
}

/// One hesitation point in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HesitationPoint {
    /// 1-based word number.
    pub word_no: u32,
    pub word: String,
    /// Hesitations that landed on it.
    pub count: u32,
}

/// Where the hesitations landed: the words of the passage (`words`, word
/// number = index + 1) with their counts, in text order. When there are
/// more than `max`, the words hit most are kept.
#[must_use]
pub fn hesitation_points(words: &[&str], marks: &[u32], max: usize) -> Vec<HesitationPoint> {
    let mut points: Vec<HesitationPoint> = Vec::new();
    for &n in marks {
        let Some(word) = n
            .checked_sub(1)
            .and_then(|i| words.get(usize::try_from(i).ok()?))
        else {
            continue;
        };
        match points.iter_mut().find(|p| p.word_no == n) {
            Some(p) => p.count = p.count.saturating_add(1),
            None => points.push(HesitationPoint {
                word_no: n,
                word: (*word).to_owned(),
                count: 1,
            }),
        }
    }
    if points.len() > max {
        points.sort_by(|a, b| b.count.cmp(&a.count).then(a.word_no.cmp(&b.word_no)));
        points.truncate(max);
    }
    points.sort_by_key(|p| p.word_no);
    points
}

/// The comparison with the personal best: the line and its tone token
/// ("best" | "close" | "below" | "first" | "none"). `prev_exercise` and
/// `prev_any` are the bests before this session.
#[must_use]
pub fn best_line(
    now: Option<f64>,
    prev_exercise: Option<f64>,
    prev_any: Option<f64>,
) -> (String, &'static str) {
    let Some(now) = now.filter(|v| v.is_finite()) else {
        return (
            "No rhythm was scored, so there is nothing to compare yet".to_owned(),
            "none",
        );
    };
    match (
        prev_exercise.filter(|v| v.is_finite()),
        prev_any.filter(|v| v.is_finite()),
    ) {
        (Some(b), _) if now > b => (
            format!("New personal best on this flow: {now:.0}% (was {b:.0}%)"),
            "best",
        ),
        (Some(b), _) if b - now < 3.0 => (
            format!("{now:.0}% · within 3 points of your best here ({b:.0}%)"),
            "close",
        ),
        (Some(b), _) => (
            format!(
                "{now:.0}% · {:.0} points below your best here ({b:.0}%)",
                b - now
            ),
            "below",
        ),
        (None, Some(any)) => (
            format!("First rhythm on this flow: {now:.0}% · your best on any flow is {any:.0}%"),
            "first",
        ),
        (None, None) => (format!("Your first Flow rhythm: {now:.0}%"), "first"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radha_core::flow::{FlowConfig, FlowTracker};
    use std::time::{Duration, Instant};

    #[test]
    fn card_and_pace_lines_are_real_or_a_dash() {
        assert_eq!(card_rhythm(None), "—");
        assert_eq!(card_rhythm(Some((f64::NAN, 1.0))), "—");
        assert_eq!(card_rhythm(Some((93.6, 97.2))), "Smooth 94% · best 97%");
        assert_eq!(pace_text(None), "—");
        assert_eq!(pace_text(Some(0.0)), "—");
        assert_eq!(pace_text(Some(200.0)), "200 ms a key · ≈ 60 WPM pace");
    }

    #[test]
    fn live_flow_counts_runs_hesitations_and_misses() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let mut live = LiveFlow::new();
        let t0 = Instant::now();
        let mut ms = 0u64;
        let mut press =
            |t: &mut FlowTracker, live: &mut LiveFlow, gap: u64, key: char, ok: bool, word: u32| {
                ms += gap;
                t.key_at(t0 + Duration::from_millis(ms), key, ok);
                live.after_key(t, word, !ok)
            };
        for i in 0..40 {
            press(&mut t, &mut live, 200, 'a', true, 1 + i / 5);
        }
        assert!(
            live.streak > 20,
            "an even beat builds a run: {}",
            live.streak
        );
        let run = live.streak;
        // One long gap (a hesitation) breaks the run and marks word 9.
        press(&mut t, &mut live, 700, 'b', true, 9);
        assert_eq!(live.streak, 0);
        assert_eq!(live.best_streak, run);
        assert_eq!(live.hesitations(), t.counts().hesitations);
        assert_eq!(live.marks, vec![9]);
        // Misses: a key is weak only from its second miss.
        assert!(!press(&mut t, &mut live, 200, 'q', false, 10));
        assert!(press(&mut t, &mut live, 200, 'q', false, 10));
        assert_eq!(live.weak[0], Some(('q', 2)));
        live.reset();
        assert_eq!(live, LiveFlow::new());
        assert!(live.marks.capacity() >= HESITATION_MARKS);
    }

    #[test]
    fn hesitation_marks_never_grow_past_the_buffer() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let mut live = LiveFlow::new();
        let t0 = Instant::now();
        let mut ms = 0;
        for i in 0..2_000u64 {
            // Nine beats and a hesitation, over and over.
            ms += if i % 10 == 9 { 1_500 } else { 200 };
            t.key_at(t0 + Duration::from_millis(ms), 'a', true);
            live.after_key(&t, 1, false);
        }
        assert_eq!(live.marks.len(), HESITATION_MARKS);
        assert_eq!(live.marks.capacity(), HESITATION_MARKS);
    }

    #[test]
    fn the_cadence_chart_and_hesitation_points_use_real_data() {
        assert!(cadence_chart(&[], Some(200.0), 80.0).is_empty());
        let pts = [
            CorridorPoint {
                smoothness: 90.0,
                mean_ms: 200.0,
            },
            CorridorPoint {
                smoothness: 40.0,
                mean_ms: 400.0,
            },
        ];
        assert!(cadence_chart(&pts, None, 80.0).is_empty());
        let c = cadence_chart(&pts, Some(200.0), 80.0);
        assert_eq!(
            c[0],
            CadenceSlice {
                level: 0.5,
                steady: true
            }
        );
        assert_eq!(
            c[1],
            CadenceSlice {
                level: 1.0,
                steady: false
            }
        );

        let words = ["the", "quick", "brown", "fox"];
        let p = hesitation_points(&words, &[3, 2, 3, 99, 0], 8);
        assert_eq!(
            p,
            vec![
                HesitationPoint {
                    word_no: 2,
                    word: "quick".into(),
                    count: 1
                },
                HesitationPoint {
                    word_no: 3,
                    word: "brown".into(),
                    count: 2
                },
            ]
        );
        let p = hesitation_points(&words, &[1, 3, 3, 4], 1);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].word, "brown");
    }

    #[test]
    fn the_best_comparison_is_honest() {
        assert_eq!(best_line(None, Some(90.0), None).1, "none");
        assert_eq!(
            best_line(Some(94.0), Some(91.0), Some(99.0)),
            (
                "New personal best on this flow: 94% (was 91%)".to_owned(),
                "best"
            )
        );
        assert_eq!(best_line(Some(90.0), Some(92.0), None).1, "close");
        assert_eq!(
            best_line(Some(80.0), Some(92.0), None).0,
            "80% · 12 points below your best here (92%)"
        );
        assert_eq!(best_line(Some(80.0), None, Some(92.0)).1, "first");
        assert_eq!(
            best_line(Some(80.0), None, None).0,
            "Your first Flow rhythm: 80%"
        );
    }

    #[test]
    fn the_home_preview_never_invents_a_figure() {
        assert_eq!(home_smoothness(None), "—");
        assert_eq!(home_smoothness(Some(f64::NAN)), "—");
        assert_eq!(home_smoothness(Some(93.6)), "94%");
        assert_eq!(home_smoothness(Some(140.0)), "100%");
        assert_eq!(percent(None), "—");
        assert_eq!(variance(Some(0.214)), "21%");
    }

    #[test]
    fn the_corridor_fits_the_viewbox() {
        assert!(corridor_path(&[]).is_none());
        let one = corridor_path(&[CorridorPoint {
            smoothness: 50.0,
            mean_ms: 200.0,
        }])
        .unwrap();
        assert!(one.line.starts_with("M 0.0 36.0") && one.line.ends_with("L 360.0 36.0"));
        let pts: Vec<CorridorPoint> = [100.0, 0.0, 75.0, f64::NAN]
            .into_iter()
            .map(|s| CorridorPoint {
                smoothness: s,
                mean_ms: 1.0,
            })
            .collect();
        let c = corridor_path(&pts).unwrap();
        assert!(c.area.ends_with("L 360 72 L 0 72 Z"));
        assert_eq!(corridor_y(100.0), 4.0);
        assert_eq!(corridor_y(0.0), 68.0);
        assert_eq!((c.peak_x, c.peak_y), (0.0, 4.0));
        for v in c.line.split(' ').filter_map(|t| t.parse::<f32>().ok()) {
            assert!((0.0..=360.0).contains(&v));
        }
    }

    #[test]
    fn report_wording_counts_honestly() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let t0 = Instant::now();
        for i in 0..60u64 {
            t.key_at(t0 + Duration::from_millis(i * 200), 'a', i != 30);
        }
        let r = FlowReport::from_tracker(&t);
        let footer = corridor_footer(&r);
        assert!(footer.starts_with("55 scored keystrokes"), "{footer}");
        assert!(footer.contains("0 hesitations · 0 pauses · 0 corrections"));
        let k = WeakKey {
            key: ' ',
            attempts: 10,
            errors: 2,
            mean_latency_ms: Some(300.0),
            score: 30.0,
        };
        assert_eq!(key_label(k.key), "Space");
        assert_eq!(
            weak_detail(&k, Some(200.0)),
            "missed 2 of 10 (80% clean) · 50% slower than your beat"
        );
        assert_eq!(weak_detail(&k, None), "missed 2 of 10 (80% clean)");
        let row = record(&r);
        assert_eq!(row.engine_schema, 1);
        assert_eq!(row.scored_intervals, 55);
        assert_eq!(row.smoothness, r.smoothness);
    }

    #[test]
    fn begin_labels_state_the_timer() {
        use radha_core::{TimeLimit, TimerPreset};
        let two = TimerMode::Countdown(TimeLimit::from_preset(TimerPreset::Min2));
        assert_eq!(begin_label(two), "Begin Flow · 02:00");
        assert_eq!(begin_label(TimerMode::Off), "Begin Flow · untimed");
        assert_eq!(begin_label(TimerMode::Stopwatch), "Begin Flow · stopwatch");
    }

    #[test]
    fn the_dial_follows_the_timer_itself() {
        use radha_core::{TimeLimit, TimerPreset};
        let one = TimerMode::Countdown(TimeLimit::from_preset(TimerPreset::Min1));
        assert_eq!(dial_index(one), Some(1));
        let five = TimerMode::Countdown(TimeLimit::from_preset(TimerPreset::Min5));
        assert_eq!(dial_index(five), Some(4));
        let custom = TimerMode::Countdown(TimeLimit::parse_mm_ss("1:30").unwrap());
        assert_eq!(dial_index(custom), None);
        assert_eq!(dial_index(TimerMode::Stopwatch), None);
        assert_eq!(dial_index(TimerMode::Off), None);
    }
}
