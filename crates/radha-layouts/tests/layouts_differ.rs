//! GAIL != CBI != Kruti Dev: the tables are different, and they differ in
//! exactly the documented places (so a copied table, or an undocumented
//! drift, fails).

mod common;

use std::collections::{BTreeMap, BTreeSet};

use radha_layouts::krutidev::to_legacy;
use radha_layouts::{KeyboardLayout, Layer, PhysicalKey, TokenKind};
use serde_json::Value;

use common::{layout, tests_dir, type_keys};

type Slot = (char, Layer);

/// (key, layer) -> (Devanagari text, kind). Kruti Dev uses the meaning.
fn slots(l: &'static dyn KeyboardLayout) -> BTreeMap<Slot, (&'static str, TokenKind)> {
    l.table()
        .entries()
        .iter()
        .map(|e| ((e.key.base_char(), e.layer), (e.text(), e.kind)))
        .collect()
}

fn layer(s: &str) -> Layer {
    match s {
        "base" => Layer::Base,
        "shift" => Layer::Shift,
        _ => Layer::AltGr,
    }
}

#[test]
fn the_three_hindi_tables_are_pairwise_different() {
    let (g, c, k) = (
        slots(layout("gail")),
        slots(layout("cbi")),
        slots(layout("krutidev")),
    );
    assert_ne!(g, c);
    assert_ne!(c, k);
    assert_ne!(g, k);
}

#[test]
fn gail_and_cbi_differ_in_exactly_the_documented_slots() {
    let (g, c) = (slots(layout("gail")), slots(layout("cbi")));
    let all: BTreeSet<Slot> = g.keys().chain(c.keys()).copied().collect();
    let differing: BTreeSet<Slot> = all.into_iter().filter(|s| g.get(s) != c.get(s)).collect();
    // Books 01 and 03: the Normal/Shift slots are the same key for key; the
    // short-i key differs in kind (GAIL pre-base, CBI post-base) and only
    // GAIL has an AltGr layer.
    let mut expected: BTreeSet<Slot> = BTreeSet::new();
    expected.insert(('f', Layer::Base)); // same ि, Preposed vs Matra
    for (slot, _) in g
        .iter()
        .filter(|(s, _)| matches!(s.1, Layer::AltGr | Layer::ShiftAltGr))
    {
        expected.insert(*slot); // GAIL AltGr layer (GAIL-MAP-108..125)
    }
    assert_eq!(expected.len(), 1 + 18);
    assert_eq!(differing, expected);
    assert_eq!(g[&('f', Layer::Base)], ("\u{093F}", TokenKind::Preposed));
    assert_eq!(c[&('f', Layer::Base)], ("\u{093F}", TokenKind::Matra));
}

#[test]
fn cbi_and_krutidev_differ_in_exactly_the_mapping_doc_slots() {
    let path = tests_dir()
        .join("fixtures")
        .join("krutidev_vs_cbi_divergences.json");
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json");
    assert!(fixture["source"]["sha256"]
        .as_str()
        .is_some_and(|s| s.len() == 64));
    let (c, k) = (slots(layout("cbi")), slots(layout("krutidev")));
    let mut pinned: BTreeSet<Slot> = BTreeSet::new();
    for s in fixture["slots"].as_array().expect("slots") {
        let key = s["key"]
            .as_str()
            .and_then(|x| x.chars().next())
            .expect("key");
        let slot = (key, layer(s["layer"].as_str().unwrap_or("")));
        assert_eq!(
            k.get(&slot).map(|v| v.0),
            s["krutidev"].as_str(),
            "{slot:?}"
        );
        assert_eq!(c.get(&slot).map(|v| v.0), s["cbi"].as_str(), "{slot:?}");
        pinned.insert(slot);
    }
    // Base and Shift slots whose drawn text differs.
    let text_differs: BTreeSet<Slot> = k
        .iter()
        .filter(|(s, v)| c.get(s).map(|x| x.0) != Some(v.0))
        .map(|(s, _)| *s)
        .collect();
    assert_eq!(text_differs, pinned);
    // Same text, different kind: only the short-i key.
    let kind_differs: BTreeSet<Slot> = k
        .iter()
        .filter(|(s, v)| c.get(s).is_some_and(|x| x.0 == v.0 && x.1 != v.1))
        .map(|(s, _)| *s)
        .collect();
    assert_eq!(kind_differs, BTreeSet::from([('f', Layer::Base)]));
    // 51 of 52 letter keys draw the same glyph (mapping doc L39).
    let letters_same = PhysicalKey::all()
        .filter(|key| key.is_letter())
        .flat_map(|key| {
            [
                (key.base_char(), Layer::Base),
                (key.base_char(), Layer::Shift),
            ]
        })
        .filter(|s| c.get(s).map(|v| v.0) == k.get(s).map(|v| v.0))
        .count();
    assert_eq!(letters_same, 51);
    // Neither Kruti Dev nor CBI (book 03) has an AltGr layer.
    assert!(k.keys().all(|s| s.1 != Layer::AltGr));
    assert_eq!(c.keys().filter(|s| s.1 == Layer::AltGr).count(), 0);
}

#[test]
fn shift_a_is_the_aa_completer_on_gail_and_cbi_and_the_danda_on_krutidev() {
    let (g, c, k) = (
        slots(layout("gail")),
        slots(layout("cbi")),
        slots(layout("krutidev")),
    );
    let shift_a = ('a', Layer::Shift);
    assert_eq!(g[&shift_a], ("\u{093E}", TokenKind::Completer));
    assert_eq!(c[&shift_a], ("\u{093E}", TokenKind::Completer));
    assert_eq!(k[&shift_a], ("।", TokenKind::Punctuation));
    // The converter never confuses them.
    assert_eq!(to_legacy("।").legacy, "A");
    assert_eq!(to_legacy("\u{093E}").legacy, "k");
    assert_eq!(type_keys(layout("cbi"), "dA", None).0, "का");
    assert_eq!(type_keys(layout("krutidev"), "dA", None).0, "dA");
}

#[test]
fn the_same_keystrokes_give_three_answers() {
    assert_eq!(type_keys(layout("gail"), "fd", None).0, "कि");
    assert_eq!(type_keys(layout("cbi"), "fd", None).0, "\u{093F}क");
    assert_eq!(type_keys(layout("krutidev"), "fd", None).0, "fd");
}

#[test]
fn key_order_contract() {
    // golu_key_order_contract: GAIL and CBI both have a reph key; only GAIL
    // pre-poses the short i; Kruti Dev's stream is never reordered.
    for slug in ["gail", "cbi"] {
        assert!(layout(slug)
            .table()
            .entries()
            .iter()
            .any(|e| e.kind == TokenKind::Reph));
    }
    let preposed = |slug: &str| {
        layout(slug)
            .table()
            .entries()
            .iter()
            .any(|e| e.kind == TokenKind::Preposed)
    };
    assert!(preposed("gail"));
    assert!(!preposed("cbi"));
    assert_eq!(type_keys(layout("krutidev"), "dk;Z", None).0, "dk;Z");
}
