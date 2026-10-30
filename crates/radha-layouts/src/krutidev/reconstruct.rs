//! Kruti Dev legacy key stream -> Unicode Hindi.
//!
//! A separate code path from [`super::convert`], re-derived from the key
//! table and the composition rules alone (Golu `krutidev-convert-core.js`
//! `reconstruct`), so a round trip catches converter bugs instead of
//! repeating them.

use radha_core::akshar::normalize;

use super::alt_codes::ALT_CODES;
use super::convert::{alt_kind, AltKind};
use super::rules::RULES;
use super::table::TABLE;
use crate::script::VIRAMA;
use crate::table::TokenKind;

/// The result of [`to_unicode`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reconstruction {
    /// The Unicode text (NFC).
    pub text: String,
    /// Characters with no meaning, as `(char index, char)`; emitted verbatim.
    pub unknown: Vec<(usize, char)>,
}

#[derive(Debug, Clone)]
struct Tok {
    u: String,
    k: TokenKind,
}

fn is_matra_kind(k: TokenKind) -> bool {
    matches!(k, TokenKind::Matra | TokenKind::Preposed)
}

/// Non-signs in typed order, then signs, then ligatures, then NFC.
fn compose(cluster: &[Tok]) -> String {
    let mut out = String::new();
    for t in cluster.iter().filter(|t| !is_matra_kind(t.k)) {
        out.push_str(&t.u);
    }
    for t in cluster.iter().filter(|t| is_matra_kind(t.k)) {
        out.push_str(&t.u);
    }
    for lig in RULES.ligatures() {
        let parts = lig.joined();
        if out.contains(&parts) {
            out = out.replace(&parts, lig.result);
        }
    }
    normalize(&out)
}

/// Convert a Kruti Dev key stream to Unicode.
#[must_use]
pub fn to_unicode(legacy: &str) -> Reconstruction {
    let mut out = String::new();
    let mut unknown = Vec::new();
    let mut cluster: Vec<Tok> = Vec::new();
    let flush = |out: &mut String, cluster: &mut Vec<Tok>| {
        if !cluster.is_empty() {
            out.push_str(&compose(cluster));
            cluster.clear();
        }
    };
    let opens = |k: TokenKind| matches!(k, TokenKind::Halant | TokenKind::Half);

    for (idx, ch) in legacy.chars().enumerate() {
        if let Some(alt) = ALT_CODES.iter().find(|a| a.ansi == ch) {
            match alt_kind(alt.meaning) {
                AltKind::Punct => {
                    flush(&mut out, &mut cluster);
                    out.push_str(alt.meaning);
                }
                // A glyph that starts with a virama (`ª` = ्र, `Ó` = ्य)
                // belongs to the base, in typed order; other signs are
                // marks.
                AltKind::Matra => cluster.push(Tok {
                    u: alt.meaning.to_owned(),
                    k: if alt.meaning.starts_with(VIRAMA) {
                        TokenKind::Rakar
                    } else {
                        TokenKind::Matra
                    },
                }),
                AltKind::Consonant => {
                    let open = cluster.last().is_some_and(|t| opens(t.k));
                    let only_pre =
                        !cluster.is_empty() && cluster.iter().all(|t| t.k == TokenKind::Preposed);
                    if !open && !only_pre {
                        flush(&mut out, &mut cluster);
                    }
                    cluster.push(Tok {
                        u: alt.meaning.to_owned(),
                        k: TokenKind::Consonant,
                    });
                }
            }
            continue;
        }
        let mut buf = [0u8; 4];
        let key: &str = ch.encode_utf8(&mut buf);
        let Some(entry) = TABLE.iter().find(|e| e.output == key) else {
            flush(&mut out, &mut cluster);
            out.push(ch);
            if !ch.is_whitespace() {
                unknown.push((idx, ch));
            }
            continue;
        };
        let meaning = entry.text();
        match entry.kind {
            TokenKind::Completer => {
                if let Some(last) = cluster.last_mut() {
                    if last.k == TokenKind::Half && last.u.ends_with(VIRAMA) {
                        last.u.pop();
                        last.k = TokenKind::Consonant;
                        continue;
                    }
                }
                cluster.push(Tok {
                    u: meaning.to_owned(),
                    k: TokenKind::Matra,
                });
            }
            TokenKind::Reph => {
                let tok = Tok {
                    u: meaning.to_owned(),
                    k: TokenKind::Half,
                };
                if cluster.is_empty() || cluster.iter().any(|t| t.k == TokenKind::Vowel) {
                    cluster.push(tok);
                } else {
                    cluster.insert(0, tok);
                }
            }
            TokenKind::Punctuation | TokenKind::Digit | TokenKind::Literal => {
                flush(&mut out, &mut cluster);
                out.push_str(meaning);
            }
            TokenKind::Preposed => {
                flush(&mut out, &mut cluster);
                cluster.push(Tok {
                    u: meaning.to_owned(),
                    k: TokenKind::Preposed,
                });
            }
            TokenKind::Matra | TokenKind::Halant | TokenKind::Nukta | TokenKind::Rakar => {
                cluster.push(Tok {
                    u: meaning.to_owned(),
                    k: entry.kind,
                });
            }
            TokenKind::Consonant | TokenKind::Vowel | TokenKind::Conjunct | TokenKind::Half => {
                // A half form or bare halant leaves the cluster waiting for
                // its next consonant; a rakar does not.
                let awaiting = cluster.last().is_some_and(|t| opens(t.k));
                let only_pre =
                    !cluster.is_empty() && cluster.iter().all(|t| t.k == TokenKind::Preposed);
                if !awaiting && !only_pre {
                    flush(&mut out, &mut cluster);
                }
                cluster.push(Tok {
                    u: meaning.to_owned(),
                    k: entry.kind,
                });
            }
        }
    }
    flush(&mut out, &mut cluster);
    // Book 04: `AA` draws the double danda.
    if out.contains("।।") {
        out = out.replace("।।", "॥");
    }
    Reconstruction { text: out, unknown }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ligature_order_pins() {
        assert_eq!(to_unicode("vkSj").text, "और");
        assert_eq!(to_unicode("vkW").text, "ऑ");
        assert_eq!(to_unicode("dkW").text, "कॉ");
    }

    #[test]
    fn visual_order_is_undone() {
        assert_eq!(to_unicode("fganh").text, "हिंदी");
        assert_eq!(to_unicode("vFkZ").text, "अर्थ");
        assert_eq!(to_unicode("bZ").text, "ई");
        assert_eq!(to_unicode("fdz;k").text, "क्रिया");
    }

    #[test]
    fn unknown_characters_are_kept_and_listed() {
        // U+00E9 became Alt+0233 (न्न) in book 04, so the unknown sample is
        // U+00FC, which no Kruti Dev key or code produces.
        let r = to_unicode("d\u{00FC} d");
        assert_eq!(r.text, "क\u{00FC} क");
        assert_eq!(r.unknown, [(1, '\u{00FC}')]);
    }
}
