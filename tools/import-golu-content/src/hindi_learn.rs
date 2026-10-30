//! Hindi Learn authoring pipeline (P04): 200 lessons per layout,
//! Beginner 100 + Classic 100, status `generated-review-pending`.
//!
//! - **Beginner** (finger-first): keys are introduced two at a time in
//!   touch-typing order (home row, top row, bottom row, number row; base
//!   layer, then Shift), each introduction followed by a practice lesson,
//!   then review lessons. Every token of a lesson is typeable with the keys
//!   learned so far, as decided by the P03 keystroke planner (Unicode) or
//!   the Kruti Dev table (legacy stream).
//! - **Classic**: the tier plan of Expert AI's `CURRICULUM.md`: progressive
//!   word practice by length buckets (≤3, 4–5, 6–7, 8–9, 10+ code points),
//!   sentence practice (short to long), paragraph practice.
//!
//! The vocabulary and sentences come from the layout's Hindi Mastery source
//! (Unicode; Kruti Dev lessons are written in Unicode from the CBI source
//! and converted to the legacy stream). Nothing is invented: generation only
//! selects and orders existing words and sentences. Every lesson is checked
//! unit by unit; a lesson that cannot be fully typed aborts the import.

use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{bail, Context};
use radha_content::codec::sha256_hex;
use radha_content::import::{unknown_legacy_chars, untypeable_chars, ImportedExercise};
use radha_content::{ContentStatus, ProductId};
use radha_core::akshar::segment;
use radha_layouts::krutidev::to_legacy;
use radha_layouts::{
    load_layout, Encoding, Guide, KeyboardLayout, Layer, LayoutId, PhysicalKey, PlanStep, Planner,
    TokenKind,
};

use crate::rng::Rng;

/// One physical stroke.
type Stroke = (PhysicalKey, Layer);

/// Touch-typing introduction order of the 47 keys: home row from the index
/// fingers outwards, then top, bottom and number rows.
const KEY_ORDER: &str = "fjdkslag;h'rueiwoqptyvmc,x.z/bn[]\\4738291056`-=";

/// Pipeline label stored as provenance.
pub const PIPELINE: &str = "generated:radha-hindi-learn-v1";

/// Lessons per track.
const PER_TRACK: usize = 100;

/// Which Unicode source feeds a layout's lessons.
#[must_use]
pub fn vocabulary_layout(layout: LayoutId) -> LayoutId {
    match layout {
        LayoutId::KrutiDev => LayoutId::RemingtonCbi,
        other => other,
    }
}

/// Words and sentences mined from the source passages.
pub struct Corpus {
    /// Unique sentences (with their `।`), in source order.
    pub sentences: Vec<String>,
    /// Sentences grouped by source passage, in order (for paragraphs).
    pub passages: Vec<Vec<String>>,
    /// Unique words, in first-seen order.
    pub words: Vec<String>,
}

fn is_hindi_word(token: &str) -> bool {
    !token.is_empty()
        && token.chars().all(|c| {
            ('\u{0900}'..='\u{0963}').contains(&c) || ('\u{0970}'..='\u{097F}').contains(&c)
        })
}

/// Mine sentences and words from Unicode passages.
#[must_use]
pub fn mine(texts: &[&str]) -> Corpus {
    let mut seen_s = HashSet::new();
    let mut seen_w = HashSet::new();
    let mut corpus = Corpus {
        sentences: Vec::new(),
        passages: Vec::new(),
        words: Vec::new(),
    };
    for text in texts {
        let mut passage = Vec::new();
        for raw in text.split('\u{0964}') {
            let s = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            let words: Vec<&str> = s.split(' ').filter(|w| !w.is_empty()).collect();
            let prose = (3..=25).contains(&words.len())
                && words
                    .iter()
                    .all(|w| is_hindi_word(w.trim_end_matches(',')) && !w.starts_with(','));
            if prose {
                let sentence = format!("{s}\u{0964}");
                if !passage.contains(&sentence) {
                    passage.push(sentence.clone());
                }
                if seen_s.insert(sentence.clone()) {
                    corpus.sentences.push(sentence);
                }
            }
            for w in words {
                let w = w.trim_end_matches(',');
                if is_hindi_word(w) && seen_w.insert(w.to_owned()) {
                    corpus.words.push(w.to_owned());
                }
            }
        }
        corpus.passages.push(passage);
    }
    corpus
}

