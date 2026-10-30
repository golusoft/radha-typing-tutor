//! P09 Exam Mode tests over the embedded profiles (EXAM-MODE-AND-GOLU-REUSE-PLAN.md
//! Section 5.9): layout selection, timer expiry, qualification thresholds,
//! profile versioning, certification derivation, the unofficial Clerk
//! 01/2022 profile, invalid data and the BSSC grep rule.

use std::time::{Duration, Instant};

use radha_core::{CompletionReason, SessionConfig, TimerMode, TypingSession};
use radha_exams::{
    card_summary, evaluate, parse_profile, Certification, ExamProfile, Field, Language, PolicyKind,
    ProfileError, Qualification, Registry, Status, TestInput, Verdict, PROFILE_SCHEMA_VERSION,
};
use radha_layouts::LayoutId;

fn registry() -> Registry {
    let report = Registry::embedded();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    report.registry
}

fn profile(id: &str) -> ExamProfile {
    registry()
        .latest(id)
        .cloned()
        .unwrap_or_else(|| panic!("{id}"))
}

/// The ids of the Exam Hub cards (crates/radha-app/ui/data/exams.slint).
const CARD_IDS: [&str; 22] = [
    "all-in-one-mangal",
    "ssc-chsl",
    "rrb-ntpc",
    "ssc-cgl-dest",
    "aiims-cre",
    "crpf-hcm",
    "capf-hcm",
    "epfo-ssa",
    "upsssc-ja",
    "mp-cpct",
    "rajasthan-rssb",
    "delhi-police-hcm",
    "dsssb",
    "beltron-deo",
    "bssc-inter",
    "up-police-asi",
    "uppcl",
    "patna-hc",
    "bihar-civil-court",
    "rajasthan-hc",
    "allahabad-hc",
    "mp-hc",
];

#[test]
fn every_embedded_profile_loads_once_in_hub_order() {
    let r = registry();
    assert_eq!(r.len(), 22, "20 reference cards + BSSC + All-in-One Mangal");
    assert_eq!(r.ids(), CARD_IDS);
    for p in r.all() {
        assert_eq!(p.schema_version, PROFILE_SCHEMA_VERSION);
        assert_eq!(p.profile_version, 1, "{}", p.id);
        assert_eq!(p.key(), format!("{}@1", p.id));
        assert!(!p.tests.is_empty(), "{}", p.id);
    }
}

#[test]
fn layout_selection_follows_the_profile() {
    let patna = profile("patna-hc");
    let hindi = patna.test(Language::Hindi).unwrap();
    assert_eq!(hindi.allowed_layouts(), [LayoutId::RemingtonGail]);
    for other in [
        LayoutId::RemingtonCbi,
        LayoutId::KrutiDev,
        LayoutId::Inscript,
    ] {
        assert!(!hindi.allows_layout(other), "{other:?} on a GAIL profile");
        assert_eq!(hindi.default_layout(other), LayoutId::RemingtonGail);
    }
    let english = patna.test(Language::English).unwrap();
    assert_eq!(english.allowed_layouts(), [LayoutId::Qwerty]);
    assert_eq!(
        english.default_layout(LayoutId::RemingtonGail),
        LayoutId::Qwerty
    );
    assert_eq!(
        hindi.font.known().map(String::as_str),
        Some("Mangal"),
        "Hindi = Remington Gail + Mangal (S-03)"
    );
    assert_eq!(hindi.layouts.status(), Status::Verified);

    let beltron = profile("beltron-deo")
        .test(Language::Hindi)
        .cloned()
        .unwrap();
    assert_eq!(beltron.allowed_layouts(), [LayoutId::RemingtonGail]);
    assert_eq!(beltron.layouts.status(), Status::UserAsserted);

    let mangal = profile("all-in-one-mangal");
    assert!(mangal.hindi_only);
    assert_eq!(mangal.languages(), [Language::Hindi], "no English");
    let t = mangal.test(Language::Hindi).unwrap();
    assert_eq!(
        t.allowed_layouts(),
        [
            LayoutId::Inscript,
            LayoutId::RemingtonGail,
            LayoutId::RemingtonCbi
        ]
    );
    assert!(!t.allows_layout(LayoutId::KrutiDev));
    assert_eq!(
        t.default_layout(LayoutId::RemingtonCbi),
        LayoutId::RemingtonCbi
    );
    assert_eq!(t.default_layout(LayoutId::Qwerty), LayoutId::Inscript);

    // A reference profile knows no layout: every Hindi layout, the user's
    // preferred one first.
    let chsl = profile("ssc-chsl");
    let h = chsl.test(Language::Hindi).unwrap();
    assert_eq!(h.layouts, Field::Unknown);
    assert_eq!(h.allowed_layouts().len(), 4);
    assert_eq!(h.default_layout(LayoutId::KrutiDev), LayoutId::KrutiDev);
    assert_eq!(h.default_layout(LayoutId::Qwerty), LayoutId::RemingtonGail);
}

