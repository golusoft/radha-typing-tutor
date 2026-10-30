//! Property tests over random keystroke streams: the engine never panics and
//! every metric stays in range, whatever the user mashes.
//!
//! Deterministic by construction: a hand-rolled SplitMix64 with fixed seeds,
//! no new dependency and no shrinking (invariant checks do not need it). A
//! failure prints the seed and the event log so it can be replayed as a fixed
//! regression test.

use std::time::{Duration, Instant};

use radha_core::{
    word_accuracy, Akshar, Matcher, SessionConfig, SessionState, TextUnit, TimeLimit, TimerMode,
    TypingSession,
};

/// SplitMix64 (Steele, Lea, Flood 2014), the standard constants.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const ASCII_WORDS: [&str; 12] = [
    "the", "quick", "brown", "fox", "Jumps", "over", "lazy", "dog,", "asdf", "jkl;", "a", "I'm",
];
const ASCII_KEYS: [char; 16] = [
    'a', 's', 'd', 'f', 'j', 'k', 'l', ';', 'T', 'h', 'e', ' ', ' ', ',', '.', 'Q',
];
const HINDI_WORDS: [&str; 10] = [
    "नमस्ते",
    "क्षेत्र",
    "राष्ट्रीय",
    "विद्यालय",
    "हँसना",
    "दुःख",
    "ज़रूर",
    "श्री",
    "कार्य",
    "ं",
];
const HINDI_KEYS: [char; 16] = [
    'क', 'ष', 'त', 'र', '\u{094D}', '\u{093F}', '\u{093E}', '\u{0947}', '\u{0902}', '\u{0901}',
    '\u{093C}', '\u{200D}', '\u{200C}', '\u{0903}', ' ', 'न',
];

fn target(rng: &mut Rng, words: &[&str]) -> String {
    let n = 1 + rng.below(6);
    (0..n)
        .map(|_| *rng.pick(words))
        .collect::<Vec<_>>()
        .join(if rng.below(5) == 0 { "  " } else { " " })
}

fn timer(rng: &mut Rng) -> TimerMode {
    match rng.below(3) {
        0 => TimerMode::Off,
        1 => TimerMode::Stopwatch,
        _ => match TimeLimit::custom(1 + rng.below(120) as u32) {
            Ok(l) => TimerMode::Countdown(l),
            Err(e) => panic!("valid range: {e}"),
        },
    }
}