/// Typeability of Unicode text on one layout, with per-akshar key sets.
pub struct Typist {
    layout: &'static dyn KeyboardLayout,
    planner: Planner,
    cache: HashMap<String, Option<BTreeSet<Stroke>>>,
}

impl Typist {
    /// For `layout`.
    pub fn new(layout: LayoutId) -> anyhow::Result<Self> {
        let layout = load_layout(layout).context("layout")?;
        Ok(Typist {
            layout,
            planner: Planner::new(layout),
            cache: HashMap::new(),
        })
    }

    /// The strokes needed to type Unicode `text` (spaces excluded), or
    /// `None` when any part has no plan (or needs an Alt code and
    /// `allow_alt` is false). Kruti Dev converts first; a conversion failure
    /// is `None`.
    pub fn strokes(&mut self, text: &str, allow_alt: bool) -> Option<BTreeSet<Stroke>> {
        let mut all = BTreeSet::new();
        match self.layout.encoding() {
            Encoding::Legacy => {
                let conv = to_legacy(text);
                if !conv.failures.is_empty() {
                    return None;
                }
                for c in conv.legacy.chars() {
                    if c == ' ' {
                        continue;
                    }
                    let entry = self
                        .layout
                        .table()
                        .producing(&c.to_string())
                        .into_iter()
                        .next();
                    match entry {
                        Some(e) => {
                            all.insert((e.key, e.layer));
                        }
                        None if allow_alt => {}
                        None => return None,
                    }
                }
            }
            Encoding::Unicode | Encoding::Latin => {
                for unit in segment(text) {
                    if unit == " " {
                        continue;
                    }
                    let keys = self.unit(&unit, allow_alt)?;
                    all.extend(keys);
                }
            }
        }
        Some(all)
    }

    fn unit(&mut self, unit: &str, allow_alt: bool) -> Option<BTreeSet<Stroke>> {
        let key = format!("{allow_alt}{unit}");
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let planned = self.planner.plan(unit);
        let result = planned.shortest().and_then(|plan| {
            let mut keys = BTreeSet::new();
            for step in &plan.steps {
                match step {
                    PlanStep::Key(e) => {
                        keys.insert((e.key, e.layer));
                    }
                    PlanStep::Space => {}
                    PlanStep::AltCode(_) if allow_alt => {}
                    PlanStep::AltCode(_) => return None,
                }
            }
            Some(keys)
        });
        self.cache.insert(key, result.clone());
        result
    }

    /// The stored form of Unicode lesson text: itself, or the Kruti Dev
    /// legacy stream.
    pub fn stored(&self, unicode: &str) -> anyhow::Result<String> {
        match self.layout.encoding() {
            Encoding::Legacy => {
                let conv = to_legacy(unicode);
                if !conv.failures.is_empty() {
                    bail!("Kruti Dev conversion failed: {:?}", conv.failures);
                }
                Ok(conv.legacy)
            }
            Encoding::Unicode | Encoding::Latin => Ok(unicode.to_owned()),
        }
    }

    /// Abort unless every unit of the stored lesson is typeable.
    pub fn check(&self, stored: &str, what: &str) -> anyhow::Result<()> {
        let id = self.layout.id();
        let chars = untypeable_chars(id, stored);
        let units = Guide::new(self.layout, stored);
        let units = units.untypeable();
        let unknown = if id == LayoutId::KrutiDev {
            unknown_legacy_chars(stored)
        } else {
            Vec::new()
        };
        if !chars.is_empty() || !units.is_empty() || !unknown.is_empty() {
            bail!(
                "{what}: untypeable on {} ({chars:?} {units:?} {unknown:?})",
                id.as_str()
            );
        }
        Ok(())
    }
}

/// Where a stroke sits, for titles and difficulty.
fn row_name(stroke: Stroke) -> &'static str {
    let shift = stroke.1 != Layer::Base;
    match (stroke.0.row(), shift) {
        (Some(2), false) => "Home row",
        (Some(1), false) => "Top row",
        (Some(3), false) => "Bottom row",
        (Some(0), false) => "Number row",
        (Some(2), true) => "Shift: home row",
        (Some(1), true) => "Shift: top row",
        (Some(3), true) => "Shift: bottom row",
        _ => "Shift: number row",
    }
}

