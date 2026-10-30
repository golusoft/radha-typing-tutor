//! P09 Exam Mode wiring: every Exam Hub card has a `radha-exams` profile with
//! the same id, and the card literals equal the values Rust derives from the
//! profiles at startup (so the fallback never claims more than the data).

use std::path::PathBuf;

use radha_exams::{card_summary, Registry};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo().join(rel)).unwrap()
}

/// `(id, field → value)` for every `ExamEntry` literal in exams.slint.
fn cards() -> Vec<(String, Vec<(String, String)>)> {
    let text = read("crates/radha-app/ui/data/exams.slint");
    text.split("id: \"")
        .skip(1)
        .map(|chunk| {
            let id = chunk.split('"').next().unwrap().to_owned();
            let body = &chunk[..chunk.find("},").unwrap_or(chunk.len())];
            let mut fields = Vec::new();
            for key in ["eng-speed", "hin-speed", "rule", "level"] {
                let pat = format!("{key}: \"");
                let value = body
                    .split(&pat)
                    .nth(1)
                    .and_then(|s| s.split('"').next())
                    .unwrap_or_else(|| panic!("{id}: {key}"));
                fields.push((key.to_owned(), value.to_owned()));
            }
            (id, fields)
        })
        .collect()
}

#[test]
fn every_exam_card_has_a_profile_and_matches_it() {
    let report = Registry::embedded();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let registry = report.registry;
    let cards = cards();
    let ids: Vec<&str> = cards.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, registry.ids(), "same cards, same order");
    assert_eq!(
        ids.len(),
        22,
        "All-in-One Mangal + 20 reference exams + BSSC"
    );
    assert_eq!(ids[0], "all-in-one-mangal");
    for (id, fields) in &cards {
        let p = registry.latest(id).unwrap();
        let derived = card_summary(p);
        let expected = [
            ("eng-speed", derived.eng_speed.as_str()),
            ("hin-speed", derived.hin_speed.as_str()),
            ("rule", derived.rule.as_str()),
            ("level", derived.level),
        ];
        for ((key, literal), (k, value)) in fields.iter().zip(expected) {
            assert_eq!(key, k);
            assert_eq!(literal, value, "{id}: {key}");
        }
    }
}

#[test]
fn exam_typing_locks_rules_guide_custom_passages_and_timer() {
    let flow = read("crates/radha-app/src/ui/flow.rs");
    for guard in [
        // RULES 38: the Rules sidebar is read-only in an exam.
        "self.rules.locked = ctx.is_exam();",
        // Custom passages are refused in an exam.
        "Exams type their own passages",
        // The timer comes from the profile and the header menu ignores it.
        "self.edit_draft(|d| d.timer = timer);",
        "exam_mode::timer(self.exam.test(language))",
        // Passage size and Backspace from the profile.
        "exam_mode::word_limit(",
        "exam_mode::backspace(exam_test)",
        // The result: the profile's policy and verdict.
        "exam_mode::outcome(profile, t, &input)",
        "exam_mode::verdict_text(profile, out)",
        "(\"Exam profile\", profile.key())",
    ] {
        assert!(flow.contains(guard), "missing: {guard}");
    }
    let ui = read("crates/radha-app/src/ui.rs");
    let profiles = ui.find("apply_exam_profiles(&mut catalog.exams)").unwrap();
    let counts = ui
        .find("apply_exam_pool_counts(&mut catalog.exams")
        .unwrap();
    assert!(profiles < counts, "the pool count chip is written last");
    // Guide: off in exams (typing_rules::effective_guide, unit-tested).
    let rules = read("crates/radha-app/src/typing_rules.rs");
    assert!(
        rules.contains("pub fn effective_guide(chosen: GuideMode, key_guide: bool, exam: bool)")
    );
}
