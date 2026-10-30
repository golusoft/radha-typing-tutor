# Golu Expert AI → radha-core gap log (P02)

What radha-core lacked compared with Golu Expert AI, what Golu gets wrong, and every place Radha deliberately differs. Golden vectors live in `crates/radha-core/tests/fixtures/*.json` and run in `crates/radha-core/tests/golden_vectors.rs`.

Rules followed: Golu is read-only (never edited or built). PRD wins over Golu: a PRD formula is never weakened to match Golu. Each divergence below is an `intentional_divergence` fixture case with a written reason.

Expert AI root: `E:\golu typing software\GOLU TYPING EXPERT AI\`. The planner hashed every file below on both `E:` and the `D:\GOLU TYPING EXPERT AI` copy and found them identical; the coder re-measured the E: hashes at extraction (2026-10-25) and they matched.

| File (relative to the root) | sha256 |
|---|---|
| `src\Core\Golu.EngineCore\Typing\TypingSession.cs` | `e6a6dd3a67d59421283a805be9ba0b3ba91be278efbecabcbb2446989298257c` |
| `src\Core\Golu.EngineCore\Scoring\ScoringEngine.cs` | `9c1b06d0672066b1966b831c80fd2202486d078d2a015c7c4b13dad47123c761` |
| `src\Core\Golu.EngineCore\Results\MistakeClassifier.cs` | `c13214eb87c95257e06eed098b29f5f8faaf202246559ce13be2cef34193d788` |
| `src\Core\Golu.EngineCore\Results\ResultAnalyzer.cs` | `890517be7117ed540e409ca3c18ed2712d584521705e162bc67b29f1baabd563` |
| `src\Core\Golu.EngineCore\Session\SessionEngine.cs` | `194148346d3b75c9015a1576960180b9a79afd31df88858614c321a9bb9d7338` |
| `src\Core\Golu.EngineCore\Session\SessionPolicy.cs` | `39bb97ba7015f377cacf53e7830db245922b55a96d925a6cae017606b046f250` |
| `src\Core\Golu.EngineCore\Statistics\StatisticsEngine.cs` | `17b26ecbf8d386f34fed614e5107592b78e5ed7b3d15cd55bda3f9834db99354` |
| `src\Application\Golu.Application\Text\IndicGraphemeSegmenter.cs` | `4839e2d1489c43fa32e53d20c2a0dc6f57cadaff091adb052e995255c22d1a31` |

Test files the vectors come from (hash recorded in each fixture): `ScoringEngineTests.cs`, `WordAlignedTypingTests.cs`, `WordSkipSessionTests.cs`, `ClockArmingTests.cs`, `TypingFlowFixesTests.cs`, `SessionEngineTests.cs`, `TypingSessionBufferTests.cs`, `KeyStatProjectionTests.cs`, `IsolatedMatraSessionTests.cs`, `TargetTextNormalizationTests.cs` (all `tests\Golu.Tests\Engine\`), `ResultAnalyzerTests.cs`, `ResultGradeAndClassifierTests.cs` (`tests\Golu.Tests\Results\`), `IsolatedMatraSegmentationTests.cs` (`tests\Golu.Tests\Text\`). The PRD worked examples are in `prd_formulas.json` (source `PRD.md`, sha256 `396f055a…270f7c`).

Counts at P02: 134 vectors in 14 files, 112 `must_match`, 22 `intentional_divergence`.

## 1. What radha-core lacked, and P02 status

| Feature | Golu location | Radha before P02 | P02 status | Owner if deferred |
|---|---|---|---|---|
| Countdown with time limit and expiry | `SessionPolicy.TimeLimitSeconds` (in `SessionEngine.cs` it only appears in log lines; where expiry is enforced was not traced) | none | done: `timer.rs` + `TypingSession::tick_at`, presets per RULES 30 | — |
| KDPH | `ScoringEngine` (`TotalKeystrokes / hours`) | none | done: `scoring::kdph` | — |
| Full/half mistake classification | `MistakeClassifier` | none | done, classification only (`mistake.rs`); not applied to net WPM | weighting: P09 |
| Per-key and per-finger stats, latency (≤ 2000 ms) | `StatisticsEngine` | none | done, RAM only (`stats.rs`); finger via layout closure | — |
| Weak-key detection threshold | `WeakKeyDetector` | none | not ported (teaching heuristic) | P08 |
| NFC target normalisation | `TypingSession.Normalize` | none | done in akshar mode (target and typed) | — |
| Grapheme clusters | `IndicGraphemeSegmenter` (display only) | none | done, and used for matching, not only display (`akshar.rs`, `Matcher<Akshar>`) | — |
| KEY / WORD display states | `TypingViewModel` cells | outcomes only | done: `mode.rs` (`unit_states`) | UI wiring: P06 |
| Current-word index | `CurrentWordText` | none | done: `Evaluation::current_word` | — |
| Backspace modes (Full / OneWord / Disabled / WordLocked …), Ctrl+Backspace `DeleteWord` | `SessionPolicy.BackspaceMode`, `SessionEngine.DeleteWord` | plain backspace | not ported | P06 (TY-05) / P09 |
| Strict error handling (`ErrorHandling.Stop`) | `SessionPolicy.ErrorHandling` | none | not ported | P06 / P09 |
| Composition replacement (`ReplacePreviousCount`) | `TypingSession.Feed` | none | not ported (layouts emit plain characters for now) | P03 |
| Checkpoint / `RestoreProgress` | `SessionEngine.RestoreProgress` | none | not ported | P06 (TY-14) |
| Exam scoring strategies, half-mistake weight, proctored clock | `ExamScoringStrategies.cs` (about ten exam-specific strategy classes), `SessionPolicy.HalfMistakeWeight`, `ClockStartsOnFirstKeystroke=false` | none | not in the engine by rule (no exam names / formulas in engine crates) | P09 (radha-exams) |
| `CountPausedTime` option | `SessionPolicy.CountPausedTime` | paused time always excluded | not ported: RULES 30 always excludes paused time | — |
| Grade bands and remarks | `ResultGrade` | none | not ported (invented grading; PRD 17.10 #9, RULES 32) | — |
| Consistency | none in Golu | none | done: `scoring::consistency` (Radha's own formula, below) | — |
| Word accuracy % | none in Golu | `word_accuracy` | kept | — |
| Incremental (changed-word) evaluation | — | full re-evaluation | unchanged (O(n) per key) | P06/P07 (RULES 34) |

## 2. What Golu gets wrong, and what Radha does instead

- **Skipped letters inflate speed.** `TypingSession.Skip` writes `SkipMarker` into each skipped slot with `TotalKeystrokes++` (TypingSession.cs ~L258-270), so a space mid-word adds keystrokes that were never pressed to gross WPM and KDPH. Radha counts skipped letters as Missing errors but not as typed characters (`scoring::characters_typed`).
- **A double space skips the next word.** In word-aligned mode every whitespace runs `SkipToNextWord` (SessionEngine.cs ~L211-221). Radha treats a stray space at the start of a word as one Extra error and the next word stays typeable.
- **A space inside the final word.** Golu's skip completes the passage. Radha records Missing + Extra and keeps the session open so the typist can correct; time-up or submit end it.
- **Positional result analysis.** `ResultAnalyzer` compares positionally and documents the shift as a LIMITATION (ResultAnalyzer.cs class doc): one dropped letter marks later words wrong. Radha's matcher is word-aligned, so the error stays in its word.
- **Hindi scored per UTF-16 unit.** `TypingSession.Feed` iterates `ProducedText` per `char`; the grapheme segmenter is display-only. Radha matches per akshar (RULES 35); see `docs/hindi-grapheme-decision.md`.
- **Accuracy 0 when nothing is typed.** `ScoringEngine` returns 0 for zero keystrokes; PRD 8.2 says 100.
- **Golu's half/full split vs the court vocabulary it cites.** Golu's character rules make a wrong letter a full mistake, while the court-style vocabulary treats a spelling slip in a word as half. Radha exposes both schemes (`MistakeKind` for Golu parity, `WordMistake` for word level) and applies neither to net WPM.
- **Punctuation test.** Golu uses .NET `char.IsPunctuation` (all of Unicode P*). Radha uses a documented P* subset (`unit::is_punctuation_char`); it agrees on every Golu vector and differs only on rare P* code points outside the table.
- **Clock policies.** With the default policy Golu's clock starts at `Start()` and charges reading time; Radha's always starts on the first keystroke (RULES 30).
- **Superseded PRD text.** PRD v1 7.4 timer table (6 presets, custom "1-60 minutes") and the "11 options incl. 45 min" row are superseded by RULES 30 / PRD 17.1: 10 presets (30 s – 30 min) + custom 1 s – 60 min, no 45-minute preset.
- **PRD 9.3 example arithmetic.** "100, 300, 150 ms → ~65%" does not follow from the formula printed above it; the exact value is 53.645%. Radha implements the formula. Radha's definition: population σ over intervals in f64 ms, CV = σ/μ, consistency = clamp(100 × (1 − min(CV, 1)), 0, 100); no intervals or μ = 0 → 100. Intervals are the gaps between accepted presses (characters and effective backspaces) on the pause-excluded clock.
- **Multi-space runs in targets.** Radha requires every space of a run to be typed (`"asdf   jkl;"` typed identically is all correct). The content importer should normalise runs (P04).
- **Omission is unreachable in the word-aligned matcher.** A whole word can only be skipped with a skip gesture, which Radha does not have; `classify_word` supports Omission for when an exam profile adds one (P09).
- **"Wrong word substitution" and "word addition".** Both need a similarity threshold that no verified source defines; not detected in P02 (P09).

## 3. Intentional divergences (one row per fixture case)

| Fixture | Case | Golu value | Radha value | Reason |
|---|---|---|---|---|
| golu_scoring_engine | Full_And_Half_Mistakes_Are_Weighted | net 56 (2 full + 4 half × 0.5) | net 54 (6 errors, unweighted) | PRD 8.2 default; weighting is exam policy (P09) |
| golu_scoring_engine | Zero_Keystrokes_Gives_Zero_Accuracy | accuracy 0 | accuracy 100 | PRD 8.2 `total == 0 → 100` |
| golu_word_aligned | Without_The_Flag_Typing_Is_Still_Purely_Positional | caret 3 | caret 2, 1 extra | word-aligned is Radha's only matcher |
| golu_word_skip | Skip_Advances_The_Caret_And_Charges_Full_Mistakes | 3 full, TotalKeystrokes 3 | 3 missing, characters_typed 4 | skipped letters are not typed characters |
| golu_word_skip | A_Skipped_Word_Is_Reported_By_What_Was_Actually_Typed#word_level | "Missing Character" | word mistake Spelling | word-level vocabulary (EXAM-MODE plan 5.4) |
| golu_word_skip | A_Skip_Is_Unwound_By_Backspace_Like_Any_Other_Mistake | 3 backspaces | 1 backspace | one backspace removes the one key pressed |
| golu_word_skip | Skipping_The_Final_Word_Completes_The_Passage | complete | Active, outcomes `ccccmmx` | auto-complete only on a correct final unit |
| golu_typing_flow_fixes | Skipped_Letters_Are_Charged_As_Full_Mistakes#gross | TotalKeystrokes 7 | characters_typed 4 | as above |
| golu_typing_flow_fixes | With_The_Skip_Turned_Off_Space_Is_Evaluated_Where_It_Was_Pressed | caret 4, 1 half | caret 7, 3 missing | no positional mode |
| golu_typing_flow_fixes | With_The_Skip_Turned_Off_Extra_Letters_Are_Committed_As_Before | caret 7 | caret 2 | no positional mode |
| golu_clock_arming | A_Pause_Taken_Before_Typing_Does_Not_Corrupt_Elapsed | pause/resume silently accepted, 6 s | `InvalidSessionState` errors, 6 s | typed errors, not silent no-ops (RULES 23) |
| golu_clock_arming | A_Caller_That_Does_Not_Opt_In_Is_Completely_Unaffected | 30 s | 0 s, Idle | RULES 30: first-keystroke start |
| golu_clock_arming | An_Explicitly_Proctored_Policy_Starts_Timing_Immediately | 25 s | 0 s, 600 s remaining | proctored clock is exam policy (P09) |
| golu_buffer | MultiChar_ProducedText_Appends_All_Units#akshar | 3 states | 2 outcomes, 3 characters typed | RULES 35 per-akshar matching |
| golu_buffer | Grapheme_Level_Correctness_Is_Per_Unit#akshar_mid_cluster | first code point Correct | pending, no outcome | RULES 35 |
| golu_buffer | Grapheme_Level_Correctness_Is_Per_Unit#akshar_complete | 2 correct | 1 correct akshar | RULES 35 |
| golu_result_analyzer | Untyped_Words_Are_Not_Counted_As_Wrong | 1 typed, 1 correct | no committed word yet | only committed words are judged |
| golu_result_analyzer | A_Missing_Space_Reports_The_Word_As_Extra_Character | 1 row "Extra Character" | 5 Extra, word not committed | word-aligned matcher |
| golu_result_analyzer | A_Short_Final_Word_Reports_Missing_Character | Missing Character | no error yet, Active | final word judged at the end / on submit |
| golu_result_analyzer | A_Dropped_Character_Shifts_And_Later_Words_Are_Reported_Wrong_Too | 2 wrong words | `IC`, 1 missing | word-aligned (Golu's documented limitation) |
| golu_result_analyzer | Typing_Past_The_End_Of_The_Passage_Is_An_Extra_Character | Extra Character | session completed on "asdf", "xy" ignored | auto-complete on a correct final unit |
| prd_formulas | 9.3_consistency_variable_intervals | PRD comment ~65% | 53.6452771491565 | the formula wins over its example |

## 4. Not extracted, and why

- .NET- or UI-specific: `StringInfo_Really_Does_Glue_…` (platform probe), `An_Approved_Cbi_Exercise_Segments_Losslessly…` (reads a content file; P04), view-model tests (toolbar labels, error-sound toggle, caret `IsCurrent` agreement, `RangeObservableCollection`), the dotted-circle display of isolated marks.
- APIs Radha does not have yet: `Skip(n)` / `SkipToNextWord` as a separate call (`Skip_Cannot_Run_Past_The_End…`, `Skip_Of_Nothing…`, `SkipToNextWord_Needs_A_Running_Session`), `DeleteWord` and Ctrl+Backspace (P06), backspace-disabled policy (`Opting_In_Does_Not_Disturb_Any_Other_Policy_Behaviour`, `Ctrl_Backspace_Is_Refused…`; P06/P09), composition replacement (P03).
- Invented grading: all `ResultGrade` band, remark and status tests (PRD 17.10 #9, RULES 32).
- `A_Malformed_Passage_Does_Not_Throw`: a Rust `&str` cannot hold a lone surrogate, so the case cannot be constructed.
- `The_State_Array_Is_Sized_From_The_Normalised_Text`: covered structurally (unit states are built from the NFC units).
- Weak-key detector thresholds (`WeakKeyDetectorTests`): P08 analytics.
- `Backspaces_Are_Passed_Through_And_Never_Negative`: Radha's backspace count is a `u32` counted by the session, never passed in.

## 5. Open items for later prompts

- P03: layouts must push Unicode in logical order (GAIL types `ि` before its consonant; it must reach the matcher after it). Multi-key composition replacement. Kruti Dev stays on the `char` path.
- P04: normalise whitespace runs in imported content; NFC at import as well.
- P06: wire `tick()` to the UI timer, KEY/WORD toggle, backspace modes, Ctrl+Backspace, checkpoint/restore, incremental evaluation if profiling needs it.
- P08: weak-key heuristics over `KeyStats`.
- P09: exam profiles: half-mistake weights over `MistakeCounts`, proctored clocks, exam-specific deduction rules, word-substitution/addition detection where a source defines it.