#[test]
fn the_exam_timer_expires_exactly_at_the_profile_duration() {
    let patna = profile("patna-hc");
    let english = patna.test(Language::English).unwrap();
    assert_eq!(english.duration_seconds.known(), Some(&600));
    let timer = english.timer_mode(1);
    assert!(matches!(timer, TimerMode::Countdown(l) if l.seconds() == 600));

    let t0 = Instant::now();
    let mut s: TypingSession =
        TypingSession::configured("the quick brown fox", SessionConfig { timer });
    s.type_char_at('t', t0);
    assert!(
        !s.tick_at(t0 + Duration::from_millis(599_900)),
        "599.9 s: running"
    );
    assert!(s.tick_at(t0 + Duration::from_secs(600)), "600 s: over");
    assert_eq!(s.snapshot().completion, Some(CompletionReason::TimeUp));

    // Paused time is not counted.
    let mut p: TypingSession =
        TypingSession::configured("the quick brown fox", SessionConfig { timer });
    p.type_char_at('t', t0);
    p.pause_at(t0 + Duration::from_secs(100)).unwrap();
    p.resume_at(t0 + Duration::from_secs(200)).unwrap();
    assert!(!p.tick_at(t0 + Duration::from_millis(699_900)));
    assert!(p.tick_at(t0 + Duration::from_secs(700)));

    // No duration in any source: the app's own (labelled) practice timer.
    let beltron = profile("beltron-deo");
    let t = beltron.test(Language::English).unwrap();
    assert_eq!(t.duration_seconds.status(), Status::Configurable);
    assert_eq!(t.time_limit(), None);
    assert!(matches!(t.timer_mode(600), TimerMode::Countdown(l) if l.seconds() == 600));
}

fn marks_input(accuracy: f64) -> TestInput {
    TestInput {
        accuracy,
        words_typed: 400,
        words_correct: 360,
        elapsed_seconds: 600.0,
        ..TestInput::default()
    }
}

/// `words_correct` over 100 minutes: hundredths of a correct WPM.
fn wpm_input(hundredths: u32) -> TestInput {
    TestInput {
        accuracy: 95.0,
        words_typed: hundredths,
        words_correct: hundredths,
        elapsed_seconds: 6000.0,
        ..TestInput::default()
    }
}

#[test]
fn qualification_thresholds_are_inclusive_at_two_decimals() {
    let patna = profile("patna-hc");
    let judge = |lang, input: TestInput| {
        let t = patna.test(lang).unwrap();
        evaluate(&patna, t, &input)
    };
    for (lang, below, at) in [
        (Language::English, 89.99, 90.00),
        (Language::Hindi, 84.99, 85.00),
    ] {
        let fail = judge(lang, marks_input(below));
        assert_eq!(fail.policy, PolicyKind::AccuracyAsMarks);
        assert_eq!(fail.measure, Some(("marks", below)));
        assert_eq!(
            fail.verdict,
            Some(Verdict::NotQualified),
            "{lang:?} {below}"
        );
        let pass = judge(lang, marks_input(at));
        assert_eq!(pass.verdict, Some(Verdict::Qualified), "{lang:?} {at}");
        assert_eq!(pass.certification, Certification::Provisional);
    }

    let beltron = profile("beltron-deo");
    for (lang, below, at) in [
        (Language::English, 2999, 3000),
        (Language::Hindi, 2499, 2500),
    ] {
        let t = beltron.test(lang).unwrap();
        let fail = evaluate(&beltron, t, &wpm_input(below));
        assert_eq!(fail.policy, PolicyKind::CorrectWpmThreshold);
        assert_eq!(
            fail.measure,
            Some(("correct WPM", f64::from(below) / 100.0))
        );
        assert_eq!(
            fail.verdict,
            Some(Verdict::NotQualified),
            "{lang:?} {below}"
        );
        let pass = evaluate(&beltron, t, &wpm_input(at));
        assert_eq!(pass.verdict, Some(Verdict::Qualified), "{lang:?} {at}");
        assert!(
            pass.notes
                .iter()
                .any(|n| n.starts_with("Duration: configurable")),
            "{:?}",
            pass.notes
        );
    }
}

