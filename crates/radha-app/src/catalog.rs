//! Home catalogue filtering.
//!
//! The Home screen's search box (HOME-03) and the Exam Hub category filter
//! (HOME-08) decide which cards are shown. The *rule* for what matches lives here,
//! as plain functions over strings, so it is one tested definition rather than
//! expressions scattered through Slint (`RULES.MD` Rule 16: business logic stays
//! out of the UI layer). `ui.rs` does the thin glue that applies the result to the
//! Slint models.
//!
//! Matching rule: the query is split on whitespace; **every** term must appear
//! (case-insensitively) in **at least one** of the card's fields. So
//! "ssc skill" finds a card whose title contains "SSC" and whose tag contains
//! "Skill test", while "ssc zzz" finds nothing. An empty query matches everything.
//!
//! Devanagari needs no special handling: it has no letter case, so lowercasing is a
//! no-op for it and substring search is correct for whole akshars typed by the user.

/// Whether `fields` satisfy `query` under the matching rule above.
#[must_use]
pub fn matches_query(query: &str, fields: &[&str]) -> bool {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return true;
    }

    let haystack: Vec<String> = fields.iter().map(|field| field.to_lowercase()).collect();

    terms
        .iter()
        .all(|term| haystack.iter().any(|field| field.contains(term.as_str())))
}

/// A product card's count chip from what is really installed: "200
/// Exercises", or an honest "Drills coming soon" for a product whose
/// content has not been supplied (never the planned budget as if it were
/// there). `planned` is the card's catalogue chip ("100 Drills"), whose
/// noun is kept.
#[must_use]
pub fn count_chip(installed: usize, planned: &str) -> String {
    let noun = planned
        .split_whitespace()
        .last()
        .filter(|w| w.chars().all(char::is_alphabetic))
        .unwrap_or("Exercises");
    match installed {
        0 => format!("{noun} coming soon"),
        1 => format!("1 {}", noun.strip_suffix('s').unwrap_or(noun)),
        n => format!("{n} {noun}"),
    }
}

/// The Hindi-only exam card's chip: the installed Hindi pool size, which
/// every Mangal layout (InScript, GAIL, CBI) types. Never a planned figure.
#[must_use]
pub fn pool_chip(installed: usize) -> String {
    match installed {
        0 => "Passages coming soon".to_owned(),
        1 => "1 passage per layout".to_owned(),
        n => format!("{n} passages per layout"),
    }
}

/// Whether an entry in category `entry` is shown when `selected` is chosen.
///
/// `"all"` (and an empty selection) shows everything.
#[must_use]
pub fn category_matches(selected: &str, entry: &str) -> bool {
    selected.is_empty() || selected == "all" || selected == entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_chips_say_what_is_installed() {
        assert_eq!(count_chip(0, "100 Drills"), "Drills coming soon");
        assert_eq!(count_chip(100, "100 Exercises"), "100 Exercises");
        assert_eq!(count_chip(195, "200 Exercises"), "195 Exercises");
        assert_eq!(count_chip(1, "100 Drills"), "1 Drill");
        assert_eq!(count_chip(3, ""), "3 Exercises");
    }

    /// P09: the Home chips are derived from the installed data. With the
    /// shipped bundles, Track 03 reads 50 and the Hindi pool 500 per layout.
    #[test]
    fn chips_from_the_shipped_bundles_read_the_real_counts() {
        use radha_content::import::{find_bundle_file, parse_bundle_bytes};
        use radha_content::{ContentTarget, ExamPool, ProductId};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/exercises");
        let count = |t: ContentTarget| {
            let path = find_bundle_file(&dir, t).expect("bundle");
            let bytes = std::fs::read(&path).unwrap();
            parse_bundle_bytes(&path, &bytes).unwrap().exercises.len()
        };
        let products = include_str!("../ui/data/products.slint");
        assert!(
            products.contains("meta-1: \"50 Drills\""),
            "planned Track 03 chip"
        );
        assert_eq!(
            count_chip(
                count(ContentTarget::Product(ProductId::NumberPunctuation)),
                "50 Drills"
            ),
            "50 Drills"
        );
        assert_eq!(
            pool_chip(count(ContentTarget::Pool(ExamPool::Hindi))),
            "500 passages per layout"
        );
    }

    #[test]
    fn pool_chip_says_what_is_installed() {
        assert_eq!(pool_chip(500), "500 passages per layout");
        assert_eq!(pool_chip(1), "1 passage per layout");
        assert_eq!(pool_chip(0), "Passages coming soon");
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(matches_query("", &["anything"]));
        assert!(matches_query("   ", &[]));
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(matches_query("PATNA", &["Patna High Court — COT 2026"]));
        assert!(matches_query("patna", &["PATNA HIGH COURT"]));
    }

    #[test]
    fn every_term_must_match_somewhere() {
        let fields = [
            "SSC CHSL — LDC / JSA",
            "Skill test",
            "Staff Selection Commission",
        ];
        assert!(matches_query("ssc skill", &fields));
        assert!(matches_query("selection chsl", &fields));
        assert!(!matches_query("ssc zzz", &fields));
    }

    #[test]
    fn terms_may_match_different_fields() {
        // "bihar" is only in the organisation, "deo" only in the title.
        assert!(matches_query(
            "bihar deo",
            &[
                "BELTRON Bihar — DEO",
                "Bihar State Electronics Development Corp."
            ]
        ));
        assert!(matches_query(
            "electronics deo",
            &["BELTRON — DEO", "Bihar State Electronics"]
        ));
    }

    #[test]
    fn no_fields_means_no_match_for_a_non_empty_query() {
        assert!(!matches_query("ssc", &[]));
    }

    #[test]
    fn devanagari_substrings_match() {
        assert!(matches_query("गेल", &["Remington GAIL", "गेल लेआउट अभ्यास"]));
        assert!(!matches_query("कृति", &["Remington GAIL", "गेल लेआउट अभ्यास"]));
    }

    #[test]
    fn extra_whitespace_between_terms_is_ignored() {
        assert!(matches_query("  ssc    chsl  ", &["SSC CHSL"]));
    }

    #[test]
    fn category_all_and_empty_show_everything() {
        assert!(category_matches("all", "court"));
        assert!(category_matches("", "state"));
    }

    #[test]
    fn category_must_equal_the_entry_otherwise() {
        assert!(category_matches("court", "court"));
        assert!(!category_matches("court", "state"));
        assert!(!category_matches("central", "court"));
    }
}
