//! `Field<T>`: an exam rule value together with how far it can be trusted
//! (RULES.MD Rule 32, EXAM-MODE-AND-GOLU-REUSE-PLAN.md Section 5.2).

use serde::Deserialize;

use crate::ProfileError;

/// How a rule value is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// Read in a primary notification; carries its source ids.
    Verified,
    /// Stated by Golu; the primary document is still pending.
    UserAsserted,
    /// Not verified: the app may run with a default, labelled unverified.
    Configurable,
    /// No value at all.
    Unknown,
}

impl Status {
    /// The TOML spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Verified => "verified",
            Status::UserAsserted => "user_asserted",
            Status::Configurable => "configurable",
            Status::Unknown => "unknown",
        }
    }

    /// Short on-screen label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Status::Verified => "verified",
            Status::UserAsserted => "user-asserted",
            Status::Configurable => "configurable, unverified",
            Status::Unknown => "unknown",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "verified" => Some(Status::Verified),
            "user_asserted" => Some(Status::UserAsserted),
            "configurable" => Some(Status::Configurable),
            "unknown" => Some(Status::Unknown),
            _ => None,
        }
    }
}

/// One exam rule value and its verification status.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Field<T> {
    /// From a primary source; `source_ids` name entries of the profile's
    /// `[[sources]]` list (never empty).
    Verified { value: T, source_ids: Vec<String> },
    /// Golu's statement; primary document pending.
    UserAsserted { value: T, note: Option<String> },
    /// Unverified. `default` is what the app runs with (`None`: the app's
    /// own practice default), always labelled as unverified.
    Configurable {
        default: Option<T>,
        note: Option<String>,
    },
    /// Nothing is known.
    #[default]
    Unknown,
}

impl<T> Field<T> {
    /// The status of this value.
    #[must_use]
    pub fn status(&self) -> Status {
        match self {
            Field::Verified { .. } => Status::Verified,
            Field::UserAsserted { .. } => Status::UserAsserted,
            Field::Configurable { .. } => Status::Configurable,
            Field::Unknown => Status::Unknown,
        }
    }

    /// A value rules may rely on: Verified or UserAsserted.
    #[must_use]
    pub fn known(&self) -> Option<&T> {
        match self {
            Field::Verified { value, .. } | Field::UserAsserted { value, .. } => Some(value),
            Field::Configurable { .. } | Field::Unknown => None,
        }
    }

    /// The value to run with: the known value, else the configurable default.
    #[must_use]
    pub fn effective(&self) -> Option<&T> {
        match self {
            Field::Configurable { default, .. } => default.as_ref(),
            other => other.known(),
        }
    }

    /// Whether the value is Verified.
    #[must_use]
    pub fn is_verified(&self) -> bool {
        matches!(self, Field::Verified { .. })
    }

    /// Source ids of a Verified value (empty otherwise).
    #[must_use]
    pub fn source_ids(&self) -> &[String] {
        match self {
            Field::Verified { source_ids, .. } => source_ids,
            _ => &[],
        }
    }

    /// The note of a UserAsserted or Configurable value.
    #[must_use]
    pub fn note(&self) -> Option<&str> {
        match self {
            Field::UserAsserted { note, .. } | Field::Configurable { note, .. } => note.as_deref(),
            _ => None,
        }
    }

    /// Convert the value, keeping the status.
    pub(crate) fn try_map<U>(
        self,
        mut f: impl FnMut(T) -> Result<U, String>,
    ) -> Result<Field<U>, String> {
        Ok(match self {
            Field::Verified { value, source_ids } => Field::Verified {
                value: f(value)?,
                source_ids,
            },
            Field::UserAsserted { value, note } => Field::UserAsserted {
                value: f(value)?,
                note,
            },
            Field::Configurable { default, note } => Field::Configurable {
                default: default.map(&mut f).transpose()?,
                note,
            },
            Field::Unknown => Field::Unknown,
        })
    }
}

/// A field as written in TOML:
/// `{ status = "verified", value = 600, sources = ["S-03"] }`,
/// `{ status = "user_asserted", value = 30, note = "…" }`,
/// `{ status = "configurable", default = "Arial", note = "…" }`,
/// `{ status = "unknown" }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawField<T> {
    status: String,
    value: Option<T>,
    default: Option<T>,
    #[serde(default)]
    sources: Vec<String>,
    note: Option<String>,
}

