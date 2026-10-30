//! Flow engine soak test (PRD FL-05): eight simulated hours of typing on the
//! injectable clock, in well under a second of real time.
//!
//! Proves three things mechanically:
//!
//! 1. **Zero allocation on the hot path.** A counting global allocator (this
//!    test binary only) counts heap allocations made by *this thread*; the
//!    count across every `key_at` / `correction_at` / `break_rhythm` /
//!    `fill_bars` call of the soak must be exactly 0.
//! 2. **Bounded memory.** The live window, the key and pair tables and the
//!    corridor never grow: capacity is identical after 8 hours.
//! 3. **Stable numbers.** Smoothness, cadence variance and the tempo stay
//!    finite and in range throughout; the corridor loses no interval.
//!
//! The stream: ~5 keys/s with jitter, a word boundary every 6 keys, a
//! hesitation every few hundred keys, a correction every ~150 keys, a pause
//! every ~10 minutes, a sleep/resume clock jump every 2 hours, and a few
//! zero intervals and backwards instants.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use radha_core::flow::{CadenceBar, FlowConfig, FlowReport, FlowTracker};

struct Counting;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: forwards to the system allocator unchanged; only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocs() -> u64 {
    ALLOCS.with(Cell::get)
}

/// SplitMix64 (Steele, Lea, Flood 2014).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog while steady hands keep time ";

#[test]
fn eight_hours_of_typing_allocate_nothing_and_stay_bounded() {
    let mut t = FlowTracker::new(FlowConfig::default());
    let capacity = t.window_capacity();
    let mut bars = [CadenceBar::default(); 48];
    let mut rng = Rng(0x0F10_5EED);
    let t0 = Instant::now();
    let eight_hours = Duration::from_secs(8 * 3600);
    let mut now = t0;
    let mut keys: u64 = 0;
    let wall = Instant::now();

    let before = allocs();
    while now.saturating_duration_since(t0) < eight_hours {
        keys += 1;
        let c = char::from(TEXT[(keys % TEXT.len() as u64) as usize]);
        // ~200 ms ± 60 ms; boundaries a little slower.
        let mut gap = 140 + rng.below(120) + if c == ' ' { 60 } else { 0 };
        if keys % 347 == 0 {
            gap = 700; // hesitation
        }
        if keys % 3_000 == 0 {
            gap = 45_000; // a short break (pause)
        }
        if keys % 36_000 == 0 {
            gap = 3 * 3600 * 1000 / 4; // sleep/resume jump
        }
        if keys % 997 == 0 {
            gap = 0; // rollover / zero interval
        }
        now += Duration::from_millis(gap);
        if keys % 5_003 == 0 {
            // An injected instant that goes backwards.
            let back = now.checked_sub(Duration::from_millis(500)).unwrap_or(now);
            t.key_at(back, c, true);
        } else if keys % 151 == 0 {
            t.correction_at(now);
        } else if keys % 7_919 == 0 {
            t.break_rhythm();
        } else {
            t.key_at(now, c, rng.below(50) != 0);
        }
        if keys % 64 == 0 {
            t.fill_bars(&mut bars);
            let s = t.live_smoothness().unwrap_or(100.0);
            assert!((0.0..=100.0).contains(&s));
            assert!(t.tempo_ms().is_none_or(f64::is_finite));
        }
    }
    let hot_path_allocs = allocs() - before;
    let elapsed = wall.elapsed();

    let c = t.counts();
    eprintln!(
        "flow soak: {keys} keys over {:.1} simulated hours in {elapsed:?} real; \
         scored {}, pauses {}, rollovers {}, corrections {}, clock anomalies {}, hesitations {}; \
         heap allocations on the hot path: {hot_path_allocs}",
        now.saturating_duration_since(t0).as_secs_f64() / 3600.0,
        c.scored,
        c.pauses,
        c.rollovers,
        c.corrections,
        c.clock_anomalies,
        c.hesitations
    );
    assert_eq!(hot_path_allocs, 0, "the per-key path must not allocate");
    assert!(keys > 80_000, "{keys}");
    assert_eq!(t.window_capacity(), capacity, "the window never grows");
    assert!(t.window_len() <= capacity);
    assert!(t.corridor().len() <= radha_core::flow::tracker::CORRIDOR_BUCKETS);
    let total: u64 = t.corridor().iter().map(|b| u64::from(b.scored)).sum();
    assert_eq!(total, c.scored, "no interval lost to downsampling");
    assert!(c.pauses >= 25 && c.clock_anomalies > 0 && c.rollovers > 0);
    let s = t.smoothness().unwrap();
    let cv = t.cadence_cv().unwrap();
    assert!((50.0..=100.0).contains(&s), "{s}");
    assert!(cv.is_finite() && cv > 0.0 && cv < 1.0, "{cv}");
    assert!(
        elapsed < Duration::from_secs(20),
        "soak must stay fast: {elapsed:?}"
    );

    // The report (once per session, may allocate) is sane after 8 hours.
    let r = FlowReport::from_tracker(&t);
    assert!(r.corridor.len() <= radha_core::flow::tracker::CORRIDOR_BUCKETS);
    assert!(r
        .corridor
        .iter()
        .all(|p| p.smoothness.is_finite() && p.mean_ms.is_finite()));
    assert!(r.weak_keys.len() <= FlowConfig::default().max_weak_keys);
}

#[test]
fn a_reset_tracker_reuses_its_buffers_without_allocating() {
    let mut t = FlowTracker::new(FlowConfig::default());
    let t0 = Instant::now();
    for i in 0..500u64 {
        t.key_at(t0 + Duration::from_millis(i * 200), 'a', true);
    }
    let before = allocs();
    t.reset();
    for i in 0..500u64 {
        t.key_at(t0 + Duration::from_millis(i * 210), 'b', i % 9 != 0);
    }
    assert_eq!(allocs() - before, 0);
}
