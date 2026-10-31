//! P09 Part A content checks on the shipped bundles (`content/exercises`):
//! exact counts per track and pool, no placeholder text, no duplicates
//! within or across pools, minimum lengths, typability per layout, the
//! Track 03 arithmetic / date / time validator, and card counts that come
//! from the data.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use radha_content::import::{find_bundle_file, parse_bundle_bytes, typable_on, ContentBundle};
use radha_content::{numeric, quality, ContentTarget, ExamPool, ProductId};
use radha_layouts::LayoutId;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(target: ContentTarget) -> ContentBundle {
    let dir = repo().join("content/exercises");
    let path =
        find_bundle_file(&dir, target).unwrap_or_else(|| panic!("{} has no bundle", target.slug()));
    let bytes = std::fs::read(&path).unwrap();
    parse_bundle_bytes(&path, &bytes).unwrap()
}

fn tool_data(name: &str) -> String {
    std::fs::read_to_string(repo().join("tools/import-golu-content/data").join(name)).unwrap()
}

const TRACK03: ContentTarget = ContentTarget::Product(ProductId::NumberPunctuation);
const ENGLISH_POOL: ContentTarget = ContentTarget::Pool(ExamPool::English);
const HINDI_POOL: ContentTarget = ContentTarget::Pool(ExamPool::Hindi);

#[test]
fn track03_is_golus_50_drills_with_their_own_titles() {
    let b = load(TRACK03);
    assert_eq!(b.exercises.len(), 50);
    let seed = tool_data("track03-alphanumeric-50-seed.md");
    for (i, e) in b.exercises.iter().enumerate() {
        let id = format!("ALN-{:03}", i + 1);
        assert_eq!(e.source_id, id);
        assert!(
            seed.contains(&format!("## {id} — {}", e.title)),
            "{id} title {:?} is not Golu's",
            e.title
        );
        assert!(e.word_count() >= 2000, "{id}: {} words", e.word_count());
    }
}

#[test]
fn track03_arithmetic_dates_and_times_are_right() {
    for e in &load(TRACK03).exercises {
        let problems = numeric::check_text(&e.content, numeric::rules_for(&e.source_id));
        assert!(problems.is_empty(), "{}: {problems:#?}", e.source_id);
    }
}

#[test]
fn track03_repeats_no_line_and_types_on_a_us_keyboard() {
    let b = load(TRACK03);
    let texts: Vec<&str> = b.exercises.iter().map(|e| e.content.as_str()).collect();
    let repeated = quality::repeated_lines(&texts);
    assert!(
        repeated.is_empty(),
        "{:?}",
        &repeated[..repeated.len().min(5)]
    );
    for e in &b.exercises {
        assert!(
            e.content.is_ascii(),
            "{}: ₹ × ÷ – — € ≈ are typed as ASCII",
            e.source_id
        );
        assert!(typable_on(LayoutId::Qwerty, &e.content), "{}", e.source_id);
        assert!(!e.content.contains("  "), "{}: double space", e.source_id);
    }
}

#[test]
fn the_english_exam_pool_is_the_title_bank_1_to_500() {
    let b = load(ENGLISH_POOL);
    assert_eq!(b.exercises.len(), 500);
    let bank = tool_data("exam-500-english-titles.md");
    let rows: Vec<(String, String)> = bank
        .lines()
        .filter_map(|l| l.split_once(" | "))
        .filter(|(id, _)| id.contains('-') && id.chars().last().is_some_and(|c| c.is_ascii_digit()))
        .map(|(id, t)| (id.trim().to_owned(), t.trim().to_owned()))
        .collect();
    assert_eq!(rows.len(), 500);
    for (e, (id, title)) in b.exercises.iter().zip(&rows) {
        assert_eq!(&e.source_id, id);
        assert_eq!(&e.title, title, "{id}");
        assert!(
            e.word_count() >= 450,
            "{id}: {} words (10-15 min test)",
            e.word_count()
        );
        assert!(typable_on(LayoutId::Qwerty, &e.content), "{id}");
        if id.starts_with("CGL-DEST-") || id.starts_with("EPFO-") {
            assert!(e.content.chars().count() >= 2000, "{id}: key depressions");
            assert!(
                e.content.chars().any(|c| c.is_ascii_digit()),
                "{id}: numeric fields"
            );
        }
    }
}

/// "9:15 a.m." / "2:05 p.m." as minutes after midnight.
fn clock_minutes(s: &str) -> Option<i64> {
    let (hm, half) = s.trim().split_once(' ')?;
    let (h, m) = hm.split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    let h = match (half, h) {
        ("a.m.", 12) => 0,
        ("a.m.", h) => h,
        ("p.m.", 12) => 12,
        ("p.m.", h) => h + 12,
        _ => return None,
    };
    Some(h * 60 + m)
}

