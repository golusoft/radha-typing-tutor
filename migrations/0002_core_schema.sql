-- ============================================================================
-- Migration 0002 — Core schema
-- ============================================================================
--
-- The product schema from TASK.MD Section 5: thirteen tables, plus seed rows for
-- the two lookup tables (layouts, products). The seeds are reference data this
-- schema defines, so they live with it.
--
-- Conventions
-- -----------
--   * Integer surrogate keys. Stable string identifiers live in `slug` columns
--     and are exactly the values the Rust side uses: LayoutId::as_str() from
--     radha-layouts and ProductId::slug() from radha-content. Tests assert the
--     seeds against those enums, so the two cannot drift silently.
--   * Timestamps are ISO-8601 text, UTC, from datetime('now').
--   * Booleans are INTEGER 0/1 guarded by a CHECK constraint.
--
-- ON DELETE policy (decided once, applied throughout)
-- ---------------------------------------------------
--   * Rows owned by an exercise (content, progress, sessions, per-exercise
--     bests) CASCADE: removing an exercise removes what only exists because of
--     it.
--   * References to a profile RESTRICT: a profile's history is never deleted as
--     a side effect. Removing a profile is an explicit, deliberate operation.
--   * References to the lookup tables (layouts, products) RESTRICT: they are
--     seeded reference data, and deleting a product must fail loudly rather
--     than silently take its exercises with it.
--
-- RAM-first (RULES.MD Rule 14)
-- ----------------------------
-- Nothing here is keystroke-grained. A completed session is one typing_sessions
-- row, one session_metrics row and a handful of aggregated typing_errors rows,
-- all written in a single transaction when the session ends.
-- ============================================================================


-- ----------------------------------------------------------------------------
-- layouts — the four keyboard layouts (lookup, seeded)
-- ----------------------------------------------------------------------------
CREATE TABLE layouts (
    id           INTEGER PRIMARY KEY,
    -- LayoutId::as_str()
    slug         TEXT    NOT NULL UNIQUE,
    display_name TEXT    NOT NULL
);

INSERT INTO layouts (id, slug, display_name) VALUES
    (1, 'qwerty',   'English QWERTY'),
    (2, 'gail',     'Remington GAIL'),
    (3, 'cbi',      'Remington CBI'),
    (4, 'krutidev', 'Kruti Dev');


-- ----------------------------------------------------------------------------
-- products — the nine Home-screen products (lookup, seeded)
-- ----------------------------------------------------------------------------
CREATE TABLE products (
    id            INTEGER PRIMARY KEY,
    -- ProductId::slug()
    slug          TEXT    NOT NULL UNIQUE,
    display_name  TEXT    NOT NULL,
    layout_id     INTEGER NOT NULL REFERENCES layouts (id) ON DELETE RESTRICT,
    is_beginner   INTEGER NOT NULL DEFAULT 0 CHECK (is_beginner IN (0, 1)),
    -- Home-screen order, matching ProductId::all().
    display_order INTEGER NOT NULL UNIQUE
);

INSERT INTO products (id, slug, display_name, layout_id, is_beginner, display_order) VALUES
    (1, 'english-classic',        'English Typing Mastery',       1, 0, 1),
    (2, 'typing-flow',            'Typing Flow',                  1, 0, 2),
    (3, 'number-punctuation',     'Number + Punctuation Mastery', 1, 0, 3),
    (4, 'hindi-mastery-gail',     'Remington GAIL Typing',        2, 0, 4),
    (5, 'hindi-mastery-cbi',      'Remington CBI Typing',         3, 0, 5),
    (6, 'hindi-mastery-krutidev', 'Kruti Dev Typing',             4, 0, 6),
    (7, 'hindi-learn-gail',       'Remington GAIL',               2, 1, 7),
    (8, 'hindi-learn-cbi',        'Remington CBI',                3, 1, 8),
    (9, 'hindi-learn-krutidev',   'Kruti Dev',                    4, 1, 9);


-- ----------------------------------------------------------------------------
-- profiles
-- ----------------------------------------------------------------------------
CREATE TABLE profiles (
    id           INTEGER PRIMARY KEY,
    display_name TEXT    NOT NULL CHECK (length(trim(display_name)) > 0),
    created_at   TEXT    NOT NULL DEFAULT (datetime('now')),
    last_active  TEXT
);


