//! The per-keystroke half of the Flow engine: rhythm, cadence and key
//! tallies, in fixed memory.
//!
//! [`FlowTracker::key_at`] runs on every keystroke. It does no allocation, no
//! hashing and no wall-clock read: every buffer is allocated once in
//! [`FlowTracker::new`] and time arrives as an injected [`Instant`]. Memory is
//! bounded by the configured window, a fixed key table and a fixed corridor,
//! however long the session runs (an 8-hour soak test checks this).
//!
//! ## What an interval is
//!
//! The time between two consecutive key events. It is classified once:
//!
//! - **clock anomaly**: the new instant is *earlier* than the last one (a
//!   replayed or injected clock going backwards). Re-anchored, not scored.
//! - **rollover**: shorter than `min_interval_ms` (two keys pressed together,
//!   or a zero interval). Not rhythm, not scored, never divided by.
//! - **pause**: `pause_ms` or longer (a break, a sleep/resume clock jump).
//!   The rhythm restarts with a fresh warm-up; not scored.
//! - **after a correction**: the interval into and out of a Backspace is
//!   not the typist's cadence; not scored.
//! - **warm-up**: the first `warmup` intervals of a rhythm only set the tempo.
//! - **scored**: everything else. It is *smooth* when it lies within the
//!   band around the tempo (a wider band at word boundaries, where a short
//!   pause is normal rhythm — the gap Golu OS worked around by abandoning
//!   per-key variance), and a *hesitation* when it is slower than the tempo
//!   by `hesitation_factor`.
//!
//! The tempo is an exponential moving average of scored intervals, so the
//! engine judges evenness against the typist's own pace.
//!
//! Sums are exact integers (microseconds, `u64` / `u128`): no floating
//! drift over hours, and the variance never goes negative.

use std::time::{Duration, Instant};

use super::config::FlowConfig;

/// Keys tracked individually: printable ASCII (space to `~`). Everything
/// else shares one "other" slot.
pub const KEY_SLOTS: usize = 96;

/// Letter-to-letter transitions tracked (`a`–`z` × `a`–`z`).
pub const PAIR_SLOTS: usize = 26 * 26;

/// Buckets of the session's smoothness corridor (the Result graph).
pub const CORRIDOR_BUCKETS: usize = 96;

/// Bars the live cadence wave can show at most.
pub const MAX_BARS: usize = 64;

const OTHER_SLOT: usize = KEY_SLOTS - 1;

/// One key's tallies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyTally {
    /// Times the key was the expected one.
    pub attempts: u32,
    /// Times it was missed.
    pub errors: u32,
    /// Sum of the scored intervals that ended on it (µs).
    pub latency_us: u64,
    /// Number of those intervals.
    pub latency_n: u32,
}

/// One letter pair's tallies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PairTally {
    /// Times the pair was typed.
    pub attempts: u32,
    /// Times either of its two keys was missed.
    pub errors: u32,
}

/// One live bar: the interval relative to the tempo, and whether it was
/// smooth.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CadenceBar {
    /// 0–1; 0.5 is exactly on tempo.
    pub level: f32,
    /// Inside the smooth band.
    pub smooth: bool,
}

/// One corridor bucket: scored intervals, smooth ones, their sum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CorridorBucket {
    /// Scored intervals in the bucket.
    pub scored: u32,
    /// Smooth ones.
    pub smooth: u32,
    /// Sum of their lengths (µs).
    pub sum_us: u64,
}

/// What the live rhythm sounds like right now (DESIGN 8.2 rhythm language).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RhythmLabel {
    /// Not enough intervals yet to judge.
    WarmingUp,
    /// Live smoothness at or above `steady_smoothness`.
    Steady,
    /// Between the two thresholds.
    Settling,
    /// Below `settling_smoothness`.
    Uneven,
}