fn run_stream<U: TextUnit>(seed: u64, words: &[&str], keys: &[char]) {
    let mut rng = Rng(seed);
    let target = target(&mut rng, words);
    let timer = timer(&mut rng);
    let mut s = TypingSession::<U>::configured(&target, SessionConfig { timer });
    let t0 = Instant::now();
    let mut now_ms = 0u64;
    let mut log: Vec<String> = Vec::new();
    let mut last_elapsed = 0.0f64;
    let mut last_state = s.state();
    let limit = timer.limit().map(|l| f64::from(l.seconds()));

    let events = rng.below(150);
    for _ in 0..events {
        now_ms += rng.below(3000) as u64;
        let now = t0 + Duration::from_millis(now_ms);
        let roll = rng.below(100);
        let event = match roll {
            0..=69 => {
                let c = *rng.pick(keys);
                s.type_char_at(c, now);
                format!("{now_ms}:type {c:?}")
            }
            70..=84 => {
                s.backspace_at(now);
                format!("{now_ms}:backspace")
            }
            85..=88 => format!("{now_ms}:pause {:?}", s.pause_at(now).is_ok()),
            89..=93 => format!("{now_ms}:resume {:?}", s.resume_at(now).is_ok()),
            94..=97 => format!("{now_ms}:tick {}", s.tick_at(now)),
            _ => {
                s.complete_at(now);
                format!("{now_ms}:complete")
            }
        };
        log.push(event);

        let snap = s.snapshot_at(now);
        let ctx = || {
            format!(
                "seed {seed} target {target:?} timer {timer:?}\n{}",
                log.join("\n")
            )
        };
        assert!(
            (0.0..=100.0).contains(&snap.accuracy),
            "accuracy {} {}",
            snap.accuracy,
            ctx()
        );
        assert!(
            (0.0..=100.0).contains(&snap.word_accuracy),
            "word accuracy {}",
            ctx()
        );
        assert!(
            (0.0..=100.0).contains(&snap.consistency),
            "consistency {} {}",
            snap.consistency,
            ctx()
        );
        for (name, v) in [
            ("wpm", snap.wpm),
            ("net", snap.net_wpm),
            ("kdph", snap.kdph),
        ] {
            assert!(v.is_finite() && v >= 0.0, "{name} = {v} {}", ctx());
        }
        assert!(snap.net_wpm <= snap.wpm, "net above gross {}", ctx());
        assert_eq!(
            snap.error_count,
            snap.breakdown.wrong + snap.breakdown.extra + snap.breakdown.missing,
            "{}",
            ctx()
        );
        assert!(snap.breakdown.space <= snap.error_count, "{}", ctx());
        assert_eq!(snap.word_accuracy, word_accuracy(&snap.word_outcomes));
        assert!(
            snap.elapsed_seconds >= last_elapsed,
            "elapsed went back {}",
            ctx()
        );
        if let Some(limit) = limit {
            assert!(
                snap.elapsed_seconds <= limit,
                "elapsed past the limit {}",
                ctx()
            );
            let left = snap.remaining_seconds.unwrap_or(-1.0);
            assert!((0.0..=limit).contains(&left), "remaining {left} {}", ctx());
        }
        let allowed = last_state == snap.state
            || matches!(
                (last_state, snap.state),
                (SessionState::Idle, SessionState::Active)
                    | (SessionState::Active, SessionState::Paused)
                    | (SessionState::Paused, SessionState::Active)
                    | (_, SessionState::Complete)
            );
        assert!(allowed, "{last_state:?} -> {:?} {}", snap.state, ctx());
        assert!(last_state != SessionState::Complete || snap.state == SessionState::Complete);
        last_elapsed = snap.elapsed_seconds;
        last_state = snap.state;
    }
}

#[test]
fn random_ascii_streams_keep_every_invariant() {
    for seed in 0..300 {
        run_stream::<char>(seed, &ASCII_WORDS, &ASCII_KEYS);
    }
}

#[test]
fn random_hindi_streams_keep_every_invariant() {
    for seed in 1000..1300 {
        run_stream::<Akshar>(seed, &HINDI_WORDS, &HINDI_KEYS);
    }
}

#[test]
fn random_hindi_streams_on_the_char_path_keep_every_invariant() {
    // Legacy-stream path (char units) fed Devanagari: must not panic either.
    for seed in 2000..2100 {
        run_stream::<char>(seed, &HINDI_WORDS, &HINDI_KEYS);
    }
}

#[test]
fn char_and_akshar_matchers_agree_on_ascii() {
    let mut rng = Rng(42);
    for _ in 0..300 {
        let target = target(&mut rng, &ASCII_WORDS);
        let mut a = Matcher::new(&target);
        let mut b = Matcher::<Akshar>::from_target(&target);
        for _ in 0..rng.below(60) {
            if rng.below(6) == 0 {
                a.pop();
                b.pop();
            } else {
                let c = *rng.pick(&ASCII_KEYS);
                a.push(c);
                b.push(c);
            }
            let ea = a.evaluate_detailed();
            let eb = b.evaluate_detailed();
            let codes_a: Vec<bool> = ea.outcomes.iter().map(|o| o.is_error()).collect();
            let codes_b: Vec<bool> = eb.outcomes.iter().map(|o| o.is_error()).collect();
            assert_eq!(codes_a, codes_b, "target {target:?} typed {:?}", a.typed());
            assert_eq!(ea.word_outcomes, eb.word_outcomes);
            assert_eq!((ea.cursor, ea.finished), (eb.cursor, eb.finished));
        }
    }
}
