//! Content import: Golu exercise JSON to validated Radha content bundles.
//!
//! The pipeline has two halves that meet at [`ContentBundle`]:
//!
//! 1. `tools/import-golu-content` reads the read-only Golu source tree, turns
//!    every `Exercise-NNN.json` into an [`ImportedExercise`] with
//!    [`from_golu_json`], validates the product with [`validate_bundle`] and
//!    writes one bundle file per product under `content/exercises/`.
//! 2. `radha-db` seeds SQLite from a bundle in one transaction, validating it
//!    again first, so a hand-edited or stale bundle can never reach the
//!    database half-checked.
//!
//! Validation is strict and explains itself: non-empty text, NFC for every
//! Unicode layout, the Golu layout matching the product, the script matching
//! the layout (Devanagari for GAIL/CBI, never Devanagari for QWERTY or the
//! Kruti Dev legacy stream), and exercise numbers that run `1..=n` with no gap.
//!
//! Golu's Hindi content is AI-drafted (Golu README); it is imported as-is and
//! flagged [`ContentBundle::provisional`], never rewritten here.

use std::collections::HashSet;
use std::path::Path;

use radha_core::akshar::normalize;
use radha_core::{CoreError, Result};
use radha_layouts::{load_layout, Guide, KeyboardLayout, LayoutId};
use serde::{Deserialize, Serialize};

use crate::codec::sha256_hex;
use crate::{ContentStatus, ContentTarget, ProductId, EXAM_MAX_WORDS};

/// Version of the bundle file format written by this build. Format 1
/// (plain JSON, no status or hashes) is still read.
pub const BUNDLE_FORMAT: u32 = 2;

/// Oldest bundle format still read.
pub const OLDEST_BUNDLE_FORMAT: u32 = 1;

/// Hindi Learn tracks (Beginner | Classic toggle).
pub const TRACKS: [&str; 2] = ["beginner", "classic"];

/// One exercise, ready for SQLite.
///
/// The fields after `source_id` arrived with format 2; they default when a
/// format-1 bundle is read.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ImportedExercise {
    /// 1-based position within the product.
    pub number: u32,
    /// Title shown in the exercise list.
    pub title: String,
    /// Curriculum grouping (Golu `category`).
    pub category: Option<String>,
    /// 1 (foundation) to 5 (expert).
    pub difficulty: Option<u8>,
    /// The text to type: NFC Unicode, or the raw Kruti Dev key stream.
    pub content: String,
    /// The Golu id it came from (provenance).
    pub source_id: String,
    /// Review status.
    #[serde(default)]
    pub status: ContentStatus,
    /// Hindi Learn track (`beginner` / `classic`); `None` elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// sha256 of `content` (UTF-8). Empty only in format-1 bundles.
    #[serde(default)]
    pub content_sha256: String,
    /// Kruti Dev legacy key stream of `content` (Hindi exam pool only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<String>,
    /// sha256 of `legacy`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub legacy_sha256: String,
    /// Source file, relative to its source root (provenance).
    #[serde(default)]
    pub source_path: String,
    /// sha256 of the source file bytes (provenance).
    #[serde(default)]
    pub source_sha256: String,
}

impl ImportedExercise {
    /// Words in `content`.
    #[must_use]
    pub fn word_count(&self) -> usize {
        self.content.split_whitespace().count()
    }

    /// Fill `content_sha256` (and `legacy_sha256`) from the text.
    pub fn seal(&mut self) {
        self.content_sha256 = sha256_hex(self.content.as_bytes());
        self.legacy_sha256 = self
            .legacy
            .as_deref()
            .map(|l| sha256_hex(l.as_bytes()))
            .unwrap_or_default();
    }
}

/// Every exercise of one product.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentBundle {
    /// [`BUNDLE_FORMAT`] of the writer.
    pub format: u32,
    /// [`ProductId::slug`].
    pub product: String,
    /// Where the content came from, e.g. `golu-expert-ai:Hindi/RemingtonGAIL`.
    pub source: String,
    /// The content is a draft awaiting review (Golu's Hindi content).
    pub provisional: bool,
    /// Ordered by number.
    pub exercises: Vec<ImportedExercise>,
}

/// Where a product's content lives in the Golu tree:
/// `content/languages/{language}/{layout}/Exercise-NNN.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoluSource {
    /// Language folder, e.g. `Hindi`.
    pub language: &'static str,
    /// Layout folder and the `layout` field of every file, e.g. `KrutiDev`.
    pub layout: &'static str,
    /// Hindi content is AI-drafted and flagged provisional.
    pub provisional: bool,
}

/// The Golu source of a product, or `None` when Golu has no directly mappable
/// content (Typing Flow, Number + Punctuation, Hindi Learn): those products
/// stay empty and show a "content coming" state instead of invented text.
#[must_use]
pub fn golu_source(product: ProductId) -> Option<GoluSource> {
    let (language, layout, provisional) = match product {
        ProductId::EnglishClassic => ("English", "QWERTY", false),
        ProductId::HindiMastery(LayoutId::RemingtonGail) => ("Hindi", "RemingtonGAIL", true),
        ProductId::HindiMastery(LayoutId::RemingtonCbi) => ("Hindi", "RemingtonCBI", true),
        ProductId::HindiMastery(LayoutId::KrutiDev) => ("Hindi", "KrutiDev", true),
        _ => return None,
    };
    Some(GoluSource {
        language,
        layout,
        provisional,
    })
}

/// Whether imported content for `product` is provisional (AI-drafted).
#[must_use]
pub fn is_provisional(product: ProductId) -> bool {
    golu_source(product).is_some_and(|s| s.provisional)
}