/// A unit (Unicode) that drills one stroke: the stroke's own text when it
/// stands alone, else the text on a learned consonant.
fn drill_unit(
    typist: &mut Typist,
    layout: &'static dyn KeyboardLayout,
    stroke: Stroke,
    learned: &BTreeSet<Stroke>,
    consonants: &[String],
) -> Option<String> {
    let entry = layout
        .table()
        .entries()
        .iter()
        .find(|e| (e.key, e.layer) == stroke)?;
    let text = entry.text();
    let mut candidates: Vec<String> = Vec::new();
    match entry.kind {
        TokenKind::Consonant
        | TokenKind::Vowel
        | TokenKind::Conjunct
        | TokenKind::Digit
        | TokenKind::Punctuation
        | TokenKind::Literal => candidates.push(text.to_owned()),
        TokenKind::Half => candidates.push(text.trim_end_matches('\u{094D}').to_owned()),
        TokenKind::Reph => candidates.extend(consonants.iter().map(|c| format!("{text}{c}"))),
        TokenKind::Halant => candidates.extend(consonants.iter().map(|c| format!("{c}{text}{c}"))),
        TokenKind::Matra
        | TokenKind::Preposed
        | TokenKind::Completer
        | TokenKind::Rakar
        | TokenKind::Nukta => candidates.extend(consonants.iter().map(|c| format!("{c}{text}"))),
    }
    candidates.into_iter().find(|u| {
        !u.trim().is_empty()
            && typist
                .strokes(u, false)
                .is_some_and(|keys| keys.contains(&stroke) && keys.is_subset(learned))
    })
}

struct Lesson {
    title: String,
    category: String,
    difficulty: u8,
    unicode: String,
}

/// The ordered strokes a layout's beginner track introduces (base layer,
/// then Shift), restricted to slots the layout maps.
fn stroke_order(layout: &'static dyn KeyboardLayout) -> Vec<Stroke> {
    let mut order = Vec::new();
    for layer in [Layer::Base, Layer::Shift] {
        for c in KEY_ORDER.chars() {
            let Some(key) = PhysicalKey::from_base(c) else {
                continue;
            };
            if key == radha_layouts::SPACE {
                continue;
            }
            let mapped = layout
                .table()
                .entries()
                .iter()
                .any(|e| e.key == key && e.layer == layer);
            if mapped && !order.contains(&(key, layer)) {
                order.push((key, layer));
            }
        }
    }
    order
}

fn join_tokens(tokens: &[String]) -> String {
    tokens.join(" ")
}

