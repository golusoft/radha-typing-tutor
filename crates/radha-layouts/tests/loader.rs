//! Loader and table-validation rules (S1-S3, P1-P3, K1-K9).

use radha_core::CoreError;
use radha_layouts::{
    load_layout, KeyEntry, Keystroke, Layer, LayoutError, LayoutId, LayoutTable, PhysicalKey,
    TableProfile, TokenKind,
};

fn key(c: char) -> PhysicalKey {
    PhysicalKey::from_base(c).expect("key")
}

/// Validate a one-entry (or more) synthetic table; the rule id on failure.
fn rule(profile: TableProfile, entries: &'static [KeyEntry]) -> Option<String> {
    LayoutTable::new(LayoutId::RemingtonGail, profile, entries)
        .validate()
        .err()
        .map(|LayoutError::InvalidTable { detail, .. }| {
            detail.split(' ').next().unwrap_or("").to_owned()
        })
}

fn leak(entries: Vec<KeyEntry>) -> &'static [KeyEntry] {
    Box::leak(entries.into_boxed_slice())
}

#[test]
fn every_layout_loads_and_reports_its_id() {
    for &id in LayoutId::all() {
        let l = load_layout(id).expect("built-in tables are valid");
        assert_eq!(l.id(), id);
        assert_eq!(l.table().id(), id);
    }
}

#[test]
fn structural_rules_reject_bad_tables() {
    let u = TableProfile::Unicode;
    let bad_key = KeyEntry::new(
        PhysicalKey::new_unchecked(b'A'),
        Layer::Base,
        "क",
        TokenKind::Consonant,
    );
    assert_eq!(rule(u, leak(vec![bad_key])).as_deref(), Some("S1"));
    let ka = KeyEntry::new(key('d'), Layer::Base, "क", TokenKind::Consonant);
    assert_eq!(rule(u, leak(vec![ka, ka])).as_deref(), Some("S2"));
    let empty = KeyEntry::new(key('d'), Layer::Base, "", TokenKind::Consonant);
    assert_eq!(rule(u, leak(vec![empty])).as_deref(), Some("S3"));
    let latin = KeyEntry::new(key('d'), Layer::Shift, "d", TokenKind::Literal);
    assert_eq!(
        rule(TableProfile::Latin, leak(vec![latin])).as_deref(),
        Some("P1")
    );
    let legacy = KeyEntry::new(key('d'), Layer::Base, "d", TokenKind::Consonant);
    assert_eq!(
        rule(TableProfile::LegacyIdentity, leak(vec![legacy])).as_deref(),
        Some("P2")
    );
    let nfd = KeyEntry::new(key('d'), Layer::Base, "\u{0958}", TokenKind::Consonant);
    assert_eq!(rule(u, leak(vec![nfd])).as_deref(), Some("P3"));
}

#[test]
fn kind_rules_accept_real_entries_and_reject_synthetic_ones() {
    let gail = load_layout(LayoutId::RemingtonGail).expect("gail");
    let cbi = load_layout(LayoutId::RemingtonCbi).expect("cbi");
    let kd = load_layout(LayoutId::KrutiDev).expect("krutidev");
    let get = |l: &'static dyn radha_layouts::KeyboardLayout, c: char, layer: Layer| {
        *l.table()
            .get(Keystroke::new(key(c), layer))
            .expect("real slot")
    };
    // Positive: one real entry per rule, from the real tables.
    let positives = [
        (get(gail, 'd', Layer::Shift), TokenKind::Half),
        (get(gail, 'z', Layer::Shift), TokenKind::Reph),
        (get(gail, 'z', Layer::Base), TokenKind::Rakar),
        (get(gail, '=', Layer::Shift), TokenKind::Halant),
        (get(gail, '`', Layer::Base), TokenKind::Nukta),
        (get(gail, 'f', Layer::Base), TokenKind::Preposed),
        (get(gail, 'd', Layer::Base), TokenKind::Consonant),
        (get(gail, '9', Layer::Shift), TokenKind::Conjunct),
        // Book 03: CBI has no AltGr layer; `v` is अ.
        (get(cbi, 'v', Layer::Base), TokenKind::Vowel),
    ];
    for (entry, kind) in positives {
        assert_eq!(entry.kind, kind);
        assert_eq!(
            rule(TableProfile::Unicode, leak(vec![entry])),
            None,
            "{entry:?}"
        );
    }
    for (c, layer, kind) in [
        ('D', Layer::Base, TokenKind::Half),
        ('f', Layer::Base, TokenKind::Preposed),
        ('=', Layer::Shift, TokenKind::Nukta),
    ] {
        let entry = get(
            kd,
            c.to_ascii_lowercase(),
            if c.is_ascii_uppercase() {
                Layer::Shift
            } else {
                layer
            },
        );
        assert_eq!(entry.kind, kind);
        assert_eq!(rule(TableProfile::LegacyIdentity, leak(vec![entry])), None);
    }
    // Negative: a synthetic entry breaking each rule.
    let d = key('d');
    let negatives = [
        ("K1", KeyEntry::new(d, Layer::Base, "क", TokenKind::Half)),
        ("K2", KeyEntry::new(d, Layer::Base, "र", TokenKind::Reph)),
        ("K3", KeyEntry::new(d, Layer::Base, "्", TokenKind::Rakar)),
        ("K4", KeyEntry::new(d, Layer::Base, "्र", TokenKind::Halant)),
        ("K5", KeyEntry::new(d, Layer::Base, "ं", TokenKind::Nukta)),
        ("K6", KeyEntry::new(d, Layer::Base, "क", TokenKind::Matra)),
        (
            "K7",
            KeyEntry::new(d, Layer::Base, "कि", TokenKind::Consonant),
        ),
        (
            "K8",
            KeyEntry::new(d, Layer::Base, "क्", TokenKind::Conjunct),
        ),
        ("K9", KeyEntry::new(d, Layer::Base, "क", TokenKind::Vowel)),
    ];
    for (id, entry) in negatives {
        assert_eq!(
            rule(TableProfile::Unicode, leak(vec![entry])).as_deref(),
            Some(id),
            "{entry:?}"
        );
    }
}

#[test]
fn a_table_error_converts_to_a_recoverable_core_error() {
    let err = LayoutError::InvalidTable {
        layout: "gail",
        index: 3,
        detail: "K5 nukta must be exactly U+093C".to_owned(),
    };
    let core: CoreError = err.into();
    assert!(core.is_recoverable());
    assert!(core.to_string().contains("K5"));
}