/// Whether `name` is a Golu exercise file (`Exercise-NNN.json`). Report and
/// schema-example files next to them (`_CONVERSION-REPORT.json`, …) are not.
#[must_use]
pub fn is_exercise_file(name: &str) -> bool {
    name.strip_prefix("Exercise-")
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Golu difficulty label to the schema's 1–5 scale.
#[must_use]
pub fn difficulty_level(label: &str) -> Option<u8> {
    match label.trim().to_ascii_lowercase().as_str() {
        "foundation" => Some(1),
        "beginner" => Some(2),
        "intermediate" => Some(3),
        "advanced" => Some(4),
        "expert" => Some(5),
        _ => None,
    }
}

#[derive(Deserialize)]
struct GoluExercise {
    id: Option<String>,
    layout: String,
    exercise_number: u32,
    title: String,
    category: Option<String>,
    difficulty: Option<String>,
    content: GoluContent,
    #[serde(default)]
    review: Option<GoluReview>,
    #[serde(default)]
    conversion: Option<GoluConversion>,
}

#[derive(Deserialize)]
struct GoluContent {
    unicode: String,
}

#[derive(Deserialize)]
struct GoluReview {
    status: Option<String>,
}

#[derive(Deserialize)]
struct GoluConversion {
    source_unicode_sha256: Option<String>,
}

/// The status a Golu Expert AI file earns: provisional when its product is
/// flagged provisional, verified only when the file records an approved
/// review.
fn golu_status(provisional: bool, review: Option<&GoluReview>) -> ContentStatus {
    let approved = review
        .and_then(|r| r.status.as_deref())
        .is_some_and(|s| s.eq_ignore_ascii_case("approved"));
    if approved && !provisional {
        ContentStatus::Verified
    } else {
        ContentStatus::Provisional
    }
}

fn source_path_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Normalise exercise text for `layout`: line endings to `\n`, and NFC for
/// every Unicode layout. The Kruti Dev legacy stream is a key sequence, not
/// Unicode text, so it is left byte-for-byte.
#[must_use]
pub fn normalize_content(layout: LayoutId, raw: &str) -> String {
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    match layout {
        LayoutId::KrutiDev => text,
        LayoutId::Qwerty
        | LayoutId::RemingtonGail
        | LayoutId::RemingtonCbi
        | LayoutId::Inscript => normalize(&text),
    }
}

/// Parse one Golu exercise file for `product`.
///
/// # Errors
///
/// [`CoreError::Malformed`] when the JSON does not parse, the product has no
/// Golu source, or the file's `layout` is not the product's layout.
pub fn from_golu_json(product: ProductId, path: &Path, json: &str) -> Result<ImportedExercise> {
    let source = golu_source(product).ok_or_else(|| {
        CoreError::malformed(
            "exercise",
            path,
            format!("{} has no Golu content source", product.slug()),
        )
    })?;
    from_golu_json_with(product, source, path, json)
}

/// [`from_golu_json`] with an explicit source description, for products
/// whose Expert-AI-schema files arrive from elsewhere (Number + Punctuation,
/// supplied by Golu later).
///
/// # Errors
///
/// As [`from_golu_json`].
pub fn from_golu_json_with(
    product: ProductId,
    source: GoluSource,
    path: &Path,
    json: &str,
) -> Result<ImportedExercise> {
    let golu: GoluExercise = serde_json::from_str(json)
        .map_err(|e| CoreError::malformed("exercise", path, e.to_string()))?;
    if golu.layout != source.layout {
        return Err(CoreError::malformed(
            "exercise",
            path,
            format!(
                "layout {:?} does not match {} ({:?})",
                golu.layout,
                product.slug(),
                source.layout
            ),
        ));
    }
    let mut exercise = ImportedExercise {
        number: golu.exercise_number,
        title: golu.title.trim().to_owned(),
        category: golu
            .category
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty()),
        difficulty: golu.difficulty.as_deref().and_then(difficulty_level),
        content: normalize_content(product.layout(), &golu.content.unicode),
        source_id: golu.id.unwrap_or_default(),
        status: golu_status(source.provisional, golu.review.as_ref()),
        source_path: source_path_of(path),
        source_sha256: sha256_hex(json.as_bytes()),
        ..ImportedExercise::default()
    };
    exercise.seal();
    Ok(exercise)
}

/// Parse one Golu Expert AI exam passage (`content/exams/{Language}/{Layout}`)
/// for `pool`: verifies the source's own `source_unicode_sha256` when it
/// declares one (P04 "sha256 match"), normalises, and trims the passage to
/// at most [`EXAM_MAX_WORDS`] words at a sentence end.
///
/// # Errors
///
/// [`CoreError::Malformed`] when the JSON does not parse, the layout is not
/// `expected_layout`, or the declared hash does not match the text.
pub fn exam_from_golu_json(
    pool: crate::ExamPool,
    expected_layout: &str,
    path: &Path,
    json: &str,
) -> Result<(ImportedExercise, usize)> {
    let golu: GoluExercise = serde_json::from_str(json)
        .map_err(|e| CoreError::malformed("exam passage", path, e.to_string()))?;
    if golu.layout != expected_layout {
        return Err(CoreError::malformed(
            "exam passage",
            path,
            format!("layout {:?} (expected {expected_layout:?})", golu.layout),
        ));
    }
    if let Some(declared) = golu
        .conversion
        .as_ref()
        .and_then(|c| c.source_unicode_sha256.as_deref())
    {
        let actual = sha256_hex(golu.content.unicode.as_bytes());
        if !declared.eq_ignore_ascii_case(&actual) {
            return Err(CoreError::malformed(
                "exam passage",
                path,
                format!("sha256 mismatch: source declares {declared}, text is {actual}"),
            ));
        }
    }
    let target = ContentTarget::Pool(pool);
    let full = normalize_content(target.layout(), &golu.content.unicode);
    let original_words = full.split_whitespace().count();
    let content = trim_to_words(&full, EXAM_MAX_WORDS, sentence_ends(target.layout()));
    let mut exercise = ImportedExercise {
        number: golu.exercise_number,
        title: golu.title.trim().to_owned(),
        category: golu
            .category
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty()),
        difficulty: golu.difficulty.as_deref().and_then(difficulty_level),
        content,
        source_id: golu.id.unwrap_or_default(),
        status: golu_status(pool == crate::ExamPool::Hindi, golu.review.as_ref()),
        source_path: source_path_of(path),
        source_sha256: sha256_hex(json.as_bytes()),
        ..ImportedExercise::default()
    };
    exercise.seal();
    Ok((exercise, original_words))
}

/// Golu Typing OS lesson JSON (`public/content/{mode}/NNN.json`).
#[derive(Deserialize)]
struct OsLesson {
    id: String,
    mode: String,
    title: String,
    #[serde(default)]
    phase: Option<String>,
    order: u32,
    #[serde(default)]
    difficulty: Option<u32>,
    content: String,
    #[serde(default)]
    content_hash: Option<String>,
}

/// Parse one Golu Typing OS lesson for `product` (Typing Flow: mode
/// `smooth`, decision DV-04). Verifies the lesson's `content_hash` (sha256
/// of `content`) before any normalisation.
///
/// # Errors
///
/// [`CoreError::Malformed`] when the JSON does not parse, the mode is not
/// `expected_mode`, or the hash does not match.
pub fn from_os_json(
    product: ProductId,
    expected_mode: &str,
    path: &Path,
    json: &str,
) -> Result<ImportedExercise> {
    let os: OsLesson = serde_json::from_str(json)
        .map_err(|e| CoreError::malformed("lesson", path, e.to_string()))?;
    if os.mode != expected_mode {
        return Err(CoreError::malformed(
            "lesson",
            path,
            format!("mode {:?} (expected {expected_mode:?})", os.mode),
        ));
    }
    if let Some(declared) = os.content_hash.as_deref() {
        let actual = sha256_hex(os.content.as_bytes());
        if !declared.eq_ignore_ascii_case(&actual) {
            return Err(CoreError::malformed(
                "lesson",
                path,
                format!("sha256 mismatch: content_hash {declared}, text is {actual}"),
            ));
        }
    }
    let mut exercise = ImportedExercise {
        number: os.order,
        title: os.title.trim().to_owned(),
        category: os
            .phase
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty()),
        // OS difficulty is 1–10; Radha's schema is 1–5.
        difficulty: os
            .difficulty
            .and_then(|d| u8::try_from(d.div_ceil(2).clamp(1, 5)).ok()),
        content: normalize_content(product.layout(), &os.content),
        source_id: os.id,
        status: ContentStatus::Provisional,
        source_path: source_path_of(path),
        source_sha256: sha256_hex(json.as_bytes()),
        ..ImportedExercise::default()
    };
    exercise.seal();
    Ok(exercise)
}

// --- Radha English Flow exercises (FT-NNN.json, P07) ---------------------------

/// A Radha English Flow exercise file (`FT-001.json` … `FT-100.json`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlowFile {
    exercise_id: String,
    exercise_number: u32,
    title: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    difficulty: Option<String>,
    language: String,
    mode: String,
    #[serde(default)]
    passage_word_count: Option<u32>,
    passage: String,
}

