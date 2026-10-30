//! Random keystroke streams through every layout: no panic, edits stay in
//! range, Unicode text stays NFC, Kruti Dev text stays in its alphabet, and a
//! key never rewrites an akshar older than the tail it is allowed to touch.

mod common;

use std::time::Instant;

use radha_core::akshar::{normalize, segment};
use radha_core::{Akshar, Outcome, SessionConfig, TextUnit, TypingSession};
use radha_layouts::krutidev::alt_codes;
use radha_layouts::{
    press_alt_code, press_backspace, press_key, Composer, Encoding, KeyboardLayout, Keystroke,
    Layer, PhysicalKey, TokenKind, SPACE,
};

use common::{layout, SplitMix64};

fn random_stroke(rng: &mut SplitMix64) -> Keystroke {
    if rng.below(8) == 0 {
        return Keystroke::new(SPACE, Layer::Base);
    }
    let keys: Vec<PhysicalKey> = PhysicalKey::all().collect();
    let key = keys[rng.below(keys.len())];
    let layer = [
        Layer::Base,
        Layer::Base,
        Layer::Shift,
        Layer::AltGr,
        Layer::ShiftAltGr,
    ][rng.below(5)];
    Keystroke::new(key, layer)
}

/// Byte offset where the last akshar of `text` starts.
fn tail_start(text: &str) -> usize {
    let last = segment(text).pop().map_or(0, |s| s.len());
    text.len().saturating_sub(last)
}

fn run<U: TextUnit>(l: &'static dyn KeyboardLayout, mut s: TypingSession<U>, seed: u64) {
    let alphabet: Vec<char> = alt_codes().iter().map(|a| a.ansi).collect();
    let mut rng = SplitMix64(seed);
    let mut c = Composer::new(l);
    let t0 = Instant::now();
    // Start byte of the tail akshar when the pending sign was struck.
    let mut sign_tail: Option<usize> = None;
    for _ in 0..400 {
        let before = s.matcher().typed().to_owned();
        let before_eval = s.evaluation();
        let r = rng.below(10);
        let stroke = random_stroke(&mut rng);
        let entry = l.table().get(stroke);
        let is_key = r >= 2;
        if r == 0 {
            press_backspace(&mut s, &mut c, t0);
        } else if r == 1 {
            press_alt_code(
                &mut s,
                &mut c,
                [161u16, 165, 176, 188, 189, 200][rng.below(6)],
                t0,
            );
        } else {
            press_key(&mut s, &mut c, stroke, t0);
        }
        let after = s.matcher().typed();
        let snap = s.snapshot_at(t0);
        assert!((0.0..=100.0).contains(&snap.accuracy));
        assert!(snap.net_wpm >= 0.0);
        match l.encoding() {
            Encoding::Unicode => assert_eq!(normalize(after), after, "not NFC: {after:?}"),
            Encoding::Legacy => assert!(after
                .chars()
                .all(|ch| ch.is_ascii_graphic() || ch == ' ' || alphabet.contains(&ch))),
            Encoding::Latin => assert!(after.chars().all(|ch| ch.is_ascii_graphic() || ch == ' ')),
        }
        if l.encoding() == Encoding::Unicode && is_key {
            // Edit scope: nothing before the allowed tail changes.
            let floor = sign_tail.unwrap_or(usize::MAX).min(tail_start(&before));
            let keep = before.get(..floor).unwrap_or("");
            assert!(
                after.starts_with(keep),
                "{before:?} -> {after:?} rewrote before byte {floor}"
            );
            // Correct outcomes before the tail stay correct.
            let n = before_eval
                .outcomes
                .len()
                .saturating_sub(if sign_tail.is_some() { 2 } else { 1 });
            let after_eval = s.evaluation();
            for i in 0..n {
                if matches!(before_eval.outcomes[i], Outcome::Correct(_)) {
                    assert_eq!(
                        after_eval.outcomes.get(i),
                        Some(&before_eval.outcomes[i]),
                        "{before:?} -> {after:?}"
                    );
                }
            }
            if entry.is_some_and(|e| e.kind == TokenKind::Preposed) && c.preedit().is_some() {
                sign_tail = Some(
                    sign_tail
                        .unwrap_or(usize::MAX)
                        .min(tail_start(after).min(after.len())),
                );
            }
        }
        if c.preedit().is_none() {
            sign_tail = None;
        }
    }
}

#[test]
fn random_streams_never_panic_and_keep_their_invariants() {
    let hindi = "क्षेत्र में विद्यार्थी कार्य करते हैं";
    for seed in 1..=12u64 {
        for slug in ["gail", "cbi"] {
            let l = layout(slug);
            run(
                l,
                TypingSession::<Akshar>::configured(hindi, SessionConfig::default()),
                seed,
            );
        }
        run(
            layout("qwerty"),
            TypingSession::new("the quick brown fox"),
            seed,
        );
        run(
            layout("krutidev"),
            TypingSession::new("{ks= esa fo|kFkhZ"),
            seed,
        );
    }
}