-- ----------------------------------------------------------------------------
-- exercises — metadata only (the text lives in exercise_content)
-- ----------------------------------------------------------------------------
CREATE TABLE exercises (
    id         INTEGER PRIMARY KEY,
    product_id INTEGER NOT NULL REFERENCES products (id) ON DELETE RESTRICT,
    -- 1-based position within the product ("Exercise 12 / 200").
    number     INTEGER NOT NULL CHECK (number >= 1),
    title      TEXT    NOT NULL,
    -- Curriculum grouping, e.g. 'Home Row'. NULL until content is imported.
    category   TEXT,
    difficulty INTEGER CHECK (difficulty BETWEEN 1 AND 5),
    -- Parent key for the composite foreign key on personal_bests, which
    -- guarantees a per-exercise best can never name the wrong product.
    UNIQUE (id, product_id)
);

-- list_for_product runs WHERE product_id = ? ORDER BY number, served entirely
-- by this index. Unique as well, because two exercises in one product cannot
-- share a position.
CREATE UNIQUE INDEX idx_exercises_product_number ON exercises (product_id, number);


-- ----------------------------------------------------------------------------
-- exercise_content — the text, split out so list queries never read it
-- ----------------------------------------------------------------------------
-- exercise_id is the INTEGER PRIMARY KEY, i.e. this table's own rowid, so the
-- lookup by exercise_id in load() is a direct b-tree seek and needs no separate
-- index. Content is constrained non-empty, which lets an empty string in a list
-- result mean "not loaded" unambiguously.
CREATE TABLE exercise_content (
    exercise_id INTEGER PRIMARY KEY REFERENCES exercises (id) ON DELETE CASCADE,
    content     TEXT    NOT NULL CHECK (length(content) > 0)
);


-- ----------------------------------------------------------------------------
-- exercise_progress — per profile, per exercise
-- ----------------------------------------------------------------------------
-- Bests live in personal_bests, not here, so each figure has exactly one home.
CREATE TABLE exercise_progress (
    profile_id          INTEGER NOT NULL REFERENCES profiles (id) ON DELETE RESTRICT,
    exercise_id         INTEGER NOT NULL REFERENCES exercises (id) ON DELETE CASCADE,
    attempts            INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    completed           INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1)),
    total_practice_secs INTEGER NOT NULL DEFAULT 0 CHECK (total_practice_secs >= 0),
    first_attempt_at    TEXT,
    last_attempt_at     TEXT,
    PRIMARY KEY (profile_id, exercise_id)
);


-- ----------------------------------------------------------------------------
-- typing_sessions — one row per COMPLETED session
-- ----------------------------------------------------------------------------
-- Written once, when the session ends. Abandoned sessions are never persisted.
CREATE TABLE typing_sessions (
    id           INTEGER PRIMARY KEY,
    profile_id   INTEGER NOT NULL REFERENCES profiles (id) ON DELETE RESTRICT,
    exercise_id  INTEGER NOT NULL REFERENCES exercises (id) ON DELETE CASCADE,
    started_at   TEXT    NOT NULL,
    completed_at TEXT    NOT NULL,
    -- Selected timer. NULL for untimed experiences such as Hindi Learn steps.
    timer_secs   INTEGER CHECK (timer_secs > 0),
    elapsed_secs INTEGER NOT NULL CHECK (elapsed_secs >= 0),
    display_mode TEXT    NOT NULL DEFAULT 'word' CHECK (display_mode IN ('key', 'word')),
    -- 'off' also covers products that have no guide (English).
    guide_mode   TEXT    NOT NULL DEFAULT 'off'
                         CHECK (guide_mode IN ('full', 'smart', 'minimal', 'off'))
);

-- Practice history and analytics roll-ups read a profile's sessions by date.
CREATE INDEX idx_typing_sessions_profile_completed
    ON typing_sessions (profile_id, completed_at);
-- Per-exercise history.
CREATE INDEX idx_typing_sessions_exercise ON typing_sessions (exercise_id);


