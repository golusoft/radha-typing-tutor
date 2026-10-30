//! Exam Mode glue (PRD 17.9, RULES 31–33, 38): the embedded `radha-exams`
//! profiles applied to the shared EXERCISE → TYPING → RESULT flow.
//!
//! Everything exam-specific is read from a profile: the fixed timer, the
//! allowed layouts, the passage size, the Backspace rule, the certification
//! and the scoring policy. When a profile gives no value the app keeps its
//! own practice default and says so on screen; nothing is invented.

use std::sync::OnceLock;

use radha_content::ProductId;
use radha_core::TimerMode;
use radha_exams::{
    Backspace, Certification as Cert, ExamProfile, Language, Registry, Status, TestInput,
    TestOutcome, TypingTest, Verdict,
};
use radha_layouts::LayoutId;

use crate::exercise::{mm_ss, timer_for, ExamLanguage, PracticeMode, EXAM_PRACTICE_SECONDS};
use crate::report::Certification;
use crate::typing_rules::BackspacePolicy;

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// The embedded profiles, loaded (and logged) once.
pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| {
        let report = Registry::embedded();
        for e in &report.skipped {
            log::warn!("Exam profile skipped: {e}");
        }
        for e in &report.errors {
            log::error!("Exam profile not loaded: {e}");
        }
        log::info!("{} exam profiles loaded", report.registry.len());
        report.registry
    })
}

/// The newest profile of an Exam Hub card id.
#[must_use]
pub fn profile(id: &str) -> Option<&'static ExamProfile> {
    registry().latest(id)
}

/// The profile language of an exam toggle language.
#[must_use]
pub fn language(l: ExamLanguage) -> Language {
    match l {
        ExamLanguage::English => Language::English,
        ExamLanguage::Hindi => Language::Hindi,
    }
}

/// The profile's test in `l`.
#[must_use]
pub fn test(profile: Option<&ExamProfile>, l: ExamLanguage) -> Option<&TypingTest> {
    profile.and_then(|p| p.test(language(l)))
}

/// Whether the card offers `l` (a profile without that test does not).
#[must_use]
pub fn offers(profile: Option<&ExamProfile>, l: ExamLanguage) -> bool {
    profile.is_none_or(|p| p.test(language(l)).is_some())
}

/// The result badge level of a profile certification.
#[must_use]
pub fn certification(c: Cert) -> Certification {
    match c {
        Cert::PracticeOnly => Certification::PracticeOnly,
        Cert::Provisional => Certification::Provisional,
        Cert::Official => Certification::Official,
    }
}

/// Certification of one test (PracticeOnly without a profile).
#[must_use]
pub fn test_certification(profile: Option<&ExamProfile>, t: Option<&TypingTest>) -> Certification {
    match (profile, t) {
        (Some(p), Some(t)) => certification(p.test_certification(t)),
        _ => Certification::PracticeOnly,
    }
}

/// The badge text: "Unofficial practice" for an explicitly unofficial
/// profile, else the certification pill.
#[must_use]
pub fn pill(profile: Option<&ExamProfile>, t: Option<&TypingTest>) -> &'static str {
    if profile.is_some_and(|p| p.unofficial_practice) {
        "Unofficial practice · no pass/fail"
    } else {
        test_certification(profile, t).pill()
    }
}

/// The fixed exam timer: the profile's duration, else the practice timer.
#[must_use]
pub fn timer(t: Option<&TypingTest>) -> TimerMode {
    t.map_or_else(
        || timer_for(PracticeMode::Exam, TimerMode::Off),
        |t| t.timer_mode(EXAM_PRACTICE_SECONDS),
    )
}

/// Where the timer comes from, for the banner.
#[must_use]
pub fn timer_note(t: Option<&TypingTest>) -> String {
    let clock = |s: u32| mm_ss(s);
    match t.map(|t| (&t.duration_seconds, t.duration_seconds.effective())) {
        Some((f, Some(s))) if f.known().is_some() => {
            format!(
                "{} timer from the notice ({})",
                clock(*s),
                f.status().label()
            )
        }
        Some((_, Some(s))) => format!("{} timer (configurable, unverified)", clock(*s)),
        _ => format!(
            "{} practice timer (the exam's duration is not verified)",
            clock(EXAM_PRACTICE_SECONDS)
        ),
    }
}

/// The Backspace rule of a test: off only when a known rule disables it,
/// otherwise the exam default (allowed).
#[must_use]
pub fn backspace(t: Option<&TypingTest>) -> BackspacePolicy {
    match t.and_then(|t| t.backspace.known()) {
        Some(Backspace::Disabled) => BackspacePolicy::Off,
        _ => BackspacePolicy::Full,
    }
}

