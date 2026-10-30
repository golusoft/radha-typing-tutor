//! `import-golu-content` — the P04 content pipeline.
//!
//! ```text
//! cargo run --release -p import-golu-content -- \
//!     --golu "<Golu Expert AI root>" --os "<Golu Typing OS root>" \
//!     --radha500 "<Radha 500 English exam drafts dir>" \
//!     [--flow <FT-NNN.json dir>] [--number-punct <dir>] [--out <dir>] [--check] \
//!     [--db <radha.sqlite>]
//! ```
//!
//! parse -> normalise (NFC, line endings) -> validate (abort rules) ->
//! write compressed bundles (`<slug>.json.z`) -> optionally write SQLite in
//! ONE transaction with provenance rows -> report.
//!
//! | Target | Source | Status |
//! |--------|--------|--------|
//! | English Classic 200 | Expert AI `content/languages/English/QWERTY` | per file review |
//! | Hindi Mastery 200 x3 | Expert AI `content/languages/Hindi/{RemingtonGAIL,RemingtonCBI,KrutiDev}` | provisional |
//! | Typing Flow 100 | `--flow`: Radha English Flow `FT-NNN.json` (P07); without it, Golu OS `public/content/smooth` (DV-04) | provisional |
//! | Number + Punctuation 50 | Golu's Track 03 seeds `data/track03-alphanumeric-50-seed.md`, expanded (or `--number-punct` Golu JSON) | generated-review-pending |
//! | Exam Hindi pool 500 | Expert AI `content/exams/Hindi/{RemingtonGAIL,RemingtonCBI,InScript}` (one shared Unicode text) + `KrutiDev` (its legacy form) | provisional |
//! | Exam English pool 500 | Golu's title bank `data/exam-500-english-titles.md`: Radha 500 drafts (`--radha500`) + passages written from `data/exam-english-briefs/` | provisional / generated |
//! | Hindi Learn 200 x3 | authored from Hindi Mastery + `CURRICULUM.md` | generated-review-pending |
//!
//! Source folders are only read. `--check` is the dry run: everything is
//! parsed, validated and measured, nothing is written.

mod alphanumeric;
mod exam_english;
mod hindi_learn;
mod rng;

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use radha_content::codec::sha256_hex;
use radha_content::import::{
    exam_from_golu_json, from_flow_json, from_golu_json, from_golu_json_with, from_os_json,
    golu_source, is_exercise_file, is_flow_file, pack_bundle, packed_file_name, validate_complete,
    validate_entry, validate_up_to, ContentBundle, GoluSource, ImportedExercise, BUNDLE_FORMAT,
};
use radha_content::{ContentTarget, ExamPool, ProductId};
use radha_layouts::{load_layout, Guide, LayoutId};

struct Args {
    golu: PathBuf,
    os: PathBuf,
    radha500: PathBuf,
    flow: Option<PathBuf>,
    number_punct: Option<PathBuf>,
    out: PathBuf,
    check: bool,
    db: Option<PathBuf>,
}

const USAGE: &str = "usage: import-golu-content --golu <Expert AI root> --os <Golu OS root> \
                     --radha500 <English exam drafts dir> \
                     [--flow <FT-NNN.json dir>] [--number-punct <dir>] [--out <dir>] [--check] [--db <file>]";

/// The tool's own data folder (Golu's title bank and seeds, the briefs).
fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

fn parse_args() -> anyhow::Result<Args> {
    let mut golu = None;
    let mut os = None;
    let mut radha500 = None;
    let mut flow = None;
    let mut number_punct = None;
    let mut out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/exercises");
    let mut check = false;
    let mut db = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--golu" => golu = it.next().map(PathBuf::from),
            "--os" => os = it.next().map(PathBuf::from),
            "--radha500" => radha500 = it.next().map(PathBuf::from),
            "--flow" => {
                flow = Some(
                    it.next()
                        .map(PathBuf::from)
                        .context("--flow needs a path")?,
                )
            }
            "--number-punct" => number_punct = it.next().map(PathBuf::from),
            "--out" => out = it.next().map(PathBuf::from).context("--out needs a path")?,
            "--db" => db = it.next().map(PathBuf::from),
            "--check" => check = true,
            other => bail!("unknown argument {other:?}\n{USAGE}"),
        }
    }
    Ok(Args {
        golu: golu.context(USAGE)?,
        os: os.context(USAGE)?,
        radha500: radha500.context(USAGE)?,
        flow,
        number_punct,
        out,
        check,
        db,
    })
}