impl RhythmLabel {
    /// The word the Typing screen shows.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            RhythmLabel::WarmingUp => "finding your rhythm",
            RhythmLabel::Steady => "steady",
            RhythmLabel::Settling => "settling",
            RhythmLabel::Uneven => "uneven",
        }
    }

    /// The tone token the screen colours by.
    #[must_use]
    pub fn tone(self) -> &'static str {
        match self {
            RhythmLabel::WarmingUp => "neutral",
            RhythmLabel::Steady => "steady",
            RhythmLabel::Settling => "settling",
            RhythmLabel::Uneven => "uneven",
        }
    }
}

/// Counts of every interval class (see the module docs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IntervalCounts {
    /// Key events seen (characters and corrections).
    pub keystrokes: u64,
    /// Scored intervals.
    pub scored: u64,
    /// Smooth scored intervals.
    pub smooth: u64,
    /// Hesitations among the scored ones.
    pub hesitations: u32,
    /// Pauses (rhythm restarts).
    pub pauses: u32,
    /// Rollovers (too short to be rhythm, including zero).
    pub rollovers: u32,
    /// Corrections (Backspace).
    pub corrections: u32,
    /// Instants that went backwards.
    pub clock_anomalies: u32,
    /// Warm-up intervals.
    pub warmup: u32,
}

/// The fixed-memory Flow tracker. Build one per session (or reuse one with
/// [`FlowTracker::reset`]).
#[derive(Debug, Clone)]
pub struct FlowTracker {
    cfg: FlowConfig,
    // Thresholds in microseconds, derived once.
    min_us: u64,
    pause_us: u64,

    // Live window (ring buffers, allocated once).
    ring_us: Box<[u32]>,
    ring_smooth: Box<[bool]>,
    head: usize,
    len: usize,
    ring_smooth_count: usize,

    // Session totals (exact).
    counts: IntervalCounts,
    sum_us: u64,
    sumsq_us: u128,

    // Rhythm state.
    tempo_us: f64,
    warm: u32,
    last: Option<Instant>,
    last_boundary: bool,
    skip_next: bool,

    // Key and pair tables.
    keys: Box<[KeyTally; KEY_SLOTS]>,
    pairs: Box<[PairTally; PAIR_SLOTS]>,
    prev_letter: Option<(u8, bool)>,

    // Smoothness corridor (bounded downsampling: buckets merge pairwise
    // and the span doubles when they fill).
    corridor: [CorridorBucket; CORRIDOR_BUCKETS],
    corridor_len: usize,
    corridor_span: u64,
    corridor_fill: u64,
}

