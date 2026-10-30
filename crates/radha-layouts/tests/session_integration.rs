//! Layout + guide + session, end to end: following the guide's hints types
//! the target with no mistake on the way, legitimate intermediate states stay
//! pending, and the guide's units line up with the session's.

mod common;

use std::sync::Arc;
use std::time::Instant;

use radha_core::akshar::segment;
use radha_core::{Akshar, Outcome, SessionConfig, SessionState, TextUnit, TypingSession};
use radha_layouts::krutidev::to_legacy;
use radha_layouts::{
    press_backspace, press_key, Composer, Encoding, Finger, Guide, Hand, KeyEffect, KeyboardLayout,
    Keystroke, Layer, NextAction, SPACE,
};

use common::layout;

/// Words both Remington layouts can type (`कॉलेज` below: `ॉ` is k +
/// Shift+W on both, GAIL-MAP-136).
const WORDS: [&str; 13] = [
    "कार्य",
    "हिन्दी",
    "भारत",
    "नमस्ते",
    "क्षेत्र",
    "अर्थ",
    "विद्यार्थी",
    "शिक्षा",
    "कृषि",
    "क्रिया",
    "और",
    "ईद",
    "पूर्ण धर्म",
];

/// Follow the guide until the session completes; assert no outcome is ever an
/// error and no key stat records one. Returns the keys pressed.
fn follow<U: TextUnit>(
    l: &'static dyn KeyboardLayout,
    guide: &Guide,
    mut s: TypingSession<U>,
    pending_text: impl Fn(&str) -> Option<String>,
) -> usize {
    let t0 = Instant::now();
    let mut c = Composer::new(l);
    let mut presses = 0;
    while s.state() != SessionState::Complete {
        let e = s.evaluation();
        let tail = if e.pending {
            pending_text(s.matcher().typed())
        } else {
            None
        };
        let action = guide
            .next_action(e.cursor, tail.as_deref(), c.preedit(), false)
            .unwrap_or_else(|| {
                panic!(
                    "no hint at cursor {} typed {:?}",
                    e.cursor,
                    s.matcher().typed()
                )
            });
        let NextAction::Key(hint) = action else {
            panic!("unexpected alt code");
        };
        let stroke = Keystroke::new(hint.key, hint.layer);
        assert!(matches!(
            press_key(&mut s, &mut c, stroke, t0),
            KeyEffect::Applied { .. }
        ));
        presses += 1;
        let snap = s.snapshot_at(t0);
        assert!(
            snap.outcomes.iter().all(|o| !o.is_error()),
            "a mistake while following the guide: typed {:?} -> {:?}",
            s.matcher().typed(),
            snap.outcomes
        );
        assert!(presses < 200, "guide loops");
    }
    let errors: u32 = s.key_stats().sorted().iter().map(|(_, t)| t.errors).sum();
    assert_eq!(errors, 0, "key stats recorded an error");
    presses
}

fn akshar_session(
    l: &'static dyn KeyboardLayout,
    target: &str,
) -> (Arc<Guide>, TypingSession<Akshar>) {
    let guide = Arc::new(Guide::new(l, target));
    let s = TypingSession::<Akshar>::configured(target, SessionConfig::default())
        .with_composition(guide.clone());
    (guide, s)
}

fn last_akshar(typed: &str) -> Option<String> {
    segment(typed).pop()
}

#[test]
fn following_the_guide_types_every_word_on_gail_and_cbi() {
    for slug in ["gail", "cbi"] {
        let l = layout(slug);
        for word in WORDS {
            let (guide, s) = akshar_session(l, word);
            assert!(
                guide.untypeable().is_empty(),
                "{slug} {word}: {:?}",
                guide.untypeable()
            );
            follow(l, &guide, s, last_akshar);
        }
        let sentence = WORDS.join(" ");
        let (guide, s) = akshar_session(l, &sentence);
        follow(l, &guide, s, last_akshar);
    }
    for slug in ["cbi", "gail"] {
        let l = layout(slug);
        let (guide, s) = akshar_session(l, "कॉलेज");
        // Book 01 GAIL-MAP-136 gives GAIL `ॉ` = k + Shift+W (was untypeable).
        assert!(guide.untypeable().is_empty(), "{slug}");
        follow(l, &guide, s, last_akshar);
    }
}

