//! The Typing Flow engine (PRD 17.8): rhythm, cadence variance, smoothness,
//! weak keys, drills and recommendations.
//!
//! Inspired by Golu Typing OS (`src-tauri/src/adaptive/*`,
//! `src/lib/typingEngine.ts`) and re-implemented with the gaps found in
//! review closed (`docs/flow-gap-log.md`).
//!
//! Two halves:
//!
//! - [`tracker`]: per keystroke, fixed memory, no allocation, time only from
//!   injected [`std::time::Instant`]s.
//! - [`analysis`]: once per session — the report, the drill, the advice.
//!
//! Every threshold is data ([`config::FlowConfig`], schema-versioned).

pub mod analysis;
pub mod config;
pub mod tracker;

pub use analysis::{
    drill, recommend, weakness_score, CorridorPoint, DrillStage, FlowDrill, FlowReport,
    Recommendation, RecommendationKind, WeakKey, WeakPair,
};
pub use config::{FlowConfig, DEFAULT_CONFIG_TEXT, FLOW_CONFIG_SCHEMA};
pub use tracker::{CadenceBar, FlowTracker, IntervalCounts, RhythmLabel, MAX_BARS};