impl<T> RawField<T> {
    /// Check the shape for its status. `known_sources` are the profile's
    /// source ids; a Verified value must cite at least one of them.
    pub(crate) fn into_field(
        self,
        profile: &str,
        field: &str,
        known_sources: &[String],
    ) -> Result<Field<T>, ProfileError> {
        let shape = |detail: &str| ProfileError::FieldShape {
            profile: profile.to_owned(),
            field: field.to_owned(),
            detail: detail.to_owned(),
        };
        let status = Status::parse(&self.status).ok_or_else(|| ProfileError::InvalidStatus {
            profile: profile.to_owned(),
            field: field.to_owned(),
            status: self.status.clone(),
        })?;
        if status != Status::Verified && !self.sources.is_empty() {
            return Err(shape(
                "only a verified value cites sources; unverified claims go to reference_claims",
            ));
        }
        for id in &self.sources {
            if !known_sources.iter().any(|s| s == id) {
                return Err(ProfileError::UnknownSource {
                    profile: profile.to_owned(),
                    field: field.to_owned(),
                    source_id: id.clone(),
                });
            }
        }
        match status {
            Status::Verified | Status::UserAsserted => {
                if self.default.is_some() {
                    return Err(shape("a known value has `value`, not `default`"));
                }
                let value = self.value.ok_or_else(|| shape("missing `value`"))?;
                if status == Status::Verified {
                    if self.sources.is_empty() {
                        return Err(ProfileError::MissingSource {
                            profile: profile.to_owned(),
                            field: field.to_owned(),
                        });
                    }
                    Ok(Field::Verified {
                        value,
                        source_ids: self.sources,
                    })
                } else {
                    Ok(Field::UserAsserted {
                        value,
                        note: self.note,
                    })
                }
            }
            Status::Configurable => {
                if self.value.is_some() {
                    return Err(shape("a configurable field has `default`, not `value`"));
                }
                Ok(Field::Configurable {
                    default: self.default,
                    note: self.note,
                })
            }
            Status::Unknown => {
                if self.value.is_some() || self.default.is_some() {
                    return Err(shape("an unknown field carries no value"));
                }
                Ok(Field::Unknown)
            }
        }
    }
}

/// A missing TOML field is Unknown.
pub(crate) fn field_or_unknown<T>(
    raw: Option<RawField<T>>,
    profile: &str,
    field: &str,
    known_sources: &[String],
) -> Result<Field<T>, ProfileError> {
    match raw {
        Some(r) => r.into_field(profile, field, known_sources),
        None => Ok(Field::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(
        status: &str,
        value: Option<u32>,
        default: Option<u32>,
        sources: &[&str],
    ) -> RawField<u32> {
        RawField {
            status: status.to_owned(),
            value,
            default,
            sources: sources.iter().map(|s| (*s).to_owned()).collect(),
            note: None,
        }
    }

    #[test]
    fn statuses_and_values() {
        let known = vec!["S-03".to_owned()];
        let v = raw("verified", Some(600), None, &["S-03"])
            .into_field("p", "d", &known)
            .unwrap();
        assert_eq!(v.status(), Status::Verified);
        assert_eq!(v.known(), Some(&600));
        assert_eq!(v.source_ids(), ["S-03".to_owned()]);
        let c = raw("configurable", None, Some(600), &[])
            .into_field("p", "d", &known)
            .unwrap();
        assert_eq!(c.known(), None, "a default is never a known rule");
        assert_eq!(c.effective(), Some(&600));
        let u = raw("unknown", None, None, &[])
            .into_field("p", "d", &known)
            .unwrap();
        assert_eq!(u, Field::Unknown);
        assert_eq!(u.effective(), None);
    }

    #[test]
    fn shapes_are_checked() {
        let known = vec!["S-03".to_owned()];
        let err = |r: RawField<u32>| r.into_field("p", "d", &known).unwrap_err();
        assert!(matches!(
            err(raw("verified", Some(1), None, &[])),
            ProfileError::MissingSource { .. }
        ));
        assert!(matches!(
            err(raw("verified", Some(1), None, &["S-99"])),
            ProfileError::UnknownSource { .. }
        ));
        assert!(matches!(
            err(raw("maybe", Some(1), None, &[])),
            ProfileError::InvalidStatus { .. }
        ));
        assert!(matches!(
            err(raw("unknown", Some(1), None, &[])),
            ProfileError::FieldShape { .. }
        ));
        assert!(matches!(
            err(raw("configurable", Some(1), None, &[])),
            ProfileError::FieldShape { .. }
        ));
        assert!(matches!(
            err(raw("user_asserted", None, None, &[])),
            ProfileError::FieldShape { .. }
        ));
        assert!(matches!(
            err(raw("configurable", None, Some(1), &["S-03"])),
            ProfileError::FieldShape { .. }
        ));
    }
}
