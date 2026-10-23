-- ============================================================================
-- Migration 0001 — Initialise migration tracking
-- ============================================================================
--
-- Phase 0 scope: this migration intentionally creates ONLY the tracking table.
--
-- The product schema (profiles, products, layouts, exercises, exercise_progress,
-- typing_sessions, session_metrics, typing_errors, settings, guide_settings) is
-- designed in Phase 3 per TASK.MD Section 5, and arrives as migration 0002+.
-- Creating those tables now would lock in a schema before the engine that uses
-- them exists.
-- ============================================================================

CREATE TABLE IF NOT EXISTS schema_migrations (
    -- Numeric version, taken from the migration filename prefix.
    version     INTEGER PRIMARY KEY,

    -- Filename, retained so a human reading the table can identify the
    -- migration without cross-referencing the source tree.
    name        TEXT    NOT NULL,

    -- When it was applied. Useful when diagnosing an upgrade on a user machine.
    applied_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);
