//! Loading profiles: embedded TOML files, versioned by `profile_id@version`
//! (EXAM-MODE-AND-GOLU-REUSE-PLAN.md Section 5.7).

use crate::profile::{parse_profile, ExamProfile};
use crate::ProfileError;

/// Every profile shipped in the exe, as `(file name, text)`, in Exam Hub
/// order.
pub const EMBEDDED: &[(&str, &str)] = &[
    (
        "all-in-one-mangal.toml",
        include_str!("../profiles/all-in-one-mangal.toml"),
    ),
    ("ssc-chsl.toml", include_str!("../profiles/ssc-chsl.toml")),
    ("rrb-ntpc.toml", include_str!("../profiles/rrb-ntpc.toml")),
    (
        "ssc-cgl-dest.toml",
        include_str!("../profiles/ssc-cgl-dest.toml"),
    ),
    ("aiims-cre.toml", include_str!("../profiles/aiims-cre.toml")),
    ("crpf-hcm.toml", include_str!("../profiles/crpf-hcm.toml")),
    ("capf-hcm.toml", include_str!("../profiles/capf-hcm.toml")),
    ("epfo-ssa.toml", include_str!("../profiles/epfo-ssa.toml")),
    ("upsssc-ja.toml", include_str!("../profiles/upsssc-ja.toml")),
    ("mp-cpct.toml", include_str!("../profiles/mp-cpct.toml")),
    (
        "rajasthan-rssb.toml",
        include_str!("../profiles/rajasthan-rssb.toml"),
    ),
    (
        "delhi-police-hcm.toml",
        include_str!("../profiles/delhi-police-hcm.toml"),
    ),
    ("dsssb.toml", include_str!("../profiles/dsssb.toml")),
    (
        "beltron-deo.toml",
        include_str!("../profiles/beltron-deo.toml"),
    ),
    (
        "bssc-inter.toml",
        include_str!("../profiles/bssc-inter.toml"),
    ),
    (
        "up-police-asi.toml",
        include_str!("../profiles/up-police-asi.toml"),
    ),
    ("uppcl.toml", include_str!("../profiles/uppcl.toml")),
    (
        "patna-hc-cot-2026.toml",
        include_str!("../profiles/patna-hc-cot-2026.toml"),
    ),
    (
        "bihar-civil-court-clerk-01-2022.toml",
        include_str!("../profiles/bihar-civil-court-clerk-01-2022.toml"),
    ),
    (
        "rajasthan-hc.toml",
        include_str!("../profiles/rajasthan-hc.toml"),
    ),
    (
        "allahabad-hc.toml",
        include_str!("../profiles/allahabad-hc.toml"),
    ),
    ("mp-hc.toml", include_str!("../profiles/mp-hc.toml")),
];

/// Split `id@version`.
#[must_use]
pub fn parse_key(key: &str) -> Option<(&str, u32)> {
    let (id, v) = key.split_once('@')?;
    let v: u32 = v.parse().ok()?;
    (!id.is_empty() && v > 0).then_some((id, v))
}

/// Loaded profiles; several versions of one id may coexist.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Registry {
    profiles: Vec<ExamProfile>,
}

/// The result of loading a set of files: never a panic, never a partial
/// profile.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub registry: Registry,
    /// Files of another schema version: skipped (logged by the caller).
    pub skipped: Vec<ProfileError>,
    /// Invalid files and duplicate keys: left out.
    pub errors: Vec<ProfileError>,
}

impl Registry {
    /// Load the profiles embedded in the exe.
    #[must_use]
    pub fn embedded() -> LoadReport {
        Self::load(EMBEDDED)
    }

    /// Load `(file name, text)` documents.
    #[must_use]
    pub fn load(files: &[(&str, &str)]) -> LoadReport {
        let mut report = LoadReport::default();
        for (file, text) in files {
            match parse_profile(file, text) {
                Ok(p) => {
                    if report.registry.get(&p.id, p.profile_version).is_some() {
                        report
                            .errors
                            .push(ProfileError::DuplicateProfile { key: p.key() });
                    } else {
                        report.registry.profiles.push(p);
                    }
                }
                Err(e @ ProfileError::UnsupportedSchema { .. }) => report.skipped.push(e),
                Err(e) => report.errors.push(e),
            }
        }
        report
    }

    /// Number of profiles (all versions).
    #[must_use]
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Whether nothing loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Every profile, in load order.
    #[must_use]
    pub fn all(&self) -> &[ExamProfile] {
        &self.profiles
    }

    /// One exact version.
    #[must_use]
    pub fn get(&self, id: &str, version: u32) -> Option<&ExamProfile> {
        self.profiles
            .iter()
            .find(|p| p.id == id && p.profile_version == version)
    }

    /// The newest version of `id`.
    #[must_use]
    pub fn latest(&self, id: &str) -> Option<&ExamProfile> {
        self.profiles
            .iter()
            .filter(|p| p.id == id)
            .max_by_key(|p| p.profile_version)
    }

    /// The profile a stored `id@version` key names.
    #[must_use]
    pub fn by_key(&self, key: &str) -> Option<&ExamProfile> {
        let (id, v) = parse_key(key)?;
        self.get(id, v)
    }

    /// Distinct ids, in load order.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for p in &self.profiles {
            if !out.contains(&p.id.as_str()) {
                out.push(&p.id);
            }
        }
        out
    }
}