fn beginner(
    typist: &mut Typist,
    layout: &'static dyn KeyboardLayout,
    corpus: &Corpus,
    seed: u64,
) -> anyhow::Result<(Vec<Lesson>, Vec<String>)> {
    let order = stroke_order(layout);
    // Every word with its key set (Alt-code words never reach beginners).
    let words: Vec<(String, BTreeSet<Stroke>)> = corpus
        .words
        .iter()
        .filter_map(|w| typist.strokes(w, false).map(|k| (w.clone(), k)))
        .collect();
    let consonants: Vec<String> = layout
        .table()
        .entries()
        .iter()
        .filter(|e| e.kind == TokenKind::Consonant)
        .map(|e| e.text().to_owned())
        .collect();

    // Group strokes in twos; a stroke that cannot be drilled yet rides along
    // with the next group, and is reported if it never can be.
    let mut learned: BTreeSet<Stroke> = BTreeSet::new();
    let mut groups: Vec<(Vec<Stroke>, Vec<String>, BTreeSet<Stroke>)> = Vec::new();
    let mut carry: Vec<Stroke> = Vec::new();
    for stroke in order {
        carry.push(stroke);
        if carry.len() < 2 {
            continue;
        }
        let mut trial = learned.clone();
        trial.extend(carry.iter().copied());
        let units: Vec<String> = carry
            .iter()
            .filter_map(|&s| drill_unit(typist, layout, s, &trial, &consonants))
            .collect();
        if units.is_empty() {
            continue;
        }
        learned = trial;
        groups.push((std::mem::take(&mut carry), units, learned.clone()));
    }
    let skipped: Vec<String> = carry
        .iter()
        .map(|(k, l)| {
            format!(
                "{}{}",
                k.legend(),
                if *l == Layer::Base { "" } else { "+Shift" }
            )
        })
        .collect();
    if groups.is_empty() {
        bail!("{}: no drillable key group", layout.id().as_str());
    }

    let mut rng = Rng::new(seed);
    let mut lessons = Vec::new();
    let mut known_units: Vec<String> = Vec::new();
    for (strokes, units, set) in &groups {
        let legends: Vec<String> = strokes
            .iter()
            .map(|(k, l)| {
                format!(
                    "{}{}",
                    k.legend(),
                    if *l == Layer::Base { "" } else { "+Shift" }
                )
            })
            .collect();
        let category = row_name(strokes[0]).to_owned();
        let difficulty = match category.as_str() {
            "Home row" => 1,
            "Top row" | "Bottom row" => 2,
            _ => 3,
        };
        // Introduction: the new units in patterns, then with earlier units.
        let mut tokens = Vec::new();
        for _ in 0..4 {
            for u in units {
                tokens.extend([u.clone(), u.clone(), format!("{u}{u}"), u.clone()]);
            }
        }
        for _ in 0..12 {
            if known_units.is_empty() {
                break;
            }
            let p = &known_units[rng.below(known_units.len())];
            let u = &units[rng.below(units.len())];
            tokens.push(format!("{u}{p}"));
            tokens.push(format!("{p}{u}"));
        }
        let fresh: Vec<&String> = words
            .iter()
            .filter(|(_, k)| k.is_subset(set) && strokes.iter().any(|s| k.contains(s)))
            .map(|(w, _)| w)
            .collect();
        for _ in 0..fresh.len().min(24) {
            tokens.push(fresh[rng.below(fresh.len())].clone());
        }
        lessons.push(Lesson {
            title: format!("New keys: {} ({})", legends.join(" "), units.join(" ")),
            category: category.clone(),
            difficulty,
            unicode: join_tokens(&tokens),
        });
        known_units.extend(units.iter().cloned());

        // Practice: words over every key learned so far.
        let pool: Vec<&String> = words
            .iter()
            .filter(|(_, k)| k.is_subset(set))
            .map(|(w, _)| w)
            .collect();
        let mut tokens = Vec::new();
        for _ in 0..pool.len().min(60) {
            tokens.push(pool[rng.below(pool.len())].clone());
        }
        while tokens.len() < 40 {
            let a = &known_units[rng.below(known_units.len())];
            let b = &known_units[rng.below(known_units.len())];
            tokens.push(format!("{a}{b}"));
        }
        lessons.push(Lesson {
            title: format!("Practice: {} and earlier keys", legends.join(" ")),
            category,
            difficulty,
            unicode: join_tokens(&tokens),
        });
    }
    // Trim to the track size (dropping later practice lessons first), or
    // pad with review lessons over every learned key.
    while lessons.len() > PER_TRACK {
        let last_practice = lessons
            .iter()
            .rposition(|l| l.title.starts_with("Practice"))
            .unwrap_or(lessons.len() - 1);
        lessons.remove(last_practice);
    }
    let full = groups.last().map(|g| g.2.clone()).unwrap_or_default();
    let review: Vec<&String> = words
        .iter()
        .filter(|(_, k)| k.is_subset(&full))
        .map(|(w, _)| w)
        .collect();
    if review.is_empty() && lessons.len() < PER_TRACK {
        bail!("{}: no review vocabulary", layout.id().as_str());
    }
    let mut r = 1;
    while lessons.len() < PER_TRACK {
        let mut tokens = Vec::new();
        for _ in 0..80 {
            tokens.push(review[rng.below(review.len())].clone());
        }
        lessons.push(Lesson {
            title: format!("Review {r}: all learned keys"),
            category: "Review".into(),
            difficulty: 4,
            unicode: join_tokens(&tokens),
        });
        r += 1;
    }
    Ok((lessons, skipped))
}

