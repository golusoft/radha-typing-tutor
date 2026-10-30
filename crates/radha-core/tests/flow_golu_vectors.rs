//! Shared vectors: Golu Typing OS's Flow/adaptive formulas, ported verbatim,
//! against Radha's Flow engine on the same inputs (P07 "compare against
//! Golu OS algorithm outputs on shared vectors").
//!
//! The ports are line-for-line copies of the Golu sources (read-only
//! reference, never modified):
//!
//! - `golu_weakness_score`: `src-tauri/src/adaptive/weak_keys.rs:28-55`
//! - `golu_consistency`: `src/lib/typingEngine.ts:489-497` (per-second WPM)
//!   and `:575-586` (100 − CV, clamped; null under two seconds)
//! - `golu_latency_cv`: the Welford latency variance Golu tracks but no
//!   longer reports, `typingEngine.ts:372-379`
//! - `golu_stage_for`: `src-tauri/src/adaptive/drill.rs:128-151`
//!
//! Where the outputs agree the test asserts equality; where they differ on
//! purpose the test asserts the documented difference
//! (`docs/flow-gap-log.md`, "Shared vectors"). Each vector's numbers are
//! printed for the verification note.

use std::time::{Duration, Instant};

use radha_core::flow::{weakness_score, FlowConfig, FlowTracker, WeakKey};

// --- Golu ports ---------------------------------------------------------------

/// `weak_keys.rs:28-55`, verbatim.
fn golu_weakness_score(
    error_rate: f64,
    average_latency_ms: Option<f64>,
    baseline_latency_ms: Option<f64>,
    attempts: i64,
    trend: f64,
) -> f64 {
    let error_rate = error_rate.clamp(0.0, 1.0);
    let error_component = error_rate * 60.0;
    let latency_component = match (average_latency_ms, baseline_latency_ms) {
        (Some(latency), Some(baseline)) if baseline > 0.0 => {
            ((latency - baseline) / baseline).clamp(0.0, 1.0) * 25.0
        }
        _ => 0.0,
    };
    let error_count = error_rate * attempts as f64;
    let recurrence_component = (error_count / 10.0).min(1.0) * 15.0;
    let trend_component = trend.clamp(-1.0, 1.0) * 20.0;
    (error_component + latency_component + recurrence_component - trend_component).clamp(0.0, 100.0)
}

/// `typingEngine.ts` per-second consistency. `keys` are (ms since start,
/// correct). The first key starts the clock; the session finishes on the
/// last key.
fn golu_consistency(keys: &[(f64, bool)]) -> Option<f64> {
    let start = keys.first()?.0;
    let mut correct = 0u64;
    let mut seconds_closed = 0u64;
    let mut chars_at_last = 0u64;
    let mut second_wpm: Vec<f64> = Vec::new();
    let mut close = |elapsed: f64, correct: u64, second_wpm: &mut Vec<f64>| {
        let completed = (elapsed / 1000.0).floor() as u64;
        while seconds_closed < completed {
            let chars = correct - chars_at_last;
            chars_at_last = correct;
            seconds_closed += 1;
            second_wpm.push((chars as f64 / 5.0) * 60.0);
        }
    };
    for &(t, ok) in keys {
        if ok {
            correct += 1;
        }
        let elapsed = (t - start).max(0.0);
        if elapsed > 0.0 {
            close(elapsed, correct, &mut second_wpm);
        }
    }
    if second_wpm.len() < 2 {
        return None;
    }
    let count = second_wpm.len() as f64;
    let mean = second_wpm.iter().sum::<f64>() / count;
    if mean <= 0.0 {
        return None;
    }
    let variance = second_wpm.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count;
    let cv = variance.sqrt() / mean;
    Some((100.0 - cv * 100.0).clamp(0.0, 100.0))
}

/// `typingEngine.ts:372-379` Welford over every inter-key latency.
fn golu_latency_cv(keys: &[(f64, bool)]) -> Option<f64> {
    let (mut n, mut mean, mut m2) = (0.0_f64, 0.0_f64, 0.0_f64);
    for w in keys.windows(2) {
        let latency = w[1].0 - w[0].0;
        n += 1.0;
        let delta = latency - mean;
        mean += delta / n;
        m2 += delta * (latency - mean);
    }
    (n > 0.0 && mean > 0.0).then(|| (m2 / n).sqrt() / mean)
}

#[derive(Debug, PartialEq, Eq)]
enum GoluStage {
    Key,
    Bigram,
    Trigram,
    Word,
    Sentence,
}

