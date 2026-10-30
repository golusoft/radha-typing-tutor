//! Rhythm consistency, per `PRD.md` Section 9.3.
//!
//! Neither Golu codebase has a consistency metric, so Radha defines its own
//! from the PRD formula: one minus the coefficient of variation (CV) of the
//! intervals between accepted key presses, on a 0–100 scale.
//!
//! ```text
//! n  = number of intervals            (n == 0  → 100)
//! μ  = Σxᵢ / n                         (μ == 0  → 100)
//! σ  = sqrt(Σ(xᵢ − μ)² / n)            population standard deviation
//! CV = σ / μ
//! consistency = clamp(100 × (1 − min(CV, 1)), 0, 100)
//! ```
//!
//! Intervals are taken in f64 milliseconds (`as_secs_f64() × 1000`), not the
//! PRD sketch's truncating `as_millis`, so sub-millisecond rhythm is not
//! rounded away. The PRD's worked example "100, 300, 150 ms → ~65%" does not
//! follow from its own formula; the exact value is ≈53.6%, and the formula
//! wins (logged in `docs/golu-gap-log.md`).
//!
//! Why CV rather than raw σ: CV is scale-free, so a slow steady typist and a
//! fast steady typist both score high. Why "no intervals → 100": nothing
//! typed has no irregularity, matching the accuracy convention.

use std::time::Duration;

/// Consistency of `intervals`, 0–100 (higher = steadier rhythm).
#[must_use]
pub fn consistency(intervals: &[Duration]) -> f64 {
    if intervals.is_empty() {
        return 100.0;
    }
    let n = intervals.len() as f64;
    let ms: Vec<f64> = intervals.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    let mean = ms.iter().sum::<f64>() / n;
    if mean <= 0.0 || mean.is_nan() {
        return 100.0;
    }
    let variance = ms.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let cv = variance.sqrt() / mean;
    (100.0 * (1.0 - cv.min(1.0))).clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(values: &[u64]) -> Vec<Duration> {
        values.iter().map(|&v| Duration::from_millis(v)).collect()
    }

    #[test]
    fn uniform_rhythm_is_one_hundred() {
        assert_eq!(consistency(&ms(&[200, 200, 200])), 100.0);
    }

    #[test]
    fn the_prd_variable_example_follows_the_formula() {
        let c = consistency(&ms(&[100, 300, 150]));
        // μ = 183.33, σ = 84.98, CV = 0.4635 → 53.65 (the PRD comment says ~65).
        assert!((c - 53.645).abs() < 1e-2, "got {c}");
    }

    #[test]
    fn empty_single_and_zero_mean_are_one_hundred() {
        assert_eq!(consistency(&[]), 100.0);
        assert_eq!(consistency(&ms(&[250])), 100.0);
        assert_eq!(consistency(&ms(&[0, 0])), 100.0);
    }

    #[test]
    fn a_coefficient_of_variation_above_one_is_zero() {
        // μ = 250, σ ≈ 433 → CV ≈ 1.73 → clamped to 0.
        assert_eq!(consistency(&ms(&[0, 0, 0, 1000])), 0.0);
    }
}
