# Typing Flow gap log (P07)

Bugs, edge cases and gaps found in Golu Typing OS's Flow ("smooth") and
adaptive code, and how Radha's Flow engine handles each. Golu sources were
read only, never modified. Paths are relative to `E:\GOLU TYPING OS\`.

Radha code: `crates/radha-core/src/flow/` (engine), `crates/radha-app/src/rhythm.rs`
and `src/ui/flow.rs` (screens), `content/flow/flow-config.toml` (thresholds),
`migrations/0004_flow_rhythm.sql` (storage).

## Engine (`src/lib/typingEngine.ts`, `src/hooks/useTypingSession.ts`)

| # | Golu file:line | Problem | Radha |
|---|---|---|---|
| 1 | typingEngine.ts:298, :304; useTypingSession.ts:162 | `key.length !== 1` and `text[cursor]` index UTF-16 code units: an astral character (emoji) or a surrogate pair is rejected or split. | The tracker takes a `char`; anything outside printable ASCII shares one "other" slot (`tracker.rs` `key_slot`). The fuzz test feeds emoji, Devanagari and `\0`. |
| 2 | useTypingSession.ts:161 | `ctrlKey \|\| altKey` drops every AltGr key on Windows (AltGr arrives as Ctrl+Alt). | Radha's key path (`practice::classify_key`) already treats Alt+printable as AltGr; Flow uses the same path. |
| 3 | typingEngine.ts:322-327, :372-379 | No pause cap: a 10-minute break, or a sleep/resume clock jump, is one "latency" sample. It inflates the mean latency, the Welford variance and every key's latency, which feeds the weak-key score. | An interval of `pause_ms` (2 s) or more is a pause: counted, rhythm restarted with a fresh warm-up, never scored. Vector C below. |
| 4 | typingEngine.ts:323 | An injected `now` that goes backwards gives a negative latency, which enters the variance. A zero interval is scored too. | `checked_duration_since`: a backwards instant is a clock anomaly (counted, re-anchored). Intervals under `min_interval_ms` are rollovers (counted, not scored), so nothing is ever divided by zero. |
| 5 | typingEngine.ts:294-296 vs :329 | Backspace returns before `lastInputAt` is updated, so the key after a correction is timed from the key before it. Correction time is scored as hesitation on an innocent key. | `correction_at` re-anchors the clock and skips the next interval. |
| 6 | typingEngine.ts:415-418 | A bigram's latency is the session's running mean (`latencyMean`), not the pair's own time, so every pattern shows nearly the same latency. | Per-key latency is the real interval ending on that key; pairs are judged on errors only (honest). |
| 7 | typingEngine.ts:307-308, :496 | `typed`, `results` and `secondWpm` grow without bound (8 h = ~100 k entries and 28 800 seconds). | Fixed memory: 64-interval ring, 96 key slots, 676 pair slots, a 96-bucket corridor that merges pairwise. The 8 h soak asserts capacity is unchanged. |
| 8 | typingEngine.ts:383, :405-412, :691-692 | Per keystroke: a new accumulator object, a regex, a template string, and two full array copies (`[...this.typed]`) — O(typed) allocation per key. | `key_at` allocates nothing (counting allocator in `tests/flow_soak.rs`: 0 allocations over 103 531 keys). |
| 9 | typingEngine.ts:575-586 | Consistency is recomputed with two passes over all seconds on every publish: O(session length). | Exact integer sums (µs, `u64`/`u128`): O(1) per read, no floating drift, variance never negative. |
| 10 | typingEngine.ts:489-497 | Consistency is per-second pace, not rhythm. A pause adds zero-WPM seconds (punished), while five keys crammed into 160 ms then a wait scores as even as a metronome. The key that closes a second is counted inside it, so a perfect metronome scores 97.8, not 100. | Smoothness = share of scored intervals within a band of the typist's own tempo (EWMA). Word-boundary intervals get a wider band. Vectors A, C and F. |
| 11 | typingEngine.ts:703 | Word mode grades every word span on every keystroke: O(words). | Not used by Flow; Radha's live view is the existing matcher (`live_at`). |

## Adaptive (`src-tauri/src/adaptive/`)

| # | Golu file:line | Problem | Radha |
|---|---|---|---|
| 12 | weak_keys.rs:35-54 | `clamp` does not remove NaN: a NaN error rate or latency gives a NaN score, stored and then sorted with `partial_cmp(..).unwrap_or(Equal)` (unstable order). | `weakness_score` maps any non-finite input to 0 and always returns a finite 0–100 (fuzz and vector tests). Sorting uses `total_cmp`. |
| 13 | weak_keys.rs:49-50 | Negative `attempts` (a corrupt row) makes recurrence negative. | `attempts: u32`; errors are clamped to attempts. |
| 14 | weak_keys.rs:60-99 | No minimum sample: one press, one miss = error rate 1.0 = "weak". | `weak_min_attempts` (5) and `weak_score` (20) from config. |
| 15 | weak_keys.rs:96-99 | Trend is always 0, so `TREND_WEIGHT` is dead code. | Kept in the formula (vectors match Golu), passed as 0 until per-key history exists; documented, not hidden. |
| 16 | weak_keys.rs:61-82 | Baseline is an average of per-session averages (not weighted by count) and includes pause time (#3). | Baseline is the session's mean scored interval, pauses excluded. |
| 17 | weak_keys.rs:60, :101-116 | Re-aggregates the whole history after every session (grows forever) and writes one row per key outside a transaction. | Per-session report from fixed tables; one `flow_rhythm` row inside the single `save_session` transaction. |
| 18 | drill.rs:85-90 | `errors > attempts` gives negative accuracy. | `WeakKey::accuracy` clamps to 0–100. |
| 19 | drill.rs:196 | `is_alphanumeric` drops punctuation targets: a weak `,` yields no target, then the full word bank — the drill never contains the weak key. | Drill words come from the passage just typed and keep punctuation; test `a_punctuation_weak_key_reaches_the_drill`. |
| 20 | drill.rs:153-158 | Doc says words containing *every* target; code uses `any`. | Words are ranked by how many targets they carry (pairs count double). |
| 21 | drill.rs:200-202 | The "bigram" rung for a single key repeats the key (`kkk kkk`). | Below `raw_key_accuracy` the key is isolated (`qqq` ×4), then real words. |
| 22 | drill.rs:212 | Id `DRILL-{stage}-{units joined}` collides (`ab`+`c` vs `a`+`bc`). | Drills are not stored as rows; nothing to collide. |
| 23 | drill.rs:236-252 | `build_repetition` loops forever on empty units (guarded only by the caller). | No unbounded loop: `cycle().take(drill_words)`. |
| 24 | drill.rs:95-114 | Lowercase English word bank only: no Shift practice, nothing for other scripts. | Words from the passage as written (capitals included); ASCII only, so a drill is always typeable on QWERTY. |
| 25 | patterns.rs:12-14, :63-77 | Hand and row maps hard-code QWERTY. | Flow is QWERTY by product definition; layout-aware hand data stays in `radha-layouts` for other products. |
| 26 | recommend.rs:157-160 | Recent accuracy averaged without a NaN guard. | `recommend` treats non-finite accuracy as 0 (ease off). |
| 27 | recommend.rs:249 | History ordered by wall-clock `started_at`: a clock set back reorders it. | `last_flow_smoothness` orders by session id. |

## UI and content

| # | Golu file:line | Problem | Radha |
|---|---|---|---|
| 28 | caretController.ts:166-177 | The rAF loop runs every frame for the whole session even when nothing moves, and allocates an array per frame (`:173`). | No per-frame loop: the cursor glow is a Slint `animate` on a property Rust flips once per scored key. |
| 29 | personalBest.ts:25-27 | Key `${id}\|${seconds}s`: fractional custom durations print as floats and fork bests; an id containing `\|` collides. | Bests are rows keyed by exercise id (existing `personal_bests`). |
| 30 | tools/generate-smooth.mjs:63-77 | Every third part is the same practice line, so each 2 000-word lesson repeats one sentence ~30 times; `countWords(parts.join(" "))` per loop is O(n²). | Replaced by the 100 authored `FT-NNN.json` passages (P07 content import). |

## Content import (FT-001 … FT-100)

- 100 files read, 100 imported, 0 excluded. Each file is parsed and validated on
  its own; a bad file would be excluded with its reason in the report.
- Checked per file: JSON shape, `exerciseId` matches the file name, unique id,
  `language = en`, `mode = flow_typing`, non-empty passage, NFC, every character
  typeable on QWERTY, sha256 sealed, provenance `radha-flow:FT-NNN.json` plus the
  file's sha256.
- Typography is folded to what a US keyboard types, and reported: `’` ×118 → `'`,
  `—` ×52 → ` - `, `é` ×1 → `e`, `ç` ×1 → `c` (42 files).
