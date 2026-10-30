//! The exam profile model and its TOML form (EXAM-MODE-AND-GOLU-REUSE-PLAN.md
//! Section 5.2). Parsing validates everything; a profile that loads is
//! internally consistent.

use radha_core::{TimeLimit, TimerMode};
use radha_layouts::LayoutId;
use serde::Deserialize;

use crate::field::{field_or_unknown, Field, RawField};
use crate::scoring::PolicyKind;
use crate::{Certification, ProfileError, PROFILE_SCHEMA_VERSION};

/// The language of one typing test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    English,
    Hindi,
}

impl Language {
    /// The TOML spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Language::English => "english",
            Language::Hindi => "hindi",
        }
    }

    /// Display name.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Hindi => "Hindi",
        }
    }

    /// Every layout this language can be typed on, in presentation order.
    #[must_use]
    pub fn layouts(self) -> &'static [LayoutId] {
        match self {
            Language::English => &[LayoutId::Qwerty],
            Language::Hindi => &[
                LayoutId::RemingtonGail,
                LayoutId::RemingtonCbi,
                LayoutId::KrutiDev,
                LayoutId::Inscript,
            ],
        }
    }

    /// The language a layout types.
    #[must_use]
    pub fn of_layout(layout: LayoutId) -> Self {
        if layout == LayoutId::Qwerty {
            Language::English
        } else {
            Language::Hindi
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "english" => Some(Language::English),
            "hindi" => Some(Language::Hindi),
            _ => None,
        }
    }
}

/// What a candidate must reach to qualify.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Qualification {
    /// Minimum marks (out of `max_marks`).
    MinMarks(f64),
    /// Minimum accuracy, percent.
    MinAccuracy(f64),
    /// Minimum correct words per minute.
    MinCorrectWpm(f64),
    /// Minimum net words per minute.
    MinNetWpm(f64),
}

impl Qualification {
    /// The threshold number.
    #[must_use]
    pub fn threshold(self) -> f64 {
        match self {
            Qualification::MinMarks(v)
            | Qualification::MinAccuracy(v)
            | Qualification::MinCorrectWpm(v)
            | Qualification::MinNetWpm(v) => v,
        }
    }

    /// What the threshold measures.
    #[must_use]
    pub fn measure(self) -> &'static str {
        match self {
            Qualification::MinMarks(_) => "marks",
            Qualification::MinAccuracy(_) => "accuracy %",
            Qualification::MinCorrectWpm(_) => "correct WPM",
            Qualification::MinNetWpm(_) => "net WPM",
        }
    }
}

/// Backspace rule of a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backspace {
    Allowed,
    Disabled,
}

/// Full / half mistake weights (FullHalfMistake policy).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MistakeWeights {
    pub full: f64,
    pub half: f64,
}

/// Kind of a cited document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// The issuing body's own document.
    Primary,
    /// A copy of a primary document hosted elsewhere (still to be matched
    /// with the original).
    PrimaryMirror,
    /// Coaching / news site: a pointer, never a rule.
    Secondary,
    /// Golu's statement.
    User,
    /// Golu's own software or documents.
    Internal,
    /// A design mock-up.
    Reference,
}

impl SourceKind {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "primary" => Some(SourceKind::Primary),
            "primary_mirror" => Some(SourceKind::PrimaryMirror),
            "secondary" => Some(SourceKind::Secondary),
            "user" => Some(SourceKind::User),
            "internal" => Some(SourceKind::Internal),
            "reference" => Some(SourceKind::Reference),
            _ => None,
        }
    }

    /// Whether a Verified value may cite it.
    #[must_use]
    pub fn can_verify(self) -> bool {
        matches!(self, SourceKind::Primary | SourceKind::PrimaryMirror)
    }
}

/// One cited document.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub id: String,
    pub kind: SourceKind,
    pub title: String,
    pub url: Option<String>,
    /// Whether its text was read (not just a search snippet).
    pub read: bool,
}

/// The recruitment notice a profile is about.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub title: String,
    pub reference: Option<String>,
    pub source_ids: Vec<String>,
    pub note: Option<String>,
}