-- ----------------------------------------------------------------------------
-- session_metrics — the results of one session, 1:1 with typing_sessions
-- ----------------------------------------------------------------------------
-- Written in the same transaction as its session. Separate so that a session
-- list can be read without the metric payload.
CREATE TABLE session_metrics (
    session_id     INTEGER PRIMARY KEY REFERENCES typing_sessions (id) ON DELETE CASCADE,
    gross_wpm      REAL    NOT NULL CHECK (gross_wpm >= 0),
    net_wpm        REAL    NOT NULL CHECK (net_wpm >= 0),
    accuracy       REAL    NOT NULL CHECK (accuracy BETWEEN 0 AND 100),
    consistency    REAL    CHECK (consistency BETWEEN 0 AND 100),
    word_accuracy  REAL    CHECK (word_accuracy BETWEEN 0 AND 100),
    correct_chars  INTEGER NOT NULL DEFAULT 0 CHECK (correct_chars >= 0),
    wrong_chars    INTEGER NOT NULL DEFAULT 0 CHECK (wrong_chars >= 0),
    extra_chars    INTEGER NOT NULL DEFAULT 0 CHECK (extra_chars >= 0),
    missing_chars  INTEGER NOT NULL DEFAULT 0 CHECK (missing_chars >= 0),
    backspaces     INTEGER NOT NULL DEFAULT 0 CHECK (backspaces >= 0),
    space_errors   INTEGER NOT NULL DEFAULT 0 CHECK (space_errors >= 0),
    avg_latency_ms REAL    CHECK (avg_latency_ms >= 0)
);


-- ----------------------------------------------------------------------------
-- typing_errors — AGGREGATED error counts per session
-- ----------------------------------------------------------------------------
-- Counted in RAM while typing and written once at the end: one row per distinct
-- (key, expected character, kind) with a count. There is deliberately no
-- position or timestamp column. This is a weak-key summary, not a keystroke log.
--
-- physical_key is the key the error is attributed to: the key that should have
-- been pressed for 'wrong' and 'missing', the key that was pressed for 'extra'.
-- expected_char is empty for 'extra', where nothing was expected.
CREATE TABLE typing_errors (
    session_id    INTEGER NOT NULL REFERENCES typing_sessions (id) ON DELETE CASCADE,
    physical_key  TEXT    NOT NULL,
    expected_char TEXT    NOT NULL DEFAULT '',
    error_kind    TEXT    NOT NULL
                          CHECK (error_kind IN ('wrong', 'extra', 'missing', 'space')),
    occurrences   INTEGER NOT NULL CHECK (occurrences > 0),
    PRIMARY KEY (session_id, physical_key, expected_char, error_kind)
);


-- ----------------------------------------------------------------------------
-- personal_bests — per exercise AND per product (TASK.MD Section 32)
-- ----------------------------------------------------------------------------
--   exercise_id NOT NULL  ->  best on that exercise
--   exercise_id NULL      ->  best across the whole product
--
-- Each metric is an independent maximum and may come from a different session,
-- so no single session is referenced.
CREATE TABLE personal_bests (
    id               INTEGER PRIMARY KEY,
    profile_id       INTEGER NOT NULL REFERENCES profiles (id) ON DELETE RESTRICT,
    product_id       INTEGER NOT NULL REFERENCES products (id) ON DELETE RESTRICT,
    exercise_id      INTEGER,
    best_wpm         REAL    NOT NULL CHECK (best_wpm >= 0),
    best_net_wpm     REAL    NOT NULL CHECK (best_net_wpm >= 0),
    best_accuracy    REAL    NOT NULL CHECK (best_accuracy BETWEEN 0 AND 100),
    best_consistency REAL    CHECK (best_consistency BETWEEN 0 AND 100),
    -- Composite score. Its formula arrives with the Result engine (TASK.MD
    -- Section 22); NULL until then.
    best_score       REAL,
    updated_at       TEXT    NOT NULL DEFAULT (datetime('now')),
    -- A per-exercise best must name the exercise's own product. Product-level
    -- rows are exempt: SQLite skips a foreign-key check when any child column is
    -- NULL, and for those rows exercise_id is NULL.
    FOREIGN KEY (exercise_id, product_id)
        REFERENCES exercises (id, product_id) ON DELETE CASCADE
);

