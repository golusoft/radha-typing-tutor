-- ============================================================================
-- 0003_content_v2 — P04 content system
-- ============================================================================
-- Backward compatible: only new tables and new nullable / defaulted columns.
-- Rows written by 0002 stay valid and readable:
--   * exercise_content (plain text) is still read when an exercise has no row
--     in exercise_text yet; the next content import moves it.
--   * typing_sessions rows keep mode = NULL ("recorded before modes").

-- ----------------------------------------------------------------------------
-- exercises: review status, Hindi Learn track, list metadata
-- ----------------------------------------------------------------------------
-- status: 'verified' (source records an approved review), 'provisional'
-- (imported without one), 'generated-review-pending' (Radha's own generator).
-- Existing rows default to 'provisional' until re-imported; never upgraded
-- silently.
ALTER TABLE exercises ADD COLUMN status TEXT NOT NULL DEFAULT 'provisional'
    CHECK (status IN ('verified', 'provisional', 'generated-review-pending'));
-- Hindi Learn: BEGINNER | CLASSIC toggle. NULL for every other product.
ALTER TABLE exercises ADD COLUMN track TEXT CHECK (track IN ('beginner', 'classic'));
-- Metadata for list cards, so a list never touches the text.
ALTER TABLE exercises ADD COLUMN word_count INTEGER CHECK (word_count >= 0);
ALTER TABLE exercises ADD COLUMN char_count INTEGER CHECK (char_count >= 0);

-- ----------------------------------------------------------------------------
-- exercise_text — compressed text, split from metadata
-- ----------------------------------------------------------------------------
-- codec 'deflate' (raw deflate of the UTF-8 text) or 'plain'. sha256 is of
-- the decoded UTF-8 text and is checked on import.
CREATE TABLE exercise_text (
    exercise_id INTEGER PRIMARY KEY REFERENCES exercises (id) ON DELETE CASCADE,
    codec       TEXT    NOT NULL CHECK (codec IN ('plain', 'deflate')),
    body        BLOB    NOT NULL CHECK (length(body) > 0),
    sha256      TEXT    NOT NULL CHECK (length(sha256) = 64)
);

-- ----------------------------------------------------------------------------
-- Exam passage pools (one per language) and their passages
-- ----------------------------------------------------------------------------
CREATE TABLE exam_pools (
    id           INTEGER PRIMARY KEY,
    slug         TEXT    NOT NULL UNIQUE,
    language     TEXT    NOT NULL UNIQUE CHECK (language IN ('hindi', 'english')),
    display_name TEXT    NOT NULL
);
INSERT INTO exam_pools (slug, language, display_name) VALUES
    ('exam-pool-hindi',   'hindi',   'Exam passages (Hindi)'),
    ('exam-pool-english', 'english', 'Exam passages (English)');

CREATE TABLE exam_passages (
    id         INTEGER PRIMARY KEY,
    pool_id    INTEGER NOT NULL REFERENCES exam_pools (id) ON DELETE RESTRICT,
    number     INTEGER NOT NULL CHECK (number >= 1),
    title      TEXT    NOT NULL,
    category   TEXT,
    difficulty INTEGER CHECK (difficulty BETWEEN 1 AND 5),
    word_count INTEGER NOT NULL CHECK (word_count > 0),
    char_count INTEGER NOT NULL CHECK (char_count > 0),
    status     TEXT    NOT NULL
               CHECK (status IN ('verified', 'provisional', 'generated-review-pending'))
);
-- The pool grid lists by (pool, number), served by this index.
CREATE UNIQUE INDEX idx_exam_passages_pool_number ON exam_passages (pool_id, number);

-- One row per encoding: 'unicode' (GAIL / CBI / QWERTY) and, for the Hindi
-- pool, 'krutidev' (legacy key stream generated at import).
CREATE TABLE exam_passage_text (
    passage_id INTEGER NOT NULL REFERENCES exam_passages (id) ON DELETE CASCADE,
    encoding   TEXT    NOT NULL CHECK (encoding IN ('unicode', 'krutidev')),
    codec      TEXT    NOT NULL CHECK (codec IN ('plain', 'deflate')),
    body       BLOB    NOT NULL CHECK (length(body) > 0),
    sha256     TEXT    NOT NULL CHECK (length(sha256) = 64),
    PRIMARY KEY (passage_id, encoding)
) WITHOUT ROWID;

-- Exam pool linkage: which pool an exam profile (radha-exams registry id)
-- draws its passages from, per language. Filled by the profile registry.
CREATE TABLE exam_profile_pools (
    exam_profile_id TEXT    NOT NULL CHECK (length(exam_profile_id) > 0),
    language        TEXT    NOT NULL CHECK (language IN ('hindi', 'english')),
    pool_id         INTEGER NOT NULL REFERENCES exam_pools (id) ON DELETE CASCADE,
    PRIMARY KEY (exam_profile_id, language)
) WITHOUT ROWID;

-- ----------------------------------------------------------------------------
-- content_provenance — where every stored passage came from
-- ----------------------------------------------------------------------------
CREATE TABLE content_provenance (
    id             INTEGER PRIMARY KEY,
    target_kind    TEXT    NOT NULL CHECK (target_kind IN ('exercise', 'exam_passage')),
    target_id      INTEGER NOT NULL,
    -- Source file relative to its source root, or 'generated:<pipeline>'.
    source_path    TEXT    NOT NULL CHECK (length(source_path) > 0),
    source_id      TEXT,
    -- sha256 of the source file bytes (generated: of the generator inputs).
    source_sha256  TEXT    NOT NULL CHECK (length(source_sha256) = 64),
    -- sha256 of the stored (normalised, trimmed) text.
    content_sha256 TEXT    NOT NULL CHECK (length(content_sha256) = 64),
    status         TEXT    NOT NULL
                   CHECK (status IN ('verified', 'provisional', 'generated-review-pending')),
    imported_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (target_kind, target_id)
);

-- One row per imported bundle, so a launch can tell whether the shipped
-- bundle changed since the last import.
CREATE TABLE content_imports (
    target_slug   TEXT    PRIMARY KEY,
    bundle_sha256 TEXT    NOT NULL CHECK (length(bundle_sha256) = 64),
    item_count    INTEGER NOT NULL CHECK (item_count >= 0),
    source        TEXT    NOT NULL,
    imported_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
) WITHOUT ROWID;

-- ----------------------------------------------------------------------------
-- typing_sessions: mode and exam linkage
-- ----------------------------------------------------------------------------
-- NULL for sessions recorded before 0003.
ALTER TABLE typing_sessions ADD COLUMN mode TEXT
    CHECK (mode IN ('learn', 'test', 'beginner', 'classic', 'flow', 'exam'));
ALTER TABLE typing_sessions ADD COLUMN exam_profile_id TEXT;
ALTER TABLE typing_sessions ADD COLUMN exam_passage_id INTEGER
    REFERENCES exam_passages (id) ON DELETE SET NULL;