/// A figure seen somewhere (mock-up, coaching site, user's research list)
/// but not read in a primary notification. Always unverified: it is shown
/// as an "unverified claim" and never used as a rule.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceClaim {
    pub claim: String,
    /// Where the claim was seen.
    pub source: String,
    pub url: Option<String>,
}

/// A post of an advertisement cycle.
#[derive(Debug, Clone, PartialEq)]
pub struct Post {
    pub name: String,
    pub typing_required: Field<bool>,
}

/// One advertisement / recruitment cycle (BSSC-style profiles).
#[derive(Debug, Clone, PartialEq)]
pub struct Cycle {
    pub advertisement: String,
    pub label: String,
    pub posts: Vec<Post>,
    pub note: Option<String>,
}

/// One language's typing test.
#[derive(Debug, Clone, PartialEq)]
pub struct TypingTest {
    pub language: Language,
    pub layouts: Field<Vec<LayoutId>>,
    pub font: Field<String>,
    pub word_count: Field<u32>,
    pub speed_wpm: Field<u32>,
    pub duration_seconds: Field<u32>,
    pub max_marks: Field<u32>,
    pub qualification: Field<Qualification>,
    pub scoring: PolicyKind,
    pub error_policy: Field<String>,
    pub backspace: Field<Backspace>,
    pub mistake_weights: Field<MistakeWeights>,
    /// The key guide may be used (RULES 37: off in exams unless allowed).
    pub guide_allowed: bool,
}

impl TypingTest {
    /// Layouts the test may be typed on: exactly the known list, else
    /// every layout of the language (the configurable default first).
    #[must_use]
    pub fn allowed_layouts(&self) -> Vec<LayoutId> {
        if let Some(list) = self.layouts.known() {
            return list.clone();
        }
        let mut out: Vec<LayoutId> = self.layouts.effective().cloned().unwrap_or_default();
        for l in self.language.layouts() {
            if !out.contains(l) {
                out.push(*l);
            }
        }
        out
    }

    /// Whether `layout` may be used.
    #[must_use]
    pub fn allows_layout(&self, layout: LayoutId) -> bool {
        self.allowed_layouts().contains(&layout)
    }

    /// The layout to open on: `preferred` when allowed, else the first
    /// allowed layout (the known list, or the configurable default first).
    #[must_use]
    pub fn default_layout(&self, preferred: LayoutId) -> LayoutId {
        let allowed = self.allowed_layouts();
        if allowed.contains(&preferred) {
            preferred
        } else {
            allowed.first().copied().unwrap_or(preferred)
        }
    }

    /// The countdown of the test: the known or configurable duration.
    /// `None` when the profile gives none (the app then uses its own,
    /// labelled practice timer).
    #[must_use]
    pub fn time_limit(&self) -> Option<TimeLimit> {
        self.duration_seconds
            .effective()
            .and_then(|s| TimeLimit::custom(*s).ok())
    }

    /// The session timer for this test, `fallback_seconds` when the profile
    /// gives no duration. Always a countdown (exam timers are fixed).
    #[must_use]
    pub fn timer_mode(&self, fallback_seconds: u32) -> TimerMode {
        self.time_limit()
            .or_else(|| TimeLimit::custom(fallback_seconds).ok())
            .map_or(TimerMode::Stopwatch, TimerMode::Countdown)
    }

    /// Words of the passage to type: the known word count, else `None`
    /// (the whole passage).
    #[must_use]
    pub fn passage_words(&self) -> Option<u32> {
        self.word_count.known().copied()
    }

    /// Certification of this test (derived, never read from the file).
    #[must_use]
    pub fn certification(&self) -> Certification {
        crate::certification::derive_test(self)
    }
}

/// One exam profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ExamProfile {
    /// Stable id: the Exam Hub card id.
    pub id: String,
    /// Data version; bumped on every rule change.
    pub profile_version: u32,
    pub schema_version: u32,
    pub display_name: String,
    /// Issuing body.
    pub body: String,
    /// Explicitly unofficial practice: never Official, never pass/fail.
    pub unofficial_practice: bool,
    /// No English test; Hindi on the Mangal Unicode layouts.
    pub hindi_only: bool,
    pub practice_notice: Option<String>,
    pub notice: Notice,
    pub sources: Vec<Source>,
    pub tests: Vec<TypingTest>,
    pub cycles: Vec<Cycle>,
    pub reference_claims: Vec<ReferenceClaim>,
}

