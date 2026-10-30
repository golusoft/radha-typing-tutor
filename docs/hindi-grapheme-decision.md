# Hindi grapheme decision (P02 spike, RULES 35)

**Decision: implemented.** Unicode Hindi is matched per akshar. `Matcher<U: TextUnit = char>` runs one word-aligned algorithm over either `char` (QWERTY, Kruti Dev legacy stream) or `Akshar` (Unicode Hindi). QWERTY behaviour is unchanged: every pre-P02 matcher, session, accuracy and speed test passes with its assertions untouched.

## Context

- RULES 35 (RULES.MD L550-551): no Hindi comparison on UTF-16 units, bytes or char indices; akshar-aware matching.
- PRD 17.5 TY-13, EXAM-MODE plan G-04.
- Golu Expert AI compares per UTF-16 unit (`TypingSession.Feed` loops over `ProducedText` per `char`). Its `IndicGraphemeSegmenter` is used for display cells only.

## Options considered

| Option | Result | Why not / why |
|---|---|---|
| A. Keep `char` matching | `क्षे` is 4 slots; one slip can show as several errors; a cluster being built shows as wrong; NFC spellings of a nukta letter mismatch | violates RULES 35 |
| B. Plain extended grapheme clusters | conjuncts stay together (Unicode 15.1+ GB9c), but `" ं"` (space + baseless mark) is one cluster, so a drill keystroke "does nothing" | one defect left |
| **C. NFC + extended clusters + Indic post-rule R1** | all 27 spike words and all Golu segmentation vectors match | **chosen** |
| D. Custom akshar parser | full control | reimplements Unicode tables; more code to get wrong |

## The rules

Pinned crates: `unicode-segmentation = "=1.13.3"` (Unicode 17.0.0, implements GB9c: Consonant [Extend|Linker]* Linker [Extend|Linker]* × Consonant) and `unicode-normalization = "=0.1.25"`.

1. **NFC** on target and typed text. `U+0958` is a composition exclusion, so both spellings of क़ converge on `U+0915 U+093C`.
2. **Extended grapheme clusters** (`graphemes(true)`).
3. **R1, baseless marks.** If a cluster starts with a non-base (not a letter, or itself a combining mark) and every remaining code point is a combining mark, split off the first code point. Ported from Golu defect 1. `"\r\n"` is not split (LF is not a mark).

Golu's second fix (a virama must not absorb a following space) is unnecessary: GB9c only joins a following consonant. The spike asserts `"श् श्"` → `श्`, ` `, `श्`.

## Typing-time behaviour

- The typed buffer is kept as text and re-segmented on every evaluation, so a cluster built over several keys is judged as a whole.
- **Pending prefix:** while the last typed akshar is a strict prefix of the expected one (क → क् → क्ष toward क्षे), it gets no outcome. It is never shown wrong mid-build. For `char` this never fires.
- Accuracy and errors are per akshar: a one-akshar typo is exactly one error (asserted for every spike word).
- Speed (WPM, KDPH) counts code points of typed units, which is what Unicode layouts emit per key press, keeping WPM comparable with Golu.
- Backspace removes one code point (one key), and the akshar is re-judged.

## Spike results