#[test]
fn the_guide_covers_a_lone_preposed_sign_and_nasal_conjuncts() {
    // P04 importer gaps: an isolated `ि` (47 GAIL drills) and conjunct +
    // anusvara akshars showed "—" in the guide.
    let gail = layout("gail");
    let (guide, s) = akshar_session(gail, "ि का र्षों");
    let first = guide.next_action(0, None, None, false);
    let Some(NextAction::Key(h)) = first else {
        panic!("lone ि needs a key hint, got {first:?}");
    };
    assert_eq!(h.legend, "F");
    follow(gail, &guide, s, last_akshar);

    let cbi = layout("cbi");
    let (guide, s) = akshar_session(cbi, "क्ष्यों ह्नों र्थों");
    assert!(guide.untypeable().is_empty(), "{:?}", guide.untypeable());
    follow(cbi, &guide, s, last_akshar);
}

#[test]
fn following_the_guide_types_english_and_kruti_dev() {
    let q = layout("qwerty");
    let text = "The quick brown fox, \"jumps\" over 12 lazy dogs!";
    let guide = Guide::new(q, text);
    assert_eq!(
        follow(q, &guide, TypingSession::new(text), |_| None),
        text.chars().count()
    );

    let kd = layout("krutidev");
    let legacy = to_legacy(&WORDS.join(" ")).legacy;
    let guide = Guide::new(kd, &legacy);
    follow(kd, &guide, TypingSession::new(&legacy), |_| None);
}

#[test]
fn guide_units_line_up_with_the_session() {
    let hindi = "  क्षेत्र में विद्यार्थी  ";
    for slug in ["qwerty", "gail", "cbi", "krutidev"] {
        let l = layout(slug);
        let mut texts = vec![
            "  asdf jkl;  ".to_owned(),
            "a\r\nb".to_owned(),
            "cafe\u{301}".to_owned(),
            hindi.to_owned(),
        ];
        texts.push(to_legacy(hindi.trim()).legacy);
        for t in texts {
            let guide = Guide::new(l, &t);
            let units = match l.encoding() {
                Encoding::Unicode => {
                    TypingSession::<Akshar>::configured(&t, SessionConfig::default())
                        .matcher()
                        .target_units()
                        .len()
                }
                _ => TypingSession::new(&t).matcher().target_units().len(),
            };
            assert_eq!(guide.unit_count(), units, "{slug} {t:?}");
        }
    }
}

/// Type keys (US characters) into an akshar session; returns the session.
fn type_into(slug: &str, target: &str, keys: &str) -> TypingSession<Akshar> {
    let l = layout(slug);
    let (_, mut s) = akshar_session(l, target);
    let mut c = Composer::new(l);
    let t0 = Instant::now();
    for k in keys.chars() {
        if k == '\u{8}' {
            press_backspace(&mut s, &mut c, t0);
        } else {
            press_key(
                &mut s,
                &mut c,
                Keystroke::from_us_char(k, false).expect("US key"),
                t0,
            );
        }
    }
    s
}

#[test]
fn legitimate_intermediate_states_are_pending_not_wrong() {
    for (slug, target, keys) in [
        ("gail", "कार्य", "dk;Z"),
        ("cbi", "कार्य", "dk;Z"),
        ("gail", "कि", "fd"),
        ("cbi", "भ", "Hk"),
        ("gail", "ओ", "vks"),
        ("cbi", "ओ", "vks"),
        ("gail", "र्थी", "FkhZ"),
    ] {
        for n in 1..=keys.chars().count() {
            let prefix: String = keys.chars().take(n).collect();
            let s = type_into(slug, target, &prefix);
            let e = s.evaluation();
            assert!(
                e.outcomes.iter().all(|o| !o.is_error()),
                "{slug} {prefix}: {:?}",
                e.outcomes
            );
        }
        let s = type_into(slug, target, keys);
        assert_eq!(s.state(), SessionState::Complete, "{slug} {keys}");
    }
    // A genuinely wrong consonant is wrong at once.
    let s = type_into("gail", "कार्य", "x");
    assert!(matches!(
        s.evaluation().outcomes[..],
        [Outcome::Wrong { .. }]
    ));
}

