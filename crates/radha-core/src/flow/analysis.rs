//! The report half of the Flow engine: runs once per session, after the
//! last key, so it may allocate.
//!
//! - [`FlowReport::from_tracker`]: smoothness, cadence variance, interval
//!   classes, weak keys and pairs, and the corridor series.
//! - [`weakness_score`]: the Golu Typing OS formula
//!   (`adaptive/weak_keys.rs`), with its gaps closed: NaN and infinite inputs,
//!   negative counts and a zero or missing baseline all score 0 instead of
//!   propagating NaN, and a key needs `weak_min_attempts` before it can be
//!   called weak (one unlucky press is not a weakness).
//! - [`drill`]: a recommended drill built from the passage's own words, so
//!   it is real English and always typeable on the layout the passage was
//!   validated for. Golu's ladder (key → word) is kept; its gaps (a weak
//!   punctuation key dropped from the drill, a bigram rung that repeats a
//!   single key, ids that collide) are not.
//! - [`recommend`]: one next step, worded about rhythm (DESIGN 8.2).

use super::config::FlowConfig;
use super::tracker::{slot_char, CorridorBucket, FlowTracker, IntervalCounts, KEY_SLOTS};

/// One weak key.
#[derive(Debug, Clone, PartialEq)]
pub struct WeakKey {
    /// The key's character.
    pub key: char,
    /// Times it was expected.
    pub attempts: u32,
    /// Times it was missed.
    pub errors: u32,
    /// Mean interval that ended on it (ms).
    pub mean_latency_ms: Option<f64>,
    /// Weakness score, 0–100.
    pub score: f64,
}

impl WeakKey {
    /// Accuracy % on this key (100 with no attempts).
    #[must_use]
    pub fn accuracy(&self) -> f64 {
        if self.attempts == 0 {
            return 100.0;
        }
        let ok = self.attempts.saturating_sub(self.errors.min(self.attempts));
        f64::from(ok) * 100.0 / f64::from(self.attempts)
    }
}

/// One weak letter pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeakPair {
    /// The two letters, lowercase.
    pub pair: [char; 2],
    /// Times typed.
    pub attempts: u32,
    /// Times failed.
    pub errors: u32,
}

/// One corridor point for the Result graph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorridorPoint {
    /// Smoothness % in the bucket.
    pub smoothness: f64,
    /// Mean interval in the bucket (ms).
    pub mean_ms: f64,
}

/// Everything the Flow Result shows about rhythm.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowReport {
    /// Schema of the configuration that produced it.
    pub schema: u32,
    /// Interval classes.
    pub counts: IntervalCounts,
    /// Smooth scored intervals %, `None` with nothing scored.
    pub smoothness: Option<f64>,
    /// Coefficient of variation of scored intervals.
    pub cadence_cv: Option<f64>,
    /// Mean scored interval (ms).
    pub mean_ms: Option<f64>,
    /// Standard deviation of scored intervals (ms).
    pub stdev_ms: Option<f64>,
    /// Weak keys, worst first.
    pub weak_keys: Vec<WeakKey>,
    /// Weak letter pairs, worst first (at most 3).
    pub weak_pairs: Vec<WeakPair>,
    /// The smoothness corridor, oldest first.
    pub corridor: Vec<CorridorPoint>,
}

