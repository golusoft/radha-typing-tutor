//! Flow engine thresholds as versioned data (PRD FL-05, FL-06).
//!
//! The shipped values live in `content/flow/flow-config.toml`. The file is a
//! flat `key = number` list (a TOML subset) with a mandatory `schema`
//! version. [`FlowConfig::parse`] is strict: an unknown key, a repeated key, a
//! value that is not a finite number or a value out of range rejects the
//! whole file, so a typo can never silently change the engine. A key that is
//! absent keeps its built-in default.
//!
//! Nothing here runs per keystroke: the tracker copies the values it needs
//! into integers once, when it is built.

use crate::{CoreError, Result};

/// The schema this build reads. A file with any other `schema` is rejected.
pub const FLOW_CONFIG_SCHEMA: u32 = 1;

/// Largest live window the tracker accepts (intervals).
pub const MAX_WINDOW: usize = 4096;

/// The shipped configuration file, embedded so the engine always has it.
pub const DEFAULT_CONFIG_TEXT: &str = include_str!("../../../../content/flow/flow-config.toml");

/// Every threshold the Flow engine uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowConfig {
    /// Schema version of the data ([`FLOW_CONFIG_SCHEMA`]).
    pub schema: u32,
    /// Intervals in the live window.
    pub window: usize,
    /// Below this an interval is a rollover, not rhythm (ms).
    pub min_interval_ms: f64,
    /// At or above this an interval is a pause (ms).
    pub pause_ms: f64,
    /// Intervals after a (re)start that only set the tempo.
    pub warmup: u32,
    /// EWMA weight of the newest interval.
    pub tempo_alpha: f64,
    /// Smooth band around the tempo (fraction).
    pub smooth_band: f64,
    /// Smooth band for word-boundary intervals (fraction).
    pub boundary_band: f64,
    /// Hesitation: slower than tempo × this.
    pub hesitation_factor: f64,
    /// Live smoothness % for "steady".
    pub steady_smoothness: f64,
    /// Live smoothness % for "settling" (below is "uneven").
    pub settling_smoothness: f64,
    /// Session cadence variance (CV) that still counts as even.
    pub even_cv: f64,
    /// Attempts before a key can be judged weak.
    pub weak_min_attempts: u32,
    /// Weakness score at or above which a key is weak (0–100).
    pub weak_score: f64,
    /// Weight of the error rate in the weakness score.
    pub weak_error_weight: f64,
    /// Weight of the latency penalty.
    pub weak_latency_weight: f64,
    /// Weight of error recurrence.
    pub weak_recurrence_weight: f64,
    /// Errors at which recurrence saturates.
    pub weak_recurrence_errors: f64,
    /// Weak keys reported at most.
    pub max_weak_keys: usize,
    /// Words in a recommended drill.
    pub drill_words: usize,
    /// Below this key accuracy % the drill isolates the key.
    pub raw_key_accuracy: f64,
    /// Repetitions of an isolated key group.
    pub key_reps: usize,
}

impl Default for FlowConfig {
    /// The values of `content/flow/flow-config.toml` (a test keeps the two
    /// equal).
    fn default() -> Self {
        FlowConfig {
            schema: FLOW_CONFIG_SCHEMA,
            window: 64,
            min_interval_ms: 8.0,
            pause_ms: 2000.0,
            warmup: 4,
            tempo_alpha: 0.15,
            smooth_band: 0.45,
            boundary_band: 0.75,
            hesitation_factor: 2.5,
            steady_smoothness: 85.0,
            settling_smoothness: 65.0,
            even_cv: 0.35,
            weak_min_attempts: 5,
            weak_score: 20.0,
            weak_error_weight: 60.0,
            weak_latency_weight: 25.0,
            weak_recurrence_weight: 15.0,
            weak_recurrence_errors: 10.0,
            max_weak_keys: 5,
            drill_words: 48,
            raw_key_accuracy: 80.0,
            key_reps: 4,
        }
    }
}