fn bucket_of(word: &str) -> usize {
    match word.chars().count() {
        0..=3 => 0,
        4..=5 => 1,
        6..=7 => 2,
        8..=9 => 3,
        _ => 4,
    }
}

const BUCKET_LABELS: [&str; 5] = ["3 अक्षर तक", "4-5 अक्षर", "6-7 अक्षर", "8-9 अक्षर", "10+ अक्षर"];

fn classic(typist: &mut Typist, corpus: &Corpus, seed: u64) -> anyhow::Result<Vec<Lesson>> {
    let mut rng = Rng::new(seed);
    let mut buckets: [Vec<String>; 5] = Default::default();
    for w in &corpus.words {
        if typist.strokes(w, true).is_some() {
            buckets[bucket_of(w)].push(w.clone());
        }
    }
    let sentences: Vec<String> = corpus
        .sentences
        .iter()
        .filter(|s| typist.strokes(s, true).is_some())
        .cloned()
        .collect();
    if sentences.len() < 40 {
        bail!("only {} typeable sentences", sentences.len());
    }
    let mut lessons = Vec::new();

    // 1–20: word practice, four lessons per length bucket.
    for (b, label) in BUCKET_LABELS.iter().enumerate() {
        let pool = if buckets[b].is_empty() {
            // Borrow the nearest non-empty bucket rather than inventing words.
            (0..5)
                .filter(|&i| !buckets[i].is_empty())
                .min_by_key(|&i| i.abs_diff(b))
                .map(|i| buckets[i].clone())
                .unwrap_or_default()
        } else {
            buckets[b].clone()
        };
        if pool.is_empty() {
            bail!("no typeable words");
        }
        for n in 1..=4 {
            let tokens: Vec<String> = (0..60)
                .map(|_| pool[rng.below(pool.len())].clone())
                .collect();
            lessons.push(Lesson {
                title: format!("शब्द अभ्यास ({label}) {n}"),
                category: "Word practice".into(),
                difficulty: if b < 2 { 1 } else { 2 },
                unicode: join_tokens(&tokens),
            });
        }
    }

    // 21–60: sentence practice, short to long.
    let mut by_len = sentences.clone();
    by_len.sort_by_key(|s| s.split(' ').count());
    let bands = 40;
    for band in 0..bands {
        let lo = band * by_len.len() / bands;
        let hi = ((band + 1) * by_len.len() / bands).max(lo + 1);
        let slice = &by_len[lo..hi.min(by_len.len())];
        let mut picked: Vec<String> = Vec::new();
        let mut words = 0;
        let mut guard = 0;
        while words < 90 && guard < 400 {
            let s = &slice[rng.below(slice.len())];
            if !picked.contains(s) || picked.len() >= slice.len() {
                words += s.split(' ').count();
                picked.push(s.clone());
            }
            guard += 1;
        }
        lessons.push(Lesson {
            title: format!("वाक्य अभ्यास {}", band + 1),
            category: "Sentence practice".into(),
            difficulty: if band < 14 {
                2
            } else if band < 28 {
                3
            } else {
                4
            },
            unicode: picked.join(" "),
        });
    }

    // 61–100: paragraph practice from consecutive sentences of one passage.
    let mut paragraphs: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for passage in &corpus.passages {
        let ok: Vec<&String> = passage
            .iter()
            .filter(|s| typist.strokes(s, true).is_some())
            .collect();
        let mut start = 0;
        while start < ok.len() {
            let mut words = 0;
            let mut para = Vec::new();
            let mut i = start;
            while i < ok.len() && words < 180 {
                words += ok[i].split(' ').count();
                para.push(ok[i].as_str());
                i += 1;
            }
            if words >= 120 {
                let text = para.join(" ");
                if seen.insert(text.clone()) {
                    paragraphs.push(text);
                }
            }
            start = i.max(start + 1);
        }
    }
    if paragraphs.is_empty() {
        bail!("no paragraph material");
    }
    for n in 0..40 {
        let i = n * paragraphs.len() / 40;
        lessons.push(Lesson {
            title: format!("अनुच्छेद अभ्यास {}", n + 1),
            category: "Paragraph practice".into(),
            difficulty: if n < 20 { 4 } else { 5 },
            unicode: paragraphs[i.min(paragraphs.len() - 1)].clone(),
        });
    }
    Ok(lessons)
}