#[test]
fn the_guide_follows_an_alternative_path_to_long_ii() {
    for slug in ["gail", "cbi"] {
        let l = layout(slug);
        let (guide, mut s) = akshar_session(l, "ई");
        let mut c = Composer::new(l);
        let t0 = Instant::now();
        let first = guide.next_action(0, None, None, false);
        // GAIL teaches AltGr+B (GAIL-MAP-122); CBI has no AltGr layer
        // (book 03) and teaches b + Shift+Z.
        let want = if slug == "gail" {
            Layer::AltGr
        } else {
            Layer::Base
        };
        assert!(
            matches!(first, Some(NextAction::Key(ref h)) if h.layer == want && h.legend == "B"),
            "{slug}: {first:?}"
        );
        press_key(
            &mut s,
            &mut c,
            Keystroke::from_us_char('b', false).expect("b"),
            t0,
        );
        let e = s.evaluation();
        assert!(
            e.pending && e.outcomes.is_empty(),
            "{slug}: इ toward ई is pending"
        );
        let next = guide.next_action(e.cursor, Some("इ"), None, false);
        let Some(NextAction::Key(h)) = next else {
            panic!("{slug}: no hint after b");
        };
        assert_eq!((h.legend, h.layer, h.hold_shift), ("Z", Layer::Shift, true));
        assert_eq!(h.shift_hand, Some(Hand::Right));
        press_key(&mut s, &mut c, Keystroke::new(h.key, h.layer), t0);
        assert_eq!(s.state(), SessionState::Complete);
        let errors: u32 = s.key_stats().sorted().iter().map(|(_, t)| t.errors).sum();
        assert_eq!(errors, 0);
    }
}

#[test]
fn preposed_sign_slips_stay_inside_the_current_akshar() {
    for (keys, committed) in [
        ("fv", "अि"),
        ("ffd", "िकि"),
        ("f ", "ि "),
        ("df ", "क "),
        ("dffd", "ककि"),
        ("dfk", "का"),
        ("fL ", "स्ि "),
        ("Dfkd", "ककि"),
        ("dfZ", "र्कि"),
        ("df`", "क़ि"),
    ] {
        let s = type_into("gail", "कक कक", keys);
        assert_eq!(s.matcher().typed(), committed, "{keys}");
        // The akshar that was complete before the slip keeps its Correct.
        if ["df ", "dffd", "Dfkd"].contains(&keys) {
            let ka = <Akshar as TextUnit>::segment("क").remove(0);
            assert_eq!(
                s.evaluation().outcomes.first(),
                Some(&Outcome::Correct(ka)),
                "{keys}"
            );
        }
    }
}

#[test]
fn a_preedit_backspace_cancels_the_sign_and_counts() {
    let s = type_into("gail", "कि", "f\u{8}");
    assert_eq!(s.matcher().typed(), "");
    assert_eq!(s.snapshot().backspaces, 1);
    assert_eq!(s.snapshot().presses, 2);
}

#[test]
fn qwerty_through_the_composer_equals_type_char() {
    let q = layout("qwerty");
    for text in ["the cat sat", "Hello, World! 123", "a  b", "x~y_z"] {
        let t0 = Instant::now();
        let mut direct = TypingSession::new(text);
        let mut composed = TypingSession::new(text);
        let mut c = Composer::new(q);
        for ch in text.chars().chain("q".chars()) {
            direct.type_char_at(ch, t0);
            press_key(
                &mut composed,
                &mut c,
                Keystroke::from_us_char(ch, false).expect("US"),
                t0,
            );
        }
        assert_eq!(direct.snapshot_at(t0), composed.snapshot_at(t0), "{text}");
    }
}

#[test]
fn every_slot_has_a_hand_and_finger_and_caps_lock_flips_letters_only() {
    for slug in ["qwerty", "gail", "cbi", "krutidev"] {
        let l = layout(slug);
        let guide = Guide::new(l, "");
        for e in l.table().entries() {
            let h = guide
                .hint_for(e.keystroke(), false)
                .expect("every slot hints");
            assert_ne!(h.finger, Finger::Thumb, "{slug} {:?}", e.key);
            let caps = guide.hint_for(e.keystroke(), true).expect("hint");
            if e.key.is_letter() {
                assert_ne!(h.hold_shift, caps.hold_shift);
            } else {
                assert_eq!(h.hold_shift, caps.hold_shift);
            }
        }
        let space = guide
            .hint_for(Keystroke::new(SPACE, Layer::Base), false)
            .expect("space");
        assert_eq!((space.hand, space.finger), (Hand::Right, Finger::Thumb));
    }
    let kd = layout("krutidev");
    let guide = Guide::new(kd, &to_legacy("ञ").legacy);
    assert!(matches!(
        guide.next_action(0, None, None, false),
        Some(NextAction::AltCode { code: 165, .. })
    ));
}

#[test]
fn every_akshar_of_the_word_list_plans_within_the_budget() {
    for slug in ["gail", "cbi"] {
        let guide = Guide::new(layout(slug), &WORDS.join(" "));
        for unit in guide.units() {
            let outcome = guide.outcome_for(unit).expect("planned");
            assert!(outcome.shortest().is_some(), "{slug} {unit}");
            assert!(outcome.nodes() <= radha_layouts::plan::NODE_BUDGET);
        }
    }
}