fn bad(detail: impl Into<String>) -> CoreError {
    CoreError::malformed("flow config", "flow-config.toml", detail.into())
}

/// A non-negative whole number that fits `T`.
fn whole<T: TryFrom<u64>>(key: &str, v: f64) -> Result<T> {
    if v < 0.0 || v.fract() != 0.0 || v > 1e12 {
        return Err(bad(format!("{key} must be a whole number, got {v}")));
    }
    T::try_from(v as u64).map_err(|_| bad(format!("{key} is too large: {v}")))
}

impl FlowConfig {
    /// The embedded shipped configuration, or the built-in defaults if it
    /// were ever unreadable (it is checked by a test, so it is not).
    #[must_use]
    pub fn shipped() -> Self {
        Self::parse(DEFAULT_CONFIG_TEXT).unwrap_or_default()
    }

    /// Parse a configuration file (see the module docs for the rules).
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] naming the first broken rule.
    pub fn parse(text: &str) -> Result<Self> {
        let mut cfg = FlowConfig::default();
        let mut seen: Vec<&str> = Vec::new();
        let mut schema = None;
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(bad(format!("line {}: expected `key = value`", n + 1)));
            };
            let key = key.trim();
            if seen.contains(&key) {
                return Err(bad(format!("line {}: {key} is set twice", n + 1)));
            }
            seen.push(key);
            let v: f64 = value
                .trim()
                .parse()
                .map_err(|_| bad(format!("line {}: {key} is not a number", n + 1)))?;
            if !v.is_finite() {
                return Err(bad(format!("line {}: {key} is not finite", n + 1)));
            }
            match key {
                "schema" => schema = Some(whole::<u32>(key, v)?),
                "window" => cfg.window = whole(key, v)?,
                "min_interval_ms" => cfg.min_interval_ms = v,
                "pause_ms" => cfg.pause_ms = v,
                "warmup" => cfg.warmup = whole(key, v)?,
                "tempo_alpha" => cfg.tempo_alpha = v,
                "smooth_band" => cfg.smooth_band = v,
                "boundary_band" => cfg.boundary_band = v,
                "hesitation_factor" => cfg.hesitation_factor = v,
                "steady_smoothness" => cfg.steady_smoothness = v,
                "settling_smoothness" => cfg.settling_smoothness = v,
                "even_cv" => cfg.even_cv = v,
                "weak_min_attempts" => cfg.weak_min_attempts = whole(key, v)?,
                "weak_score" => cfg.weak_score = v,
                "weak_error_weight" => cfg.weak_error_weight = v,
                "weak_latency_weight" => cfg.weak_latency_weight = v,
                "weak_recurrence_weight" => cfg.weak_recurrence_weight = v,
                "weak_recurrence_errors" => cfg.weak_recurrence_errors = v,
                "max_weak_keys" => cfg.max_weak_keys = whole(key, v)?,
                "drill_words" => cfg.drill_words = whole(key, v)?,
                "raw_key_accuracy" => cfg.raw_key_accuracy = v,
                "key_reps" => cfg.key_reps = whole(key, v)?,
                other => return Err(bad(format!("line {}: unknown key {other:?}", n + 1))),
            }
        }
        match schema {
            Some(FLOW_CONFIG_SCHEMA) => cfg.schema = FLOW_CONFIG_SCHEMA,
            Some(other) => {
                return Err(bad(format!(
                    "schema {other} is not supported (this build reads {FLOW_CONFIG_SCHEMA})"
                )))
            }
            None => return Err(bad("missing `schema`")),
        }
        cfg.validate()?;
        Ok(cfg)
    }

    /// Check every value is in range.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] naming the first value out of range.
    pub fn validate(&self) -> Result<()> {
        let range = |name: &str, v: f64, lo: f64, hi: f64| -> Result<()> {
            if v.is_finite() && (lo..=hi).contains(&v) {
                Ok(())
            } else {
                Err(bad(format!("{name} = {v} is outside {lo}..={hi}")))
            }
        };
        if self.window == 0 || self.window > MAX_WINDOW {
            return Err(bad(format!(
                "window = {} is outside 1..={MAX_WINDOW}",
                self.window
            )));
        }
        range("min_interval_ms", self.min_interval_ms, 1.0, 200.0)?;
        range("pause_ms", self.pause_ms, 300.0, 60_000.0)?;
        if self.pause_ms <= self.min_interval_ms {
            return Err(bad("pause_ms must exceed min_interval_ms"));
        }
        range("warmup", f64::from(self.warmup), 1.0, 64.0)?;
        range("tempo_alpha", self.tempo_alpha, 0.001, 1.0)?;
        range("smooth_band", self.smooth_band, 0.01, 5.0)?;
        range("boundary_band", self.boundary_band, 0.01, 5.0)?;
        range("hesitation_factor", self.hesitation_factor, 1.0, 50.0)?;
        range("steady_smoothness", self.steady_smoothness, 0.0, 100.0)?;
        range("settling_smoothness", self.settling_smoothness, 0.0, 100.0)?;
        if self.settling_smoothness > self.steady_smoothness {
            return Err(bad("settling_smoothness must not exceed steady_smoothness"));
        }
        range("even_cv", self.even_cv, 0.0, 10.0)?;
        range("weak_score", self.weak_score, 0.0, 100.0)?;
        range("weak_error_weight", self.weak_error_weight, 0.0, 100.0)?;
        range("weak_latency_weight", self.weak_latency_weight, 0.0, 100.0)?;
        range(
            "weak_recurrence_weight",
            self.weak_recurrence_weight,
            0.0,
            100.0,
        )?;
        range(
            "weak_recurrence_errors",
            self.weak_recurrence_errors,
            1.0,
            10_000.0,
        )?;
        range("max_weak_keys", self.max_weak_keys as f64, 0.0, 32.0)?;
        range("drill_words", self.drill_words as f64, 1.0, 500.0)?;
        range("raw_key_accuracy", self.raw_key_accuracy, 0.0, 100.0)?;
        range("key_reps", self.key_reps as f64, 1.0, 32.0)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_file_parses_to_the_built_in_defaults() {
        let parsed = FlowConfig::parse(DEFAULT_CONFIG_TEXT).unwrap();
        assert_eq!(parsed, FlowConfig::default());
        assert_eq!(FlowConfig::shipped(), FlowConfig::default());
        FlowConfig::default().validate().unwrap();
    }

    #[test]
    fn missing_keys_keep_defaults_and_set_keys_apply() {
        let cfg = FlowConfig::parse("schema = 1\nwindow = 32 # live\n\n# note\n").unwrap();
        assert_eq!(cfg.window, 32);
        assert_eq!(cfg.pause_ms, FlowConfig::default().pause_ms);
    }

    #[test]
    fn strict_rules_reject_bad_files() {
        for (text, why) in [
            ("window = 32", "missing schema"),
            ("schema = 2", "future schema"),
            ("schema = 1\nwindw = 3", "unknown key"),
            ("schema = 1\nwindow = 3\nwindow = 4", "twice"),
            ("schema = 1\nwindow = lots", "not a number"),
            ("schema = 1\nwindow = 2.5", "fraction"),
            ("schema = 1\nwindow = 0", "zero window"),
            ("schema = 1\nwindow = 100000", "huge window"),
            ("schema = 1\npause_ms = inf", "infinite"),
            ("schema = 1\npause_ms = NaN", "nan"),
            ("schema = 1\nsmooth_band = -1", "negative band"),
            (
                "schema = 1\nsettling_smoothness = 90",
                "settling above steady",
            ),
            ("schema = 1\njust words", "no equals"),
            ("schema = -1", "negative schema"),
        ] {
            assert!(FlowConfig::parse(text).is_err(), "{why}: {text:?}");
        }
    }
}