/// Golu Typing OS's weakness score (error rate, latency vs baseline,
/// recurrence, trend), made total: any non-finite or negative input
/// contributes 0, so the result is always a finite 0–100.
#[must_use]
pub fn weakness_score(
    cfg: &FlowConfig,
    error_rate: f64,
    mean_latency_ms: Option<f64>,
    baseline_latency_ms: Option<f64>,
    attempts: u32,
    trend: f64,
) -> f64 {
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    let error_rate = finite(error_rate).clamp(0.0, 1.0);
    let latency = match (mean_latency_ms, baseline_latency_ms) {
        (Some(l), Some(b)) if l.is_finite() && b.is_finite() && b > 0.0 => {
            ((l - b) / b).clamp(0.0, 1.0)
        }
        _ => 0.0,
    };
    let errors = error_rate * f64::from(attempts);
    let recurrence = (errors / cfg.weak_recurrence_errors.max(1.0)).min(1.0);
    let trend = finite(trend).clamp(-1.0, 1.0);
    let score = error_rate * cfg.weak_error_weight
        + latency * cfg.weak_latency_weight
        + recurrence * cfg.weak_recurrence_weight
        - trend * 20.0;
    finite(score).clamp(0.0, 100.0)
}

impl FlowReport {
    /// Build the report of a tracker.
    #[must_use]
    pub fn from_tracker(t: &FlowTracker) -> Self {
        let cfg = *t.config();
        let baseline = t.mean_ms();
        let mut weak: Vec<WeakKey> = t
            .keys()
            .iter()
            .enumerate()
            .take(KEY_SLOTS)
            .filter_map(|(slot, tally)| {
                let key = slot_char(slot)?;
                if key == ' ' || tally.attempts < cfg.weak_min_attempts.max(1) {
                    return None;
                }
                let mean = (tally.latency_n > 0)
                    .then(|| tally.latency_us as f64 / f64::from(tally.latency_n) / 1000.0);
                let rate = f64::from(tally.errors.min(tally.attempts)) / f64::from(tally.attempts);
                let score = weakness_score(&cfg, rate, mean, baseline, tally.attempts, 0.0);
                (score >= cfg.weak_score && tally.errors > 0).then_some(WeakKey {
                    key,
                    attempts: tally.attempts,
                    errors: tally.errors,
                    mean_latency_ms: mean,
                    score,
                })
            })
            .collect();
        weak.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(b.errors.cmp(&a.errors))
                .then(a.key.cmp(&b.key))
        });
        weak.truncate(cfg.max_weak_keys);

        let mut pairs: Vec<WeakPair> = t
            .pairs()
            .iter()
            .enumerate()
            .filter(|(_, p)| p.attempts >= cfg.weak_min_attempts.max(1) && p.errors > 0)
            .filter_map(|(i, p)| {
                let a = char::from(b'a' + u8::try_from(i / 26).ok()?);
                let b = char::from(b'a' + u8::try_from(i % 26).ok()?);
                Some(WeakPair {
                    pair: [a, b],
                    attempts: p.attempts,
                    errors: p.errors,
                })
            })
            .collect();
        // Worst error rate first (cross-multiplied: no division).
        pairs.sort_by(|x, y| {
            (u64::from(y.errors) * u64::from(x.attempts))
                .cmp(&(u64::from(x.errors) * u64::from(y.attempts)))
                .then(y.errors.cmp(&x.errors))
                .then(x.pair.cmp(&y.pair))
        });
        pairs.truncate(3);

        let corridor = t
            .corridor()
            .iter()
            .filter(|b| b.scored > 0)
            .map(|b: &CorridorBucket| CorridorPoint {
                smoothness: f64::from(b.smooth) * 100.0 / f64::from(b.scored),
                mean_ms: b.sum_us as f64 / f64::from(b.scored) / 1000.0,
            })
            .collect();

        FlowReport {
            schema: cfg.schema,
            counts: t.counts(),
            smoothness: t.smoothness(),
            cadence_cv: t.cadence_cv(),
            mean_ms: t.mean_ms(),
            stdev_ms: t.stdev_ms(),
            weak_keys: weak,
            weak_pairs: pairs,
            corridor,
        }
    }

    /// The hero sentence of the Result screen.
    #[must_use]
    pub fn headline(&self) -> String {
        match self.smoothness {
            Some(s) => format!("Your rhythm held steady for {s:.0}% of this session"),
            None => "Too few keystrokes to read your rhythm yet".to_owned(),
        }
    }
}

