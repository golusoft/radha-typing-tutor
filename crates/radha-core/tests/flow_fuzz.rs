//! Flow engine fuzz: random event streams (any interval including zero and
//! hours, instants that go backwards, any `char`, corrections, rhythm breaks,
//! random configurations) never panic and every figure stays in range.
//!
//! Deterministic: a hand-rolled SplitMix64 with fixed seeds (no proptest).
//! A failure prints the seed so the stream can be replayed.

use std::time::{Duration, Instant};

use radha_core::flow::{
    drill, recommend, CadenceBar, FlowConfig, FlowReport, FlowTracker, RhythmLabel,
};

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
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

const CHARS: [char; 14] = [
    'a',
    'e',
    't',
    ' ',
    ' ',
    ',',
    '.',
    'Q',
    ';',
    'é',
    '—',
    '\u{1F600}',
    'क',
    '\0',
];

fn random_config(rng: &mut Rng) -> FlowConfig {
    let d = FlowConfig::default();
    if rng.below(3) == 0 {
        return d;
    }
    FlowConfig {
        window: 1 + rng.below(300) as usize,
        min_interval_ms: 1.0 + rng.unit() * 50.0,
        pause_ms: 400.0 + rng.unit() * 10_000.0,
        warmup: 1 + rng.below(10) as u32,
        tempo_alpha: 0.01 + rng.unit() * 0.98,
        smooth_band: 0.05 + rng.unit() * 2.0,
        boundary_band: 0.05 + rng.unit() * 3.0,
        hesitation_factor: 1.0 + rng.unit() * 10.0,
        weak_min_attempts: rng.below(10) as u32,
        max_weak_keys: rng.below(8) as usize,
        drill_words: 1 + rng.below(80) as usize,
        key_reps: 1 + rng.below(6) as usize,
        ..d
    }
}

fn check(seed: u64) {
    let mut rng = Rng(seed);
    let cfg = random_config(&mut rng);
    let mut t = FlowTracker::new(cfg);
    let cap = t.window_capacity();
    let base = Instant::now() + Duration::from_secs(48 * 3600);
    let mut now = base;
    let mut bars = [CadenceBar::default(); 64];
    let events = 200 + rng.below(3000);
    for _ in 0..events {
        let step = match rng.below(20) {
            0 => 0,
            1 => rng.below(5),
            2 => 3_600_000 + rng.below(36_000_000), // hours (clock jump)
            3 => rng.below(20_000),
            _ => 60 + rng.below(400),
        };
        now += Duration::from_millis(step);
        let at = if rng.below(25) == 0 {
            // Backwards, possibly before the base.
            now.checked_sub(Duration::from_millis(rng.below(100_000)))
                .unwrap_or(now)
        } else {
            now
        };
        let c = CHARS[rng.below(CHARS.len() as u64) as usize];
        match rng.below(30) {
            0 => t.correction_at(at),
            1 => t.break_rhythm(),
            2 if rng.below(10) == 0 => t.reset(),
            _ => t.key_at(at, c, rng.below(4) != 0),
        }
        t.fill_bars(&mut bars);
        for b in &bars {
            assert!(
                b.level.is_finite() && (0.0..=1.0).contains(&b.level),
                "seed {seed}: bar {b:?}"
            );
        }
        if let Some(s) = t.live_smoothness() {
            assert!((0.0..=100.0).contains(&s), "seed {seed}: live {s}");
        }
        let _: RhythmLabel = t.label();
        assert_eq!(t.window_capacity(), cap, "seed {seed}");
    }
    let c = t.counts();
    assert!(c.smooth <= c.scored, "seed {seed}");
    for v in [t.smoothness(), t.mean_ms(), t.stdev_ms(), t.cadence_cv()]
        .into_iter()
        .flatten()
    {
        assert!(v.is_finite() && v >= 0.0, "seed {seed}: {v}");
    }
    assert!(t.smoothness().is_none_or(|s| s <= 100.0));
    let report = FlowReport::from_tracker(&t);
    let cfg = *t.config();
    assert!(report.weak_keys.len() <= cfg.max_weak_keys, "seed {seed}");
    for k in &report.weak_keys {
        assert!(k.score.is_finite() && (0.0..=100.0).contains(&k.score));
        assert!((0.0..=100.0).contains(&k.accuracy()));
    }
    let source = "the quick, brown fox; jumps over lazy dogs. Quietly — café नमस्ते";
    if let Some(d) = drill(&report, source, &cfg) {
        assert!(
            d.text.is_ascii(),
            "seed {seed}: drill must stay ASCII: {}",
            d.text
        );
        assert!(!d.text.contains("  "), "seed {seed}");
        assert!(!d.text.is_empty());
    }
    let accuracy = rng.unit() * 120.0 - 10.0;
    let rec = recommend(&report, accuracy, &cfg);
    assert!(!rec.headline.is_empty() && !rec.detail.is_empty());
    assert!(!report.headline().is_empty());
}

#[test]
fn random_streams_never_panic_and_stay_in_range() {
    for seed in 0..400u64 {
        check(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xF10);
    }
}