/// Whether `name` is a Flow exercise file (`FT-NNN.json`).
#[must_use]
pub fn is_flow_file(name: &str) -> bool {
    name.strip_prefix("FT-")
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Flow difficulty label to the schema's 1–5 scale.
#[must_use]
pub fn flow_difficulty(label: &str) -> Option<u8> {
    match label.trim().to_ascii_lowercase().as_str() {
        "beginner" => Some(1),
        "elementary" => Some(2),
        "intermediate" => Some(3),
        "upper_intermediate" | "upper-intermediate" => Some(4),
        "advanced" => Some(5),
        _ => None,
    }
}

/// The ASCII spelling of a typographic character, where one is standard:
/// curly quotes, dashes, the ellipsis, a no-break space and accented Latin
/// letters (`café` → `cafe`). `None` keeps the character (and the typability
/// rule then decides).
#[must_use]
pub fn ascii_fold(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => "'",
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => "\"",
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2212}' => "-",
        '\u{2026}' => "...",
        '\u{00A0}' | '\u{2009}' | '\u{202F}' => " ",
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a",
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => "A",
        'ç' => "c",
        'Ç' => "C",
        'è' | 'é' | 'ê' | 'ë' => "e",
        'È' | 'É' | 'Ê' | 'Ë' => "E",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'Ì' | 'Í' | 'Î' | 'Ï' => "I",
        'ñ' => "n",
        'Ñ' => "N",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => "o",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => "O",
        'ù' | 'ú' | 'û' | 'ü' => "u",
        'Ù' | 'Ú' | 'Û' | 'Ü' => "U",
        _ => return None,
    })
}

/// Fold typographic characters of English text to what a US keyboard
/// types (see [`ascii_fold`]). An em dash becomes a spaced hyphen
/// (`checks—loosening` → `checks - loosening`) without doubling any space.
/// Returns the text and every folded character with its count, so the
/// importer can report each change instead of making it silently.
#[must_use]
pub fn fold_typography(text: &str) -> (String, Vec<(char, usize)>) {
    let mut out = String::with_capacity(text.len());
    let mut folded: Vec<(char, usize)> = Vec::new();
    let mut note = |c: char| match folded.iter_mut().find(|(f, _)| *f == c) {
        Some((_, n)) => *n += 1,
        None => folded.push((c, 1)),
    };
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{2014}' || c == '\u{2015}' {
            note(c);
            while out.ends_with(' ') {
                out.pop();
            }
            while chars.peek() == Some(&' ') {
                chars.next();
            }
            let at_line_start = out.is_empty() || out.ends_with('\n');
            let at_line_end = matches!(chars.peek(), None | Some('\n'));
            if !at_line_start {
                out.push(' ');
            }
            out.push('-');
            if !at_line_end {
                out.push(' ');
            }
            continue;
        }
        match ascii_fold(c) {
            Some(s) => {
                note(c);
                out.push_str(s);
            }
            None => out.push(c),
        }
    }
    (out, folded)
}

/// What [`from_flow_json`] changed or noticed in a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowFileNotes {
    /// Typographic characters folded to ASCII, with counts.
    pub folded: Vec<(char, usize)>,
    /// `passageWordCount` as the file declares it.
    pub declared_words: Option<u32>,
}

/// Parse one Radha English Flow exercise file (`FT-NNN.json`) for Typing
/// Flow: checks `language` (`en`), `mode` (`flow_typing`), a non-empty id
/// and passage, and that the id names the file; normalises line endings,
/// folds typography ([`fold_typography`]) and applies NFC.
///
/// # Errors
///
/// [`CoreError::Malformed`] naming the broken rule.
pub fn from_flow_json(path: &Path, json: &str) -> Result<(ImportedExercise, FlowFileNotes)> {
    let f: FlowFile = serde_json::from_str(json)
        .map_err(|e| CoreError::malformed("flow exercise", path, e.to_string()))?;
    let fail = |detail: String| Err(CoreError::malformed("flow exercise", path, detail));
    let id = f.exercise_id.trim().to_owned();
    if id.is_empty() {
        return fail("empty exerciseId".into());
    }
    // The file name, also inside a `label:relative/path` provenance path.
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let name = name.rsplit([':', '/']).next().unwrap_or(name);
    if let Some(stem) = name.strip_suffix(".json").filter(|_| is_flow_file(name)) {
        if stem != id {
            return fail(format!("exerciseId {id:?} does not match the file {stem}"));
        }
    }
    if !f.language.trim().eq_ignore_ascii_case("en") {
        return fail(format!("language {:?} (expected \"en\")", f.language));
    }
    if f.mode.trim() != "flow_typing" {
        return fail(format!("mode {:?} (expected \"flow_typing\")", f.mode));
    }
    if f.passage.trim().is_empty() {
        return fail("empty passage".into());
    }
    let unix = f.passage.replace("\r\n", "\n").replace('\r', "\n");
    let (folded_text, folded) = fold_typography(unix.trim());
    let mut exercise = ImportedExercise {
        number: f.exercise_number,
        title: f.title.trim().to_owned(),
        category: f
            .category
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty()),
        difficulty: f.difficulty.as_deref().and_then(flow_difficulty),
        content: normalize(&folded_text),
        source_id: id,
        status: ContentStatus::Provisional,
        source_path: source_path_of(path),
        source_sha256: sha256_hex(json.as_bytes()),
        ..ImportedExercise::default()
    };
    exercise.seal();
    Ok((
        exercise,
        FlowFileNotes {
            folded,
            declared_words: f.passage_word_count,
        },
    ))
}

/// Characters that end a sentence on `layout`'s script.
#[must_use]
pub fn sentence_ends(layout: LayoutId) -> &'static [char] {
    match layout {
        LayoutId::RemingtonGail | LayoutId::RemingtonCbi | LayoutId::Inscript => {
            &['\u{0964}', '?', '!']
        }
        LayoutId::KrutiDev => &['A', '?', '!'],
        LayoutId::Qwerty => &['.', '?', '!'],
    }
}

/// Cut `text` to at most `max_words` words, at the last sentence end
/// (`ends`) that keeps at least half of them; at exactly `max_words` words
/// when there is none. Text already within the limit is returned unchanged.
#[must_use]
pub fn trim_to_words(text: &str, max_words: usize, ends: &[char]) -> String {
    // Byte offset just past each word.
    let mut word_ends = Vec::new();
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if in_word {
                word_ends.push(i);
            }
            in_word = false;
        } else {
            in_word = true;
        }
    }
    if in_word {
        word_ends.push(text.len());
    }
    if word_ends.len() <= max_words || max_words == 0 {
        return text.to_owned();
    }
    let hard = word_ends[max_words - 1];
    let floor = word_ends[(max_words / 2).saturating_sub(1)];
    let cut = text[..hard]
        .char_indices()
        .rev()
        .find(|&(i, c)| ends.contains(&c) && i + c.len_utf8() >= floor)
        .map_or(hard, |(i, c)| i + c.len_utf8());
    text[..cut].trim_end().to_owned()
}

fn invalid(target: impl Into<ContentTarget>, number: u32, detail: impl Into<String>) -> CoreError {
    CoreError::malformed(
        "exercise",
        format!("{}#{number}", target.into().slug()),
        detail.into(),
    )
}

impl From<ProductId> for ContentTarget {
    fn from(product: ProductId) -> Self {
        ContentTarget::Product(product)
    }
}