/// Which rung a drill starts on (Golu's ladder, trimmed to what a Flow
/// drill needs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrillStage {
    /// The weakest key is below `raw_key_accuracy`: isolate it first.
    Key,
    /// Real words that carry the weak keys and pairs.
    Word,
}

/// A recommended drill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowDrill {
    /// Where it starts.
    pub stage: DrillStage,
    /// The keys it targets.
    pub keys: Vec<char>,
    /// The text to type.
    pub text: String,
}

/// Trim ASCII punctuation from both ends of a token.
fn core_word(token: &str) -> &str {
    token.trim_matches(|c: char| c.is_ascii_punctuation())
}

/// Build the drill for `report` from words of `source` (the passage just
/// typed). `None` when there is nothing to work on or the passage holds no
/// usable word — never an invented drill.
#[must_use]
pub fn drill(report: &FlowReport, source: &str, cfg: &FlowConfig) -> Option<FlowDrill> {
    let mut keys: Vec<char> = report.weak_keys.iter().map(|k| k.key).collect();
    for p in &report.weak_pairs {
        for c in p.pair {
            if !keys.contains(&c) {
                keys.push(c);
            }
        }
    }
    if keys.is_empty() {
        return None;
    }
    let lower: Vec<char> = keys.iter().map(char::to_ascii_lowercase).collect();
    let pairs: Vec<String> = report
        .weak_pairs
        .iter()
        .map(|p| p.pair.iter().collect())
        .collect();
    // Candidate words: ASCII tokens of the passage holding a target key
    // (letters case-insensitively, punctuation as typed), scored by how
    // many targets they carry; first-seen order breaks ties.
    let mut seen: Vec<&str> = Vec::new();
    let mut scored: Vec<(usize, usize, &str)> = Vec::new();
    for token in source.split_whitespace() {
        if !token.is_ascii() || token.len() > 24 || seen.contains(&token) {
            continue;
        }
        let word = core_word(token);
        if word.len() < 2 {
            continue;
        }
        let lw = token.to_ascii_lowercase();
        let mut hits = lower.iter().filter(|c| lw.contains(**c)).count();
        hits += pairs.iter().filter(|p| lw.contains(p.as_str())).count() * 2;
        if hits > 0 {
            scored.push((hits, seen.len(), token));
            seen.push(token);
        }
        if seen.len() >= 400 {
            break;
        }
    }
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let pool: Vec<&str> = scored.iter().take(24).map(|s| s.2).collect();

    let worst = report
        .weak_keys
        .iter()
        .map(WeakKey::accuracy)
        .fold(100.0_f64, f64::min);
    let stage = if worst < cfg.raw_key_accuracy {
        DrillStage::Key
    } else {
        DrillStage::Word
    };
    let mut parts: Vec<String> = Vec::new();
    if stage == DrillStage::Key {
        for k in report
            .weak_keys
            .iter()
            .filter(|k| k.accuracy() < cfg.raw_key_accuracy)
            .take(3)
        {
            let group = k.key.to_string().repeat(3);
            for _ in 0..cfg.key_reps.max(1) {
                parts.push(group.clone());
            }
        }
    }
    let words = cfg.drill_words.max(1);
    parts.extend(pool.iter().cycle().take(words).map(|w| (*w).to_owned()));
    Some(FlowDrill {
        stage,
        keys,
        text: parts.join(" "),
    })
}

/// What to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendationKind {
    /// Too little was typed to judge.
    KeepTyping,
    /// Accuracy is too low for rhythm work: ease off.
    EaseOff,
    /// The rhythm broke often: slow to a pace you can hold.
    EvenOut,
    /// A few keys break the flow: drill them.
    DrillWeakKeys,
    /// Smooth and even: step up the duration or the next passage.
    StepUp,
    /// Good; keep the same pace.
    HoldPace,
}

