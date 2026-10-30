//! Certification is derived from the field statuses, never read from a file
//! (RULES.MD Rule 32, PRD EM-04).

use crate::field::Status;
use crate::profile::{ExamProfile, TypingTest};
use crate::scoring::PolicyKind;

/// How far a result under a profile can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Certification {
    /// Threshold unknown (or an explicitly unofficial profile): no pass or
    /// fail, a practice-only notice.
    PracticeOnly,
    /// Thresholds known (Verified or UserAsserted) but another required
    /// field is not Verified: a pass/fail labelled "provisional".
    Provisional,
    /// Every required field (duration, qualification, error policy) is
    /// Verified.
    Official,
}

impl Certification {
    /// Stable id ("official" | "provisional" | "practice").
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Certification::PracticeOnly => "practice",
            Certification::Provisional => "provisional",
            Certification::Official => "official",
        }
    }

    /// Whether a pass/fail verdict may be shown.
    #[must_use]
    pub fn gives_verdict(self) -> bool {
        self != Certification::PracticeOnly
    }
}

/// One test's certification on its own (see
/// [`ExamProfile::test_certification`] for the capped value).
pub(crate) fn derive_test(test: &TypingTest) -> Certification {
    let threshold = test.qualification.status();
    if !matches!(threshold, Status::Verified | Status::UserAsserted)
        || test.scoring == PolicyKind::PracticeOnly
    {
        return Certification::PracticeOnly;
    }
    let required = [
        test.duration_seconds.status(),
        threshold,
        test.error_policy.status(),
    ];
    if required.iter().all(|s| *s == Status::Verified) {
        Certification::Official
    } else {
        Certification::Provisional
    }
}

/// A profile's certification: its weakest test, and never above
/// PracticeOnly for an unofficial-practice profile.
pub(crate) fn derive_profile(profile: &ExamProfile) -> Certification {
    if profile.unofficial_practice {
        return Certification::PracticeOnly;
    }
    profile
        .tests
        .iter()
        .map(TypingTest::certification)
        .min()
        .unwrap_or(Certification::PracticeOnly)
}