- `passageWordCount` disagrees with the text in 30 files (the text wins; mostly the
  spaced em dash adding a word).
- P04 review finding 5 is fixed: a re-import deletes rows numbered past the new
  count (and their provenance) in the same transaction, and drops a stale Kruti Dev
  text a passage no longer has.

## Shared vectors (`crates/radha-core/tests/flow_golu_vectors.rs`)

Golu's formulas are ported line for line and run on the same keystroke streams
(400 intervals, all keys correct).

| Vector | Golu consistency | Golu latency CV | Radha smoothness | Radha cadence CV |
|---|---|---|---|---|
| A metronome 200 ms | 97.8 | 0.000 | 100.0 | 0.000 |
| B word boundaries 1.6× | 88.3 | 0.216 | 100.0 | 0.216 |
| C one 3 s pause | 81.6 | 0.675 | 100.0 | 0.000 (1 pause) |
| D jitter ±40 % | 84.8 | 0.229 | 98.5 | 0.228 |
| E tempo 300 → 150 ms | 78.2 | 0.192 | 100.0 | 0.191 |
| F 4 × 40 ms + 840 ms | 97.8 | 1.600 | 0.3 | 1.593 |

Matches: the weakness score equals Golu's on all 8 finite vectors (to 1e-9);
cadence CV equals Golu's latency CV within the warm-up intervals (D, E); drill
stage thresholds (80 % / key isolation) agree.

Intentional differences: A (Golu's second-boundary off-by-one), B (word-boundary
pauses are rhythm, not unevenness), C (a pause restarts the rhythm instead of
punishing it), E (the tempo follows a typist who speeds up), F (stop-start bursts
are uneven, whatever the per-second pace says), NaN and negative inputs score 0
(Golu: NaN).