/// `drill.rs:128-151` for one signal (`value`, attempts, errors).
fn golu_stage_for(value: &str, attempts: i64, errors: i64) -> GoluStage {
    let accuracy = if attempts <= 0 {
        100.0
    } else {
        ((attempts - errors) as f64 / attempts as f64) * 100.0
    };
    let length = value.chars().count();
    if accuracy < 80.0 {
        return if length >= 2 {
            GoluStage::Bigram
        } else {
            GoluStage::Key
        };
    }
    if accuracy >= 95.0 {
        return GoluStage::Sentence;
    }
    match length {
        0 | 1 => GoluStage::Bigram,
        2 => GoluStage::Trigram,
        _ => GoluStage::Word,
    }
}

// --- Shared streams -------------------------------------------------------------

/// SplitMix64 (Steele, Lea, Flood 2014).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

const TEXT: &str = "the quick brown fox jumps over the lazy dog ";

/// (name, gaps in ms between consecutive keys).
fn streams() -> Vec<(&'static str, Vec<u64>)> {
    let mut rng = Rng(0x00C0_FFEE);
    let chars: Vec<char> = TEXT.chars().collect();
    let boundary: Vec<u64> = (1..400)
        .map(|i| {
            if chars[i % chars.len()] == ' ' {
                320
            } else {
                200
            }
        })
        .collect();
    let mut pause = vec![200u64; 400];
    pause[200] = 3000;
    vec![
        ("A even 200 ms", vec![200; 400]),
        ("B word boundaries 1.6x", boundary),
        ("C one 3 s pause", pause),
        (
            "D jitter +/-40%",
            (0..400).map(|_| 120 + rng.next() % 161).collect(),
        ),
        (
            "E tempo 300 -> 150 ms",
            (0..400).map(|i| 300 - i * 150 / 400).collect(),
        ),
        (
            "F clustered 4 x 40 ms + 840 ms",
            (0..400)
                .map(|i| if i % 5 == 4 { 840 } else { 40 })
                .collect(),
        ),
    ]
}

struct Outcome {
    golu_consistency: Option<f64>,
    golu_latency_cv: Option<f64>,
    smoothness: Option<f64>,
    cadence_cv: Option<f64>,
    pauses: u32,
}

fn run(gaps: &[u64]) -> Outcome {
    let chars: Vec<char> = TEXT.chars().collect();
    let mut t = FlowTracker::new(FlowConfig::default());
    let t0 = Instant::now();
    let mut ms = 0u64;
    let mut keys = vec![(0.0, true)];
    t.key_at(t0, chars[0], true);
    for (i, g) in gaps.iter().enumerate() {
        ms += g;
        keys.push((ms as f64, true));
        t.key_at(
            t0 + Duration::from_millis(ms),
            chars[(i + 1) % chars.len()],
            true,
        );
    }
    Outcome {
        golu_consistency: golu_consistency(&keys),
        golu_latency_cv: golu_latency_cv(&keys),
        smoothness: t.smoothness(),
        cadence_cv: t.cadence_cv(),
        pauses: t.counts().pauses,
    }
}

fn fmt(v: Option<f64>) -> String {
    v.map_or_else(|| "null".to_owned(), |v| format!("{v:.3}"))
}

#[test]
fn rhythm_vectors_match_where_intended_and_differ_where_documented() {
    eprintln!("vector | golu consistency | golu latency CV | radha smoothness | radha cadence CV | radha pauses");
    let mut results = Vec::new();
    for (name, gaps) in streams() {
        let o = run(&gaps);
        eprintln!(
            "{name} | {} | {} | {} | {} | {}",
            fmt(o.golu_consistency),
            fmt(o.golu_latency_cv),
            fmt(o.smoothness),
            fmt(o.cadence_cv),
            o.pauses
        );
        results.push((name, o));
    }
    let get = |p: &str| &results.iter().find(|(n, _)| n.starts_with(p)).unwrap().1;

    // A: a metronome. Radha: perfect. Golu: just under 100, because the key
    // that closes a second is counted inside it (typingEngine.ts:311 then
    // :489), so the first second holds 6 keys and the rest 5.
    let a = get("A");
    let golu_a = a.golu_consistency.unwrap();
    assert!(golu_a > 95.0 && golu_a < 100.0, "{golu_a}");
    assert_eq!(a.smoothness, Some(100.0));
    assert_eq!(a.cadence_cv, Some(0.0));
    assert_eq!(a.golu_latency_cv, Some(0.0));

    // B: word-boundary slowdowns are normal rhythm. Radha's boundary band
    // keeps them smooth; Golu's per-second WPM wobbles with word length.
    let b = get("B");
    assert_eq!(b.smoothness, Some(100.0));
    assert!(b.golu_consistency.unwrap() < 100.0);

    // C: one 3 s pause. Golu folds three empty seconds into the pace series
    // and the latency CV explodes; Radha counts a pause, restarts the rhythm
    // and scores the typing on either side as it was: perfectly even.
    let c = get("C");
    assert_eq!(c.pauses, 1);
    assert_eq!(c.smoothness, Some(100.0));
    assert_eq!(c.cadence_cv, Some(0.0));
    assert!(c.golu_consistency.unwrap() < a.golu_consistency.unwrap());
    assert!(c.golu_latency_cv.unwrap() > 0.5);

    // D: honest jitter is seen by both, and Radha's cadence CV over scored
    // intervals equals Golu's latency CV within the few warm-up intervals.
    let d = get("D");
    assert!(d.smoothness.unwrap() > 85.0 && d.smoothness.unwrap() <= 100.0);
    assert!((d.cadence_cv.unwrap() - d.golu_latency_cv.unwrap()).abs() < 0.03);

    // E: a steady acceleration. Radha's tempo follows the typist, so the
    // rhythm stays smooth; the CV still reports the spread of the session.
    let e = get("E");
    assert_eq!(e.smoothness, Some(100.0));
    assert!(e.cadence_cv.unwrap() > 0.1);

    // F: five keys crammed into the first 160 ms of every second, then a
    // wait. Exactly 5 keys per second, so Golu's per-second pace calls it
    // as even as the metronome; Radha hears the stop-start and calls it
    // uneven.
    let f = get("F");
    assert!(f.smoothness.unwrap() < 70.0, "{:?}", f.smoothness);
    assert!(f.golu_consistency.unwrap() > 95.0);
}