Produced by `cargo test -p radha-core --test grapheme_spike -- --nocapture`. "Golu (emulated)" is a test-only port of `IndicGraphemeSegmenter` over pre-GB9c clusters (Expert AI is read-only and never built, so the C# could not be run). The port reproduces all of Golu's own segmentation vectors before its column is trusted. The two error columns retype the word with its last akshar replaced by `ॐ`. The char matcher column also shows the NFC problem: for precomposed nukta targets (`U+095E`, `U+095C`) the char path compares unnormalised target code points, so a correct prefix scores errors.

| word | code points (NFC) | expected | Radha | Golu (emulated) | Radha = expected | char-matcher errors, 1 typo | akshar errors, 1 typo |
|---|---|---|---|---|---|---|---|
| namaste | 0928 092E 0938 094D 0924 0947 | न·म·स्ते | न·म·स्ते | न·म·स्ते | yes | 1 | 1 |
| kshetra | 0915 094D 0937 0947 0924 094D 0930 | क्षे·त्र | क्षे·त्र | क्षे·त्र | yes | 1 | 1 |
| rashtriya | 0930 093E 0937 094D 091F 094D 0930 0940 092F | रा·ष्ट्री·य | रा·ष्ट्री·य | रा·ष्ट्री·य | yes | 1 | 1 |
| vidyalaya | 0935 093F 0926 094D 092F 093E 0932 092F | वि·द्या·ल·य | वि·द्या·ल·य | वि·द्या·ल·य | yes | 1 | 1 |
| hindi | 0939 093F 0928 094D 0926 0940 | हि·न्दी | हि·न्दी | हि·न्दी | yes | 1 | 1 |
| karya (reph) | 0915 093E 0930 094D 092F | का·र्य | का·र्य | का·र्य | yes | 1 | 1 |
| dharma (reph) | 0927 0930 094D 092E | ध·र्म | ध·र्म | ध·र्म | yes | 1 | 1 |
| shri | 0936 094D 0930 0940 | श्री | श्री | श्री | yes | 1 | 1 |
| gyan | 091C 094D 091E 093E 0928 | ज्ञा·न | ज्ञा·न | ज्ञा·न | yes | 1 | 1 |
| trishul | 0924 094D 0930 093F 0936 0942 0932 | त्रि·शू·ल | त्रि·शू·ल | त्रि·शू·ल | yes | 1 | 1 |
| stri (2 viramas) | 0938 094D 0924 094D 0930 0940 | स्त्री | स्त्री | स्त्री | yes | 1 | 1 |
| sanskrit (anusvara) | 0938 0902 0938 094D 0915 0943 0924 | सं·स्कृ·त | सं·स्कृ·त | सं·स्कृ·त | yes | 1 | 1 |
| drishti | 0926 0943 0937 094D 091F 093F | दृ·ष्टि | दृ·ष्टि | दृ·ष्टि | yes | 1 | 1 |
| dvar | 0926 094D 0935 093E 0930 | द्वा·र | द्वा·र | द्वा·र | yes | 1 | 1 |
| udghatan | 0909 0926 094D 0918 093E 091F 0928 | उ·द्घा·ट·न | उ·द्घा·ट·न | उ·द्घा·ट·न | yes | 1 | 1 |
| hansna (chandrabindu) | 0939 0901 0938 0928 093E | हँ·स·ना | हँ·स·ना | हँ·स·ना | yes | 1 | 1 |
| chand (chandrabindu) | 091A 093E 0901 0926 | चाँ·द | चाँ·द | चाँ·द | yes | 1 | 1 |
| aankh (chandrabindu) | 0906 0901 0916 | आँ·ख | आँ·ख | आँ·ख | yes | 1 | 1 |
| dukh (visarga) | 0926 0941 0903 0916 | दुः·ख | दुः·ख | दुः·ख | yes | 1 | 1 |
| rishi | 090B 0937 093F | ऋ·षि | ऋ·षि | ऋ·षि | yes | 1 | 1 |
| zaroor (nukta, decomposed) | 091C 093C 0930 0942 0930 | ज़·रू·र | ज़·रू·र | ज़·रू·र | yes | 1 | 1 |
| phone (U+095E precomposed) | 092B 093C 094B 0928 | फ़ो·न | फ़ो·न | फ़ो·न | yes | 4 | 1 |
| sadak (U+095C precomposed) | 0938 0921 093C 0915 | स·ड़·क | स·ड़·क | स·ड़·क | yes | 3 | 1 |
| padhai (nukta + matra) | 092A 0922 093C 093E 0908 | प·ढ़ा·ई | प·ढ़ा·ई | प·ढ़ा·ई | yes | 1 | 1 |
| ksha + ZWJ | 0915 094D 200D 0937 | क्‍ष | क्‍ष | क्‍ष | yes | 1 | 1 |
| ksha + ZWNJ | 0915 094D 200C 0937 | क्‌·ष | क्‌·ष | क्‌·ष | yes | 1 | 1 |
| isolated matra drill | 0902 0020 0947 0020 093F | ं· ·े· ·ि | ं· ·े· ·ि | ं· ·े· ·ि | yes | 1 | 1 |

All 27 match the hand-authored expectations. No expectation was changed. Segmentation is lossless over NFC for every word. Radha and the Golu emulation agree on every word; the difference between them is in *matching*, not segmentation: Golu scores the code points inside each cell, Radha scores the cell.

## Differences from Golu

- Golu segments for display and scores per UTF-16 unit; Radha segments and scores per akshar.
- Golu does not normalise typed text in the segmenter; Radha NFCs both sides before segmenting.
- Golu's base test is `char.IsLetter`; Radha's is "alphabetic and not a combining mark", which is the same split for Devanagari (matras are Alphabetic in Unicode but are marks).
- ZWJ / ZWNJ: same result (ZWJ keeps the conjunct, ZWNJ breaks it).

## Left to later prompts

- **P03 layouts:** emit Unicode in logical order. GAIL types `ि` before its consonant; the layout must push it after the consonant. Multi-key composition replacement (`ReplacePreviousCount`) arrives with the layouts.
- **Kruti Dev** is a legacy font encoding, not Unicode: it stays on the `char` path (G-05).
- **Performance:** segmentation is O(n) per evaluation, like the existing matcher. Incremental evaluation is deferred (RULES 34) until profiling on the target hardware asks for it.

## Fallback (not needed)

Had the gate failed, `akshar.rs` would have shipped alone with the matcher left on `char`, plus a concrete integration plan here. The gate passed: every spike word matched and all pre-existing tests stayed green.