impl ExamProfile {
    /// `profile_id@profile_version`, stored with every result.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}@{}", self.id, self.profile_version)
    }

    /// The test of `language`.
    #[must_use]
    pub fn test(&self, language: Language) -> Option<&TypingTest> {
        self.tests.iter().find(|t| t.language == language)
    }

    /// The test of `language`, else the first test.
    #[must_use]
    pub fn test_or_first(&self, language: Language) -> Option<&TypingTest> {
        self.test(language).or_else(|| self.tests.first())
    }

    /// The languages offered, in file order.
    #[must_use]
    pub fn languages(&self) -> Vec<Language> {
        self.tests.iter().map(|t| t.language).collect()
    }

    /// Certification of the whole profile: its weakest test; always
    /// PracticeOnly for an unofficial-practice profile.
    #[must_use]
    pub fn certification(&self) -> Certification {
        crate::certification::derive_profile(self)
    }

    /// Certification of one of its tests, capped at PracticeOnly for an
    /// unofficial-practice profile.
    #[must_use]
    pub fn test_certification(&self, test: &TypingTest) -> Certification {
        if self.unofficial_practice {
            Certification::PracticeOnly
        } else {
            test.certification()
        }
    }

    /// The source with `id`.
    #[must_use]
    pub fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }
}