#[test]
fn weakness_scores_are_golus_formula_made_total() {
    let cfg = FlowConfig::default();
    /// (error rate, latency, baseline, attempts, trend)
    type Vector = (f64, Option<f64>, Option<f64>, u32, f64);
    let vectors: [Vector; 8] = [
        (0.5, None, None, 10, 0.0),
        (0.0, Some(200.0), Some(100.0), 10, 0.0),
        (0.0, Some(80.0), Some(100.0), 10, 0.0),
        (0.0, None, None, 500, 0.0),
        (0.3, None, None, 10, 0.8),
        (1.0, Some(1000.0), Some(10.0), 100, -1.0),
        (0.12, Some(260.0), Some(200.0), 40, 0.0),
        (0.02, Some(190.0), Some(200.0), 400, 0.0),
    ];
    for (rate, lat, base, attempts, trend) in vectors {
        let golu = golu_weakness_score(rate, lat, base, i64::from(attempts), trend);
        let radha = weakness_score(&cfg, rate, lat, base, attempts, trend);
        eprintln!(
            "weakness {rate} {lat:?} {base:?} {attempts} {trend}: golu {golu:.3} radha {radha:.3}"
        );
        assert!((golu - radha).abs() < 1e-9, "{golu} vs {radha}");
    }
    // Documented differences: Golu propagates NaN; Radha scores 0.
    assert!(golu_weakness_score(f64::NAN, None, None, 10, 0.0).is_nan());
    assert_eq!(weakness_score(&cfg, f64::NAN, None, None, 10, 0.0), 0.0);
    assert!(golu_weakness_score(0.5, Some(f64::NAN), Some(100.0), 10, 0.0).is_nan());
    assert_eq!(
        weakness_score(&cfg, 0.5, Some(f64::NAN), Some(100.0), 10, 0.0),
        weakness_score(&cfg, 0.5, None, None, 10, 0.0)
    );
    // Negative attempts (a corrupt row) make Golu's recurrence negative.
    assert!(golu_weakness_score(0.5, None, None, -100, 0.0) < 30.0);
}

#[test]
fn drill_stages_follow_golus_accuracy_gate() {
    let cfg = FlowConfig::default();
    let key = |attempts: u32, errors: u32| WeakKey {
        key: 'p',
        attempts,
        errors,
        mean_latency_ms: None,
        score: 50.0,
    };
    // Below 80 % both isolate the key.
    assert_eq!(golu_stage_for("p", 10, 6), GoluStage::Key);
    assert!(key(10, 6).accuracy() < cfg.raw_key_accuracy);
    // Golu's "bigram" rung for a single key repeats the key ("kkk"); Radha
    // goes straight to real words.
    assert_eq!(golu_stage_for("k", 100, 12), GoluStage::Bigram);
    assert!(key(100, 12).accuracy() >= cfg.raw_key_accuracy);
    assert_eq!(golu_stage_for("t", 400, 8), GoluStage::Sentence);
    // More errors than attempts (a corrupt row): Golu reports -50 %
    // accuracy; Radha clamps to 0 %. Both isolate the key.
    assert_eq!(golu_stage_for("p", 10, 15), GoluStage::Key);
    assert_eq!(key(10, 15).accuracy(), 0.0);
    // Zero attempts: both say 100 %.
    assert_eq!(golu_stage_for("p", 0, 0), GoluStage::Sentence);
    assert_eq!(key(0, 0).accuracy(), 100.0);
}
