//! The shipped content bundles (`content/exercises`, written by
//! `tools/import-golu-content`) are complete, valid and typeable.
//!
//! Counts per product are exact: 200 each for English Classic and the three
//! Hindi Mastery products; no bundle (and therefore an honest "content
//! coming" state) for every product Golu has no mappable content for.

use std::path::{Path, PathBuf};

use radha_content::import::{
    find_bundle_file, golu_source, parse_bundle_bytes, validate_bundle, validate_complete,
};
use radha_content::{ContentStatus, ContentTarget, ExamPool, ProductId};
use radha_core::akshar::normalize;
use radha_core::{Akshar, SessionConfig, SessionState, TypingSession};
use radha_layouts::{
    load_layout, press_key, Composer, Encoding, Guide, KeyEffect, Keystroke, LayoutId, NextAction,
};

fn content_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/exercises")
}

fn load_target(target: ContentTarget) -> Option<radha_content::import::ContentBundle> {
    let path = find_bundle_file(&content_dir(), target)?;
    let bytes = std::fs::read(&path).ok()?;
    Some(parse_bundle_bytes(&path, &bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

fn load(product: ProductId) -> Option<radha_content::import::ContentBundle> {
    load_target(ContentTarget::Product(product))
}

#[test]
fn every_mapped_product_ships_exactly_200_valid_exercises() {
    for product in ProductId::all() {
        if let Some(source) = golu_source(product) {
            let bundle =
                load(product).unwrap_or_else(|| panic!("{} has no bundle", product.slug()));
            assert_eq!(validate_bundle(&bundle).unwrap(), product);
            assert_eq!(bundle.exercises.len(), 200, "{}", product.slug());
            assert_eq!(bundle.provisional, source.provisional, "{}", product.slug());
            for e in &bundle.exercises {
                assert!(!e.content.trim().is_empty());
                if product.layout() != LayoutId::KrutiDev {
                    assert_eq!(normalize(&e.content), e.content, "NFC {}", e.number);
                }
            }
        }
    }
}

/// P04 per-product counts: every target ships exactly its count, validated
/// by the importer's full gate. Number + Punctuation ships Golu's 50 Track 03
/// drills (P09; before P09 it shipped nothing and showed an empty state).
#[test]
fn every_target_ships_its_exact_count_with_honest_status() {
    let mut total = 0;
    for target in ContentTarget::all() {
        let bundle = load_target(target);
        let bundle = bundle.unwrap_or_else(|| panic!("{} has no bundle", target.slug()));
        assert_eq!(validate_complete(&bundle).unwrap(), target);
        assert_eq!(
            bundle.exercises.len(),
            target.expected_count(),
            "{}",
            target.slug()
        );
        total += bundle.exercises.len();
        let generated = bundle
            .exercises
            .iter()
            .filter(|e| e.status == ContentStatus::GeneratedReviewPending)
            .count();
        match target {
            ContentTarget::Product(ProductId::HindiLearn(_)) => {
                assert_eq!(generated, 200, "Hindi Learn is authored, review pending");
            }
            ContentTarget::Product(ProductId::NumberPunctuation) => {
                assert_eq!(generated, 50, "Track 03 drills are expanded from seeds");
            }
            ContentTarget::Pool(ExamPool::English) => {
                // P09: the consolidated title-bank pool. 148 Radha 500 drafts
                // are reused (provisional, the drafts say "draft"); the other
                // 352 are written from briefs. None is presented as verified.
                assert_eq!(generated, 352, "352 written English exam passages");
                assert!(bundle
                    .exercises
                    .iter()
                    .all(|e| e.status != ContentStatus::Verified));
            }
            _ => assert_eq!(generated, 0, "{}: imported, not generated", target.slug()),
        }
        if matches!(target, ContentTarget::Pool(_)) {
            assert!(bundle.exercises.iter().all(|e| e.word_count() <= 600));
        }
        for e in &bundle.exercises {
            assert!(
                !e.source_path.is_empty() && e.source_sha256.len() == 64,
                "provenance"
            );
        }
    }
    assert_eq!(
        total, 2550,
        "2600 planned, Number + Punctuation is Golu's 50 (not 100)"
    );
}

/// Unicode Hindi courses keep passages their layout cannot type but do not
/// list them (book-driven: CBI book 03 has no key for ॐ or ॰). The hidden
/// set is pinned so a content or layout change that hides more is noticed;
/// every other passage is typable.
#[test]
fn passages_a_layout_cannot_type_are_counted_and_hidden_not_failed() {
    use radha_content::import::{hidden_passages, typable_on};
    // Kruti Dev: book 04 registers Shift+4 (`$`) as a conflict, so the drills
    // that teach it are hidden too.
    let expect: [(ProductId, &[u32]); 6] = [
        // #17 `$`; #21..26 `°` (old Alt+0176; book 04 ॰ is Alt+0241).
        (
            ProductId::HindiMastery(LayoutId::KrutiDev),
            &[17, 21, 22, 24, 25, 26],
        ),
        (ProductId::HindiLearn(LayoutId::KrutiDev), &[77, 87]),
        (ProductId::HindiMastery(LayoutId::RemingtonGail), &[]),
        (
            ProductId::HindiMastery(LayoutId::RemingtonCbi),
            &[21, 22, 24, 25, 26],
        ),
        (ProductId::HindiLearn(LayoutId::RemingtonGail), &[]),
        (
            ProductId::HindiLearn(LayoutId::RemingtonCbi),
            &[87, 106, 121, 159],
        ),
    ];
    for (product, hidden) in expect {
        let target = ContentTarget::Product(product);
        let bundle = load(product).expect("bundle");
        assert_eq!(
            hidden_passages(target, &bundle),
            hidden,
            "{}",
            product.slug()
        );
        let listed = bundle
            .exercises
            .iter()
            .filter(|e| !hidden.contains(&e.number));
        for e in listed {
            assert!(
                typable_on(product.layout(), &e.content),
                "{} #{}",
                product.slug(),
                e.number
            );
        }
    }
    // English and the exam pools never hide: they stay strict.
    for target in [
        ContentTarget::Product(ProductId::EnglishClassic),
        ContentTarget::Pool(ExamPool::English),
        ContentTarget::Pool(ExamPool::Hindi),
    ] {
        assert!(!radha_content::import::hides_untypeable(target));
    }
}

/// Type the first `units` units of exercise 1 by following the guide.
fn follow_guide(product: ProductId, units: usize) {
    let bundle = load(product).expect("bundle");
    let layout = load_layout(product.layout()).unwrap();
    // The first exercise whose opening `units` units all have a key plan.
    // (Some GAIL drills hold an isolated pre-posed `ि`, which the planner does
    // not plan; that is reported by the importer, not hidden here.)
    let target = bundle
        .exercises
        .iter()
        .find_map(|e| {
            let all = Guide::new(layout, &e.content);
            let prefix: String = all.units().get(..units)?.concat();
            Guide::new(layout, &prefix)
                .untypeable()
                .is_empty()
                .then_some(prefix)
        })
        .unwrap_or_else(|| panic!("{}: no plannable exercise", product.slug()));
    let guide = std::sync::Arc::new(Guide::new(layout, &target));
    let mut composer = Composer::new(layout);
    let t0 = std::time::Instant::now();
    let mut presses = 0;
    macro_rules! drive {
        ($s:expr, $tail:expr) => {{
            let mut s = $s;
            while s.state() != SessionState::Complete {
                let e = s.evaluation();
                let tail = if e.pending {
                    $tail(s.matcher().typed())
                } else {
                    None
                };
                let Some(NextAction::Key(h)) =
                    guide.next_action(e.cursor, tail.as_deref(), composer.preedit(), false)
                else {
                    panic!("{}: no key hint at {}", product.slug(), e.cursor);
                };
                let effect = press_key(&mut s, &mut composer, Keystroke::new(h.key, h.layer), t0);
                assert!(matches!(effect, KeyEffect::Applied { .. }));
                presses += 1;
                assert!(presses < units * 8, "{}: guide loops", product.slug());
            }
            assert_eq!(s.snapshot_at(t0).accuracy, 100.0, "{}", product.slug());
        }};
    }
    match layout.encoding() {
        Encoding::Unicode => drive!(
            TypingSession::<Akshar>::configured(&target, SessionConfig::default())
                .with_composition(guide.clone()),
            |typed: &str| radha_core::akshar::segment(typed).pop()
        ),
        Encoding::Latin | Encoding::Legacy => {
            drive!(TypingSession::new(&target), |_: &str| None::<String>)
        }
    }
}

/// Per-keystroke engine cost on the longest shipped passage: one key, one
/// snapshot and one evaluation, which is what the typing screen does per key.
/// The ceiling is generous (unoptimised test build); the measured p50/p99 are
/// printed for the verification note.
#[test]
fn per_keystroke_cost_on_the_longest_passage_is_bounded() {
    let bundle = load(ProductId::EnglishClassic).expect("bundle");
    let longest = bundle
        .exercises
        .iter()
        .max_by_key(|e| e.content.len())
        .expect("exercises");
    let mut s = TypingSession::new(&longest.content);
    let t0 = std::time::Instant::now();
    let mut samples = Vec::new();
    for c in longest.content.chars().take(300) {
        let start = std::time::Instant::now();
        s.type_char_at(c, t0);
        let snap = s.snapshot_at(t0);
        let eval = s.evaluation();
        samples.push(start.elapsed());
        assert_eq!(snap.cursor, eval.cursor);
    }
    samples.sort();
    let p50 = samples[samples.len() / 2];
    let p99 = samples[samples.len() * 99 / 100];
    eprintln!(
        "per-key cost over {} chars: p50 {:?}, p99 {:?}",
        longest.content.chars().count(),
        p50,
        p99
    );
    assert!(p99 < std::time::Duration::from_millis(250), "p99 {p99:?}");
}

#[test]
fn the_first_exercise_of_each_product_types_cleanly_on_its_layout() {
    for product in ProductId::all() {
        if load(product).is_some() {
            follow_guide(product, 60);
        }
    }
}