// --- TOML form -------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    schema_version: u32,
    id: String,
    profile_version: u32,
    display_name: String,
    body: String,
    #[serde(default)]
    unofficial_practice: bool,
    #[serde(default)]
    hindi_only: bool,
    practice_notice: Option<String>,
    notice: RawNotice,
    #[serde(default)]
    sources: Vec<RawSource>,
    #[serde(default)]
    tests: Vec<RawTest>,
    #[serde(default)]
    cycles: Vec<RawCycle>,
    #[serde(default)]
    reference_claims: Vec<RawClaim>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNotice {
    title: String,
    reference: Option<String>,
    #[serde(default)]
    sources: Vec<String>,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    id: String,
    kind: String,
    title: String,
    url: Option<String>,
    read: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTest {
    language: String,
    layouts: Option<RawField<Vec<String>>>,
    font: Option<RawField<String>>,
    word_count: Option<RawField<i64>>,
    speed_wpm: Option<RawField<i64>>,
    duration_seconds: Option<RawField<i64>>,
    max_marks: Option<RawField<i64>>,
    qualification: Option<RawField<Qualification>>,
    scoring: String,
    error_policy: Option<RawField<String>>,
    backspace: Option<RawField<Backspace>>,
    mistake_weights: Option<RawField<MistakeWeights>>,
    #[serde(default)]
    guide_allowed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCycle {
    advertisement: String,
    label: String,
    #[serde(default)]
    posts: Vec<RawPost>,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPost {
    name: String,
    typing_required: Option<RawField<bool>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClaim {
    claim: String,
    source: String,
    url: Option<String>,
}

/// Parse and validate one profile document. `file` names it in errors.
///
/// # Errors
///
/// [`ProfileError::UnsupportedSchema`] for another schema version (callers
/// skip such files), any other variant for invalid data.
pub fn parse_profile(file: &str, text: &str) -> Result<ExamProfile, ProfileError> {
    let header = crate::parse_header_lenient(text).map_err(|e| ProfileError::Toml {
        file: file.to_owned(),
        message: e,
    })?;
    if header != PROFILE_SCHEMA_VERSION {
        return Err(ProfileError::UnsupportedSchema {
            file: file.to_owned(),
            found: header,
        });
    }
    let raw: RawProfile = toml::from_str(text).map_err(|e| ProfileError::Toml {
        file: file.to_owned(),
        message: e.to_string(),
    })?;
    build(raw)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

fn build(raw: RawProfile) -> Result<ExamProfile, ProfileError> {
    if !valid_id(&raw.id) {
        return Err(ProfileError::InvalidId { id: raw.id });
    }
    let p = raw.id.clone();
    let invalid = |field: &str, detail: String| ProfileError::InvalidValue {
        profile: p.clone(),
        field: field.to_owned(),
        detail,
    };
    if raw.profile_version == 0 {
        return Err(invalid("profile_version", "must be 1 or more".to_owned()));
    }
    if raw.display_name.trim().is_empty() || raw.body.trim().is_empty() {
        return Err(invalid(
            "display_name",
            "display_name and body must not be empty".to_owned(),
        ));
    }

    // Sources first: fields cite them.
    let mut sources: Vec<Source> = Vec::with_capacity(raw.sources.len());
    for s in raw.sources {
        if s.id.trim().is_empty() || sources.iter().any(|x| x.id == s.id) {
            return Err(invalid(
                "sources",
                format!("source id '{}' is empty or repeated", s.id),
            ));
        }
        let kind = SourceKind::parse(&s.kind)
            .ok_or_else(|| invalid("sources", format!("unknown source kind '{}'", s.kind)))?;
        sources.push(Source {
            id: s.id,
            kind,
            title: s.title,
            url: s.url,
            read: s.read,
        });
    }
    let ids: Vec<String> = sources.iter().map(|s| s.id.clone()).collect();
    // A Verified value must come from a primary document that was read.
    let check_verified = |field: &str, cited: &[String]| -> Result<(), ProfileError> {
        for id in cited {
            if let Some(s) = sources.iter().find(|s| &s.id == id) {
                if !s.kind.can_verify() || !s.read {
                    return Err(invalid(
                        field,
                        format!(
                            "verified from '{id}', which is not a primary document that was read"
                        ),
                    ));
                }
            }
        }
        Ok(())
    };

    for id in &raw.notice.sources {
        if !ids.contains(id) {
            return Err(ProfileError::UnknownSource {
                profile: p.clone(),
                field: "notice".to_owned(),
                source_id: id.clone(),
            });
        }
    }
    let notice = Notice {
        title: raw.notice.title,
        reference: raw.notice.reference,
        source_ids: raw.notice.sources,
        note: raw.notice.note,
    };

    if raw.tests.is_empty() {
        return Err(invalid(
            "tests",
            "a profile needs at least one test".to_owned(),
        ));
    }
    let mut tests: Vec<TypingTest> = Vec::with_capacity(raw.tests.len());
    for t in raw.tests {
        let language = Language::parse(&t.language).ok_or_else(|| {
            invalid(
                "tests.language",
                format!("unknown language '{}'", t.language),
            )
        })?;
        if tests.iter().any(|x| x.language == language) {
            return Err(invalid(
                "tests.language",
                format!("two {} tests", language.as_str()),
            ));
        }
        if raw.hindi_only && language == Language::English {
            return Err(invalid(
                "tests.language",
                "a Hindi-only profile has no English test".to_owned(),
            ));
        }
        let field = |f: &str| format!("tests.{}.{f}", language.as_str());

        let layouts = field_or_unknown(t.layouts, &p, &field("layouts"), &ids)?
            .try_map(|list| {
                if list.is_empty() {
                    return Err("an empty layout list".to_owned());
                }
                let mut out = Vec::with_capacity(list.len());
                for raw in list {
                    let l = LayoutId::parse(&raw).map_err(|e| e.to_string())?;
                    if Language::of_layout(l) != language {
                        return Err(format!(
                            "layout '{raw}' does not type {}",
                            language.as_str()
                        ));
                    }
                    if !out.contains(&l) {
                        out.push(l);
                    }
                }
                Ok(out)
            })
            .map_err(|d| invalid(&field("layouts"), d))?;
        let font = field_or_unknown(t.font, &p, &field("font"), &ids)?;
        let ranged = |f: &str, raw: Option<RawField<i64>>, lo: i64, hi: i64| {
            field_or_unknown(raw, &p, &field(f), &ids)?
                .try_map(|v| {
                    if (lo..=hi).contains(&v) {
                        u32::try_from(v).map_err(|e| e.to_string())
                    } else {
                        Err(format!("{v} is outside {lo}..={hi}"))
                    }
                })
                .map_err(|d| invalid(&field(f), d))
        };
        let word_count = ranged("word_count", t.word_count, 1, 5000)?;
        let speed_wpm = ranged("speed_wpm", t.speed_wpm, 1, 300)?;
        let duration_seconds = ranged(
            "duration_seconds",
            t.duration_seconds,
            i64::from(TimeLimit::MIN_SECONDS),
            i64::from(TimeLimit::MAX_SECONDS),
        )?;
        let max_marks = ranged("max_marks", t.max_marks, 1, 1000)?;
        let marks_cap = f64::from(max_marks.effective().copied().unwrap_or(1000));
        let qualification = field_or_unknown(t.qualification, &p, &field("qualification"), &ids)?
            .try_map(|q| {
                let v = q.threshold();
                let hi = match q {
                    Qualification::MinMarks(_) => marks_cap,
                    Qualification::MinAccuracy(_) => 100.0,
                    Qualification::MinCorrectWpm(_) | Qualification::MinNetWpm(_) => 300.0,
                };
                if v.is_finite() && (0.0..=hi).contains(&v) {
                    Ok(q)
                } else {
                    Err(format!("threshold {v} is outside 0..={hi}"))
                }
            })
            .map_err(|d| invalid(&field("qualification"), d))?;
        let error_policy = field_or_unknown(t.error_policy, &p, &field("error_policy"), &ids)?;
        let backspace = field_or_unknown(t.backspace, &p, &field("backspace"), &ids)?;
        let mistake_weights =
            field_or_unknown(t.mistake_weights, &p, &field("mistake_weights"), &ids)?
                .try_map(|w| {
                    let ok = |x: f64| x.is_finite() && (0.0..=10.0).contains(&x);
                    if ok(w.full) && ok(w.half) {
                        Ok(w)
                    } else {
                        Err("weights must be within 0..=10".to_owned())
                    }
                })
                .map_err(|d| invalid(&field("mistake_weights"), d))?;
        for (f, cited) in [
            ("layouts", layouts.source_ids()),
            ("font", font.source_ids()),
            ("word_count", word_count.source_ids()),
            ("speed_wpm", speed_wpm.source_ids()),
            ("duration_seconds", duration_seconds.source_ids()),
            ("max_marks", max_marks.source_ids()),
            ("qualification", qualification.source_ids()),
            ("error_policy", error_policy.source_ids()),
            ("backspace", backspace.source_ids()),
            ("mistake_weights", mistake_weights.source_ids()),
        ] {
            check_verified(&field(f), cited)?;
        }
        let scoring = PolicyKind::parse(&t.scoring)
            .ok_or_else(|| invalid(&field("scoring"), format!("unknown policy '{}'", t.scoring)))?;
        let test = TypingTest {
            language,
            layouts,
            font,
            word_count,
            speed_wpm,
            duration_seconds,
            max_marks,
            qualification,
            scoring,
            error_policy,
            backspace,
            mistake_weights,
            guide_allowed: t.guide_allowed,
        };
        crate::scoring::check_policy(&test).map_err(|d| ProfileError::PolicyMismatch {
            profile: p.clone(),
            test: language.as_str().to_owned(),
            detail: d,
        })?;
        tests.push(test);
    }

    let mut cycles = Vec::with_capacity(raw.cycles.len());
    for c in raw.cycles {
        let mut posts = Vec::with_capacity(c.posts.len());
        for post in c.posts {
            let f = format!("cycles.{}.{}", c.advertisement, post.name);
            let typing_required = field_or_unknown(post.typing_required, &p, &f, &ids)?;
            check_verified(&f, typing_required.source_ids())?;
            posts.push(Post {
                name: post.name,
                typing_required,
            });
        }
        cycles.push(Cycle {
            advertisement: c.advertisement,
            label: c.label,
            posts,
            note: c.note,
        });
    }

    let reference_claims = raw
        .reference_claims
        .into_iter()
        .map(|c| {
            if c.claim.trim().is_empty() || c.source.trim().is_empty() {
                Err(invalid(
                    "reference_claims",
                    "a claim needs its text and where it was seen".to_owned(),
                ))
            } else {
                Ok(ReferenceClaim {
                    claim: c.claim,
                    source: c.source,
                    url: c.url,
                })
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ExamProfile {
        id: raw.id,
        profile_version: raw.profile_version,
        schema_version: raw.schema_version,
        display_name: raw.display_name,
        body: raw.body,
        unofficial_practice: raw.unofficial_practice,
        hindi_only: raw.hindi_only,
        practice_notice: raw.practice_notice,
        notice,
        sources,
        tests,
        cycles,
        reference_claims,
    })
}