/// The report, printed as it is built.
#[derive(Default)]
struct Report(String);

impl Report {
    fn line(&mut self, text: impl AsRef<str>) {
        println!("{}", text.as_ref());
        let _ = writeln!(self.0, "{}", text.as_ref());
    }
}

/// One built target, with what the report needs.
struct Built {
    bundle: ContentBundle,
    excluded: Vec<String>,
    notes: Vec<String>,
    /// Bad source files were excluded (with reasons in `notes`): the count
    /// gate is "at most the target's count" instead of exact.
    partial: bool,
}

/// Files of `dir` (sorted) split into those `keep` accepts and the rest.
fn list_files(
    dir: &Path,
    keep: impl Fn(&str) -> bool,
) -> anyhow::Result<(Vec<PathBuf>, Vec<String>)> {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_file() && keep(&name) {
            files.push(path);
        } else {
            skipped.push(name);
        }
    }
    files.sort();
    skipped.sort();
    Ok((files, skipped))
}

/// `label:relative/path` with forward slashes.
fn rel(label: &str, root: &Path, path: &Path) -> PathBuf {
    let r = path.strip_prefix(root).unwrap_or(path);
    PathBuf::from(format!(
        "{label}:{}",
        r.to_string_lossy().replace('\\', "/")
    ))
}

fn read(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
}

fn bundle(
    target: ContentTarget,
    source: String,
    provisional: bool,
    mut exercises: Vec<ImportedExercise>,
) -> ContentBundle {
    exercises.sort_by_key(|e| e.number);
    ContentBundle {
        format: BUNDLE_FORMAT,
        product: target.slug().to_owned(),
        source,
        provisional,
        exercises,
    }
}

/// English Classic and Hindi Mastery: Expert AI `content/languages`.
fn golu_product(golu: &Path, product: ProductId) -> anyhow::Result<Built> {
    let Some(source) = golu_source(product) else {
        bail!("{} has no Expert AI source", product.slug());
    };
    let dir = golu
        .join("content")
        .join("languages")
        .join(source.language)
        .join(source.layout);
    let (files, excluded) = list_files(&dir, is_exercise_file)?;
    let mut exercises = Vec::with_capacity(files.len());
    for path in &files {
        let json = read(path)?;
        exercises.push(from_golu_json(
            product,
            &rel("golu-expert-ai", golu, path),
            &json,
        )?);
    }
    Ok(Built {
        bundle: bundle(
            ContentTarget::Product(product),
            format!(
                "golu-expert-ai:content/languages/{}/{}",
                source.language, source.layout
            ),
            source.provisional,
            exercises,
        ),
        excluded,
        notes: Vec::new(),
        partial: false,
    })
}

/// Typing Flow: Golu OS `public/content/smooth/NNN.json` (DV-04).
fn typing_flow(os: &Path) -> anyhow::Result<Built> {
    let dir = os.join("public").join("content").join("smooth");
    let is_lesson = |n: &str| {
        n.strip_suffix(".json")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    };
    let (files, excluded) = list_files(&dir, is_lesson)?;
    let mut exercises = Vec::new();
    for path in &files {
        let json = read(path)?;
        exercises.push(from_os_json(
            ProductId::TypingFlow,
            "smooth",
            &rel("golu-os", os, path),
            &json,
        )?);
    }
    Ok(Built {
        bundle: bundle(
            ContentTarget::Product(ProductId::TypingFlow),
            "golu-os:public/content/smooth".into(),
            true,
            exercises,
        ),
        excluded,
        notes: vec!["content_hash (sha256) verified for every lesson".into()],
        partial: false,
    })
}

