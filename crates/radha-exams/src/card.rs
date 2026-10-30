//! The rule-derived text of an Exam Hub card. Only known values (Verified
//! or UserAsserted) appear; everything else is "—" (RULES.MD Rule 32).

use crate::profile::{ExamProfile, Language, Qualification};
use crate::Certification;

/// The rule fields of one Exam Hub card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardSummary {
    pub eng_speed: String,
    pub hin_speed: String,
    pub rule: String,
    /// "official" | "provisional" | "practice".
    pub level: &'static str,
}

fn number(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

fn speed(profile: &ExamProfile, language: Language) -> String {
    let Some(test) = profile.test(language) else {
        return "Not offered".to_owned();
    };
    if let Some(wpm) = test.speed_wpm.known() {
        return match test.word_count.known() {
            Some(words) => format!("{wpm} WPM · {words} words"),
            None => format!("{wpm} WPM"),
        };
    }
    match test.qualification.known() {
        Some(Qualification::MinCorrectWpm(v) | Qualification::MinNetWpm(v)) => {
            format!("{} WPM", number(*v))
        }
        _ => "—".to_owned(),
    }
}

/// The card text of `profile`.
#[must_use]
pub fn card_summary(profile: &ExamProfile) -> CardSummary {
    let cert = profile.certification();
    let rule = if cert == Certification::PracticeOnly {
        if profile.unofficial_practice || profile.hindi_only {
            "Practice only — no pass / fail".to_owned()
        } else if !profile.cycles.is_empty() {
            "Primary advertisement pending".to_owned()
        } else {
            "Rules pending verification".to_owned()
        }
    } else {
        let durations: Vec<Option<u32>> = profile
            .tests
            .iter()
            .map(|t| t.duration_seconds.known().copied())
            .collect();
        let duration = match durations.first() {
            Some(Some(d)) if durations.iter().all(|x| *x == Some(*d)) => {
                format!("{} min", number(f64::from(*d) / 60.0))
            }
            _ => "Duration: to verify".to_owned(),
        };
        let thresholds: Vec<Qualification> = profile
            .tests
            .iter()
            .filter_map(|t| t.qualification.known().copied())
            .collect();
        let values: Vec<String> = thresholds.iter().map(|q| number(q.threshold())).collect();
        let measure = thresholds.first().map_or("", |q| q.measure());
        let threshold = match thresholds.first() {
            Some(Qualification::MinMarks(_)) => format!("min {} marks", values.join(" / ")),
            Some(_) => format!("{} {measure}", values.join(" / ")),
            None => String::new(),
        };
        if threshold.is_empty() {
            duration
        } else {
            format!("{duration} · {threshold}")
        }
    };
    CardSummary {
        eng_speed: speed(profile, Language::English),
        hin_speed: speed(profile, Language::Hindi),
        rule,
        level: cert.as_str(),
    }
}