fn duration_us(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// The key-table slot of `c`.
#[must_use]
pub fn key_slot(c: char) -> usize {
    let code = c as u32;
    if (0x20..0x7F).contains(&code) {
        (code - 0x20) as usize
    } else {
        OTHER_SLOT
    }
}

/// The character of a key-table slot (`None` for the "other" slot).
#[must_use]
pub fn slot_char(slot: usize) -> Option<char> {
    if slot >= OTHER_SLOT {
        return None;
    }
    char::from_u32(u32::try_from(slot).ok()? + 0x20)
}

fn letter_index(c: char) -> Option<u8> {
    c.is_ascii_alphabetic()
        .then(|| c.to_ascii_lowercase() as u8 - b'a')
}

impl FlowTracker {
    /// A tracker for `cfg`. An invalid configuration falls back to the
    /// built-in defaults (the app validates before it gets here).
    #[must_use]
    pub fn new(cfg: FlowConfig) -> Self {
        let cfg = if cfg.validate().is_ok() {
            cfg
        } else {
            FlowConfig::default()
        };
        let window = cfg.window.max(1);
        FlowTracker {
            min_us: (cfg.min_interval_ms * 1000.0) as u64,
            pause_us: (cfg.pause_ms * 1000.0) as u64,
            cfg,
            ring_us: vec![0; window].into_boxed_slice(),
            ring_smooth: vec![false; window].into_boxed_slice(),
            head: 0,
            len: 0,
            ring_smooth_count: 0,
            counts: IntervalCounts::default(),
            sum_us: 0,
            sumsq_us: 0,
            tempo_us: 0.0,
            warm: 0,
            last: None,
            last_boundary: false,
            skip_next: false,
            keys: Box::new([KeyTally::default(); KEY_SLOTS]),
            pairs: Box::new([PairTally::default(); PAIR_SLOTS]),
            prev_letter: None,
            corridor: [CorridorBucket::default(); CORRIDOR_BUCKETS],
            corridor_len: 0,
            corridor_span: 1,
            corridor_fill: 0,
        }
    }

    /// Forget the session; keep the buffers (no allocation).
    pub fn reset(&mut self) {
        self.ring_us.fill(0);
        self.ring_smooth.fill(false);
        self.head = 0;
        self.len = 0;
        self.ring_smooth_count = 0;
        self.counts = IntervalCounts::default();
        self.sum_us = 0;
        self.sumsq_us = 0;
        self.tempo_us = 0.0;
        self.warm = 0;
        self.last = None;
        self.last_boundary = false;
        self.skip_next = false;
        self.keys.fill(KeyTally::default());
        self.pairs.fill(PairTally::default());
        self.prev_letter = None;
        self.corridor = [CorridorBucket::default(); CORRIDOR_BUCKETS];
        self.corridor_len = 0;
        self.corridor_span = 1;
        self.corridor_fill = 0;
    }

    /// The configuration in use.
    #[must_use]
    pub fn config(&self) -> &FlowConfig {
        &self.cfg
    }

    /// A character key at `now`: `expected` is the target character at the
    /// cursor, `correct` whether the key matched it. **Hot path: no
    /// allocation.**
    pub fn key_at(&mut self, now: Instant, expected: char, correct: bool) {
        self.counts.keystrokes = self.counts.keystrokes.saturating_add(1);
        let slot = key_slot(expected);
        let boundary = expected == ' ' || self.last_boundary;
        if let Some(t) = self.keys.get_mut(slot) {
            t.attempts = t.attempts.saturating_add(1);
            t.errors = t.errors.saturating_add(u32::from(!correct));
        }
        if let Some(prev) = self.last {
            match now.checked_duration_since(prev) {
                Some(d) => self.interval(duration_us(d), boundary, slot),
                None => {
                    self.counts.clock_anomalies = self.counts.clock_anomalies.saturating_add(1);
                    self.warm = 0;
                }
            }
        }
        self.last = Some(now);
        self.last_boundary = expected == ' ';
        // Letter pairs: the pair fails when either key was missed.
        let letter = letter_index(expected);
        if let (Some((a, a_ok)), Some(b)) = (self.prev_letter, letter) {
            let i = usize::from(a) * 26 + usize::from(b);
            if let Some(p) = self.pairs.get_mut(i) {
                p.attempts = p.attempts.saturating_add(1);
                p.errors = p.errors.saturating_add(u32::from(!(a_ok && correct)));
            }
        }
        self.prev_letter = letter.map(|l| (l, correct));
    }

    /// A Backspace at `now`. The intervals around it are not scored.
    /// **Hot path: no allocation.**
    pub fn correction_at(&mut self, now: Instant) {
        self.counts.keystrokes = self.counts.keystrokes.saturating_add(1);
        self.counts.corrections = self.counts.corrections.saturating_add(1);
        if let Some(prev) = self.last {
            if now.checked_duration_since(prev).is_none() {
                self.counts.clock_anomalies = self.counts.clock_anomalies.saturating_add(1);
            }
        }
        self.last = Some(now);
        self.skip_next = true;
        self.prev_letter = None;
    }

    /// The session was paused (or the window lost focus): the next key
    /// starts a new interval chain without scoring the gap.
    pub fn break_rhythm(&mut self) {
        self.last = None;
        self.prev_letter = None;
    }

    fn interval(&mut self, us: u64, boundary: bool, slot: usize) {
        if us >= self.pause_us {
            self.counts.pauses = self.counts.pauses.saturating_add(1);
            self.warm = 0;
            self.skip_next = false;
            return;
        }
        if us < self.min_us {
            self.counts.rollovers = self.counts.rollovers.saturating_add(1);
            return;
        }
        if self.skip_next {
            self.skip_next = false;
            return;
        }
        if let Some(t) = self.keys.get_mut(slot) {
            t.latency_us = t.latency_us.saturating_add(us);
            t.latency_n = t.latency_n.saturating_add(1);
        }
        let x = us as f64;
        if self.warm < self.cfg.warmup || self.tempo_us <= 0.0 {
            self.tempo_us = if self.warm == 0 || self.tempo_us <= 0.0 {
                x
            } else {
                self.tempo_us + self.cfg.tempo_alpha.max(0.25) * (x - self.tempo_us)
            };
            self.warm = self.warm.saturating_add(1);
            self.counts.warmup = self.counts.warmup.saturating_add(1);
            return;
        }
        // tempo_us > 0 here: it is built from intervals >= min_us >= 1 ms.
        let band = if boundary {
            self.cfg.boundary_band
        } else {
            self.cfg.smooth_band
        };
        let deviation = (x - self.tempo_us).abs() / self.tempo_us;
        let smooth = deviation <= band;
        if x > self.tempo_us * self.cfg.hesitation_factor {
            self.counts.hesitations = self.counts.hesitations.saturating_add(1);
        }
        self.tempo_us += self.cfg.tempo_alpha * (x - self.tempo_us);

        self.counts.scored = self.counts.scored.saturating_add(1);
        self.counts.smooth = self.counts.smooth.saturating_add(u64::from(smooth));
        self.sum_us = self.sum_us.saturating_add(us);
        self.sumsq_us = self
            .sumsq_us
            .saturating_add(u128::from(us) * u128::from(us));

        // Live ring.
        let cap = self.ring_us.len();
        if self.len == cap {
            if self.ring_smooth.get(self.head).copied().unwrap_or(false) {
                self.ring_smooth_count = self.ring_smooth_count.saturating_sub(1);
            }
        } else {
            self.len += 1;
        }
        if let (Some(v), Some(s)) = (
            self.ring_us.get_mut(self.head),
            self.ring_smooth.get_mut(self.head),
        ) {
            *v = u32::try_from(us).unwrap_or(u32::MAX);
            *s = smooth;
        }
        self.ring_smooth_count += usize::from(smooth);
        self.head = (self.head + 1) % cap.max(1);

        self.corridor_push(us, smooth);
    }

    fn corridor_push(&mut self, us: u64, smooth: bool) {
        if self.corridor_fill >= self.corridor_span {
            // The current bucket is full: open the next one, merging pairs
            // and doubling the span when every bucket is used.
            if self.corridor_len == CORRIDOR_BUCKETS {
                for i in 0..CORRIDOR_BUCKETS / 2 {
                    let (a, b) = (self.corridor[2 * i], self.corridor[2 * i + 1]);
                    self.corridor[i] = CorridorBucket {
                        scored: a.scored.saturating_add(b.scored),
                        smooth: a.smooth.saturating_add(b.smooth),
                        sum_us: a.sum_us.saturating_add(b.sum_us),
                    };
                }
                for b in &mut self.corridor[CORRIDOR_BUCKETS / 2..] {
                    *b = CorridorBucket::default();
                }
                self.corridor_len = CORRIDOR_BUCKETS / 2;
                self.corridor_span = self.corridor_span.saturating_mul(2);
                // The last merged bucket holds a full span.
            }
            self.corridor_fill = 0;
            self.corridor_len += 1;
        }
        if self.corridor_len == 0 {
            self.corridor_len = 1;
        }
        if let Some(b) = self.corridor.get_mut(self.corridor_len - 1) {
            b.scored = b.scored.saturating_add(1);
            b.smooth = b.smooth.saturating_add(u32::from(smooth));
            b.sum_us = b.sum_us.saturating_add(us);
        }
        self.corridor_fill += 1;
    }

    // --- Live reads (per render; no allocation) ------------------------------

    /// Interval classes so far.
    #[must_use]
    pub fn counts(&self) -> IntervalCounts {
        self.counts
    }

    /// Smoothness % of the live window, `None` before anything is scored.
    #[must_use]
    pub fn live_smoothness(&self) -> Option<f64> {
        (self.len > 0).then(|| self.ring_smooth_count as f64 * 100.0 / self.len as f64)
    }

    /// The live rhythm word.
    #[must_use]
    pub fn label(&self) -> RhythmLabel {
        match self.live_smoothness() {
            None => RhythmLabel::WarmingUp,
            Some(s) if s >= self.cfg.steady_smoothness => RhythmLabel::Steady,
            Some(s) if s >= self.cfg.settling_smoothness => RhythmLabel::Settling,
            Some(_) => RhythmLabel::Uneven,
        }
    }

    /// The current tempo (ms per key), `None` before the first interval.
    #[must_use]
    pub fn tempo_ms(&self) -> Option<f64> {
        (self.tempo_us > 0.0).then_some(self.tempo_us / 1000.0)
    }

    /// Fill `out` with the newest intervals, oldest first, right-aligned;
    /// slots without data get level 0. Writes at most [`MAX_BARS`].
    pub fn fill_bars(&self, out: &mut [CadenceBar]) {
        let n = out.len().min(MAX_BARS);
        let cap = self.ring_us.len().max(1);
        let shown = self.len.min(n);
        let tempo = if self.tempo_us > 0.0 {
            self.tempo_us
        } else {
            1.0
        };
        for (i, bar) in out.iter_mut().take(n).enumerate() {
            let age = n - 1 - i; // 0 = newest
            *bar = if age < shown {
                let idx = (self.head + cap - 1 - age) % cap;
                let us = f64::from(self.ring_us.get(idx).copied().unwrap_or(0));
                CadenceBar {
                    level: (us / (2.0 * tempo)).clamp(0.06, 1.0) as f32,
                    smooth: self.ring_smooth.get(idx).copied().unwrap_or(false),
                }
            } else {
                CadenceBar::default()
            };
        }
    }

    // --- Session reads (report time) -----------------------------------------

    /// Session smoothness %: smooth scored intervals over scored ones.
    #[must_use]
    pub fn smoothness(&self) -> Option<f64> {
        (self.counts.scored > 0)
            .then(|| self.counts.smooth as f64 * 100.0 / self.counts.scored as f64)
    }

    /// Mean scored interval (ms).
    #[must_use]
    pub fn mean_ms(&self) -> Option<f64> {
        (self.counts.scored > 0).then(|| self.sum_us as f64 / self.counts.scored as f64 / 1000.0)
    }

    /// Population standard deviation of scored intervals (ms), exact.
    #[must_use]
    pub fn stdev_ms(&self) -> Option<f64> {
        let n = u128::from(self.counts.scored);
        if n == 0 {
            return None;
        }
        let sum = u128::from(self.sum_us);
        // n·Σx² − (Σx)² is ≥ 0 exactly (Cauchy–Schwarz); saturating keeps
        // a pathological overflow at 0 rather than wrapping.
        let num = n
            .saturating_mul(self.sumsq_us)
            .saturating_sub(sum.saturating_mul(sum));
        let var = num as f64 / (n as f64 * n as f64);
        Some(var.max(0.0).sqrt() / 1000.0)
    }

    /// Cadence variance: coefficient of variation of scored intervals.
    #[must_use]
    pub fn cadence_cv(&self) -> Option<f64> {
        let mean = self.mean_ms()?;
        let sd = self.stdev_ms()?;
        (mean > 0.0).then(|| sd / mean)
    }

    /// The key table (index with [`key_slot`]).
    #[must_use]
    pub fn keys(&self) -> &[KeyTally; KEY_SLOTS] {
        &self.keys
    }

    /// The letter-pair table (`a * 26 + b`).
    #[must_use]
    pub fn pairs(&self) -> &[PairTally; PAIR_SLOTS] {
        &self.pairs
    }

    /// The used corridor buckets, oldest first.
    #[must_use]
    pub fn corridor(&self) -> &[CorridorBucket] {
        self.corridor.get(..self.corridor_len).unwrap_or(&[])
    }

    /// Capacity of the live window (fixed for the tracker's life).
    #[must_use]
    pub fn window_capacity(&self) -> usize {
        self.ring_us.len()
    }

    /// Intervals currently in the live window.
    #[must_use]
    pub fn window_len(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(t: &mut FlowTracker, t0: Instant, gaps_ms: &[u64], text: &str) -> Instant {
        let mut now = t0;
        let mut chars = text.chars().cycle();
        t.key_at(now, chars.next().unwrap_or('a'), true);
        for &g in gaps_ms {
            now += Duration::from_millis(g);
            t.key_at(now, chars.next().unwrap_or('a'), true);
        }
        now
    }

    #[test]
    fn an_even_stream_is_fully_smooth_with_zero_variance() {
        let mut t = FlowTracker::new(FlowConfig::default());
        feed(&mut t, Instant::now(), &[200; 100], "abcdefgh");
        let c = t.counts();
        assert_eq!(c.warmup, 4);
        assert_eq!(c.scored, 96);
        assert_eq!(t.smoothness(), Some(100.0));
        assert_eq!(t.cadence_cv(), Some(0.0));
        assert_eq!(t.mean_ms(), Some(200.0));
        assert_eq!(t.label(), RhythmLabel::Steady);
        assert_eq!(t.tempo_ms(), Some(200.0));
    }

    #[test]
    fn empty_and_single_key_sessions_report_nothing() {
        let mut t = FlowTracker::new(FlowConfig::default());
        assert_eq!(t.smoothness(), None);
        assert_eq!(t.cadence_cv(), None);
        assert_eq!(t.label(), RhythmLabel::WarmingUp);
        t.key_at(Instant::now(), 'a', true);
        assert_eq!(t.smoothness(), None);
        assert_eq!(t.stdev_ms(), None);
        let mut bars = [CadenceBar::default(); 8];
        t.fill_bars(&mut bars);
        assert!(bars.iter().all(|b| b.level == 0.0));
    }

    #[test]
    fn zero_intervals_backwards_clocks_and_pauses_are_never_scored() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let t0 = Instant::now() + Duration::from_secs(10);
        let now = feed(&mut t, t0, &[200; 10], "abcd");
        let scored = t.counts().scored;
        t.key_at(now, 'a', true); // zero interval
        assert_eq!(t.counts().rollovers, 1);
        let back = now - Duration::from_secs(5);
        t.key_at(back, 'b', true); // the clock went backwards
        assert_eq!(t.counts().clock_anomalies, 1);
        t.key_at(back + Duration::from_secs(3600), 'c', true); // a sleep/resume jump
        assert_eq!(t.counts().pauses, 1);
        assert_eq!(t.counts().scored, scored, "none of those were scored");
        assert!(t.smoothness().is_some_and(f64::is_finite));
    }

    #[test]
    fn a_hesitation_is_counted_and_breaks_smoothness() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let mut gaps = vec![200; 20];
        gaps.push(900);
        gaps.extend([200; 20]);
        feed(&mut t, Instant::now(), &gaps, "abcd");
        assert_eq!(t.counts().hesitations, 1);
        let s = t.smoothness().unwrap();
        assert!(s < 100.0 && s > 90.0, "{s}");
    }

    #[test]
    fn word_boundaries_get_the_wider_band() {
        // Every 5th interval is 1.6x the tempo: outside the 45 % band, inside
        // the 75 % boundary band when the key is a space.
        let gaps: Vec<u64> = (0..60)
            .map(|i| if i % 5 == 4 { 320 } else { 200 })
            .collect();
        let mut words = FlowTracker::new(FlowConfig::default());
        feed(&mut words, Instant::now(), &gaps, "abcd ");
        let mut letters = FlowTracker::new(FlowConfig::default());
        feed(&mut letters, Instant::now(), &gaps, "abcde");
        assert!(words.smoothness().unwrap() > letters.smoothness().unwrap());
    }

    #[test]
    fn corrections_are_counted_and_their_intervals_skipped() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let t0 = Instant::now();
        let now = feed(&mut t, t0, &[200; 10], "abcd");
        let before = t.counts().scored;
        t.correction_at(now + Duration::from_millis(900));
        t.key_at(now + Duration::from_millis(1500), 'a', true);
        assert_eq!(t.counts().corrections, 1);
        assert_eq!(t.counts().scored, before);
        t.key_at(now + Duration::from_millis(1700), 'b', true);
        assert_eq!(t.counts().scored, before + 1);
    }

    #[test]
    fn break_rhythm_skips_the_gap() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let now = feed(&mut t, Instant::now(), &[200; 10], "abcd");
        let before = t.counts();
        t.break_rhythm();
        t.key_at(now + Duration::from_millis(1500), 'a', true);
        assert_eq!(t.counts().scored, before.scored);
        assert_eq!(t.counts().pauses, before.pauses);
    }

    #[test]
    fn keys_pairs_and_latency_are_tallied() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let t0 = Instant::now();
        t.key_at(t0, 't', true);
        t.key_at(t0 + Duration::from_millis(200), 'h', false);
        t.key_at(t0 + Duration::from_millis(400), 'e', true);
        t.key_at(t0 + Duration::from_millis(600), 'é', true);
        let h = t.keys()[key_slot('h')];
        assert_eq!((h.attempts, h.errors, h.latency_n), (1, 1, 1));
        assert_eq!(h.latency_us, 200_000);
        assert_eq!(
            t.keys()[key_slot('é')].attempts,
            1,
            "non-ASCII shares the other slot"
        );
        assert_eq!(slot_char(key_slot('h')), Some('h'));
        assert_eq!(slot_char(key_slot('é')), None);
        let th = t.pairs()[19 * 26 + 7];
        assert_eq!((th.attempts, th.errors), (1, 1));
        let he = t.pairs()[7 * 26 + 4];
        assert_eq!((he.attempts, he.errors), (1, 1), "h was missed");
    }

    #[test]
    fn the_window_and_corridor_stay_bounded() {
        let mut t = FlowTracker::new(FlowConfig::default());
        let gaps: Vec<u64> = (0..20_000).map(|i| 180 + (i % 7) * 10).collect();
        feed(&mut t, Instant::now(), &gaps, "the quick brown fox ");
        assert_eq!(t.window_capacity(), 64);
        assert_eq!(t.window_len(), 64);
        assert!(t.corridor().len() <= CORRIDOR_BUCKETS);
        let total: u64 = t.corridor().iter().map(|b| u64::from(b.scored)).sum();
        assert_eq!(total, t.counts().scored, "the corridor loses no interval");
        let mut bars = [CadenceBar::default(); 48];
        t.fill_bars(&mut bars);
        assert!(bars.iter().all(|b| (0.06..=1.0).contains(&b.level)));
    }

    #[test]
    fn reset_keeps_capacity_and_clears_everything() {
        let mut t = FlowTracker::new(FlowConfig::default());
        feed(&mut t, Instant::now(), &[200; 300], "abc");
        t.reset();
        assert_eq!(t.counts(), IntervalCounts::default());
        assert_eq!(t.window_capacity(), 64);
        assert_eq!(t.smoothness(), None);
        assert!(t.corridor().is_empty());
        assert!(t.keys().iter().all(|k| *k == KeyTally::default()));
    }

    #[test]
    fn an_invalid_config_falls_back_to_defaults() {
        let bad = FlowConfig {
            window: 0,
            ..FlowConfig::default()
        };
        let t = FlowTracker::new(bad);
        assert_eq!(t.config(), &FlowConfig::default());
    }
}