/// Every character some key, key sequence or Alt code of `layout` produces,
/// plus space and line break. The P04 "every char typable" rule checks text
/// against this set; whole-akshar plans are reported separately by
/// [`untypeable_units`].
#[must_use]
pub fn typable_chars(layout: &'static dyn KeyboardLayout) -> HashSet<char> {
    let mut set: HashSet<char> = [' ', '\n'].into_iter().collect();
    for entry in layout.table().entries() {
        set.extend(entry.output.chars());
    }
    if let Some(rules) = layout.rules() {
        for lig in rules.ligatures() {
            set.extend(lig.result.chars());
        }
    }
    for code in 0..=u16::from(u8::MAX) {
        if let Some(alt) = layout.alt_code(code) {
            set.insert(alt.ansi);
        }
    }
    // The numpad (GAIL/CBI: Caps Lock off gives ०-९, books 01/03), and the
    // Esc dead-key layer.
    for digit in 0..=9 {
        for caps in [false, true] {
            if let Some(text) = layout.numpad_digit(digit, caps) {
                set.extend(text.chars());
            }
        }
    }
    if let Some(dead) = layout.dead_keys() {
        for entry in dead.entries() {
            set.extend(entry.output.chars());
        }
    }
    set
}

/// Characters of `text` that `layout` cannot produce, in first-seen order.
#[must_use]
pub fn untypeable_chars(layout: LayoutId, text: &str) -> Vec<char> {
    let Ok(l) = load_layout(layout) else {
        return text.chars().take(1).collect();
    };
    let set = typable_chars(l);
    first_seen(text.chars().filter(|c| !set.contains(c)))
}

fn first_seen(chars: impl Iterator<Item = char>) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in chars {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// Unicode characters with no Kruti Dev 010 spelling at all (`ॐ`, `*`):
/// text that is converted to Kruti Dev must contain none.
#[must_use]
pub fn unresolved_for_kruti_dev(unicode: &str) -> Vec<char> {
    let unresolved = radha_layouts::krutidev::unresolved();
    first_seen(
        unicode
            .chars()
            .filter(|c| unresolved.iter().any(|(u, _)| u == c)),
    )
}

/// Characters of a Kruti Dev key stream with no glyph meaning.
#[must_use]
pub fn unknown_legacy_chars(legacy: &str) -> Vec<char> {
    first_seen(
        radha_layouts::krutidev::to_unicode(legacy)
            .unknown
            .into_iter()
            .map(|(_, c)| c),
    )
}

/// Whether a passage of `target` that its layout cannot type is kept but
/// not listed (instead of failing the bundle). Only the Hindi courses: their
/// content predates the verified keyboard books (Remington CBI book 03 has
/// no key for `ॐ` or `॰`; Kruti Dev book 04 registers Shift+4 `$` as a
/// conflict), and the Exercise screen lists a passage only when the layout
/// types every character. English and the exam pools stay strict.
#[must_use]
pub fn hides_untypeable(target: ContentTarget) -> bool {
    matches!(
        target,
        ContentTarget::Product(ProductId::HindiMastery(l) | ProductId::HindiLearn(l))
            if l != LayoutId::Qwerty
    )
}

/// Characters older content uses that a verified keyboard book took off the
/// layout: CBI `ॐ` and `॰` (book 03 has no AltGr layer and Shift+` is
/// `द्य`), Kruti Dev `$` (book 04 KD010-MAP-052, conflict) and `°` (the old
/// Alt+0176 abbreviation sign; book 04 has `॰` as Alt+0241). A passage
/// holding one of them is kept but not listed; any other untypable
/// character still fails the bundle.
#[must_use]
pub fn retired_by_book(layout: LayoutId) -> &'static [char] {
    match layout {
        LayoutId::RemingtonCbi => &['\u{0950}', '\u{0970}'],
        LayoutId::KrutiDev => &['$', '\u{00B0}'],
        LayoutId::Qwerty | LayoutId::RemingtonGail | LayoutId::Inscript => &[],
    }
}

/// Whether `bad` (characters `target` cannot type) may be hidden instead of
/// failing the bundle.
fn hideable(target: ContentTarget, layout: LayoutId, bad: &[char]) -> bool {
    hides_untypeable(target) && bad.iter().all(|c| retired_by_book(layout).contains(c))
}

/// Whether `text` can be typed on `layout` (every character reachable by a
/// key, sequence, dead key, numpad digit or Alt code).
#[must_use]
pub fn typable_on(layout: LayoutId, text: &str) -> bool {
    // The character set is built once per layout: the Exercise screen asks
    // this for every passage of a course.
    static SETS: [std::sync::OnceLock<Option<HashSet<char>>>; 5] = [
        std::sync::OnceLock::new(),
        std::sync::OnceLock::new(),
        std::sync::OnceLock::new(),
        std::sync::OnceLock::new(),
        std::sync::OnceLock::new(),
    ];
    let slot = LayoutId::all().iter().position(|&l| l == layout);
    let set = slot.and_then(|i| SETS.get(i)).and_then(|s| {
        s.get_or_init(|| load_layout(layout).ok().map(typable_chars))
            .as_ref()
    });
    set.is_some_and(|set| text.chars().all(|c| set.contains(&c)))
}

/// Passages of a bundle its own layout cannot type (kept, not listed).
#[must_use]
pub fn hidden_passages(target: ContentTarget, bundle: &ContentBundle) -> Vec<u32> {
    if !hides_untypeable(target) {
        return Vec::new();
    }
    bundle
        .exercises
        .iter()
        .filter(|e| !typable_on(target.layout(), &e.content))
        .map(|e| e.number)
        .collect()
}

/// Check one passage against its target: the structural rules of
/// [`validate_exercise`] plus every-char-typable on each typing layout, no
/// unresolved characters, sha256 match, exam passage length and track.
///
/// # Errors
///
/// [`CoreError::Malformed`] naming the passage and the broken rule.
pub fn validate_entry(target: ContentTarget, exercise: &ImportedExercise) -> Result<()> {
    let n = exercise.number;
    validate_structure(target, exercise)?;
    for layout in target.typing_layouts() {
        let bad = untypeable_chars(layout, &exercise.content);
        if !bad.is_empty() && !hideable(target, layout, &bad) {
            return Err(invalid(
                target,
                n,
                format!("characters not typable on {}: {bad:?}", layout.as_str()),
            ));
        }
    }
    if target.layout() == LayoutId::KrutiDev {
        let unknown = unknown_legacy_chars(&exercise.content);
        if !unknown.is_empty() && !hideable(target, LayoutId::KrutiDev, &unknown) {
            return Err(invalid(
                target,
                n,
                format!("unresolved Kruti Dev codes: {unknown:?}"),
            ));
        }
    }
    if target.has_legacy_form() {
        let unresolved = unresolved_for_kruti_dev(&exercise.content);
        if !unresolved.is_empty() {
            return Err(invalid(
                target,
                n,
                format!("characters Kruti Dev cannot spell: {unresolved:?}"),
            ));
        }
        let Some(legacy) = exercise.legacy.as_deref().filter(|l| !l.trim().is_empty()) else {
            return Err(invalid(target, n, "missing Kruti Dev legacy form"));
        };
        let bad = untypeable_chars(LayoutId::KrutiDev, legacy);
        let unknown = unknown_legacy_chars(legacy);
        if !bad.is_empty() || !unknown.is_empty() {
            return Err(invalid(
                target,
                n,
                format!("legacy form not typable on Kruti Dev: {bad:?} {unknown:?}"),
            ));
        }
        if !exercise.legacy_sha256.is_empty()
            && exercise.legacy_sha256 != sha256_hex(legacy.as_bytes())
        {
            return Err(invalid(target, n, "legacy sha256 mismatch"));
        }
    } else if exercise.legacy.is_some() {
        return Err(invalid(
            target,
            n,
            "a legacy form on a single-encoding target",
        ));
    }
    if !exercise.content_sha256.is_empty()
        && exercise.content_sha256 != sha256_hex(exercise.content.as_bytes())
    {
        return Err(invalid(target, n, "content sha256 mismatch"));
    }
    if matches!(target, ContentTarget::Pool(_)) && exercise.word_count() > EXAM_MAX_WORDS {
        return Err(invalid(
            target,
            n,
            format!("{} words (limit {EXAM_MAX_WORDS})", exercise.word_count()),
        ));
    }
    match (
        &exercise.track,
        target.product().is_some_and(ProductId::is_beginner),
    ) {
        (Some(t), true) if TRACKS.contains(&t.as_str()) => {}
        (None, false) => {}
        (track, _) => {
            return Err(invalid(
                target,
                n,
                format!("track {track:?} not valid here"),
            ));
        }
    }
    Ok(())
}