/// Words of the passage to type (0: the whole passage), as
/// `passage::truncate_words` takes it.
#[must_use]
pub fn word_limit(t: Option<&TypingTest>) -> u32 {
    t.and_then(TypingTest::passage_words).unwrap_or(0)
}

/// Whether the profile lets `layout` be used (a missing test or profile
/// does not restrict).
#[must_use]
pub fn layout_allowed(profile: Option<&ExamProfile>, layout: LayoutId) -> bool {
    let l = ExamLanguage::of_layout(layout);
    test(profile, l).is_none_or(|t| t.allows_layout(layout))
}

/// The carrier product of a language test, honouring the profile's layouts
/// and the user's preferred layout. `fallback` is the app's own choice.
#[must_use]
pub fn carrier(
    profile: Option<&ExamProfile>,
    l: ExamLanguage,
    preferred: LayoutId,
    fallback: ProductId,
) -> ProductId {
    match (l, test(profile, l)) {
        (ExamLanguage::English, _) => ProductId::EnglishClassic,
        (ExamLanguage::Hindi, Some(t)) if !t.allows_layout(fallback.layout()) => {
            ProductId::HindiMastery(t.default_layout(preferred))
        }
        _ => fallback,
    }
}

/// Unverified parts of a test, for the banner ("error policy, backspace").
fn unverified(t: &TypingTest) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (name, status) in [
        ("duration", t.duration_seconds.status()),
        ("threshold", t.qualification.status()),
        ("error policy", t.error_policy.status()),
        ("backspace", t.backspace.status()),
    ] {
        if status != Status::Verified {
            out.push(name);
        }
    }
    out
}

/// The Exercise screen notice for an exam test, and its banner level.
#[must_use]
pub fn banner(
    profile: Option<&ExamProfile>,
    l: ExamLanguage,
    title: &str,
) -> (String, &'static str) {
    let t = test(profile, l);
    let cert = test_certification(profile, t);
    let key = profile.map_or_else(|| "no profile".to_owned(), ExamProfile::key);
    let lang = match l {
        ExamLanguage::English => "English",
        ExamLanguage::Hindi => "Hindi",
    };
    let mut parts = vec![format!("{} · {title} ({key}).", pill(profile, t))];
    if let Some(n) = profile.and_then(|p| p.practice_notice.as_deref()) {
        parts.push(n.to_owned());
    }
    let mut test_line = format!("{lang} test: ");
    if let Some(w) = t.and_then(TypingTest::passage_words) {
        test_line.push_str(&format!("{w} words, "));
    }
    test_line.push_str(&timer_note(t));
    match t.and_then(|t| t.qualification.known().map(|q| (t, q))) {
        Some((t, q)) if cert != Certification::PracticeOnly => {
            test_line.push_str(&format!(
                "; qualifies at {} {} ({})",
                q.threshold(),
                q.measure(),
                t.qualification.status().label()
            ));
        }
        _ => test_line.push_str("; no pass or fail"),
    }
    test_line.push('.');
    parts.push(test_line);
    if let Some(t) = t {
        let open = unverified(t);
        if !open.is_empty() && cert != Certification::PracticeOnly {
            parts.push(format!("Not verified: {}.", open.join(", ")));
        }
    }
    if profile.is_some_and(|p| !p.reference_claims.is_empty()) {
        parts.push(
            "Published figures for this exam are unverified claims and are not applied.".to_owned(),
        );
    }
    parts
        .push("Rules are locked; Guide off; practice passages, not the official paper.".to_owned());
    let level = match cert {
        Certification::Official => "official",
        Certification::Provisional => "provisional",
        _ => "practice",
    };
    (parts.join(" "), level)
}

/// Score a finished exam session.
#[must_use]
pub fn outcome(profile: &ExamProfile, t: &TypingTest, input: &TestInput) -> TestOutcome {
    radha_exams::evaluate(profile, t, input)
}

