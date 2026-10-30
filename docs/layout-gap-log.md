# Layout gap log (P03)

All references are Golu Expert AI files (owner's material). Radha re-transcribed and cross-checked them; nothing below was verified against an external standard, a physical typewriter or a font. Keyman / m17n files were not opened or copied.

## Chart results (`crates/radha-layouts/tests/charts.rs`)

| Layout | Slots match | No flag | Conflict | Unverified | Primary missing |
|---|---|---|---|---|---|
| qwerty | 94/94 | 94 | 0 | 0 | 0 |
| gail | 96/96 | 74 | 21 | 1 | 2 |
| cbi | 101/101 | 92 | 2 | 7 | 0 |
| krutidev | 94/94 (+5 Alt codes, 2 unresolved) | 88 | 6 | 2 | 0 |

Conflicts are resolved by majority of sub-references (ties: Golu code). Every slot's value is stated by at least one reference.

## Provenance (paths relative to the Golu root; sha256 in each chart/fixture JSON)

| Layout | Primary | Cross-checks |
|---|---|---|
| qwerty | `src\Core\Golu.EngineCore\Layout\LayoutManager.cs` FingerMap, ShiftedSymbolBase | US board |
| gail | `docs\gail-keyboard-diagram.md` | complete reference Layout 3; `RemingtonGailLayoutPlugin.cs` keycaps + AltGr; `docs\gail-character-audit.md` |
| cbi | `docs\HINDI-KEYBOARD-LAYOUTS-COMPLETE-REFERENCE.md` Layout 4 | `RemingtonCbiLayoutPlugin.cs` keycaps + AltGr; `docs\cbi-character-audit.md` |
| krutidev | `docs\krutidev-cbi-mapping-conversion.md` §2, §5b, §6 | complete reference Layout 5; `tools\content-engine\krutidev-keymap.js` KD; converter algorithm `krutidev-convert-core.js` |
| fingers | `LayoutManager.cs` FingerMap, adopted by each layout's own `finger_for` | — |

Charts were produced once by a throw-away script that parsed these files read-only; the script was deleted.

## Reference conflicts

| Id | Slot(s) | References disagree | Resolved |
|---|---|---|---|
| C-GAIL-1 | Base `3 6 7 8 9 0` | diagram cells show Shift glyphs | Latin digits (complete ref + code) |
| C-GAIL-2 | Shift `1`-`0 - =` (12) | diagram cells show US legends | complete ref + code |
| C-GAIL-3 | Shift `,` / `.` | complete ref `ष`/`ढ` | `ढ`/`झ` (diagram + code) |
| C-GAIL-4 | AltGr `,` `ँ` | audit L21: unmapped, UNVERIFIED | mapped (complete ref + code) |
| C-CBI-1 | Shift `,` / `.` | complete ref `ष`/`ढ` vs code `ढ`/`झ` | code |
| C-KD-1 | Shift `6` / `8` | complete ref ASCII `'` vs §2 `‘` `’` | §2 + code |
| C-KD-2 | Shift `,` / `.` | complete ref `ष`/`ढ` | `ढ`/`झ` (§2 + code) |
| C-KD-3 | `n`, `m` | complete ref "ASCII Stored" swaps `m`/`n` | identity rule |
| C-PRD-1 | — | `PRD.md` L1101-1110 sample GAIL map (`C -> क`) contradicts every Golu source | treated as placeholder; owner to confirm |

## Unverified entries (reference itself flags them)

| Layout | Slot | Output | Reference flag |
|---|---|---|---|
| gail | AltGr `,` | `ँ` | gail-character-audit L21 UNVERIFIED |
| cbi | AltGr `w` | `ॉ` | cbi-character-audit L20 UNVERIFIED, engineering decision |
| cbi | AltGr `` ` `` `o` `b` `,` `.` | `़ ॐ ई ँ ।` | cbi-character-audit L38-41 engineering decision |
| cbi | AltGr `/` | `?` | plugin L14-16 (classic CBI needs an Alt code) |
| krutidev | Shift `6`, Shift `8` | `‘` `’` | mapping doc L311-313 "assigned by key order, not proven" |
| cbi | `f` logical `ि` | — | Golu code/tests only; needs a real CBI chart |
| gail, cbi | Caps Lock flips letter layers (G-ASSUME-1) | — | no reference describes it |
| all | `shift_hand` = opposite hand (G-ASSUME-2) | — | Radha convention |

## Missing / untypeable (logged, not invented)

| Layout | Characters | Note |
|---|---|---|
| gail | `ॉ`, `?`, Devanagari digits, `ॐ` | no key or sequence; `Guide::untypeable()` reports them |
| cbi | Latin digits | Devanagari digit row |
| krutidev | `ॐ`, `*` | mapping doc §6; converter refuses them |
| all | ZWJ/ZWNJ, `\r` | no key; P04 must normalise `\r\n` |

## Intentional divergences from Golu

| Id | Behaviour | Golu | Radha |
|---|---|---|---|
| G-DIV-1 | Backspace after rakar (`क्र`) | removes the token (`क`) | removes one code point (`क्`) |
| G-DIV-2 | GAIL `f` alone | commits `ि` | preedit, committed `""` |
| G-DIV-3 | precomposed `क़` (U+0958) to Kruti Dev | failure | NFC first, `d+` |
| G-DIV-4 | Alt codes | any Windows-1252 code | only 161, 165, 176, 188, 189 |
| G-DIV-5 | `f` struck after a complete akshar, then no consonant | bare sign merges into it | sign discarded (keeps that akshar correct); `d f Z` gives `र्कि` |

## Decisions with residual risk

| Id | Decision |
|---|---|
| G-RULE9-FINGER | All four layouts adopt the US finger chart through their own `finger_for`. |
| G-PLAN-1 | Only plans up to one key longer than the shortest are treated as legitimate intermediates; the guide teaches fewest keys, then no late `ि`, then no sign-after-virama state, then fewest modifiers. |
| G-KD-PATH | Kruti Dev is matched per `char` on the legacy stream; accuracy is not strictly comparable with GAIL/CBI. |

## Not extracted this phase

`RemingtonGailLayoutTests`, `GailShiftStateTests`, `RemingtonGailKeyboardTests`, `RephRakarKeyTests`, `LegacyFontGoldenTests`, `GuideTeachesTheRealOrderTests`, `PlannerProbeTests` were not turned into fixtures; their single-key facts are covered by the charts, and guide-follow / pending behaviour by `session_integration.rs`. DevLys, Bijoy, InScript and Tamil cases are out of scope.