#[test]
fn english_exam_hourly_averages_match_their_times() {
    // "Between {t1} and {t2}, the {section} entered {n} records, an average
    // of [about ]{avg} each hour." The figure must be the true rate within
    // 0.5, and only a rounded figure may say "about".
    let mut checked = 0;
    for e in &load(ENGLISH_POOL).exercises {
        for part in e.content.split("Between ").skip(1) {
            let Some((sentence, _)) = part.split_once(" each hour.") else {
                continue;
            };
            let Some((times, rest)) = sentence.split_once(", the ") else {
                continue;
            };
            let Some((rest, avg)) = rest.split_once(" records, an average of ") else {
                continue;
            };
            let (t1, t2) = times.split_once(" and ").expect("two times");
            let (t1, t2) = (clock_minutes(t1).unwrap(), clock_minutes(t2).unwrap());
            let total: i64 = rest.rsplit(' ').next().unwrap().parse().unwrap();
            let (about, figure) = match avg.strip_prefix("about ") {
                Some(f) => (true, f),
                None => (false, avg),
            };
            let figure: f64 = figure.parse().unwrap();
            let minutes = t2 - t1;
            assert!(minutes > 0, "{}: {sentence}", e.source_id);
            #[allow(clippy::cast_precision_loss)]
            let exact = total as f64 * 60.0 / minutes as f64;
            assert!(
                (figure - exact).abs() <= 0.5,
                "{}: {sentence} (true rate {exact:.2})",
                e.source_id
            );
            assert_eq!(
                about,
                total * 60 % minutes != 0,
                "{}: {sentence} (\"about\" only for a rounded rate)",
                e.source_id
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no hourly-average sentence found");
}

#[test]
fn english_exam_passages_share_no_sentence_and_no_near_duplicate() {
    let b = load(ENGLISH_POOL);
    let texts: Vec<&str> = b.exercises.iter().map(|e| e.content.as_str()).collect();
    let shared = quality::shared_sentences(&texts, 6);
    assert!(shared.is_empty(), "{:?}", &shared[..shared.len().min(5)]);
    let (s, i, j) = quality::most_similar_pair(&texts).unwrap();
    assert!(
        s < 0.25,
        "{} and {} are near-duplicates ({s:.3})",
        b.exercises[i].source_id,
        b.exercises[j].source_id
    );
}

#[test]
fn the_hindi_pool_is_500_real_passages_typable_on_every_mangal_layout() {
    let b = load(HINDI_POOL);
    assert_eq!(b.exercises.len(), 500);
    let mut texts = HashSet::new();
    let mut titles = HashSet::new();
    for e in &b.exercises {
        assert!(
            texts.insert(e.content.as_str()),
            "#{} duplicate text",
            e.number
        );
        assert!(
            titles.insert(e.title.as_str()),
            "#{} duplicate title",
            e.number
        );
        for layout in [
            LayoutId::RemingtonGail,
            LayoutId::RemingtonCbi,
            LayoutId::Inscript,
        ] {
            assert!(
                typable_on(layout, &e.content),
                "#{} on {layout:?}",
                e.number
            );
        }
        let legacy = e.legacy.as_deref().expect("Kruti Dev form");
        assert!(
            typable_on(LayoutId::KrutiDev, legacy),
            "#{} on Kruti Dev",
            e.number
        );
        assert!(e.word_count() >= 450, "#{}", e.number);
    }
}

#[test]
fn no_shipped_text_holds_a_placeholder_marker() {
    let targets = [
        ContentTarget::Product(ProductId::EnglishClassic),
        TRACK03,
        ContentTarget::Product(ProductId::HindiMastery(LayoutId::RemingtonGail)),
        ContentTarget::Product(ProductId::HindiMastery(LayoutId::RemingtonCbi)),
        ContentTarget::Product(ProductId::HindiMastery(LayoutId::KrutiDev)),
        ENGLISH_POOL,
        HINDI_POOL,
    ];
    for target in targets {
        for e in &load(target).exercises {
            let found = quality::placeholder_markers(&e.content);
            assert!(
                found.is_empty(),
                "{} #{}: {found:?}",
                target.slug(),
                e.number
            );
            assert!(!e.title.trim().is_empty());
        }
    }
}

#[test]
fn no_passage_is_shared_between_pools_or_tracks() {
    let mut owner: std::collections::HashMap<String, &'static str> =
        std::collections::HashMap::new();
    let targets = [
        ContentTarget::Product(ProductId::EnglishClassic),
        ContentTarget::Product(ProductId::TypingFlow),
        TRACK03,
        ENGLISH_POOL,
        HINDI_POOL,
    ];
    for target in targets {
        for e in &load(target).exercises {
            if let Some(other) = owner.insert(e.content_sha256.clone(), target.slug()) {
                assert_eq!(other, target.slug(), "#{} shared with {other}", e.number);
                panic!(
                    "{} #{} duplicated inside the target",
                    target.slug(),
                    e.number
                );
            }
        }
    }
}

#[test]
fn remington_cbi_has_its_own_course_not_gail_copies() {
    let gail = load(ContentTarget::Product(ProductId::HindiMastery(
        LayoutId::RemingtonGail,
    )));
    let cbi = load(ContentTarget::Product(ProductId::HindiMastery(
        LayoutId::RemingtonCbi,
    )));
    let gail_texts: HashSet<&str> = gail.exercises.iter().map(|e| e.content.as_str()).collect();
    let copies: Vec<u32> = cbi
        .exercises
        .iter()
        .filter(|e| gail_texts.contains(e.content.as_str()))
        .map(|e| e.number)
        .collect();
    assert!(
        copies.is_empty(),
        "CBI passages identical to GAIL: {copies:?}"
    );
}

#[test]
fn the_all_in_one_mangal_card_is_first_and_hindi_only() {
    // Its passage chip is rewritten from the installed pool at startup
    // (`ui::apply_exam_pool_counts`, unit-tested in `catalog`).
    let exams =
        std::fs::read_to_string(repo().join("crates/radha-app/ui/data/exams.slint")).unwrap();
    let first_id = exams
        .split("id: \"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap();
    assert_eq!(first_id, "all-in-one-mangal");
    let card = exams.split("id: \"all-in-one-mangal\"").nth(1).unwrap();
    let card = &card[..card.find("},").unwrap()];
    assert!(card.contains("hindi-only: true"));
    assert!(card.contains("InScript / GAIL / CBI"));
    assert!(
        card.contains("hin-speed: \"—\""),
        "the count is not a literal"
    );
}