-- At most one best per profile per exercise, and one per profile per product.
-- Partial indexes, because a plain UNIQUE constraint treats NULLs as distinct
-- and would allow duplicate product-level rows.
CREATE UNIQUE INDEX idx_personal_bests_exercise
    ON personal_bests (profile_id, exercise_id) WHERE exercise_id IS NOT NULL;
CREATE UNIQUE INDEX idx_personal_bests_product
    ON personal_bests (profile_id, product_id) WHERE exercise_id IS NULL;


-- ----------------------------------------------------------------------------
-- analytics — daily roll-ups per profile and product
-- ----------------------------------------------------------------------------
-- Updated in the same transaction as each session write, so the Analytics
-- screen reads a few hundred rows instead of scanning every session. Per-layout
-- figures derive through products.layout_id.
CREATE TABLE analytics (
    profile_id    INTEGER NOT NULL REFERENCES profiles (id) ON DELETE RESTRICT,
    product_id    INTEGER NOT NULL REFERENCES products (id) ON DELETE RESTRICT,
    -- Local calendar date, 'YYYY-MM-DD'.
    day           TEXT    NOT NULL,
    sessions      INTEGER NOT NULL DEFAULT 0 CHECK (sessions >= 0),
    practice_secs INTEGER NOT NULL DEFAULT 0 CHECK (practice_secs >= 0),
    avg_wpm       REAL    CHECK (avg_wpm >= 0),
    avg_accuracy  REAL    CHECK (avg_accuracy BETWEEN 0 AND 100),
    best_wpm      REAL    CHECK (best_wpm >= 0),
    PRIMARY KEY (profile_id, day, product_id)
);


-- ----------------------------------------------------------------------------
-- settings — per-profile preferences as name/value pairs
-- ----------------------------------------------------------------------------
-- Name/value pairs so the Settings screen (TASK.MD Section 27) can grow without
-- a migration per preference. Machine-level shell settings (window geometry,
-- theme) stay in settings.toml, because they are needed before the database is
-- opened.
CREATE TABLE settings (
    profile_id INTEGER NOT NULL REFERENCES profiles (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL,
    value      TEXT    NOT NULL,
    updated_at TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (profile_id, name)
);


-- ----------------------------------------------------------------------------
-- guide_settings — Hindi Guide preferences per profile AND per layout
-- ----------------------------------------------------------------------------
-- Per layout because GAIL, CBI and Kruti Dev are independent (RULES.MD Rule 9):
-- a learner can keep the full guide on one layout and a minimal one on another.
-- Typed columns rather than name/value pairs, because the option set is fixed
-- by the PRD's guide customisation list. guide_mode 'off' disables the guide.
CREATE TABLE guide_settings (
    profile_id              INTEGER NOT NULL REFERENCES profiles (id) ON DELETE RESTRICT,
    layout_id               INTEGER NOT NULL REFERENCES layouts (id) ON DELETE RESTRICT,
    guide_mode              TEXT    NOT NULL DEFAULT 'full'
                                    CHECK (guide_mode IN ('full', 'smart', 'minimal', 'off')),
    show_current_key        INTEGER NOT NULL DEFAULT 1 CHECK (show_current_key IN (0, 1)),
    show_next_key           INTEGER NOT NULL DEFAULT 1 CHECK (show_next_key IN (0, 1)),
    show_character          INTEGER NOT NULL DEFAULT 1 CHECK (show_character IN (0, 1)),
    show_key_mapping        INTEGER NOT NULL DEFAULT 1 CHECK (show_key_mapping IN (0, 1)),
    show_hand               INTEGER NOT NULL DEFAULT 1 CHECK (show_hand IN (0, 1)),
    show_finger             INTEGER NOT NULL DEFAULT 1 CHECK (show_finger IN (0, 1)),
    show_keyboard_highlight INTEGER NOT NULL DEFAULT 1 CHECK (show_keyboard_highlight IN (0, 1)),
    show_hint               INTEGER NOT NULL DEFAULT 1 CHECK (show_hint IN (0, 1)),
    -- Off by default: animation is the first thing to give up on low-end PCs.
    show_finger_animation   INTEGER NOT NULL DEFAULT 0 CHECK (show_finger_animation IN (0, 1)),
    updated_at              TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (profile_id, layout_id)
);