/// The recommendation shown on the Result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    /// Which one.
    pub kind: RecommendationKind,
    /// One line.
    pub headline: String,
    /// One or two sentences of why.
    pub detail: String,
}

/// Accuracy below which rhythm advice yields to accuracy advice.
const EASE_OFF_ACCURACY: f64 = 90.0;

/// The next step for `report`, given the session's accuracy %.
#[must_use]
pub fn recommend(report: &FlowReport, accuracy: f64, cfg: &FlowConfig) -> Recommendation {
    let rec = |kind, headline: String, detail: String| Recommendation {
        kind,
        headline,
        detail,
    };
    let (Some(smooth), Some(cv)) = (report.smoothness, report.cadence_cv) else {
        return rec(
            RecommendationKind::KeepTyping,
            "Keep typing to read your rhythm".to_owned(),
            "A rhythm needs a few dozen keystrokes in a row. Try a longer session.".to_owned(),
        );
    };
    let accuracy = if accuracy.is_finite() { accuracy } else { 0.0 };
    let keys: Vec<String> = report
        .weak_keys
        .iter()
        .map(|k| match k.key {
            ' ' => "Space".to_owned(),
            c => c.to_string(),
        })
        .collect();
    if accuracy < EASE_OFF_ACCURACY {
        return rec(
            RecommendationKind::EaseOff,
            "Ease off the pace a little".to_owned(),
            format!(
                "Accuracy was {accuracy:.0}%. Flow builds on clean keys: slow down until \
                 almost every key lands, and the speed will come back on its own."
            ),
        );
    }
    if smooth < cfg.settling_smoothness {
        return rec(
            RecommendationKind::EvenOut,
            "Even out your cadence".to_owned(),
            format!(
                "Only {smooth:.0}% of your keystrokes kept the beat, with {} hesitation{}. \
                 Pick a pace you can hold through every word, even if it feels slow.",
                report.counts.hesitations,
                if report.counts.hesitations == 1 {
                    ""
                } else {
                    "s"
                }
            ),
        );
    }
    if !keys.is_empty() {
        return rec(
            RecommendationKind::DrillWeakKeys,
            format!("Smooth out {}", keys.join(" · ")),
            "These keys broke your flow most often. A short drill of real words from \
             this passage targets them."
                .to_owned(),
        );
    }
    if smooth >= cfg.steady_smoothness && cv <= cfg.even_cv {
        return rec(
            RecommendationKind::StepUp,
            "Ready for a longer flow".to_owned(),
            format!(
                "{smooth:.0}% smooth with a cadence variance of {:.0}%. Add a few minutes \
                 or move to the next passage.",
                cv * 100.0
            ),
        );
    }
    rec(
        RecommendationKind::HoldPace,
        "Hold this pace".to_owned(),
        format!(
            "{smooth:.0}% smooth. Stay at this speed until the variance settles below {:.0}%.",
            cfg.even_cv * 100.0
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn typed(t: &mut FlowTracker, text: &str, miss: &[char], gap_ms: u64) {
        let mut now = Instant::now();
        for c in text.chars() {
            t.key_at(now, c, !miss.contains(&c));
            now += Duration::from_millis(gap_ms);
        }
    }

    #[test]
    fn weakness_score_is_total() {
        let cfg = FlowConfig::default();
        for v in [
            weakness_score(&cfg, f64::NAN, None, None, 10, 0.0),
            weakness_score(&cfg, 0.5, Some(f64::NAN), Some(100.0), 10, 0.0),
            weakness_score(&cfg, 0.5, Some(200.0), Some(0.0), 10, 0.0),
            weakness_score(&cfg, 0.5, Some(200.0), Some(f64::INFINITY), 10, f64::NAN),
            weakness_score(&cfg, -3.0, None, None, 0, 0.0),
            weakness_score(&cfg, 9.0, Some(1e300), Some(1e-300), u32::MAX, -5.0),
        ] {
            assert!(v.is_finite() && (0.0..=100.0).contains(&v), "{v}");
        }
        assert_eq!(weakness_score(&cfg, 0.0, None, None, 500, 0.0), 0.0);
    }

    #[test]
    fn a_missed_key_is_weak_a_clean_one_is_not() {
        let mut t = FlowTracker::new(FlowConfig::default());
        typed(&mut t, &"the quick brown fox ".repeat(10), &['q'], 200);
        let r = FlowReport::from_tracker(&t);
        assert_eq!(r.weak_keys.len(), 1, "{:?}", r.weak_keys);
        assert_eq!(r.weak_keys[0].key, 'q');
        assert_eq!(r.weak_keys[0].accuracy(), 0.0);
        assert!(r.weak_pairs.iter().any(|p| p.pair == ['q', 'u']));
        assert_eq!(r.smoothness, Some(100.0));
        assert!(r.headline().contains("100%"));
    }

    #[test]
    fn too_few_attempts_are_never_weak() {
        let mut t = FlowTracker::new(FlowConfig::default());
        typed(&mut t, "zebra and more words here", &['z'], 200);
        assert!(FlowReport::from_tracker(&t).weak_keys.is_empty());
    }

    #[test]
    fn drills_use_passage_words_and_isolate_a_raw_key() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let text = "the quick brown fox jumps over the lazy dog, quietly. ".repeat(6);
        typed(&mut t, &text, &['q'], 200);
        let r = FlowReport::from_tracker(&t);
        let cfg = FlowConfig::default();
        let d = drill(&r, &text, &cfg).unwrap();
        assert_eq!(d.stage, DrillStage::Key);
        assert!(d.text.starts_with("qqq qqq"), "{}", d.text);
        assert!(d.text.contains("quick") && d.text.contains("quietly."));
        assert!(d.text.is_ascii());
        let words = d.text.split(' ').count();
        assert_eq!(words, cfg.drill_words + cfg.key_reps);
        assert!(!d.text.contains("  "), "no double spaces");
    }

    #[test]
    fn a_punctuation_weak_key_reaches_the_drill() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let text = "one, two, three, four, five, six, seven, eight, ".repeat(3);
        typed(&mut t, &text, &[','], 200);
        let r = FlowReport::from_tracker(&t);
        assert_eq!(r.weak_keys.first().map(|k| k.key), Some(','));
        let d = drill(&r, &text, &FlowConfig::default()).unwrap();
        assert!(d.text.contains("one,"), "{}", d.text);
    }

    #[test]
    fn no_weakness_means_no_drill() {
        let mut t = FlowTracker::new(FlowConfig::default());
        typed(&mut t, &"clean words only ".repeat(10), &[], 200);
        let r = FlowReport::from_tracker(&t);
        assert!(drill(&r, "clean words only", &FlowConfig::default()).is_none());
        assert!(drill(&r, "", &FlowConfig::default()).is_none());
    }

    #[test]
    fn recommendations_follow_the_evidence() {
        let cfg = FlowConfig::default();
        let empty = FlowReport::from_tracker(&FlowTracker::new(cfg));
        assert_eq!(
            recommend(&empty, 100.0, &cfg).kind,
            RecommendationKind::KeepTyping
        );
        let mut t = FlowTracker::new(cfg);
        typed(&mut t, &"steady words flow ".repeat(10), &[], 200);
        let r = FlowReport::from_tracker(&t);
        assert_eq!(recommend(&r, 99.0, &cfg).kind, RecommendationKind::StepUp);
        assert_eq!(recommend(&r, 80.0, &cfg).kind, RecommendationKind::EaseOff);
        assert_eq!(
            recommend(&r, f64::NAN, &cfg).kind,
            RecommendationKind::EaseOff
        );
        let mut uneven = r.clone();
        uneven.smoothness = Some(40.0);
        assert_eq!(
            recommend(&uneven, 99.0, &cfg).kind,
            RecommendationKind::EvenOut
        );
    }
}