#[test]
fn certification_is_derived_from_field_statuses() {
    let r = registry();
    for p in r.all() {
        let expected = match p.id.as_str() {
            "patna-hc" | "beltron-deo" => Certification::Provisional,
            _ => Certification::PracticeOnly,
        };
        assert_eq!(p.certification(), expected, "{}", p.id);
        assert_ne!(p.certification(), Certification::Official, "{}", p.id);
        let card = card_summary(p);
        assert_eq!(card.level, expected.as_str(), "{}", p.id);
    }
    // Unknown threshold: PracticeOnly, no verdict, a practice formula.
    let chsl = profile("ssc-chsl");
    let t = chsl.test(Language::English).unwrap();
    assert_eq!(t.qualification, Field::Unknown);
    let out = evaluate(&chsl, t, &marks_input(100.0));
    assert_eq!(out.certification, Certification::PracticeOnly);
    assert_eq!(out.policy, PolicyKind::PracticeOnly);
    assert_eq!(out.verdict, None);
    assert_eq!(out.measure, None);
    assert!(out.formula.contains("no pass or fail"));

    // All required fields verified: Official.
    let official = parse_profile("t.toml", &doc(VERIFIED_TEST, "")).unwrap();
    assert_eq!(official.certification(), Certification::Official);
    // One required field unverified: Provisional.
    let provisional = parse_profile(
        "t.toml",
        &doc(
            &VERIFIED_TEST.replace(ERROR_POLICY, "error_policy = { status = \"configurable\" }"),
            "",
        ),
    )
    .unwrap();
    assert_eq!(provisional.certification(), Certification::Provisional);
}

#[test]
fn card_summaries_show_known_values_only() {
    let patna = card_summary(&profile("patna-hc"));
    assert_eq!(patna.eng_speed, "40 WPM · 400 words");
    assert_eq!(patna.hin_speed, "30 WPM · 300 words");
    assert_eq!(patna.rule, "10 min · min 90 / 85 marks");
    let beltron = card_summary(&profile("beltron-deo"));
    assert_eq!(
        (beltron.eng_speed.as_str(), beltron.hin_speed.as_str()),
        ("30 WPM", "25 WPM")
    );
    assert_eq!(beltron.rule, "Duration: to verify · 30 / 25 correct WPM");
    let mangal = card_summary(&profile("all-in-one-mangal"));
    assert_eq!(mangal.eng_speed, "Not offered");
    for id in ["ssc-chsl", "rrb-ntpc", "bssc-inter", "bihar-civil-court"] {
        let c = card_summary(&profile(id));
        assert_eq!(
            (c.eng_speed.as_str(), c.hin_speed.as_str()),
            ("—", "—"),
            "{id}"
        );
    }
    assert_eq!(
        card_summary(&profile("bssc-inter")).rule,
        "Primary advertisement pending"
    );
    assert_eq!(
        card_summary(&profile("bihar-civil-court")).rule,
        "Practice only — no pass / fail"
    );
}