/// The Result verdict card: headline, detail and whether it reads as good.
#[must_use]
pub fn verdict_text(profile: &ExamProfile, out: &TestOutcome) -> (String, String, bool) {
    let rules = match out.certification {
        Cert::Official => "official rules",
        _ => "provisional rules",
    };
    let measure = out
        .measure
        .map(|(label, v)| format!("{v:.2} {label}"))
        .unwrap_or_default();
    let threshold = out.threshold.map(|t| format!("{t}")).unwrap_or_default();
    let notes = if out.notes.is_empty() {
        String::new()
    } else {
        format!(" Not verified: {}.", out.notes.join("; "))
    };
    match out.verdict {
        Some(Verdict::Qualified) => (
            format!("Qualified · {rules}"),
            format!(
                "{measure} reaches the {threshold} needed ({}).{notes}",
                profile.key()
            ),
            true,
        ),
        Some(Verdict::NotQualified) => (
            format!("Not qualified · {rules}"),
            format!(
                "{measure} is below the {threshold} needed ({}).{notes}",
                profile.key()
            ),
            false,
        ),
        None => (
            "Practice attempt recorded in RAM".to_owned(),
            format!(
                "{} — {} ({})",
                if profile.unofficial_practice {
                    "Unofficial practice · no pass/fail"
                } else {
                    Certification::PracticeOnly.pill()
                },
                profile
                    .practice_notice
                    .as_deref()
                    .unwrap_or("this exam's rules are not verified, so no pass or fail is shown."),
                profile.key()
            ),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::practice::{HintView, Practice};
    use radha_core::{DisplayMode, SessionState};
    use std::time::{Duration, Instant};

    fn p(id: &str) -> &'static ExamProfile {
        profile(id).unwrap()
    }

    #[test]
    fn every_hub_card_has_a_profile_and_the_registry_loads_cleanly() {
        let report = Registry::embedded();
        assert!(report.errors.is_empty() && report.skipped.is_empty());
        assert_eq!(registry().len(), 22);
        assert!(p("patna-hc").test(Language::Hindi).is_some());
    }

    #[test]
    fn exam_layout_selection_uses_the_profile() {
        let patna = Some(p("patna-hc"));
        // Hindi on a GAIL profile: CBI / Kruti Dev / InScript are refused,
        // and a CBI preference still opens on GAIL.
        assert!(layout_allowed(patna, LayoutId::RemingtonGail));
        for l in [
            LayoutId::RemingtonCbi,
            LayoutId::KrutiDev,
            LayoutId::Inscript,
        ] {
            assert!(!layout_allowed(patna, l), "{l:?}");
        }
        assert_eq!(
            carrier(
                patna,
                ExamLanguage::Hindi,
                LayoutId::RemingtonCbi,
                ProductId::HindiMastery(LayoutId::RemingtonCbi)
            ),
            ProductId::HindiMastery(LayoutId::RemingtonGail)
        );
        assert_eq!(
            carrier(
                patna,
                ExamLanguage::English,
                LayoutId::RemingtonCbi,
                ProductId::EnglishClassic
            ),
            ProductId::EnglishClassic
        );
        // A reference profile keeps the user's layout.
        let chsl = Some(p("ssc-chsl"));
        let kd = ProductId::HindiMastery(LayoutId::KrutiDev);
        assert_eq!(
            carrier(chsl, ExamLanguage::Hindi, LayoutId::KrutiDev, kd),
            kd
        );
        assert!(layout_allowed(chsl, LayoutId::Inscript));
        // All-in-One Mangal: no English, no Kruti Dev.
        let mangal = Some(p("all-in-one-mangal"));
        assert!(!offers(mangal, ExamLanguage::English));
        assert!(offers(mangal, ExamLanguage::Hindi));
        assert!(!layout_allowed(mangal, LayoutId::KrutiDev));
        assert!(layout_allowed(mangal, LayoutId::Inscript));
    }

    #[test]
    fn exam_typing_takes_timer_words_and_backspace_from_the_profile() {
        let patna = Some(p("patna-hc"));
        let en = test(patna, ExamLanguage::English);
        let hi = test(patna, ExamLanguage::Hindi);
        assert!(matches!(timer(en), TimerMode::Countdown(l) if l.seconds() == 600));
        assert_eq!(word_limit(en), 400);
        assert_eq!(word_limit(hi), 300);
        assert!(timer_note(en).starts_with("10:00 timer from the notice (verified)"));
        // Backspace is not stated: the exam default, never invented "off".
        assert_eq!(backspace(en), BackspacePolicy::Full);
        // No duration anywhere: the labelled practice timer.
        let beltron = test(Some(p("beltron-deo")), ExamLanguage::English);
        assert!(
            matches!(timer(beltron), TimerMode::Countdown(l) if l.seconds() == EXAM_PRACTICE_SECONDS)
        );
        assert!(timer_note(beltron).contains("not verified"));
        assert_eq!(word_limit(beltron), 0, "whole passage");
        assert_eq!(timer(None), timer_for(PracticeMode::Exam, TimerMode::Off));
    }

    #[test]
    fn banners_pills_and_verdicts_are_honest() {
        let clerk = Some(p("bihar-civil-court"));
        let (text, level) = banner(clerk, ExamLanguage::Hindi, "Bihar Civil Court — Clerk");
        assert_eq!(level, "practice");
        assert!(text.contains("not the official typing test"), "{text}");
        assert!(text.contains("no pass or fail"), "{text}");
        assert_eq!(
            pill(clerk, test(clerk, ExamLanguage::Hindi)),
            "Unofficial practice · no pass/fail"
        );
        let patna = Some(p("patna-hc"));
        let (text, level) = banner(patna, ExamLanguage::English, "Patna High Court — COT 2026");
        assert_eq!(level, "provisional");
        assert!(text.contains("patna-hc@1"), "{text}");
        assert!(text.contains("400 words"), "{text}");
        assert!(text.contains("qualifies at 90 marks (verified)"), "{text}");
        assert!(
            text.contains("Not verified: error policy, backspace."),
            "{text}"
        );

        let profile = p("patna-hc");
        let t = profile.test(Language::English).unwrap();
        let input = TestInput {
            accuracy: 89.99,
            elapsed_seconds: 600.0,
            ..TestInput::default()
        };
        let (head, detail, good) = verdict_text(profile, &outcome(profile, t, &input));
        assert_eq!(head, "Not qualified · provisional rules");
        assert!(
            detail.starts_with("89.99 marks is below the 90 needed (patna-hc@1)."),
            "{detail}"
        );
        assert!(!good);
        let pass = TestInput {
            accuracy: 90.0,
            ..input
        };
        let (head, _, good) = verdict_text(profile, &outcome(profile, t, &pass));
        assert_eq!(head, "Qualified · provisional rules");
        assert!(good);

        let chsl = p("ssc-chsl");
        let t = chsl.test(Language::English).unwrap();
        let (head, detail, _) = verdict_text(chsl, &outcome(chsl, t, &pass));
        assert_eq!(head, "Practice attempt recorded in RAM");
        assert!(detail.contains("no pass or fail"), "{detail}");
        assert_eq!(
            certification(chsl.certification()),
            Certification::PracticeOnly
        );
    }

    /// The US character that strikes a hint's key.
    fn hint_char(h: &HintView) -> char {
        let base = h.key_id.chars().next().unwrap_or(' ');
        if h.shift {
            radha_layouts::PhysicalKey::from_base(base)
                .map(|k| k.shifted_char())
                .unwrap_or(base)
        } else {
            base
        }
    }

    #[test]
    fn hindi_unicode_exam_text_types_and_renders_as_unicode() {
        // Conjuncts (क्ष ज्ञ श्र त्र), the pre-base ि, anusvara, chandrabindu,
        // visarga and a nukta-free mix, as an exam passage would hold them.
        let text = "क्षेत्र ज्ञान श्रद्धा पत्र किताब हिंदी हँसी दुःख";
        let patna = Some(p("patna-hc"));
        let t = test(patna, ExamLanguage::Hindi).unwrap();
        assert_eq!(t.font.known().map(String::as_str), Some("Mangal"));
        // The Hindi exam font is Mangal, or the embedded Noto Sans
        // Devanagari when Mangal is missing (never a legacy font).
        let fonts = crate::exam_fonts::ExamFonts {
            arial: true,
            mangal: false,
        };
        assert_eq!(fonts.devanagari(), crate::exam_fonts::DEVANAGARI_FALLBACK);
        let fonts = crate::exam_fonts::ExamFonts {
            arial: true,
            mangal: true,
        };
        assert_eq!(fonts.devanagari(), crate::exam_fonts::MANGAL);

        for layout in [LayoutId::RemingtonGail, LayoutId::Inscript] {
            let mut pr = Practice::new(layout, text, timer(Some(t)), DisplayMode::Word).unwrap();
            let mut now = Instant::now();
            let mut hint = pr.view(now).hint;
            let mut guard = 0;
            while let Some(h) = hint.clone() {
                guard += 1;
                assert!(guard < 400, "{layout:?}: the guide loops");
                pr.press(hint_char(&h), h.altgr, now);
                now += Duration::from_millis(150);
                let v = pr.view(now);
                assert_eq!(v.stats.errors, 0, "{layout:?}: following the guide");
                hint = v.hint;
                if pr.state() == SessionState::Complete {
                    break;
                }
            }
            // Stored and shown as NFC Unicode, akshar by akshar.
            assert_eq!(
                pr.typed(),
                radha_core::akshar::normalize(text),
                "{layout:?}"
            );
            assert_eq!(pr.state(), SessionState::Complete, "{layout:?}");
        }
        let akshars = radha_core::akshar::segment("क्षेत्र");
        assert_eq!(akshars, ["क्षे", "त्र"]);
    }
}