fn is_devanagari(c: char) -> bool {
    ('\u{0900}'..='\u{097F}').contains(&c)
}

/// Check one exercise against its product (every rule of
/// [`validate_entry`]).
///
/// # Errors
///
/// [`CoreError::Malformed`] naming the exercise and the broken rule.
pub fn validate_exercise(product: ProductId, exercise: &ImportedExercise) -> Result<()> {
    validate_entry(ContentTarget::Product(product), exercise)
}

/// Non-empty text, number, difficulty range, script vs layout, NFC.
fn validate_structure(product: ContentTarget, exercise: &ImportedExercise) -> Result<()> {
    let n = exercise.number;
    if n == 0 {
        return Err(invalid(product, n, "exercise number must be 1 or more"));
    }
    if exercise.title.trim().is_empty() {
        return Err(invalid(product, n, "empty title"));
    }
    if exercise.content.trim().is_empty() {
        return Err(invalid(product, n, "empty content"));
    }
    if exercise.difficulty.is_some_and(|d| !(1..=5).contains(&d)) {
        return Err(invalid(product, n, "difficulty outside 1–5"));
    }
    let layout = product.layout();
    let devanagari = exercise
        .content
        .chars()
        .filter(|&c| is_devanagari(c))
        .count();
    match layout {
        LayoutId::Qwerty | LayoutId::KrutiDev => {
            if devanagari > 0 {
                return Err(invalid(
                    product,
                    n,
                    format!("{devanagari} Devanagari characters in a {layout:?} text"),
                ));
            }
        }
        LayoutId::RemingtonGail | LayoutId::RemingtonCbi | LayoutId::Inscript => {
            // A number-row drill may hold no Devanagari at all, but Latin
            // letters mean a legacy key stream ended up in a Unicode product.
            if exercise.content.chars().any(|c| c.is_ascii_alphabetic()) {
                return Err(invalid(product, n, "Latin letters in a Unicode Hindi text"));
            }
        }
    }
    if layout != LayoutId::KrutiDev && normalize(&exercise.content) != exercise.content {
        return Err(invalid(product, n, "content is not NFC"));
    }
    Ok(())
}

/// Check a whole bundle: format, product, every exercise, and numbers that
/// run `1..=n` in order with no gap or duplicate.
///
/// # Errors
///
/// [`CoreError::Malformed`] for the first broken rule.
pub fn validate_bundle(bundle: &ContentBundle) -> Result<ProductId> {
    match validate_target_bundle(bundle)? {
        ContentTarget::Product(product) => Ok(product),
        ContentTarget::Pool(_) => Err(CoreError::malformed(
            "content bundle",
            &bundle.product,
            "an exam pool, not a product",
        )),
    }
}

/// [`validate_bundle`] for any [`ContentTarget`]: format, every passage
/// ([`validate_entry`]), numbers `1..=n`, and unique source ids.
///
/// # Errors
///
/// [`CoreError::Malformed`] for the first broken rule.
pub fn validate_target_bundle(bundle: &ContentBundle) -> Result<ContentTarget> {
    let target = ContentTarget::from_slug(&bundle.product)?;
    if !(OLDEST_BUNDLE_FORMAT..=BUNDLE_FORMAT).contains(&bundle.format) {
        return Err(CoreError::malformed(
            "content bundle",
            &bundle.product,
            format!("format {} (expected {BUNDLE_FORMAT})", bundle.format),
        ));
    }
    let mut ids: HashSet<&str> = HashSet::new();
    for (i, exercise) in bundle.exercises.iter().enumerate() {
        let expected = u32::try_from(i + 1).unwrap_or(u32::MAX);
        if exercise.number != expected {
            return Err(invalid(
                target,
                exercise.number,
                format!("out of sequence (expected exercise {expected})"),
            ));
        }
        if !exercise.source_id.is_empty() && !ids.insert(&exercise.source_id) {
            return Err(invalid(
                target,
                exercise.number,
                format!("duplicate id {:?}", exercise.source_id),
            ));
        }
        validate_entry(target, exercise)?;
    }
    Ok(target)
}

/// The importer's stricter gate: everything [`validate_target_bundle`]
/// checks, plus the exact count for the target, a non-empty id on every
/// passage, and a sealed sha256 on every passage.
///
/// # Errors
///
/// [`CoreError::Malformed`] naming the broken rule.
pub fn validate_complete(bundle: &ContentBundle) -> Result<ContentTarget> {
    let target = validate_target_bundle(bundle)?;
    let expected = target.expected_count();
    if bundle.exercises.len() != expected {
        return Err(CoreError::malformed(
            "content bundle",
            &bundle.product,
            format!(
                "{} passages (exactly {expected} required)",
                bundle.exercises.len()
            ),
        ));
    }
    if let Some(e) = bundle.exercises.iter().find(|e| e.source_id.is_empty()) {
        return Err(invalid(target, e.number, "missing id"));
    }
    if let Some(e) = bundle
        .exercises
        .iter()
        .find(|e| e.content_sha256.is_empty())
    {
        return Err(invalid(target, e.number, "missing content sha256"));
    }
    Ok(target)
}

/// [`validate_complete`] for a source whose bad files were excluded with a
/// logged reason: every rule except the exact count, which becomes "at
/// least one and at most the target's count".
///
/// # Errors
///
/// [`CoreError::Malformed`] naming the broken rule.
pub fn validate_up_to(bundle: &ContentBundle) -> Result<ContentTarget> {
    let target = validate_target_bundle(bundle)?;
    let n = bundle.exercises.len();
    if n == 0 || n > target.expected_count() {
        return Err(CoreError::malformed(
            "content bundle",
            &bundle.product,
            format!("{n} passages (1 to {} allowed)", target.expected_count()),
        ));
    }
    if let Some(e) = bundle.exercises.iter().find(|e| e.source_id.is_empty()) {
        return Err(invalid(target, e.number, "missing id"));
    }
    if let Some(e) = bundle
        .exercises
        .iter()
        .find(|e| e.content_sha256.is_empty())
    {
        return Err(invalid(target, e.number, "missing content sha256"));
    }
    Ok(target)
}

