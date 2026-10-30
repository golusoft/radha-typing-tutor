//! Seeding content from validated [`ContentBundle`]s.
//!
//! Bundles are written in one transaction through [`Database::transaction`]:
//! either every passage of every bundle lands, or none does. The write is an
//! upsert keyed by `(product, number)` / `(pool, number)`, so re-importing a
//! corrected bundle updates titles and text in place and keeps the ids, which
//! progress, sessions and bests reference.
//!
//! Text goes to `exercise_text` / `exam_passage_text` deflate-compressed with
//! its sha256; metadata (title, status, word count…) stays in the list
//! tables. Every passage gets a `content_provenance` row, every bundle a
//! `content_imports` row holding the bundle's sha256.

use radha_content::codec::{compress_text, sha256_hex, CODEC_DEFLATE};
use radha_content::import::{
    bundle_to_json, validate_bundle, validate_target_bundle, ContentBundle, ImportedExercise,
};
use radha_content::{ContentTarget, ExamPool, ProductId};
use radha_core::{CoreError, Result};
use rusqlite::{OptionalExtension, Transaction};

use crate::Database;

/// What one import did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportReport {
    /// Exercises newly created.
    pub inserted: usize,
    /// Exercises that already existed and were refreshed.
    pub updated: usize,
    /// Stale rows deleted: numbered past the new bundle's count for the
    /// same target (their text and provenance go with them).
    pub removed: usize,
}

fn sql(action: &str, error: rusqlite::Error) -> CoreError {
    CoreError::Database {
        detail: format!("{action}: {error}"),
    }
}

/// Validate `bundle` and upsert all its exercises in one transaction.
///
/// # Errors
///
/// [`CoreError::Malformed`] when the bundle fails validation (nothing is
/// written); [`CoreError::Database`] when a write fails (rolled back).
pub fn import_bundle(db: &mut Database, bundle: &ContentBundle) -> Result<ImportReport> {
    validate_bundle(bundle)?;
    let sha = sha256_hex(bundle_to_json(bundle)?.as_bytes());
    import_bundles(db, &[(bundle, sha)])
}

/// Validate every bundle, then write them all in **one** transaction. Each
/// bundle is paired with the sha256 of the file it came from, recorded in
/// `content_imports`.
///
/// # Errors
///
/// [`CoreError::Malformed`] when any bundle fails validation (nothing is
/// written); [`CoreError::Database`] when a write fails (all rolled back).
pub fn import_bundles(
    db: &mut Database,
    bundles: &[(&ContentBundle, String)],
) -> Result<ImportReport> {
    let mut targets = Vec::with_capacity(bundles.len());
    for (bundle, sha) in bundles {
        if sha.len() != 64 {
            return Err(CoreError::malformed(
                "content bundle",
                &bundle.product,
                "bundle sha256 must be 64 hex digits",
            ));
        }
        targets.push(validate_target_bundle(bundle)?);
    }
    db.transaction(|tx| {
        let mut report = ImportReport::default();
        for ((bundle, sha), target) in bundles.iter().zip(&targets) {
            let part = match *target {
                ContentTarget::Product(product) => write_product(tx, product, bundle)?,
                ContentTarget::Pool(pool) => write_pool(tx, pool, bundle)?,
            };
            report.inserted += part.inserted;
            report.updated += part.updated;
            report.removed += part.removed;
            tx.execute(
                "INSERT INTO content_imports (target_slug, bundle_sha256, item_count, source)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (target_slug) DO UPDATE SET
                     bundle_sha256 = excluded.bundle_sha256,
                     item_count    = excluded.item_count,
                     source        = excluded.source,
                     imported_at   = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
                rusqlite::params![
                    target.slug(),
                    sha,
                    i64::try_from(bundle.exercises.len()).unwrap_or(i64::MAX),
                    bundle.source
                ],
            )
            .map_err(|e| sql("could not record the import", e))?;
        }
        Ok(report)
    })
}

