-- ============================================================================
-- 0004_flow_rhythm — P07 Typing Flow rhythm analysis per session
-- ============================================================================
-- Additive only: one new table. Sessions of every other product have no row
-- here; Flow sessions get exactly one, written by the same single
-- save_session transaction as the session itself (RULES.MD Rule 14).
--
-- smoothness and the cadence figures are NULL when the session was too short
-- to score a rhythm (never a made-up 100 %). engine_schema is the
-- FLOW_CONFIG_SCHEMA of the thresholds that produced the row, so figures from
-- a future threshold schema are never compared blindly with these.

CREATE TABLE flow_rhythm (
    session_id       INTEGER PRIMARY KEY REFERENCES typing_sessions (id) ON DELETE CASCADE,
    engine_schema    INTEGER NOT NULL CHECK (engine_schema >= 1),
    smoothness       REAL    CHECK (smoothness BETWEEN 0 AND 100),
    cadence_cv       REAL    CHECK (cadence_cv >= 0),
    mean_interval_ms REAL    CHECK (mean_interval_ms >= 0),
    scored_intervals INTEGER NOT NULL DEFAULT 0 CHECK (scored_intervals >= 0),
    hesitations      INTEGER NOT NULL DEFAULT 0 CHECK (hesitations >= 0),
    pauses           INTEGER NOT NULL DEFAULT 0 CHECK (pauses >= 0),
    corrections      INTEGER NOT NULL DEFAULT 0 CHECK (corrections >= 0),
    -- The weak keys, worst first, as typed characters ("q,;").
    weak_keys        TEXT    NOT NULL DEFAULT ''
);
