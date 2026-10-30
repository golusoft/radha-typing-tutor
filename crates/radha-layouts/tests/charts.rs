//! Table vs reference chart, entry for entry (P03 "table-vs-chart 100%").
//!
//! Each `tests/charts/<layout>.json` is a transcription of Golu's reference
//! documents and code, one sub-reference per source region, with the value
//! each one gives every slot. The rules:
//!
//! - R1 the chart and the layout table have exactly the same (key, layer)
//!   slots, and each slot's resolved value equals the table's output (Kruti
//!   Dev: meaning, plus the ASCII identity and the kind from the mapping doc);
//! - R2 no invented mapping: some sub-reference states the resolved value;
//! - R3 the stored flags equal the flags recomputed from the references, and
//!   an `unverified` slot quotes its reference's own flag with a line locator;
//! - R4 fingers: every layout's `finger_for` equals Golu's `FingerMap`;
//! - R5 the summary counts equal the recomputed counts.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use radha_layouts::{Finger, Hand, Keystroke, Layer, PhysicalKey, TokenKind};
use serde_json::Value;

use common::{is_sha256, layout, tests_dir, type_keys};

fn chart(slug: &str) -> Value {
    let path = tests_dir().join("charts").join(format!("{slug}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

fn layer(s: &str) -> Layer {
    match s {
        "base" => Layer::Base,
        "shift" => Layer::Shift,
        "altgr" => Layer::AltGr,
        "shiftaltgr" => Layer::ShiftAltGr,
        other => panic!("unknown layer {other}"),
    }
}

fn kind_name(k: TokenKind) -> &'static str {
    match k {
        TokenKind::Literal => "literal",
        TokenKind::Consonant => "consonant",
        TokenKind::Half => "half",
        TokenKind::Conjunct => "conjunct",
        TokenKind::Vowel => "vowel",
        TokenKind::Matra => "matra",
        TokenKind::Preposed => "preposed",
        TokenKind::Completer => "completer",
        TokenKind::Reph => "reph",
        TokenKind::Rakar => "rakar",
        TokenKind::Halant => "halant",
        TokenKind::Nukta => "nukta",
        TokenKind::Digit => "digit",
        TokenKind::Punctuation => "punctuation",
    }
}

fn str_of<'a>(v: &'a Value, field: &str) -> &'a str {
    v.get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing {field} in {v}"))
}

/// Check one layout; returns the summary line and the problems.
fn check(slug: &str) -> (String, Vec<String>) {
    let c = chart(slug);
    let l = layout(slug);
    let mut problems = Vec::new();
    assert_eq!(c["schema_version"], 1);
    assert_eq!(str_of(&c, "layout"), slug);
    let roles: BTreeMap<String, String> = c["references"]
        .as_array()
        .expect("references")
        .iter()
        .map(|r| {
            assert!(is_sha256(str_of(r, "sha256")), "{slug}: bad sha256 in {r}");
            assert!(!str_of(r, "path").is_empty());
            (str_of(r, "id").to_owned(), str_of(r, "role").to_owned())
        })
        .collect();
    let value_field = str_of(&c, "value_field");

    let entries = c["entries"].as_array().expect("entries");
    let mut chart_slots = BTreeSet::new();
    let (mut no_flag, mut conflict_n, mut unverified_n, mut missing_n) = (0, 0, 0, 0);
    for e in entries {
        let key_s = str_of(e, "key");
        let key = PhysicalKey::from_base(key_s.chars().next().unwrap_or(' '))
            .unwrap_or_else(|| panic!("{slug}: bad key {key_s}"));
        let lay = layer(str_of(e, "layer"));
        chart_slots.insert((key, lay));
        let value = str_of(e, "value");
        let name = format!("{slug} {key_s} {}", str_of(e, "layer"));

        // R1.
        match l.table().get(Keystroke::new(key, lay)) {
            None => problems.push(format!("{name}: in chart, missing from table")),
            Some(entry) => {
                let actual = if value_field == "meaning" {
                    entry.meaning.unwrap_or("")
                } else {
                    entry.output
                };
                if actual != value {
                    problems.push(format!("{name}: table {actual:?} != chart {value:?}"));
                }
                if let Some(out) = e.get("output").and_then(Value::as_str) {
                    if entry.output != out {
                        problems.push(format!("{name}: output {:?} != {out:?}", entry.output));
                    }
                }
                if let Some(kind) = e.get("kind").and_then(Value::as_str) {
                    if kind_name(entry.kind) != kind {
                        problems.push(format!("{name}: kind {:?} != {kind}", entry.kind));
                    }
                }
            }
        }

        // R2 and R3.
        let refs = e["refs"].as_object().expect("refs");
        let stated: Vec<&str> = refs.values().filter_map(Value::as_str).collect();
        if !stated.contains(&value) {
            problems.push(format!("{name}: no reference states {value:?} (invented?)"));
        }
        let mut conflict = stated.iter().any(|v| *v != value);
        if let Some(ascii) = e.get("ascii_refs").and_then(|a| a.as_object()) {
            let out = str_of(e, "output");
            conflict |= ascii.values().filter_map(Value::as_str).any(|v| v != out);
        }
        let primary_missing = slug != "qwerty"
            && roles
                .iter()
                .filter(|(_, r)| r.as_str() == "primary")
                .all(|(id, _)| refs.get(id).is_none_or(Value::is_null));
        let flags = &e["flags"];
        if flags["conflict"].as_bool() != Some(conflict) {
            problems.push(format!("{name}: conflict flag should be {conflict}"));
        }
        if flags["primary_missing"].as_bool() != Some(primary_missing) {
            problems.push(format!(
                "{name}: primary_missing flag should be {primary_missing}"
            ));
        }
        let unverified = flags["unverified"].as_bool() == Some(true);
        let evidence = str_of(e, "evidence");
        if unverified {
            let quoted = evidence.starts_with('"') && evidence[1..].contains('"');
            let located = evidence
                .split(|ch: char| !ch.is_ascii_alphanumeric())
                .any(|w| {
                    w.len() > 1 && w.starts_with('L') && w[1..].chars().all(|d| d.is_ascii_digit())
                });
            if !quoted || !located {
                problems.push(format!("{name}: unverified evidence needs a quote and Lnn"));
            }
        }
        if (conflict || unverified || primary_missing) && evidence.is_empty() && !primary_missing {
            problems.push(format!("{name}: flagged slot without evidence"));
        }
        if conflict {
            conflict_n += 1;
        }
        if unverified {
            unverified_n += 1;
        }
        if primary_missing {
            missing_n += 1;
        }
        if !conflict && !unverified && !primary_missing {
            no_flag += 1;
        }
    }
    let table_slots: BTreeSet<(PhysicalKey, Layer)> = l
        .table()
        .entries()
        .iter()
        .map(|e| (e.key, e.layer))
        .collect();
    for extra in table_slots.difference(&chart_slots) {
        problems.push(format!("{slug} {extra:?}: in table, missing from chart"));
    }

    // R5.
    let s = &c["summary"];
    let n = entries.len();
    for (field, got) in [
        ("entries", n),
        ("no_flag", no_flag),
        ("conflict", conflict_n),
        ("unverified", unverified_n),
        ("primary_missing", missing_n),
    ] {
        if s[field].as_u64() != Some(got as u64) {
            problems.push(format!("{slug}: summary {field} {} != {got}", s[field]));
        }
    }
    let matched = n - problems.iter().filter(|p| p.contains(" table ")).count();
    let line = format!(
        "{slug}: {matched}/{n} slots match chart (no flag {no_flag}; conflict {conflict_n}, unverified {unverified_n}, primary_missing {missing_n})"
    );
    (line, problems)
}