/// The sha256 of the bundle last imported for `target`, if any.
///
/// # Errors
///
/// [`CoreError::Database`] if the table cannot be read.
pub fn imported_bundle_sha(db: &Database, target: ContentTarget) -> Result<Option<String>> {
    db.connection()
        .query_row(
            "SELECT bundle_sha256 FROM content_imports WHERE target_slug = ?1",
            [target.slug()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| sql("could not read content imports", e))
}

/// Counts for the provenance check: `(passages with provenance, total
/// passages)` across exercises and exam passages.
///
/// # Errors
///
/// [`CoreError::Database`] if a count fails.
pub fn provenance_coverage(db: &Database) -> Result<(usize, usize)> {
    let count = |q: &str| -> Result<usize> {
        let n: i64 = db
            .connection()
            .query_row(q, [], |r| r.get(0))
            .map_err(|e| sql("could not count provenance", e))?;
        Ok(usize::try_from(n).unwrap_or(0))
    };
    let covered = count("SELECT COUNT(*) FROM content_provenance")?;
    let total =
        count("SELECT (SELECT COUNT(*) FROM exercises) + (SELECT COUNT(*) FROM exam_passages)")?;
    Ok((covered, total))
}

fn count_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The provenance row for one passage. Format-1 bundles carry no source
/// path or hash: the bundle's own source label and the text hash stand in.
fn write_provenance(
    tx: &Transaction<'_>,
    kind: &str,
    target_id: i64,
    bundle: &ContentBundle,
    e: &ImportedExercise,
    content_sha: &str,
) -> Result<()> {
    let path = if e.source_path.is_empty() {
        bundle.source.as_str()
    } else {
        e.source_path.as_str()
    };
    let source_sha = if e.source_sha256.len() == 64 {
        e.source_sha256.as_str()
    } else {
        content_sha
    };
    tx.execute(
        "INSERT INTO content_provenance
             (target_kind, target_id, source_path, source_id, source_sha256, content_sha256, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (target_kind, target_id) DO UPDATE SET
             source_path    = excluded.source_path,
             source_id      = excluded.source_id,
             source_sha256  = excluded.source_sha256,
             content_sha256 = excluded.content_sha256,
             status         = excluded.status,
             imported_at    = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        rusqlite::params![
            kind,
            target_id,
            if path.is_empty() { "unknown" } else { path },
            (!e.source_id.is_empty()).then_some(e.source_id.as_str()),
            source_sha,
            content_sha,
            e.status.as_str()
        ],
    )
    .map_err(|e| sql("could not write provenance", e))?;
    Ok(())
}

/// The sha256 to store for a passage: the sealed one (already checked by
/// the validator) or, for format-1 bundles, computed now.
fn content_sha(e: &ImportedExercise) -> String {
    if e.content_sha256.is_empty() {
        sha256_hex(e.content.as_bytes())
    } else {
        e.content_sha256.clone()
    }
}

fn write_product(
    tx: &Transaction<'_>,
    product: ProductId,
    bundle: &ContentBundle,
) -> Result<ImportReport> {
    let product_id: i64 = tx
        .query_row(
            "SELECT id FROM products WHERE slug = ?1",
            [product.slug()],
            |r| r.get(0),
        )
        .map_err(|e| sql("unknown product", e))?;
    let mut report = ImportReport::default();
    for e in &bundle.exercises {
        let words = count_i64(e.word_count());
        let chars = count_i64(e.content.chars().count());
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM exercises WHERE product_id = ?1 AND number = ?2",
                rusqlite::params![product_id, e.number],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| sql("could not look up an exercise", e))?;
        let id = if let Some(id) = existing {
            tx.execute(
                "UPDATE exercises SET title = ?2, category = ?3, difficulty = ?4, status = ?5,
                        track = ?6, word_count = ?7, char_count = ?8
                  WHERE id = ?1",
                rusqlite::params![
                    id,
                    e.title,
                    e.category,
                    e.difficulty,
                    e.status.as_str(),
                    e.track,
                    words,
                    chars
                ],
            )
            .map_err(|e| sql("could not update an exercise", e))?;
            report.updated += 1;
            id
        } else {
            tx.execute(
                "INSERT INTO exercises
                     (product_id, number, title, category, difficulty, status, track,
                      word_count, char_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    product_id,
                    e.number,
                    e.title,
                    e.category,
                    e.difficulty,
                    e.status.as_str(),
                    e.track,
                    words,
                    chars
                ],
            )
            .map_err(|e| sql("could not insert an exercise", e))?;
            report.inserted += 1;
            tx.last_insert_rowid()
        };
        let sha = content_sha(e);
        tx.execute(
            "INSERT INTO exercise_text (exercise_id, codec, body, sha256) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (exercise_id) DO UPDATE SET
                 codec = excluded.codec, body = excluded.body, sha256 = excluded.sha256",
            rusqlite::params![id, CODEC_DEFLATE, compress_text(&e.content), sha],
        )
        .map_err(|e| sql("could not write exercise text", e))?;
        // The plain copy from before P04 is superseded.
        tx.execute("DELETE FROM exercise_content WHERE exercise_id = ?1", [id])
            .map_err(|e| sql("could not retire plain exercise content", e))?;
        write_provenance(tx, "exercise", id, bundle, e, &sha)?;
    }
    // A re-import with fewer exercises (P04 finding 5): rows numbered past
    // the new count belong to the old source; delete them in this same
    // transaction so they cannot linger. Text, progress and sessions of
    // those rows cascade; provenance has no foreign key, so it goes first.
    let n = count_i64(bundle.exercises.len());
    tx.execute(
        "DELETE FROM content_provenance WHERE target_kind = 'exercise' AND target_id IN
             (SELECT id FROM exercises WHERE product_id = ?1 AND number > ?2)",
        rusqlite::params![product_id, n],
    )
    .map_err(|e| sql("could not retire stale provenance", e))?;
    report.removed = tx
        .execute(
            "DELETE FROM exercises WHERE product_id = ?1 AND number > ?2",
            rusqlite::params![product_id, n],
        )
        .map_err(|e| sql("could not retire stale exercises", e))?;
    Ok(report)
}

fn write_pool(
    tx: &Transaction<'_>,
    pool: ExamPool,
    bundle: &ContentBundle,
) -> Result<ImportReport> {
    let pool_id: i64 = tx
        .query_row(
            "SELECT id FROM exam_pools WHERE slug = ?1",
            [pool.slug()],
            |r| r.get(0),
        )
        .map_err(|e| sql("unknown exam pool", e))?;
    let mut report = ImportReport::default();
    for e in &bundle.exercises {
        let words = count_i64(e.word_count());
        let chars = count_i64(e.content.chars().count());
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM exam_passages WHERE pool_id = ?1 AND number = ?2",
                rusqlite::params![pool_id, e.number],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| sql("could not look up an exam passage", e))?;
        let id = if let Some(id) = existing {
            tx.execute(
                "UPDATE exam_passages SET title = ?2, category = ?3, difficulty = ?4,
                        word_count = ?5, char_count = ?6, status = ?7
                  WHERE id = ?1",
                rusqlite::params![
                    id,
                    e.title,
                    e.category,
                    e.difficulty,
                    words,
                    chars,
                    e.status.as_str()
                ],
            )
            .map_err(|e| sql("could not update an exam passage", e))?;
            report.updated += 1;
            id
        } else {
            tx.execute(
                "INSERT INTO exam_passages
                     (pool_id, number, title, category, difficulty, word_count, char_count, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    pool_id,
                    e.number,
                    e.title,
                    e.category,
                    e.difficulty,
                    words,
                    chars,
                    e.status.as_str()
                ],
            )
            .map_err(|e| sql("could not insert an exam passage", e))?;
            report.inserted += 1;
            tx.last_insert_rowid()
        };
        let sha = content_sha(e);
        let mut texts = vec![("unicode", e.content.as_str(), sha.clone())];
        if let Some(legacy) = e.legacy.as_deref() {
            texts.push(("krutidev", legacy, sha256_hex(legacy.as_bytes())));
        }
        for (encoding, text, text_sha) in texts {
            tx.execute(
                "INSERT INTO exam_passage_text (passage_id, encoding, codec, body, sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (passage_id, encoding) DO UPDATE SET
                     codec = excluded.codec, body = excluded.body, sha256 = excluded.sha256",
                rusqlite::params![id, encoding, CODEC_DEFLATE, compress_text(text), text_sha],
            )
            .map_err(|e| sql("could not write exam passage text", e))?;
        }
        if e.legacy.is_none() {
            // The new data has no legacy form: a stale one must not linger.
            tx.execute(
                "DELETE FROM exam_passage_text WHERE passage_id = ?1 AND encoding = 'krutidev'",
                [id],
            )
            .map_err(|e| sql("could not retire a stale legacy text", e))?;
        }
        write_provenance(tx, "exam_passage", id, bundle, e, &sha)?;
    }
    // Stale passages past the new count (P04 finding 5), as for products.
    let n = count_i64(bundle.exercises.len());
    tx.execute(
        "DELETE FROM content_provenance WHERE target_kind = 'exam_passage' AND target_id IN
             (SELECT id FROM exam_passages WHERE pool_id = ?1 AND number > ?2)",
        rusqlite::params![pool_id, n],
    )
    .map_err(|e| sql("could not retire stale provenance", e))?;
    report.removed = tx
        .execute(
            "DELETE FROM exam_passages WHERE pool_id = ?1 AND number > ?2",
            rusqlite::params![pool_id, n],
        )
        .map_err(|e| sql("could not retire stale exam passages", e))?;
    Ok(report)
}