/// What one product's authoring produced.
pub struct Authored {
    /// The 200 lessons (Beginner 1–100, Classic 101–200), sealed.
    pub exercises: Vec<ImportedExercise>,
    /// Keys the beginner track could not drill (reported).
    pub skipped_keys: Vec<String>,
}

/// Author the 200 Hindi Learn lessons of `layout`.
///
/// `source_texts` are the Unicode Hindi Mastery passages of
/// [`vocabulary_layout`]; `inputs_sha` is the sha256 over the generator
/// inputs (source passages and `CURRICULUM.md`), stored as provenance.
pub fn author(
    layout: LayoutId,
    source_texts: &[&str],
    inputs_sha: &str,
) -> anyhow::Result<Authored> {
    let product = ProductId::HindiLearn(layout);
    let corpus = mine(source_texts);
    let mut typist = Typist::new(layout)?;
    let board = load_layout(layout).context("layout")?;
    let seed = 0x5244_4841_u64 ^ ((layout as u64) << 32);
    let (beginner, skipped_keys) = beginner(&mut typist, board, &corpus, seed)?;
    let classic = classic(&mut typist, &corpus, seed.rotate_left(17))?;
    let mut exercises = Vec::with_capacity(PER_TRACK * 2);
    let mut seen = HashSet::new();
    for (i, (lesson, track)) in beginner
        .into_iter()
        .map(|l| (l, "beginner"))
        .chain(classic.into_iter().map(|l| (l, "classic")))
        .enumerate()
    {
        let number = u32::try_from(i + 1).context("lesson number")?;
        let stored = typist.stored(&lesson.unicode)?;
        typist.check(&stored, &format!("{} lesson {number}", product.slug()))?;
        let _ = seen.insert(sha256_hex(stored.as_bytes()));
        let mut e = ImportedExercise {
            number,
            title: lesson.title,
            category: Some(lesson.category),
            difficulty: Some(lesson.difficulty),
            content: stored,
            source_id: format!("radha-{}-{number:03}", product.slug()),
            status: ContentStatus::GeneratedReviewPending,
            track: Some(track.to_owned()),
            source_path: PIPELINE.to_owned(),
            source_sha256: inputs_sha.to_owned(),
            ..ImportedExercise::default()
        };
        e.seal();
        exercises.push(e);
    }
    if exercises.len() != PER_TRACK * 2 {
        bail!("{}: {} lessons authored", product.slug(), exercises.len());
    }
    Ok(Authored {
        exercises,
        skipped_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "भारत एक विशाल देश है। यहाँ अनेक भाषाएँ बोली जाती हैं। \
        सरकार ने नई योजना बनाई है। हम सब मिलकर काम करते हैं। \
        कार्यालय में सभी कर्मचारी समय पर आते हैं। बच्चे विद्यालय जाते हैं।";

    #[test]
    fn mining_keeps_prose_sentences_and_hindi_words_only() {
        let c = mine(&[TEXT, "abc def ghi। क ख"]);
        assert_eq!(c.sentences.len(), 6);
        assert!(c.sentences.iter().all(|s| s.ends_with('\u{0964}')));
        assert!(c.words.contains(&"भारत".to_owned()));
        assert!(!c.words.iter().any(|w| w.is_ascii()));
    }

    #[test]
    fn strokes_are_found_for_typeable_words_on_every_hindi_layout() {
        for layout in [
            LayoutId::RemingtonGail,
            LayoutId::RemingtonCbi,
            LayoutId::KrutiDev,
        ] {
            let mut t = Typist::new(layout).unwrap();
            let keys = t.strokes("कमल", false).expect("typeable");
            assert!(!keys.is_empty());
            assert!(t.strokes("abc", false).is_none() || layout == LayoutId::KrutiDev);
        }
    }

    #[test]
    fn beginner_order_starts_on_the_home_row_index_fingers() {
        let board = load_layout(LayoutId::RemingtonGail).unwrap();
        let order = stroke_order(board);
        assert_eq!(order[0].0.base_char(), 'f');
        assert_eq!(order[1].0.base_char(), 'j');
        assert!(order.iter().all(|s| s.1 != Layer::AltGr));
    }
}