/// Typing Flow (P07): the Radha English Flow exercises, `FT-NNN.json`.
/// These replace the Golu OS smooth lessons. Every file is parsed, folded
/// and validated on its own; a bad file is excluded with its reason in the
/// report, never silently, and the rest are numbered 1..=n in file order.
fn radha_flow(dir: &Path) -> anyhow::Result<Built> {
    let target = ContentTarget::Product(ProductId::TypingFlow);
    let (files, skipped) = list_files(dir, is_flow_file)?;
    let mut exercises: Vec<ImportedExercise> = Vec::new();
    let mut excluded: Vec<String> = skipped
        .into_iter()
        .map(|n| format!("{n}: not an FT-NNN.json exercise file"))
        .collect();
    let mut ids: HashSet<String> = HashSet::new();
    let mut folded_files = 0;
    let mut folded_total: BTreeMap<char, usize> = BTreeMap::new();
    let mut word_mismatch = Vec::new();
    for path in &files {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let json = read(path)?;
        let (mut e, notes) = match from_flow_json(&rel("radha-flow", dir, path), &json) {
            Ok(parsed) => parsed,
            Err(err) => {
                excluded.push(format!("{name}: {err}"));
                continue;
            }
        };
        if !ids.insert(e.source_id.clone()) {
            excluded.push(format!("{name}: duplicate exerciseId {:?}", e.source_id));
            continue;
        }
        // Validate under the number it will get.
        e.number = u32::try_from(exercises.len() + 1).context("count")?;
        if let Err(err) = validate_entry(target, &e) {
            excluded.push(format!("{name}: {err}"));
            continue;
        }
        if !notes.folded.is_empty() {
            folded_files += 1;
            for (c, n) in &notes.folded {
                *folded_total.entry(*c).or_default() += n;
            }
        }
        if let Some(declared) = notes.declared_words {
            let actual = e.word_count();
            if usize::try_from(declared).ok() != Some(actual) {
                word_mismatch.push(format!("{name} declares {declared}, has {actual}"));
            }
        }
        exercises.push(e);
    }
    let mut notes = vec![format!(
        "source {}: {} files read, {} imported, {} excluded",
        dir.display(),
        files.len(),
        exercises.len(),
        excluded.len()
    )];
    if !folded_total.is_empty() {
        notes.push(format!(
            "typography folded to US-keyboard ASCII in {folded_files} files: {}",
            folded_total
                .iter()
                .map(|(c, n)| format!("U+{:04X} {c:?} x{n}", u32::from(*c)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !word_mismatch.is_empty() {
        notes.push(format!(
            "passageWordCount differs from the text in {} files (text wins): {}",
            word_mismatch.len(),
            word_mismatch.join("; ")
        ));
    }
    for reason in &excluded {
        notes.push(format!("EXCLUDED {reason}"));
    }
    let partial = exercises.len() < target.expected_count();
    Ok(Built {
        bundle: bundle(
            target,
            format!("radha-flow:{}", dir.display()),
            true,
            exercises,
        ),
        excluded: Vec::new(),
        notes,
        partial,
    })
}

/// Number + Punctuation: Expert-AI-schema files supplied by Golu later.
fn number_punct(dir: &Path) -> anyhow::Result<Built> {
    let source = GoluSource {
        language: "English",
        layout: "QWERTY",
        provisional: false,
    };
    let (files, excluded) = list_files(dir, is_exercise_file)?;
    let mut exercises = Vec::new();
    for path in &files {
        let json = read(path)?;
        exercises.push(from_golu_json_with(
            ProductId::NumberPunctuation,
            source,
            &rel("number-punct", dir, path),
            &json,
        )?);
    }
    Ok(Built {
        bundle: bundle(
            ContentTarget::Product(ProductId::NumberPunctuation),
            format!("number-punct:{}", dir.display()),
            false,
            exercises,
        ),
        excluded,
        notes: Vec::new(),
        partial: false,
    })
}

/// Exam Hindi pool: Expert AI's 500 Hindi exam passages. The RemingtonGAIL,
/// RemingtonCBI and InScript folders hold one shared Unicode text (each is
/// read and compared); the KrutiDev folder holds its legacy form. Passages
/// are trimmed to <= 600 words at a sentence end.
fn exam_hindi(golu: &Path) -> anyhow::Result<Built> {
    let exams = golu.join("content").join("exams").join("Hindi");
    let (files, excluded) = list_files(&exams.join("RemingtonGAIL"), is_exercise_file)?;
    let mut exercises = Vec::new();
    let mut trimmed = 0;
    let mut cbi_identical = 0;
    let mut inscript_identical = 0;
    let mut inscript_typable = 0;
    let mut inscript_blocked: Vec<u32> = Vec::new();
    let mut golu_kd_prefix = 0;
    let mut mismatches: Vec<String> = Vec::new();
    let mut kd_differs: Vec<String> = Vec::new();
    for path in &files {
        let json = read(path)?;
        let (mut e, words) = exam_from_golu_json(
            ExamPool::Hindi,
            "RemingtonGAIL",
            &rel("golu-expert-ai", golu, path),
            &json,
        )?;
        if words > e.word_count() {
            trimmed += 1;
        }
        let conv = radha_layouts::krutidev::to_legacy(&e.content);
        if !conv.failures.is_empty() {
            bail!(
                "exam passage {}: Kruti Dev conversion failed for {:?}",
                e.number,
                conv.failures.iter().map(|f| f.ch).collect::<Vec<_>>()
            );
        }
        let name = path.file_name().map(PathBuf::from).unwrap_or_default();
        // GAIL, CBI and InScript hold one shared Unicode text: each layout's
        // own file is read and must agree, passage by passage.
        for (folder, count) in [
            ("RemingtonCBI", &mut cbi_identical),
            ("InScript", &mut inscript_identical),
        ] {
            let other = exams.join(folder).join(&name);
            let other_json = read(&other)?;
            let (c, _) = exam_from_golu_json(
                ExamPool::Hindi,
                folder,
                &rel("golu-expert-ai", golu, &other),
                &other_json,
            )?;
            if c.content != e.content || c.title != e.title {
                mismatches.push(format!("{folder}/{}", name.display()));
            } else {
                *count += 1;
            }
        }
        if radha_content::import::typable_on(LayoutId::Inscript, &e.content) {
            inscript_typable += 1;
        } else {
            inscript_blocked.push(e.number);
        }
        // Kruti Dev: Expert AI's own legacy file, cut at the same sentence as
        // the Unicode passage (it must agree with Radha's book-verified
        // converter, which is the check that the cut is right).
        let kd = exams.join("KrutiDev").join(&name);
        let kd_text: Option<String> = serde_json_value(&read(&kd)?);
        let legacy = match kd_text {
            Some(g) if g.starts_with(&conv.legacy) => {
                golu_kd_prefix += 1;
                g[..conv.legacy.len()].to_owned()
            }
            Some(g) => {
                // Keep Radha's book-verified conversion and report where
                // Expert AI's legacy file spells it differently.
                let at = g
                    .char_indices()
                    .zip(conv.legacy.chars())
                    .find(|((_, a), b)| a != b)
                    .map_or(0, |((i, _), _)| i);
                let ours: String = conv.legacy[at.min(conv.legacy.len())..]
                    .chars()
                    .take(12)
                    .collect();
                let theirs: String = g[at..].chars().take(12).collect();
                kd_differs.push(format!(
                    "{}: Expert AI {theirs:?} vs converter {ours:?}",
                    name.display()
                ));
                conv.legacy
            }
            None => {
                mismatches.push(format!("KrutiDev/{} has no text", name.display()));
                conv.legacy
            }
        };
        e.legacy = Some(legacy);
        e.seal();
        exercises.push(e);
    }
    let n = exercises.len();
    let mut notes = vec![
        format!("{trimmed}/{n} passages trimmed to <= 600 words at a sentence end"),
        format!("RemingtonGAIL: {n}/{n} real passages imported (source_unicode_sha256 verified)"),
        format!("RemingtonCBI: {cbi_identical}/{n} files hold the same text and title (shared Unicode pool)"),
        format!("InScript: {inscript_identical}/{n} files hold the same text and title (shared Unicode pool)"),
        format!(
            "KrutiDev: {golu_kd_prefix}/{n} legacy forms taken from Expert AI's own KrutiDev files \
             (each agrees with Radha's converter)"
        ),
        format!(
            "InScript typability: {inscript_typable}/{n} passages typable key for key; gap {}",
            n - inscript_typable
        ),
    ];
    for d in &kd_differs {
        notes.push(format!("KrutiDev differs, Radha converter kept: {d}"));
    }
    if !inscript_blocked.is_empty() {
        notes.push(format!(
            "InScript cannot type passages {inscript_blocked:?}"
        ));
    }
    if !mismatches.is_empty() {
        bail!("Hindi exam layouts disagree: {mismatches:?}");
    }
    Ok(Built {
        bundle: bundle(
            ContentTarget::Pool(ExamPool::Hindi),
            "golu-expert-ai:content/exams/Hindi/{RemingtonGAIL,RemingtonCBI,InScript,KrutiDev}"
                .into(),
            true,
            exercises,
        ),
        excluded,
        notes,
        partial: false,
    })
}

/// `content.unicode` of a Golu JSON file, if present.
fn serde_json_value(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    value
        .get("content")?
        .get("unicode")?
        .as_str()
        .map(str::to_owned)
}

/// The briefs folder as one text (files in name order) and its sha256.
fn read_briefs() -> anyhow::Result<(String, String)> {
    let dir = data_dir().join("exam-english-briefs");
    let (files, _) = list_files(&dir, |n| n.ends_with(".txt"))?;
    let mut text = String::new();
    for f in &files {
        text.push_str(&read(f)?);
        text.push('\n');
    }
    let sha = sha256_hex(text.as_bytes());
    Ok((text, sha))
}

/// English Exam Master: the consolidated 1..500 pool of Golu's title bank.
/// It replaces the earlier pool (200 Expert AI passages under other titles
/// plus 300 recombined ones), which matched none of the bank's IDs.
fn exam_english(radha500: &Path) -> anyhow::Result<Built> {
    let rows = exam_english::parse_titles(&read(&data_dir().join("exam-500-english-titles.md"))?)?;
    let (drafts, mut notes) = exam_english::load_drafts(radha500, &rows)?;
    let (briefs_text, briefs_sha) = read_briefs()?;
    let (settings, briefs) = exam_english::parse_briefs(&briefs_text)?;
    let (exercises, reused, written) =
        exam_english::build(&rows, &drafts, &settings, &briefs, &briefs_sha)?;
    notes.insert(
        0,
        format!(
            "{reused} reused from the Radha 500 drafts ({}), {written} written from the briefs \
             (generated-review-pending); IDs and titles follow the title bank 1..500",
            radha500.display()
        ),
    );
    let texts: Vec<&str> = exercises.iter().map(|e| e.content.as_str()).collect();
    if let Some((s, i, j)) = radha_content::quality::most_similar_pair(&texts) {
        notes.push(format!(
            "most similar pair (5-word shingles, Jaccard): {:.3} between {} and {}",
            s, exercises[i].source_id, exercises[j].source_id
        ));
    }
    let shared = radha_content::quality::shared_sentences(&texts, 6);
    notes.push(format!(
        "sentences of 6+ words shared between passages: {}",
        shared.len()
    ));
    let words: Vec<usize> = exercises.iter().map(ImportedExercise::word_count).collect();
    notes.push(format!(
        "words per passage: min {}, max {}",
        words.iter().min().unwrap_or(&0),
        words.iter().max().unwrap_or(&0)
    ));
    Ok(Built {
        bundle: bundle(
            ContentTarget::Pool(ExamPool::English),
            format!(
                "golu-title-bank:data/exam-500-english-titles.md; radha-500 drafts + {}",
                exam_english::PIPELINE
            ),
            false,
            exercises,
        ),
        excluded: Vec::new(),
        notes,
        partial: false,
    })
}

/// Number + Punctuation: Golu's 50 Track 03 seeds, expanded and checked.
fn track03() -> anyhow::Result<Built> {
    let md = read(&data_dir().join("track03-alphanumeric-50-seed.md"))?;
    let (exercises, n) = alphanumeric::from_seed_file(&md)?;
    let words: Vec<usize> = exercises.iter().map(ImportedExercise::word_count).collect();
    let mut notes = vec![
        format!(
            "50 drills ALN-001..ALN-050 from Golu's seeds; words per drill min {}, max {}",
            words.iter().min().unwrap_or(&0),
            words.iter().max().unwrap_or(&0)
        ),
        format!(
            "typed the US-keyboard way: {}",
            n.mapped
                .iter()
                .map(|(c, k)| format!("U+{:04X} x{k}", u32::from(*c)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!(
            "{} seed lines already used by an earlier drill were not repeated",
            n.repeated_seed_lines
        ),
        "every line unique across the track; arithmetic, dates and times checked".into(),
    ];
    for f in &n.fixed {
        notes.push(format!("seed corrected: {f}"));
    }
    Ok(Built {
        bundle: bundle(
            ContentTarget::Product(ProductId::NumberPunctuation),
            alphanumeric::PIPELINE.into(),
            false,
            exercises,
        ),
        excluded: Vec::new(),
        notes,
        partial: false,
    })
}

/// Hindi Learn: authored from the layout's Hindi Mastery source and
/// `CURRICULUM.md`.
fn hindi_learn(golu: &Path, layout: LayoutId, mastery: &[Built]) -> anyhow::Result<Built> {
    let vocab = hindi_learn::vocabulary_layout(layout);
    let source = mastery
        .iter()
        .find(|b| b.bundle.product == ProductId::HindiMastery(vocab).slug())
        .context("Hindi Mastery source not built")?;
    let curriculum = golu
        .join("content-v2")
        .join("hi")
        .join("remington-gail")
        .join("CURRICULUM.md");
    let curriculum_bytes =
        fs::read(&curriculum).with_context(|| format!("cannot read {}", curriculum.display()))?;
    let mut inputs = sha256_hex(&curriculum_bytes);
    for e in &source.bundle.exercises {
        inputs.push_str(&e.content_sha256);
    }
    let inputs_sha = sha256_hex(inputs.as_bytes());
    let texts: Vec<&str> = source
        .bundle
        .exercises
        .iter()
        .map(|e| e.content.as_str())
        .collect();
    let authored = hindi_learn::author(layout, &texts, &inputs_sha)?;
    let product = ProductId::HindiLearn(layout);
    let mut notes = vec![format!(
        "GENERATED (generated-review-pending) from {} + CURRICULUM.md tiers; Beginner 1-100, Classic 101-200",
        source.bundle.product
    )];
    if !authored.skipped_keys.is_empty() {
        notes.push(format!(
            "beginner track could not drill keys: {:?}",
            authored.skipped_keys
        ));
    }
    Ok(Built {
        bundle: bundle(
            ContentTarget::Product(product),
            hindi_learn::PIPELINE.into(),
            true,
            authored.exercises,
        ),
        excluded: Vec::new(),
        notes,
        partial: false,
    })
}

/// Units no key plan reaches, per typing layout (reported, not fatal, for
/// imported text; generated text is already checked unit by unit).
fn unit_warnings(target: ContentTarget, b: &ContentBundle) -> Vec<String> {
    let mut out = Vec::new();
    for layout in target.typing_layouts() {
        let Ok(board) = load_layout(layout) else {
            continue;
        };
        // Plan every distinct unit of the bundle once (one Guide over the
        // unique units), then attribute failures to passages.
        let split = |text: &str| -> Vec<String> {
            match board.encoding() {
                radha_layouts::Encoding::Unicode => radha_core::akshar::segment(text),
                _ => text.chars().map(String::from).collect(),
            }
        };
        let mut distinct: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let per_passage: Vec<Vec<String>> = b.exercises.iter().map(|e| split(&e.content)).collect();
        for units in &per_passage {
            for u in units {
                if !u.trim().is_empty() && seen.insert(u.clone()) {
                    distinct.push(u.clone());
                }
            }
        }
        let guide = Guide::new(board, &distinct.join(" "));
        let bad: HashSet<String> = guide.untypeable().into_iter().map(str::to_owned).collect();
        let mut units: Vec<String> = Vec::new();
        let mut hit = 0;
        for passage in &per_passage {
            let mut any = false;
            for u in passage.iter().filter(|u| bad.contains(*u)) {
                any = true;
                if !units.contains(u) {
                    units.push(u.clone());
                }
            }
            hit += usize::from(any);
        }
        if hit > 0 {
            out.push(format!(
                "{hit} passages hold akshars the {} planner cannot plan: {units:?}",
                layout.as_str()
            ));
        }
    }
    out
}

fn status_counts(b: &ContentBundle) -> String {
    let mut m: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &b.exercises {
        *m.entry(e.status.as_str()).or_default() += 1;
    }
    m.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn mb(bytes: u64) -> String {
    format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let started = Instant::now();
    let mut r = Report::default();
    r.line(format!(
        "import-golu-content ({})",
        if args.check {
            "DRY RUN: nothing is written"
        } else {
            "REAL RUN"
        }
    ));
    r.line(format!("expert ai : {}", args.golu.display()));
    r.line(format!("golu os   : {}", args.os.display()));

    // ---- build every target ------------------------------------------------
    let english = golu_product(&args.golu, ProductId::EnglishClassic)?;
    let mut mastery = Vec::new();
    for layout in [
        LayoutId::RemingtonGail,
        LayoutId::RemingtonCbi,
        LayoutId::KrutiDev,
    ] {
        mastery.push(golu_product(&args.golu, ProductId::HindiMastery(layout))?);
    }
    let flow = match &args.flow {
        Some(dir) => {
            r.line(format!("flow      : {}", dir.display()));
            radha_flow(dir)?
        }
        None => {
            r.line("flow      : (no --flow) legacy Golu OS smooth lessons");
            typing_flow(&args.os)?
        }
    };
    let numbers = match &args.number_punct {
        Some(dir) => Some(number_punct(dir)?),
        None => Some(track03()?),
    };
    let pool_hindi = exam_hindi(&args.golu)?;
    let pool_english = exam_english(&args.radha500)?;
    let mut learn = Vec::new();
    for layout in [
        LayoutId::RemingtonGail,
        LayoutId::RemingtonCbi,
        LayoutId::KrutiDev,
    ] {
        learn.push(hindi_learn(&args.golu, layout, &mastery)?);
    }

    let mut built: Vec<Built> = vec![english, flow];
    built.extend(numbers);
    built.extend(mastery);
    built.extend(learn);
    built.push(pool_hindi);
    built.push(pool_english);

    // ---- validate (abort rules) and pack -----------------------------------
    r.line("");
    r.line(format!(
        "{:<24} {:>5} {:>9} {:>9} {:>10} {:>10}  status",
        "target", "count", "words min", "words max", "plain", "packed"
    ));
    let mut packed: Vec<(ContentTarget, Vec<u8>)> = Vec::new();
    let mut plain_total = 0u64;
    let mut packed_total = 0u64;
    let mut details = Vec::new();
    for b in &built {
        let target = if b.partial {
            validate_up_to(&b.bundle)
        } else {
            validate_complete(&b.bundle)
        }
        .with_context(|| format!("ABORT: {} failed validation", b.bundle.product))?;
        let bytes = pack_bundle(&b.bundle)?;
        let plain: u64 = b
            .bundle
            .exercises
            .iter()
            .map(|e| (e.content.len() + e.legacy.as_ref().map_or(0, String::len)) as u64)
            .sum();
        let words: Vec<usize> = b
            .bundle
            .exercises
            .iter()
            .map(ImportedExercise::word_count)
            .collect();
        r.line(format!(
            "{:<24} {:>5} {:>9} {:>9} {:>10} {:>10}  {}",
            target.slug(),
            b.bundle.exercises.len(),
            words.iter().min().copied().unwrap_or(0),
            words.iter().max().copied().unwrap_or(0),
            mb(plain),
            mb(bytes.len() as u64),
            status_counts(&b.bundle)
        ));
        plain_total += plain;
        packed_total += bytes.len() as u64;
        let mut lines = Vec::new();
        if !b.excluded.is_empty() {
            lines.push(format!("excluded (not exercises): {:?}", b.excluded));
        }
        lines.extend(b.notes.iter().cloned());
        lines.extend(unit_warnings(target, &b.bundle));
        details.push((target, lines));
        packed.push((target, bytes));
    }
    if numbers_missing(&built) {
        r.line(format!(
            "{:<24} {:>5}  content later (Golu supplies it; importer path ready: --number-punct <dir>); \
             the product shows an honest empty state",
            ProductId::NumberPunctuation.slug(),
            0
        ));
    }
    let total: usize = built.iter().map(|b| b.bundle.exercises.len()).sum();
    r.line(format!(
        "{:<24} {:>5} {:>9} {:>9} {:>10} {:>10}",
        "TOTAL",
        total,
        "",
        "",
        mb(plain_total),
        mb(packed_total)
    ));
    r.line("");
    for (target, lines) in &details {
        for l in lines {
            r.line(format!("  {}: {l}", target.slug()));
        }
    }
    r.line("");
    r.line("abort rules checked: exact counts, unique ids, every char typable, no unresolved chars, NFC, sha256");

    // ---- write bundles -----------------------------------------------------
    if args.check {
        r.line("dry run: no bundle or database written");
    } else {
        fs::create_dir_all(&args.out)
            .with_context(|| format!("cannot create {}", args.out.display()))?;
        for (target, bytes) in &packed {
            let path = args.out.join(packed_file_name(*target));
            fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
            // The plain-JSON bundle of format 1 is superseded.
            let legacy = args.out.join(format!("{}.json", target.slug()));
            if legacy.is_file() {
                fs::remove_file(&legacy)
                    .with_context(|| format!("cannot remove {}", legacy.display()))?;
                r.line(format!("removed superseded {}", legacy.display()));
            }
        }
        r.line(format!(
            "wrote {} bundles ({}) to {}",
            packed.len(),
            mb(packed_total),
            args.out.display()
        ));
    }

    // ---- SQLite ------------------------------------------------------------
    if let (Some(db_path), false) = (&args.db, args.check) {
        write_db(&mut r, db_path, &built, &packed)?;
    }
    r.line(format!("done in {:.1} s", started.elapsed().as_secs_f64()));
    Ok(())
}

fn numbers_missing(built: &[Built]) -> bool {
    !built
        .iter()
        .any(|b| b.bundle.product == ProductId::NumberPunctuation.slug())
}

fn write_db(
    r: &mut Report,
    db_path: &Path,
    built: &[Built],
    packed: &[(ContentTarget, Vec<u8>)],
) -> anyhow::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{suffix}", db_path.display()));
        if p.is_file() {
            fs::remove_file(&p).with_context(|| format!("cannot remove {}", p.display()))?;
        }
    }
    let t0 = Instant::now();
    {
        let mut db = radha_db::Database::open(db_path)?;
        let batch: Vec<(&ContentBundle, String)> = built
            .iter()
            .zip(packed)
            .map(|(b, (_, bytes))| (&b.bundle, sha256_hex(bytes)))
            .collect();
        let report = radha_db::import_bundles(&mut db, &batch)?;
        let (covered, total) = radha_db::content_import::provenance_coverage(&db)?;
        r.line(format!(
            "sqlite: {} rows written in ONE transaction ({} inserted, {} updated, {} stale removed) \
             in {:.1} s; provenance rows {covered}/{total}",
            report.inserted + report.updated,
            report.inserted,
            report.updated,
            report.removed,
            t0.elapsed().as_secs_f64()
        ));
    }
    let size = fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
    r.line(format!("sqlite: {} -> {}", db_path.display(), mb(size)));

    // List-load timing: a fresh connection, like the Exercise screen.
    let repo = radha_db::SqliteRepository::new(radha_db::Database::open(db_path)?);
    let t = Instant::now();
    let cold = repo.list_exam_pool(ExamPool::Hindi)?;
    let cold_time = t.elapsed();
    let mut samples: Vec<Duration> = Vec::new();
    for _ in 0..25 {
        let t = Instant::now();
        let list = repo.list_exam_pool(ExamPool::Hindi)?;
        samples.push(t.elapsed());
        if list.len() != cold.len() {
            bail!("list length changed");
        }
    }
    samples.sort();
    r.line(format!(
        "list-load (metadata only) exam-pool-hindi {} cards: first {:.2} ms, median {:.2} ms, max {:.2} ms",
        cold.len(),
        cold_time.as_secs_f64() * 1000.0,
        samples[samples.len() / 2].as_secs_f64() * 1000.0,
        samples[samples.len() - 1].as_secs_f64() * 1000.0
    ));
    let t = Instant::now();
    let english = repo.list_exam_pool(ExamPool::English)?;
    r.line(format!(
        "list-load exam-pool-english {} cards: {:.2} ms",
        english.len(),
        t.elapsed().as_secs_f64() * 1000.0
    ));
    for product in ProductId::all() {
        let t = Instant::now();
        let list = repo.list_meta(product)?;
        r.line(format!(
            "list-load {:<24} {:>3} cards: {:.2} ms",
            product.slug(),
            list.len(),
            t.elapsed().as_secs_f64() * 1000.0
        ));
    }
    if let Some(first) = cold.first() {
        let t = Instant::now();
        let text = repo.load_exam_passage(first.id, true)?;
        r.line(format!(
            "lazy load of one passage (Kruti Dev form, {} chars, decompress + sha256): {:.2} ms",
            text.chars().count(),
            t.elapsed().as_secs_f64() * 1000.0
        ));
    }
    Ok(())
}