/// How many passages an exam pool has in the database.
///
/// # Errors
///
/// [`CoreError::Database`] if the count cannot be read.
pub fn exam_passage_count(db: &Database, pool: ExamPool) -> Result<usize> {
    let n: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM exam_passages AS e JOIN exam_pools AS p ON p.id = e.pool_id
              WHERE p.slug = ?1",
            [pool.slug()],
            |r| r.get(0),
        )
        .map_err(|e| sql("could not count exam passages", e))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// How many exercises a product has in the database.
///
/// # Errors
///
/// [`CoreError::Database`] if the count cannot be read.
pub fn exercise_count(db: &Database, product: radha_content::ProductId) -> Result<usize> {
    let n: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM exercises AS e JOIN products AS p ON p.id = e.product_id
              WHERE p.slug = ?1",
            [product.slug()],
            |r| r.get(0),
        )
        .map_err(|e| sql("could not count exercises", e))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteRepository;
    use radha_content::import::BUNDLE_FORMAT;
    use radha_content::{ContentStatus, ExerciseRepository};
    use radha_layouts::LayoutId;

    fn bundle(product: ProductId, texts: &[&str]) -> ContentBundle {
        ContentBundle {
            format: BUNDLE_FORMAT,
            product: product.slug().into(),
            source: "test".into(),
            provisional: false,
            exercises: texts
                .iter()
                .enumerate()
                .map(|(i, t)| ImportedExercise {
                    number: u32::try_from(i + 1).unwrap(),
                    title: format!("Placeholder {}", i + 1),
                    category: Some("Foundation".into()),
                    difficulty: Some(2),
                    content: (*t).into(),
                    source_id: format!("test-{i}"),
                    ..ImportedExercise::default()
                })
                .collect(),
        }
    }

    fn pool_bundle(pool: ExamPool, n: usize) -> ContentBundle {
        let target = ContentTarget::Pool(pool);
        ContentBundle {
            format: BUNDLE_FORMAT,
            product: target.slug().into(),
            source: "test".into(),
            provisional: true,
            exercises: (1..=n)
                .map(|i| {
                    let content = match pool {
                        ExamPool::Hindi => "भारत एक देश है। ".repeat(40).trim().to_owned(),
                        ExamPool::English => format!("placeholder passage {i} text. ").repeat(40),
                    };
                    let legacy = (pool == ExamPool::Hindi)
                        .then(|| radha_layouts::krutidev::to_legacy(&content).legacy);
                    let mut e = ImportedExercise {
                        number: u32::try_from(i).unwrap(),
                        title: format!("Placeholder passage {i}"),
                        category: Some("Practice".into()),
                        difficulty: Some(3),
                        content: content.trim().to_owned(),
                        source_id: format!("pool-{i}"),
                        legacy,
                        source_path: format!("fixtures/{i}.json"),
                        source_sha256: sha256_hex(format!("{i}").as_bytes()),
                        ..ImportedExercise::default()
                    };
                    e.seal();
                    e
                })
                .collect(),
        }
    }

    #[test]
    fn text_is_stored_compressed_with_sha_and_provenance() {
        let mut db = Database::open_in_memory().unwrap();
        let p = ProductId::EnglishClassic;
        let text = "the quick brown fox jumps over the lazy dog ".repeat(50);
        import_bundle(&mut db, &bundle(p, &[text.trim()])).unwrap();
        let (codec, body, sha): (String, Vec<u8>, String) = db
            .connection()
            .query_row("SELECT codec, body, sha256 FROM exercise_text", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(codec, "deflate");
        assert!(body.len() * 5 < text.len(), "compressed: {}", body.len());
        assert_eq!(sha, sha256_hex(text.trim().as_bytes()));
        let plain: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM exercise_content", [], |r| r.get(0))
            .unwrap();
        assert_eq!(plain, 0, "no uncompressed copy is kept");
        assert_eq!(provenance_coverage(&db).unwrap(), (1, 1));
        assert!(imported_bundle_sha(&db, ContentTarget::Product(p))
            .unwrap()
            .is_some());
        let repo = SqliteRepository::new(db);
        let id = repo.list_for_product(p).unwrap()[0].id;
        assert_eq!(repo.load(id).unwrap().content, text.trim());
    }

    #[test]
    fn pools_import_both_encodings_and_list_metadata_only() {
        let mut db = Database::open_in_memory().unwrap();
        let hindi = pool_bundle(ExamPool::Hindi, 3);
        let english = pool_bundle(ExamPool::English, 2);
        let sha = |b: &ContentBundle| sha256_hex(bundle_to_json(b).unwrap().as_bytes());
        let report =
            import_bundles(&mut db, &[(&hindi, sha(&hindi)), (&english, sha(&english))]).unwrap();
        assert_eq!(report.inserted, 5);
        assert_eq!(exam_passage_count(&db, ExamPool::Hindi).unwrap(), 3);
        assert_eq!(exam_passage_count(&db, ExamPool::English).unwrap(), 2);
        assert_eq!(provenance_coverage(&db).unwrap(), (5, 5));

        let repo = SqliteRepository::new(db);
        let list = repo.list_exam_pool(ExamPool::Hindi).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].status, ContentStatus::Provisional);
        assert!(list[0].word_count > 0);
        let unicode = repo.load_exam_passage(list[0].id, false).unwrap();
        assert!(unicode.starts_with("भारत"));
        let legacy = repo.load_exam_passage(list[0].id, true).unwrap();
        assert!(legacy.starts_with("Hkkjr"), "{legacy}");
        let en = repo.list_exam_pool(ExamPool::English).unwrap();
        assert!(
            repo.load_exam_passage(en[0].id, true).is_err(),
            "no legacy form"
        );
    }

    #[test]
    fn one_bad_bundle_aborts_the_whole_transaction() {
        let mut db = Database::open_in_memory().unwrap();
        let good = bundle(ProductId::EnglishClassic, &["fine text"]);
        let mut bad = pool_bundle(ExamPool::Hindi, 2);
        bad.exercises[1].legacy = None;
        let sha = "0".repeat(64);
        assert!(import_bundles(&mut db, &[(&good, sha.clone()), (&bad, sha)]).is_err());
        assert_eq!(exercise_count(&db, ProductId::EnglishClassic).unwrap(), 0);
        assert_eq!(exam_passage_count(&db, ExamPool::Hindi).unwrap(), 0);
    }

    #[test]
    fn metadata_list_of_500_cards_never_reads_text_and_is_fast() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("list.sqlite");
        let mut db = Database::open(&path).unwrap();
        let pool = pool_bundle(ExamPool::English, 500);
        import_bundle_any(&mut db, &pool);
        let repo = SqliteRepository::new(Database::open(&path).unwrap());
        // Warm-up, then measure.
        assert_eq!(repo.list_exam_pool(ExamPool::English).unwrap().len(), 500);
        let start = std::time::Instant::now();
        let list = repo.list_exam_pool(ExamPool::English).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(list.len(), 500);
        eprintln!("500-card metadata list: {elapsed:?}");
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "{elapsed:?}"
        );
        // The list query plan touches the passage table and pool table only.
        let plan = repo.explain_exam_list().unwrap();
        assert!(!plan.contains("exam_passage_text"), "{plan}");
    }

    fn import_bundle_any(db: &mut Database, b: &ContentBundle) {
        let sha = sha256_hex(bundle_to_json(b).unwrap().as_bytes());
        import_bundles(db, &[(b, sha)]).unwrap();
    }

    #[test]
    fn hindi_learn_rows_carry_track_and_generated_status() {
        let mut db = Database::open_in_memory().unwrap();
        let learn = ProductId::HindiLearn(LayoutId::RemingtonCbi);
        let mut b = bundle(learn, &["कमल", "जल"]);
        for (e, track) in b.exercises.iter_mut().zip(["beginner", "classic"]) {
            e.track = Some(track.into());
            e.status = ContentStatus::GeneratedReviewPending;
            e.seal();
        }
        import_bundle(&mut db, &b).unwrap();
        let repo = SqliteRepository::new(db);
        let meta = repo.list_meta(learn).unwrap();
        assert_eq!(meta.len(), 2);
        assert_eq!(meta[0].track.as_deref(), Some("beginner"));
        assert_eq!(meta[1].track.as_deref(), Some("classic"));
        assert!(meta
            .iter()
            .all(|m| m.status == ContentStatus::GeneratedReviewPending));
        assert_eq!(meta[0].word_count, Some(1));
    }

    #[test]
    fn import_then_list_lazily_and_load_one() {
        let mut db = Database::open_in_memory().unwrap();
        let gail = ProductId::HindiMastery(LayoutId::RemingtonGail);
        let report = import_bundle(&mut db, &bundle(gail, &["भारत देश", "कार्य"])).unwrap();
        assert_eq!(
            report,
            ImportReport {
                inserted: 2,
                updated: 0,
                removed: 0
            }
        );
        assert_eq!(exercise_count(&db, gail).unwrap(), 2);
        assert_eq!(exercise_count(&db, ProductId::EnglishClassic).unwrap(), 0);

        let repo = SqliteRepository::new(db);
        let list = repo.list_for_product(gail).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|e| e.content.is_empty()), "list is lazy");
        assert_eq!(list[0].category.as_deref(), Some("Foundation"));
        assert_eq!(list[0].difficulty, Some(2));
        let loaded = repo.load(list[1].id).unwrap();
        assert_eq!(loaded.content, "कार्य");
        assert_eq!(loaded.difficulty, Some(2));
    }

    #[test]
    fn reimport_updates_in_place_and_keeps_ids() {
        let mut db = Database::open_in_memory().unwrap();
        let p = ProductId::EnglishClassic;
        import_bundle(&mut db, &bundle(p, &["first text"])).unwrap();
        let repo_ids = |db: Database| {
            let repo = SqliteRepository::new(db);
            let list = repo.list_for_product(p).unwrap();
            (list[0].id, repo.load(list[0].id).unwrap().content)
        };
        let report = import_bundle(&mut db, &bundle(p, &["second text"])).unwrap();
        assert_eq!(
            report,
            ImportReport {
                inserted: 0,
                updated: 1,
                removed: 0
            }
        );
        let (_, content) = repo_ids(db);
        assert_eq!(content, "second text");
    }

    #[test]
    fn a_shorter_reimport_removes_stale_rows_in_the_same_transaction() {
        let mut db = Database::open_in_memory().unwrap();
        let p = ProductId::TypingFlow;
        import_bundle(&mut db, &bundle(p, &["one", "two", "three", "four"])).unwrap();
        assert_eq!(exercise_count(&db, p).unwrap(), 4);
        let report = import_bundle(&mut db, &bundle(p, &["uno", "dos"])).unwrap();
        assert_eq!(
            report,
            ImportReport {
                inserted: 0,
                updated: 2,
                removed: 2
            }
        );
        assert_eq!(exercise_count(&db, p).unwrap(), 2);
        assert_eq!(
            provenance_coverage(&db).unwrap(),
            (2, 2),
            "no orphan provenance"
        );
        let texts: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM exercise_text", [], |r| r.get(0))
            .unwrap();
        assert_eq!(texts, 2, "stale text went with its rows");
        // Another product is untouched.
        import_bundle(&mut db, &bundle(ProductId::EnglishClassic, &["keep me"])).unwrap();
        import_bundle(&mut db, &bundle(p, &["uno"])).unwrap();
        assert_eq!(exercise_count(&db, ProductId::EnglishClassic).unwrap(), 1);
        assert_eq!(exercise_count(&db, p).unwrap(), 1);
    }

    #[test]
    fn a_pool_reimport_drops_stale_passages_and_a_vanished_legacy_form() {
        let mut db = Database::open_in_memory().unwrap();
        import_bundle_any(&mut db, &pool_bundle(ExamPool::Hindi, 3));
        let mut shorter = pool_bundle(ExamPool::Hindi, 2);
        let sha = sha256_hex(bundle_to_json(&shorter).unwrap().as_bytes());
        let report = import_bundles(&mut db, &[(&shorter, sha)]).unwrap();
        assert_eq!(report.removed, 1);
        assert_eq!(exam_passage_count(&db, ExamPool::Hindi).unwrap(), 2);
        assert_eq!(provenance_coverage(&db).unwrap(), (2, 2));
        // A legacy row left over from older data is retired on re-import of
        // a passage that has none (English pool rows never carry one).
        import_bundle_any(&mut db, &pool_bundle(ExamPool::English, 1));
        let english_id: i64 = db
            .connection()
            .query_row(
                "SELECT e.id FROM exam_passages e JOIN exam_pools p ON p.id = e.pool_id
                  WHERE p.slug = 'exam-pool-english'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        db.connection()
            .execute(
                "INSERT INTO exam_passage_text (passage_id, encoding, codec, body, sha256)
                 VALUES (?1, 'krutidev', 'deflate', x'00', ?2)",
                rusqlite::params![english_id, "0".repeat(64)],
            )
            .unwrap();
        shorter = pool_bundle(ExamPool::English, 1);
        import_bundle_any(&mut db, &shorter);
        let legacy: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM exam_passage_text WHERE passage_id = ?1 AND encoding = 'krutidev'",
                [english_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(legacy, 0);
    }

    #[test]
    fn an_invalid_bundle_writes_nothing() {
        let mut db = Database::open_in_memory().unwrap();
        let p = ProductId::EnglishClassic;
        let mut bad = bundle(p, &["fine", "also fine", "भारत"]);
        assert!(
            import_bundle(&mut db, &bad).is_err(),
            "Devanagari in English"
        );
        assert_eq!(exercise_count(&db, p).unwrap(), 0, "all or nothing");
        bad.exercises.truncate(2);
        bad.exercises[1].number = 5;
        assert!(import_bundle(&mut db, &bad).is_err(), "numbering gap");
        assert_eq!(exercise_count(&db, p).unwrap(), 0);
    }
}