#[test]
fn every_layout_table_matches_its_reference_chart_entry_for_entry() {
    let mut all = Vec::new();
    for slug in ["qwerty", "gail", "cbi", "krutidev"] {
        let (line, problems) = check(slug);
        println!("{line}");
        all.extend(problems);
    }
    assert!(all.is_empty(), "chart problems:\n{}", all.join("\n"));
}

#[test]
fn the_pinned_flag_counts_are_what_the_design_derived() {
    let counts = |slug: &str| {
        let s = &chart(slug)["summary"];
        [
            s["entries"].clone(),
            s["conflict"].clone(),
            s["unverified"].clone(),
            s["primary_missing"].clone(),
        ]
        .map(|v| v.as_u64().unwrap_or(u64::MAX))
    };
    // Book-driven (book-test-changes.md): the keyboard books are a verified
    // reference for every slot, so nothing stays unverified; GAIL gains the
    // book's AltGr layer (no Golu primary for 16 of its slots), CBI loses
    // its AltGr layer and takes ASCII digits, Kruti Dev loses Shift+4.
    assert_eq!(counts("qwerty"), [94, 0, 0, 0]);
    assert_eq!(counts("gail"), [112, 20, 0, 18]);
    assert_eq!(counts("cbi"), [94, 15, 0, 0]);
    assert_eq!(counts("krutidev"), [93, 6, 0, 0]);
    let kd = chart("krutidev");
    let codes: Vec<u64> = kd["alt_codes"]
        .as_array()
        .expect("alt_codes")
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .collect();
    assert_eq!(codes.len(), 58);
    assert_eq!(codes.first(), Some(&131));
    assert_eq!(codes.last(), Some(&244));
    let mine: Vec<u64> = radha_layouts::krutidev::alt_codes()
        .iter()
        .map(|a| u64::from(a.code))
        .collect();
    assert_eq!(codes, mine, "the table holds exactly the book's codes");
    assert_eq!(
        kd["unresolved"],
        serde_json::json!(["\u{0950}", "\u{20B9}", "*"])
    );
}

#[test]
fn every_layout_adopts_the_golu_finger_map_for_every_key() {
    let c = chart("qwerty");
    let fingers = c["fingers"].as_array().expect("fingers");
    assert_eq!(fingers.len(), 47);
    for slug in ["qwerty", "gail", "cbi", "krutidev", "inscript"] {
        let l = layout(slug);
        for f in fingers {
            let key =
                PhysicalKey::from_base(str_of(f, "key").chars().next().unwrap_or(' ')).unwrap();
            let hand = match str_of(f, "hand") {
                "left" => Hand::Left,
                _ => Hand::Right,
            };
            let finger = match str_of(f, "finger") {
                "pinky" => Finger::Pinky,
                "ring" => Finger::Ring,
                "middle" => Finger::Middle,
                "index" => Finger::Index,
                other => panic!("finger {other}"),
            };
            assert_eq!(l.finger_for(key), (hand, finger), "{slug} {key:?}");
        }
    }
}

#[test]
fn the_gail_chart_statement_about_shift_a_holds() {
    // gail-keyboard-diagram.md: "A and K both give the aa-matra, which also
    // completes a preceding half-form to full."
    let gail = layout("gail");
    assert_eq!(type_keys(gail, "[A", None).0, "ख");
    assert_eq!(type_keys(gail, "[k", None).0, "ख");
    assert_eq!(type_keys(gail, "dA", None).0, "का");
}