#[test]
fn clerk_01_2022_is_never_official() {
    let clerk = profile("bihar-civil-court");
    assert!(clerk.unofficial_practice);
    assert_eq!(clerk.certification(), Certification::PracticeOnly);
    let notice = clerk.practice_notice.as_deref().unwrap();
    assert!(notice.contains("01/2022") && notice.contains("not the official"));
    for t in &clerk.tests {
        assert_eq!(t.qualification, Field::Unknown);
        assert_eq!(clerk.test_certification(t), Certification::PracticeOnly);
        assert_eq!(evaluate(&clerk, t, &marks_input(100.0)).verdict, None);
    }
    // Even with every field verified, the unofficial flag keeps it practice.
    let forged = parse_profile(
        "clerk.toml",
        &doc(VERIFIED_TEST, "unofficial_practice = true\n")
            .replace("id = \"t\"", "id = \"bihar-civil-court\""),
    )
    .unwrap();
    assert_eq!(forged.tests[0].certification(), Certification::Official);
    assert_eq!(forged.certification(), Certification::PracticeOnly);
    let out = evaluate(&forged, &forged.tests[0], &marks_input(100.0));
    assert_eq!(
        (out.certification, out.verdict),
        (Certification::PracticeOnly, None)
    );
}

#[test]
fn profile_versions_coexist_and_results_keep_their_rules() {
    let v1 = doc(VERIFIED_TEST, "");
    let v2 = v1
        .replace("profile_version = 1", "profile_version = 2")
        .replace("min_marks = 90.0", "min_marks = 95.0");
    let report = Registry::load(&[("v1.toml", &v1), ("v2.toml", &v2)]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let r = report.registry;
    assert_eq!(r.len(), 2);
    assert_eq!(r.ids(), ["t"]);
    assert_eq!(r.latest("t").unwrap().profile_version, 2);
    // A result stored as t@1 is re-evaluated under v1's rules.
    let stored = r.by_key("t@1").unwrap();
    let input = marks_input(92.0);
    assert_eq!(
        evaluate(stored, &stored.tests[0], &input).verdict,
        Some(Verdict::Qualified)
    );
    let latest = r.by_key("t@2").unwrap();
    assert_eq!(
        evaluate(latest, &latest.tests[0], &input).verdict,
        Some(Verdict::NotQualified)
    );
    assert!(r.by_key("t@3").is_none());
    assert_eq!(radha_exams::parse_key("patna-hc@1"), Some(("patna-hc", 1)));
    for bad in ["patna-hc", "patna-hc@", "@1", "patna-hc@0", "patna-hc@x"] {
        assert_eq!(radha_exams::parse_key(bad), None, "{bad}");
    }

    // The same id@version twice: an error, the first one kept.
    let dup = Registry::load(&[("a.toml", &v1), ("b.toml", &v1)]);
    assert_eq!(dup.registry.len(), 1);
    assert_eq!(
        dup.errors,
        [ProfileError::DuplicateProfile {
            key: "t@1".to_owned()
        }]
    );
    // Another schema version: skipped, not an error, nothing loaded.
    let future = v1.replace("schema_version = 1", "schema_version = 2");
    let skipped = Registry::load(&[("f.toml", &future)]);
    assert!(skipped.registry.is_empty() && skipped.errors.is_empty());
    assert!(matches!(
        skipped.skipped.as_slice(),
        [ProfileError::UnsupportedSchema { found: 2, .. }]
    ));
}

#[test]
fn invalid_profile_data_is_a_typed_error() {
    let err = |text: String| parse_profile("bad.toml", &text).unwrap_err();
    // A verified value without a source id.
    assert!(matches!(
        err(doc(
            &VERIFIED_TEST.replace(", sources = [\"P-1\"] }\nmax_marks", " }\nmax_marks"),
            ""
        )),
        ProfileError::MissingSource { .. }
    ));
    // A negative duration.
    assert!(matches!(
        err(doc(&VERIFIED_TEST.replace("value = 600", "value = -600"), "")),
        ProfileError::InvalidValue { ref field, .. } if field == "tests.english.duration_seconds"
    ));
    // A zero / over-long duration.
    assert!(matches!(
        err(doc(&VERIFIED_TEST.replace("value = 600", "value = 0"), "")),
        ProfileError::InvalidValue { .. }
    ));
    assert!(matches!(
        err(doc(
            &VERIFIED_TEST.replace("value = 600", "value = 3601"),
            ""
        )),
        ProfileError::InvalidValue { .. }
    ));
    // An unknown source id.
    assert!(matches!(
        err(doc(
            &VERIFIED_TEST.replace(
                "sources = [\"P-1\"] }\nmax_marks",
                "sources = [\"P-9\"] }\nmax_marks"
            ),
            ""
        )),
        ProfileError::UnknownSource { .. }
    ));
    // Verified from a secondary source.
    assert!(matches!(
        err(doc(VERIFIED_TEST, "").replace("kind = \"primary\"", "kind = \"secondary\"")),
        ProfileError::InvalidValue { .. }
    ));
    // A policy that cannot judge the threshold.
    assert!(matches!(
        err(doc(
            &VERIFIED_TEST.replace("accuracy_as_marks", "correct_wpm_threshold"),
            ""
        )),
        ProfileError::PolicyMismatch { .. }
    ));
    // A known threshold with practice_only scoring.
    assert!(matches!(
        err(doc(
            &VERIFIED_TEST.replace("accuracy_as_marks", "practice_only"),
            ""
        )),
        ProfileError::PolicyMismatch { .. }
    ));
    // A Hindi layout on an English test.
    assert!(matches!(
        err(doc(
            &VERIFIED_TEST.replace("[\"qwerty\"]", "[\"gail\"]"),
            ""
        )),
        ProfileError::InvalidValue { .. }
    ));
    // An unknown status, an unknown field, an invalid id, no tests.
    assert!(matches!(
        err(doc(
            &VERIFIED_TEST.replace("\"configurable\"", "\"maybe\""),
            ""
        )),
        ProfileError::InvalidStatus { .. }
    ));
    assert!(matches!(
        err(doc(VERIFIED_TEST, "pass_mark = 1\n")),
        ProfileError::Toml { .. }
    ));
    assert!(matches!(
        err(doc(VERIFIED_TEST, "").replace("id = \"t\"", "id = \"Bad Id\"")),
        ProfileError::InvalidId { .. }
    ));
    assert!(matches!(
        err(doc("", "")),
        ProfileError::InvalidValue { ref field, .. } if field == "tests"
    ));
    // Hindi-only with an English test.
    assert!(matches!(
        err(doc(VERIFIED_TEST, "hindi_only = true\n")),
        ProfileError::InvalidValue { .. }
    ));
    // Every error explains itself.
    assert!(err(doc("", "")).to_string().contains("at least one test"));
}

#[test]
fn bssc_has_no_coaching_figures_as_rules() {
    let text = radha_exams::registry::EMBEDDED
        .iter()
        .find(|(f, _)| *f == "bssc-inter.toml")
        .map(|(_, t)| *t)
        .unwrap();
    // Grep: 35 / 30 appear only inside the unverified reference claims.
    let (rules, claims) = text.split_once("[[reference_claims]]").unwrap();
    let numbers: Vec<&str> = rules
        .split(|c: char| !c.is_ascii_digit())
        .filter(|n| *n == "35" || *n == "30")
        .collect();
    assert!(numbers.is_empty(), "BSSC rules carry {numbers:?}");
    assert!(!rules.contains("35/30"));
    assert!(claims.contains("35 WPM") && claims.contains("30 WPM"));

    let bssc = profile("bssc-inter");
    assert_eq!(bssc.certification(), Certification::PracticeOnly);
    for t in &bssc.tests {
        assert_eq!(t.speed_wpm, Field::Unknown);
        assert_eq!(t.duration_seconds, Field::Unknown);
        assert_eq!(t.qualification, Field::Unknown);
        assert_eq!(t.layouts, Field::Unknown);
        assert_eq!(t.scoring, PolicyKind::PracticeOnly);
    }
    // Advertisement / cycle / post structure, typing-required posts only.
    let ads: Vec<&str> = bssc
        .cycles
        .iter()
        .map(|c| c.advertisement.as_str())
        .collect();
    assert_eq!(ads, ["02/23", "02/25"]);
    for c in &bssc.cycles {
        assert!(!c.posts.is_empty());
        for p in &c.posts {
            assert_eq!(
                p.typing_required.status(),
                Status::Configurable,
                "{}",
                p.name
            );
        }
    }
}

#[test]
fn reference_exams_keep_their_figures_as_unverified_claims() {
    let r = registry();
    for p in r.all() {
        if matches!(p.id.as_str(), "all-in-one-mangal" | "bssc-inter") {
            continue;
        }
        assert!(
            p.reference_claims
                .iter()
                .any(|c| c.source.contains("01-home.html")),
            "{}: the mock-up claim",
            p.id
        );
        if p.certification() == Certification::PracticeOnly {
            for t in &p.tests {
                assert!(t.speed_wpm.known().is_none(), "{}", p.id);
                assert!(t.qualification.known().is_none(), "{}", p.id);
                assert!(t.duration_seconds.known().is_none(), "{}", p.id);
                assert!(t.backspace.known().is_none(), "{}", p.id);
            }
        }
    }
    // Patna HC: the advertisement's numbers are Verified, the mistake and
    // Backspace rules are not invented.
    let patna = profile("patna-hc");
    for t in &patna.tests {
        assert_eq!(t.error_policy.status(), Status::Configurable);
        assert_eq!(t.backspace.status(), Status::Configurable);
        assert_eq!(t.backspace.effective(), None);
        assert!(t.qualification.source_ids().contains(&"S-03".to_owned()));
    }
    assert!(matches!(
        patna.test(Language::Hindi).unwrap().qualification.known(),
        Some(Qualification::MinMarks(v)) if *v == 85.0
    ));
}

#[test]
fn full_half_mistake_weights_come_from_the_profile() {
    let text = doc(
        &VERIFIED_TEST
            .replace("accuracy_as_marks", "full_half_mistake")
            .replace("min_marks = 90.0", "min_accuracy = 90.0")
            .replace(
                ERROR_POLICY,
                "error_policy = { status = \"verified\", value = \"full and half mistakes\", sources = [\"P-1\"] }\n\
                 mistake_weights = { status = \"configurable\", default = { full = 1.0, half = 0.5 }, note = \"Expert AI convention, not notice text\" }",
            ),
        "",
    );
    let p = parse_profile("fh.toml", &text).unwrap();
    let t = &p.tests[0];
    assert_eq!(t.mistake_weights.status(), Status::Configurable);
    // 100 words, 6 full + 8 half = 10 weighted mistakes: 90.00 %.
    let input = TestInput {
        words_typed: 100,
        full_mistakes: 6,
        half_mistakes: 8,
        elapsed_seconds: 600.0,
        ..TestInput::default()
    };
    let out = evaluate(&p, t, &input);
    assert_eq!(out.policy, PolicyKind::FullHalfMistake);
    assert_eq!(out.measure, Some(("accuracy %", 90.0)));
    assert_eq!(out.verdict, Some(Verdict::Qualified));
    assert!(out.notes.iter().any(|n| n.contains("Expert AI convention")));
    let one_more = TestInput {
        half_mistakes: 9,
        ..input
    };
    assert_eq!(
        evaluate(&p, t, &one_more).verdict,
        Some(Verdict::NotQualified)
    );
    // Without weights the policy cannot load.
    let missing = text.replace(
        "mistake_weights = { status = \"configurable\", default = { full = 1.0, half = 0.5 }, note = \"Expert AI convention, not notice text\" }",
        "",
    );
    assert!(matches!(
        parse_profile("fh.toml", &missing),
        Err(ProfileError::PolicyMismatch { .. })
    ));
}

// --- Synthetic documents ---------------------------------------------------------

const ERROR_POLICY: &str =
    "error_policy = { status = \"verified\", value = \"accuracy\", sources = [\"P-1\"] }";

/// An English test with every required field verified.
const VERIFIED_TEST: &str = r#"
[[tests]]
language = "english"
layouts = { status = "verified", value = ["qwerty"], sources = ["P-1"] }
font = { status = "configurable", default = "Arial" }
duration_seconds = { status = "verified", value = 600, sources = ["P-1"] }
max_marks = { status = "verified", value = 100, sources = ["P-1"] }
qualification = { status = "verified", value = { min_marks = 90.0 }, sources = ["P-1"] }
scoring = "accuracy_as_marks"
error_policy = { status = "verified", value = "accuracy", sources = ["P-1"] }
"#;

/// A profile document around `tests`, with `extra` top-level keys.
fn doc(tests: &str, extra: &str) -> String {
    format!(
        "schema_version = 1\nid = \"t\"\nprofile_version = 1\ndisplay_name = \"Test exam\"\n\
         body = \"Test body\"\n{extra}\n[notice]\ntitle = \"Test notice\"\n\n\
         [[sources]]\nid = \"P-1\"\nkind = \"primary\"\ntitle = \"Test notice PDF\"\nread = true\n{tests}"
    )
}