/// Units of `content` no key sequence on `product`'s layout produces, in
/// first-seen order. Reported by the importer (P04 "every char typable").
#[must_use]
pub fn untypeable_units(product: ProductId, content: &str) -> Vec<String> {
    let Ok(layout) = load_layout(product.layout()) else {
        return vec![content.to_owned()];
    };
    Guide::new(layout, content)
        .untypeable()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// Parse a bundle file.
///
/// # Errors
///
/// [`CoreError::Malformed`] when the JSON does not parse.
pub fn parse_bundle(path: &Path, json: &str) -> Result<ContentBundle> {
    serde_json::from_str(json)
        .map_err(|e| CoreError::malformed("content bundle", path, e.to_string()))
}

/// Serialise a bundle (compact JSON, UTF-8, one file per product).
///
/// # Errors
///
/// [`CoreError::Malformed`] if serialisation fails (it cannot for these
/// types, but the error is surfaced rather than unwrapped).
pub fn bundle_to_json(bundle: &ContentBundle) -> Result<String> {
    serde_json::to_string(bundle)
        .map_err(|e| CoreError::malformed("content bundle", &bundle.product, e.to_string()))
}

/// The legacy (format 1, plain JSON) bundle file name for a product.
#[must_use]
pub fn bundle_file_name(product: ProductId) -> String {
    format!("{}.json", product.slug())
}

/// The shipped, compressed bundle file name for a target.
#[must_use]
pub fn packed_file_name(target: ContentTarget) -> String {
    format!("{}.json.z", target.slug())
}

/// Serialise and compress a bundle for shipping.
///
/// # Errors
///
/// As [`bundle_to_json`].
pub fn pack_bundle(bundle: &ContentBundle) -> Result<Vec<u8>> {
    Ok(crate::codec::pack(&bundle_to_json(bundle)?))
}

/// Parse a bundle file's bytes: compressed (`.json.z`) when `path` says so,
/// plain JSON otherwise.
///
/// # Errors
///
/// [`CoreError::Malformed`] when the bytes do not decode or parse.
pub fn parse_bundle_bytes(path: &Path, bytes: &[u8]) -> Result<ContentBundle> {
    let packed = path.extension().is_some_and(|e| e == "z");
    let json = if packed {
        crate::codec::unpack(bytes)
            .map_err(|e| CoreError::malformed("content bundle", path, e.to_string()))?
    } else {
        String::from_utf8(bytes.to_vec())
            .map_err(|e| CoreError::malformed("content bundle", path, e.to_string()))?
    };
    parse_bundle(path, &json)
}

/// Where a target's shipped bundle is in `dir`: the compressed file, else a
/// legacy plain-JSON one (products only), else `None`.
#[must_use]
pub fn find_bundle_file(dir: &Path, target: ContentTarget) -> Option<std::path::PathBuf> {
    let packed = dir.join(packed_file_name(target));
    if packed.is_file() {
        return Some(packed);
    }
    let legacy = dir.join(format!("{}.json", target.slug()));
    legacy.is_file().then_some(legacy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golu(layout: &str, number: u32, text: &str) -> String {
        serde_json::json!({
            "schema_version": "1.0",
            "id": format!("test-{number}"),
            "layout": layout,
            "exercise_number": number,
            "title": " Placeholder title ",
            "category": "Foundation",
            "difficulty": "intermediate",
            "content": { "unicode": text },
            "metrics": { "char_count": 3, "word_count": 1 }
        })
        .to_string()
    }

    fn exercise(number: u32, content: &str) -> ImportedExercise {
        ImportedExercise {
            number,
            title: format!("Placeholder {number}"),
            category: None,
            difficulty: Some(1),
            content: content.to_owned(),
            source_id: String::new(),
            ..ImportedExercise::default()
        }
    }

    const GAIL: ProductId = ProductId::HindiMastery(LayoutId::RemingtonGail);
    const KRUTI: ProductId = ProductId::HindiMastery(LayoutId::KrutiDev);

    #[test]
    fn only_four_products_have_a_golu_source() {
        let mapped: Vec<&str> = ProductId::all()
            .into_iter()
            .filter(|p| golu_source(*p).is_some())
            .map(ProductId::slug)
            .collect();
        assert_eq!(
            mapped,
            [
                "english-classic",
                "hindi-mastery-gail",
                "hindi-mastery-cbi",
                "hindi-mastery-krutidev"
            ]
        );
        assert!(golu_source(ProductId::TypingFlow).is_none());
        assert!(golu_source(ProductId::HindiLearn(LayoutId::RemingtonGail)).is_none());
        assert!(is_provisional(GAIL) && is_provisional(KRUTI));
        assert!(!is_provisional(ProductId::EnglishClassic));
    }

    #[test]
    fn report_and_schema_files_are_not_exercises() {
        assert!(is_exercise_file("Exercise-001.json"));
        assert!(is_exercise_file("Exercise-200.json"));
        for not in [
            "_CONVERSION-REPORT.json",
            "_SCHEMA-EXAMPLE.json",
            "Exercise-.json",
            "Exercise-01a.json",
            "Exercise-001.txt",
        ] {
            assert!(!is_exercise_file(not), "{not}");
        }
    }

    #[test]
    fn golu_json_is_parsed_normalised_and_layout_checked() {
        let path = Path::new("Exercise-007.json");
        // Precomposed क़ (U+0958) must come out NFC; CRLF becomes LF.
        let json = golu("RemingtonGAIL", 7, "\u{958}र\r\nम");
        let ex = from_golu_json(GAIL, path, &json).unwrap();
        assert_eq!(ex.content, "क\u{93C}र\nम");
        assert_eq!(ex.number, 7);
        assert_eq!(ex.title, "Placeholder title");
        assert_eq!(ex.category.as_deref(), Some("Foundation"));
        assert_eq!(ex.difficulty, Some(3));
        assert_eq!(ex.source_id, "test-7");
        validate_exercise(GAIL, &ex).unwrap();

        let wrong = golu("RemingtonCBI", 7, "कर");
        assert!(matches!(
            from_golu_json(GAIL, path, &wrong),
            Err(CoreError::Malformed { .. })
        ));
        assert!(from_golu_json(ProductId::TypingFlow, path, &wrong).is_err());
        assert!(from_golu_json(GAIL, path, "{not json").is_err());
    }

    #[test]
    fn kruti_dev_stream_is_kept_verbatim() {
        let raw = "dk;Z \u{00A1}\r\n";
        assert_eq!(
            normalize_content(LayoutId::KrutiDev, raw),
            "dk;Z \u{00A1}\n"
        );
    }

    #[test]
    fn validation_rejects_each_broken_rule() {
        let e = ProductId::EnglishClassic;
        validate_exercise(e, &exercise(1, "the cat")).unwrap();
        assert!(validate_exercise(e, &exercise(0, "the cat")).is_err());
        assert!(validate_exercise(e, &exercise(1, "   ")).is_err(), "empty");
        let mut untitled = exercise(1, "the cat");
        untitled.title = " ".into();
        assert!(validate_exercise(e, &untitled).is_err(), "title");
        assert!(
            validate_exercise(e, &exercise(1, "the कत")).is_err(),
            "Devanagari in English"
        );
        assert!(
            validate_exercise(KRUTI, &exercise(1, "क")).is_err(),
            "Unicode in the legacy stream"
        );
        validate_exercise(KRUTI, &exercise(1, "dk;Z")).unwrap();
        assert!(
            validate_exercise(GAIL, &exercise(1, "dk;Z")).is_err(),
            "legacy stream in a Unicode product"
        );
        // U+0958 is a composition exclusion: its NFC form is क + nukta.
        assert!(
            validate_exercise(GAIL, &exercise(1, "\u{958}")).is_err(),
            "not NFC"
        );
        validate_exercise(GAIL, &exercise(1, &normalize("\u{958}"))).unwrap();
    }

    #[test]
    fn bundles_must_number_one_to_n_and_round_trip() {
        let bundle = ContentBundle {
            format: BUNDLE_FORMAT,
            product: "english-classic".into(),
            source: "test".into(),
            provisional: false,
            exercises: vec![exercise(1, "a b"), exercise(2, "c d")],
        };
        assert_eq!(validate_bundle(&bundle).unwrap(), ProductId::EnglishClassic);
        let json = bundle_to_json(&bundle).unwrap();
        assert_eq!(parse_bundle(Path::new("x"), &json).unwrap(), bundle);

        let mut gap = bundle.clone();
        gap.exercises[1].number = 3;
        assert!(validate_bundle(&gap).is_err());
        let mut dup = bundle.clone();
        dup.exercises[1].number = 1;
        assert!(validate_bundle(&dup).is_err());
        let mut unknown = bundle.clone();
        unknown.product = "not-a-product".into();
        assert!(validate_bundle(&unknown).is_err());
        let mut future = bundle;
        future.format = 99;
        assert!(validate_bundle(&future).is_err());
    }

    fn sealed(number: u32, content: &str) -> ImportedExercise {
        let mut e = exercise(number, content);
        e.source_id = format!("id-{number}");
        e.seal();
        e
    }

    fn bundle_of(target: ContentTarget, exercises: Vec<ImportedExercise>) -> ContentBundle {
        ContentBundle {
            format: BUNDLE_FORMAT,
            product: target.slug().into(),
            source: "test".into(),
            provisional: false,
            exercises,
        }
    }

    const ENGLISH: ContentTarget = ContentTarget::Product(ProductId::EnglishClassic);
    const HINDI_POOL: ContentTarget = ContentTarget::Pool(crate::ExamPool::Hindi);

    #[test]
    fn abort_rule_exact_count() {
        let full: Vec<_> = (1..=200).map(|n| sealed(n, "the cat sat")).collect();
        validate_complete(&bundle_of(ENGLISH, full.clone())).unwrap();
        let short = bundle_of(ENGLISH, full[..199].to_vec());
        let err = validate_complete(&short).unwrap_err().to_string();
        assert!(err.contains("exactly 200"), "{err}");
        // The structural gate alone does not demand the count.
        validate_target_bundle(&short).unwrap();
    }

    #[test]
    fn abort_rule_unique_ids() {
        let mut list = vec![sealed(1, "a b"), sealed(2, "c d")];
        list[1].source_id = list[0].source_id.clone();
        let err = validate_target_bundle(&bundle_of(ENGLISH, list.clone()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("duplicate id"), "{err}");
        list[1].source_id.clear();
        validate_target_bundle(&bundle_of(ENGLISH, list.clone())).unwrap();
        let mut full: Vec<_> = (1..=200).map(|n| sealed(n, "the cat")).collect();
        full[7].source_id.clear();
        assert!(validate_complete(&bundle_of(ENGLISH, full))
            .unwrap_err()
            .to_string()
            .contains("missing id"));
    }

    #[test]
    fn abort_rule_every_char_typable() {
        // A curly quote has no QWERTY key.
        let err = validate_entry(ENGLISH, &sealed(1, "it\u{2019}s"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not typable on qwerty"), "{err}");
        // The Hindi pool must be typable on both GAIL and CBI.
        assert!(untypeable_chars(LayoutId::RemingtonGail, "भारत देश।").is_empty());
        assert_eq!(untypeable_chars(LayoutId::Qwerty, "a€b€"), ['€']);
        // A Kruti Dev stream with a code no key produces.
        assert!(validate_entry(KRUTI.into(), &sealed(1, "dk;Z \u{00A5}")).is_ok());
        assert!(validate_entry(KRUTI.into(), &sealed(1, "dk;Z \u{20AC}")).is_err());
        // A character a keyboard book retired is kept (and not listed);
        // only on the Hindi courses, never on an exam pool.
        assert!(validate_entry(KRUTI.into(), &sealed(1, "dk;Z $")).is_ok());
        assert!(validate_entry(KRUTI.into(), &sealed(1, "dk;Z $ \u{20AC}")).is_err());
        let cbi = ContentTarget::Product(ProductId::HindiMastery(LayoutId::RemingtonCbi));
        assert!(validate_entry(cbi, &sealed(1, "ॐ नमः")).is_ok());
        assert!(validate_entry(cbi, &sealed(1, "ॐ ₹")).is_err());
    }

    #[test]
    fn abort_rule_no_unresolved_chars() {
        assert_eq!(unresolved_for_kruti_dev("ॐ नमः * भारत"), ['ॐ', '*']);
        let mut e = sealed(1, "ॐ भारत");
        e.legacy = Some("Hkkjr".into());
        e.seal();
        let err = validate_entry(HINDI_POOL, &e).unwrap_err().to_string();
        assert!(
            err.contains("Kruti Dev cannot spell") || err.contains("not typable"),
            "{err}"
        );
        let mut ok = sealed(1, "भारत");
        ok.legacy = Some(radha_layouts::krutidev::to_legacy("भारत").legacy);
        ok.seal();
        validate_entry(HINDI_POOL, &ok).unwrap();
        let mut missing = ok.clone();
        missing.legacy = None;
        missing.seal();
        assert!(validate_entry(HINDI_POOL, &missing)
            .unwrap_err()
            .to_string()
            .contains("legacy form"));
    }

    #[test]
    fn abort_rule_nfc() {
        let e = sealed(1, "\u{958}");
        assert!(validate_entry(GAIL.into(), &e)
            .unwrap_err()
            .to_string()
            .contains("not NFC"));
    }

    #[test]
    fn abort_rule_sha256_match() {
        let mut e = sealed(1, "the cat");
        e.content.push('s');
        assert!(validate_entry(ENGLISH, &e)
            .unwrap_err()
            .to_string()
            .contains("sha256 mismatch"));
        // A source that declares a hash must match it.
        let json = serde_json::json!({
            "id": "SMOOTH-001", "mode": "smooth", "title": "T", "order": 1,
            "content": "find your keys", "content_hash": sha256_hex(b"other text")
        })
        .to_string();
        let err = from_os_json(
            ProductId::TypingFlow,
            "smooth",
            Path::new("001.json"),
            &json,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("sha256 mismatch"), "{err}");
        let good = json.replace(&sha256_hex(b"other text"), &sha256_hex(b"find your keys"));
        let flow = from_os_json(
            ProductId::TypingFlow,
            "smooth",
            Path::new("001.json"),
            &good,
        )
        .unwrap();
        assert_eq!(flow.content_sha256, sha256_hex(b"find your keys"));
        assert_eq!(flow.source_id, "SMOOTH-001");
    }

    #[test]
    fn exam_passages_are_limited_and_tracks_belong_to_hindi_learn() {
        let long = "word ".repeat(EXAM_MAX_WORDS + 1);
        let pool = ContentTarget::Pool(crate::ExamPool::English);
        assert!(validate_entry(pool, &sealed(1, long.trim())).is_err());
        let mut tracked = sealed(1, "a b");
        tracked.track = Some("beginner".into());
        assert!(validate_entry(ENGLISH, &tracked).is_err());
        let learn = ContentTarget::Product(ProductId::HindiLearn(LayoutId::RemingtonGail));
        let mut lesson = sealed(1, "कमल");
        assert!(validate_entry(learn, &lesson).is_err(), "track required");
        lesson.track = Some("classic".into());
        validate_entry(learn, &lesson).unwrap();
        lesson.track = Some("expert".into());
        assert!(validate_entry(learn, &lesson).is_err());
    }

    #[test]
    fn trimming_cuts_at_a_sentence_end_within_the_limit() {
        let text = "One two three. Four five six. Seven eight nine ten.";
        assert_eq!(
            trim_to_words(text, 7, &['.']),
            "One two three. Four five six."
        );
        assert_eq!(
            trim_to_words(text, 50, &['.']),
            text,
            "short text unchanged"
        );
        // No sentence end in reach: hard cut at the word limit.
        assert_eq!(trim_to_words("a b c d e f", 4, &['.']), "a b c d");
        // 900 words, so the cut really happens (600 exactly is returned as-is).
        let hindi = "राम घर गया। सीता वन गई। ".repeat(150);
        let cut = trim_to_words(&hindi, 600, sentence_ends(LayoutId::RemingtonGail));
        assert!(cut.split_whitespace().count() <= 600);
        assert!(cut.ends_with('\u{0964}'));
    }

    #[test]
    fn line_endings_and_nfc_are_normalised_on_every_unicode_layout() {
        assert_eq!(normalize_content(LayoutId::Qwerty, "a\r\nb\rc"), "a\nb\nc");
        assert_eq!(
            normalize_content(LayoutId::RemingtonCbi, "\u{958}\r\n"),
            "क\u{93C}\n"
        );
    }

    #[test]
    fn packed_bundles_round_trip_and_format_one_is_still_read() {
        let bundle = bundle_of(ENGLISH, vec![sealed(1, "a b"), sealed(2, "c d")]);
        let bytes = pack_bundle(&bundle).unwrap();
        let path = Path::new("english-classic.json.z");
        assert_eq!(parse_bundle_bytes(path, &bytes).unwrap(), bundle);
        // A format-1 bundle (no status, no hashes) parses with defaults.
        let v1 = r#"{"format":1,"product":"english-classic","source":"s","provisional":false,
            "exercises":[{"number":1,"title":"T","category":null,"difficulty":1,
            "content":"a b","source_id":""}]}"#;
        let old = parse_bundle_bytes(Path::new("english-classic.json"), v1.as_bytes()).unwrap();
        assert_eq!(old.exercises[0].status, ContentStatus::Provisional);
        validate_bundle(&old).unwrap();
    }

    #[test]
    fn untypeable_units_are_reported_per_layout() {
        assert!(untypeable_units(ProductId::EnglishClassic, "Hello, world!").is_empty());
        assert_eq!(
            untypeable_units(ProductId::EnglishClassic, "it\u{2019}s"),
            ["\u{2019}"]
        );
        assert!(untypeable_units(GAIL, "भारत").is_empty());
    }

    fn flow_json(id: &str, number: u32, passage: &str) -> String {
        serde_json::json!({
            "exerciseId": id,
            "exerciseNumber": number,
            "title": " The Quiet Start ",
            "category": "Flow Typing Foundations",
            "difficulty": "upper_intermediate",
            "language": "en",
            "mode": "flow_typing",
            "passageWordCount": 4,
            "passage": passage
        })
        .to_string()
    }

    const FLOW: ContentTarget = ContentTarget::Product(ProductId::TypingFlow);

    #[test]
    fn typography_folds_to_what_a_us_keyboard_types_and_is_reported() {
        let (text, folded) =
            fold_typography("checks\u{2014}loosening it\u{2019}s a caf\u{e9} fa\u{e7}ade");
        assert_eq!(text, "checks - loosening it's a cafe facade");
        assert_eq!(
            folded,
            [
                ('\u{2014}', 1),
                ('\u{2019}', 1),
                ('\u{e9}', 1),
                ('\u{e7}', 1)
            ]
        );
        // A spaced em dash never doubles a space; line edges get no space.
        assert_eq!(fold_typography("a \u{2014} b").0, "a - b");
        assert_eq!(fold_typography("\u{2014}a\nb\u{2014}\nc").0, "- a\nb -\nc");
        assert_eq!(fold_typography("“quoted”…").0, "\"quoted\"...");
        // Unknown characters are kept for the typability rule to judge.
        assert_eq!(fold_typography("€5").0, "€5");
        assert!(fold_typography("plain").1.is_empty());
    }

    #[test]
    fn flow_files_parse_fold_and_validate() {
        let path = Path::new("FT-007.json");
        let (e, notes) = from_flow_json(
            path,
            &flow_json("FT-007", 7, "One\u{2019}s steady\r\nhands\u{2014}calm."),
        )
        .unwrap();
        assert_eq!(e.content, "One's steady\nhands - calm.");
        assert_eq!(e.number, 7);
        assert_eq!(e.title, "The Quiet Start");
        assert_eq!(e.difficulty, Some(4));
        assert_eq!(e.source_id, "FT-007");
        assert_eq!(e.status, ContentStatus::Provisional);
        assert_eq!(e.source_path, "FT-007.json");
        assert_eq!(e.content_sha256, sha256_hex(e.content.as_bytes()));
        assert_eq!(notes.declared_words, Some(4));
        assert_eq!(notes.folded.len(), 2);
        validate_entry(FLOW, &e).unwrap();
        assert!(is_flow_file("FT-100.json") && !is_flow_file("FT-1a.json"));
        assert!(!is_flow_file("Exercise-001.json"));
    }

    #[test]
    fn bad_flow_files_are_rejected_with_a_reason() {
        let path = Path::new("FT-001.json");
        let reason = |json: &str| from_flow_json(path, json).unwrap_err().to_string();
        assert!(reason(&flow_json("FT-002", 1, "text")).contains("does not match"));
        assert!(reason(&flow_json("", 1, "text")).contains("empty exerciseId"));
        assert!(reason(&flow_json("FT-001", 1, "  \n ")).contains("empty passage"));
        assert!(
            reason(&flow_json("FT-001", 1, "text").replace("\"en\"", "\"hi\""))
                .contains("language")
        );
        assert!(
            reason(&flow_json("FT-001", 1, "text").replace("flow_typing", "smooth"))
                .contains("mode")
        );
        assert!(reason("{not json").contains("flow exercise"));
        // A character with no QWERTY key survives the fold and fails typability.
        let (e, _) = from_flow_json(path, &flow_json("FT-001", 1, "costs €5")).unwrap();
        assert!(validate_entry(FLOW, &e)
            .unwrap_err()
            .to_string()
            .contains("not typable"));
    }

    #[test]
    fn up_to_accepts_a_short_flow_bundle_but_not_an_empty_or_oversized_one() {
        let some: Vec<_> = (1..=98).map(|n| sealed(n, "the cat sat")).collect();
        assert_eq!(validate_up_to(&bundle_of(FLOW, some)).unwrap(), FLOW);
        assert!(validate_up_to(&bundle_of(FLOW, Vec::new())).is_err());
        let many: Vec<_> = (1..=101).map(|n| sealed(n, "the cat sat")).collect();
        assert!(validate_up_to(&bundle_of(FLOW, many))
            .unwrap_err()
            .to_string()
            .contains("1 to 100"));
    }
}
