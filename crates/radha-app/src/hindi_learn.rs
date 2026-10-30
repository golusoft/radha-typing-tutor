//! Hindi Learn, beginner track (PRD 17.7 HL-01..HL-10).
//!
//! - Finger-first steps per layout from the PRD key sequences (HL-08):
//!   GAIL `C V C V M H A V N C P N` (12), CBI `W H S` (3), Kruti Dev
//!   `B M H C R A S N` (8). A repeated letter is a reinforcement step.
//! - Unlock rule (HL-01): >= 95 % accuracy over at least
//!   [`UNLOCK_MIN_CHARS`] typed characters, finished.
//! - Mistake coach (HL-02), matra and half-letter trainers (HL-03/04),
//!   guide ramp (HL-05), weak-key repetition (HL-06), daily plan (HL-07).
//!
//! Drill text is generated from the layout's own key table, so it is always
//! typeable on that layout (tested). It is generated content: the screens
//! label it "generated · review pending" like the P04 Hindi Learn lessons.
//! Progress is RAM during a session and written once, on completion, to a
//! small TOML file (never per keystroke, RULES 14).

use std::collections::BTreeMap;
use std::path::Path;

use radha_layouts::krutidev::to_legacy;
use radha_layouts::script::{is_consonant, is_mark, VIRAMA};
use radha_layouts::{load_layout, Finger, Guide, Hand, Keystroke, Layer, LayoutId, PhysicalKey};
use serde::{Deserialize, Serialize};

use crate::practice::HintView;
use crate::typing_rules::GuideMode;

/// Accuracy a step needs to unlock the next one.
pub const UNLOCK_ACCURACY: f64 = 95.0;
/// Characters a step attempt must type before it can unlock.
pub const UNLOCK_MIN_CHARS: usize = 60;
/// Minutes of the daily plan.
pub const DAILY_MINUTES: u32 = 10;

/// The PRD key sequence of a layout (`None` for QWERTY).
#[must_use]
pub fn sequence(layout: LayoutId) -> Option<&'static str> {
    match layout {
        LayoutId::RemingtonGail => Some("CVCVMHAVNCPN"),
        LayoutId::RemingtonCbi => Some("WHS"),
        LayoutId::KrutiDev => Some("BMHCRASN"),
        // No PRD Learn track for QWERTY or InScript (InScript has no Hindi
        // Learn card; it is practised through Hindi Typing Mastery).
        LayoutId::Qwerty | LayoutId::Inscript => None,
    }
}

/// One finger-first step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnStep {
    /// 0-based.
    pub index: usize,
    /// The physical key (keycap legend, "C").
    pub legend: String,
    /// What the key types, as Devanagari (Kruti Dev: the glyph it draws).
    pub glyph: String,
    pub hand: &'static str,
    pub finger: &'static str,
    /// First time this key appears (else a reinforcement step).
    pub new_key: bool,
}

fn names(hand: Hand, finger: Finger) -> (&'static str, &'static str) {
    let h = match hand {
        Hand::Left => "Left",
        Hand::Right => "Right",
    };
    let f = match finger {
        Finger::Pinky => "Little",
        Finger::Ring => "Ring",
        Finger::Middle => "Middle",
        Finger::Index => "Index",
        Finger::Thumb => "Thumb",
    };
    (h, f)
}

/// The steps of a layout, in order.
#[must_use]
pub fn steps(layout: LayoutId) -> Vec<LearnStep> {
    let (Some(seq), Ok(board)) = (sequence(layout), load_layout(layout)) else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    seq.chars()
        .enumerate()
        .filter_map(|(index, letter)| {
            let key = PhysicalKey::from_base(letter.to_ascii_lowercase())?;
            let glyph = board
                .table()
                .keycap(key, Layer::Base)
                .unwrap_or("")
                .to_owned();
            let (hand, finger) = names(board.finger_for(key).0, board.finger_for(key).1);
            let new_key = !seen.contains(&letter);
            seen.push(letter);
            Some(LearnStep {
                index,
                legend: key.legend().to_owned(),
                glyph,
                hand,
                finger,
                new_key,
            })
        })
        .collect()
}

/// What a key types on the base layer, as text the session compares (the
/// raw legacy character on Kruti Dev).
fn key_output(layout: LayoutId, letter: char) -> Option<String> {
    let board = load_layout(layout).ok()?;
    let key = PhysicalKey::from_base(letter.to_ascii_lowercase())?;
    board
        .table()
        .get(Keystroke::new(key, Layer::Base))
        .map(|e| e.output.to_owned())
}

/// A typeable syllable for a key's output: a sign or virama rides on a
/// consonant (the first consonant key already learned, else `क`).
fn syllable(output: &str, carrier: &str) -> String {
    match output.chars().next() {
        Some(c) if is_mark(c) || c == VIRAMA => format!("{carrier}{output}"),
        _ => output.to_owned(),
    }
}

/// Keep only the words of `text` the layout can type (and that are not
/// empty), so a generated drill never asks for an impossible key.
fn typeable_words(layout: LayoutId, words: Vec<String>) -> Vec<String> {
    let Ok(board) = load_layout(layout) else {
        return Vec::new();
    };
    words
        .into_iter()
        .filter(|w| !w.is_empty() && Guide::new(board, w).untypeable().is_empty())
        .collect()
}

/// Repeat `words` until the drill holds at least `min_chars` characters.
fn fill(words: &[String], min_chars: usize) -> String {
    if words.is_empty() {
        return String::new();
    }
    let mut out: Vec<&str> = Vec::new();
    let mut len = 0;
    let mut i = 0;
    while len < min_chars || out.len() < words.len() {
        let w = &words[i % words.len()];
        len += w.chars().count() + 1;
        out.push(w);
        i += 1;
        if i > 400 {
            break;
        }
    }
    out.join(" ")
}

/// The drill text of step `index`: the step's key alone, then with every
/// key learned so far, then the weak units the typist keeps missing
/// (HL-06). Unicode on GAIL/CBI; the legacy stream on Kruti Dev.
#[must_use]
pub fn step_drill(layout: LayoutId, index: usize, weak: &[String]) -> String {
    let Some(seq) = sequence(layout) else {
        return String::new();
    };
    let letters: Vec<char> = seq.chars().collect();
    let Some(&focus) = letters.get(index) else {
        return String::new();
    };
    let mut known: Vec<char> = Vec::new();
    for &l in letters.iter().take(index + 1) {
        if !known.contains(&l) {
            known.push(l);
        }
    }
    let legacy = layout == LayoutId::KrutiDev;
    let outputs: Vec<String> = known
        .iter()
        .filter_map(|&l| key_output(layout, l))
        .collect();
    let carrier = outputs
        .iter()
        .find(|o| o.chars().next().is_some_and(is_consonant))
        .cloned()
        .unwrap_or_else(|| "क".to_owned());
    let unit = |o: &str| {
        if legacy {
            o.to_owned()
        } else {
            syllable(o, &carrier)
        }
    };
    let Some(focus_out) = key_output(layout, focus) else {
        return String::new();
    };
    let f = unit(&focus_out);
    let mut words: Vec<String> = vec![f.clone(); 4];
    for o in &outputs {
        let k = unit(o);
        if k != f {
            words.push(format!("{f}{k}"));
            words.push(format!("{k}{f}"));
        }
    }
    words.push(f.clone());
    // Weak units are session units: Unicode akshars, or legacy characters
    // on Kruti Dev, so they go in as they are.
    for w in weak.iter().take(3) {
        words.extend(std::iter::repeat_n(w.clone(), 2));
    }
    fill(&typeable_words(layout, words), UNLOCK_MIN_CHARS + 20)
}

/// The vowel signs of the matra trainer (HL-03).
pub const MATRAS: [&str; 10] = ["ि", "ी", "ु", "ू", "े", "ै", "ो", "ौ", "ं", "ः"];

/// Matra trainer: every sign on `क` (isolated), then in syllables.
#[must_use]
pub fn matra_drill(layout: LayoutId) -> String {
    let mut words: Vec<String> = MATRAS.iter().map(|m| format!("क{m}")).collect();
    for (i, c) in ["म", "न", "स", "र", "ल"].iter().enumerate() {
        for m in MATRAS.iter().skip(i % 2).step_by(2) {
            words.push(format!("{c}{m}"));
        }
    }
    words.extend(["किताब", "मीठा", "सुबह", "फूल", "मेला", "कैसा", "मोर", "कौन"].map(String::from));
    localise(layout, words)
}

/// The conjuncts of the half-letter trainer (HL-04).
pub const CONJUNCTS: [&str; 10] = ["क्ष", "त्र", "ज्ञ", "श्र", "स्त", "न्द", "प्र", "क्य", "म्ब", "द्ध"];

/// Half-letter / conjunct trainer: each conjunct, then in words.
#[must_use]
pub fn half_letter_drill(layout: LayoutId) -> String {
    let mut words: Vec<String> = CONJUNCTS.iter().map(|c| (*c).to_owned()).collect();
    words.extend(
        [
            "क्षमा",
            "त्रिशूल",
            "ज्ञान",
            "श्रम",
            "मस्त",
            "हिन्दी",
            "प्रेम",
            "वाक्य",
            "अम्बर",
            "शुद्ध",
        ]
        .map(String::from),
    );
    localise(layout, words)
}

fn localise(layout: LayoutId, words: Vec<String>) -> String {
    let words = if layout == LayoutId::KrutiDev {
        words.iter().map(|w| to_legacy(w).legacy).collect()
    } else {
        words
    };
    fill(&typeable_words(layout, words), UNLOCK_MIN_CHARS)
}

/// Whether a step attempt unlocks the next step (HL-01).
#[must_use]
pub fn unlocks(accuracy: f64, chars_typed: usize, finished: bool) -> bool {
    finished && chars_typed >= UNLOCK_MIN_CHARS && accuracy >= UNLOCK_ACCURACY
}

/// The guide ramp (HL-05): two strong runs in a row (>= 97 %) step the
/// guide down; one bad run (< 90 %) steps it back up.
#[must_use]
pub fn ramp(current: GuideMode, recent: &[f64]) -> GuideMode {
    match recent {
        [.., last] if *last < 90.0 => current.more(),
        [.., a, b] if *a >= 97.0 && *b >= 97.0 => current.less(),
        _ => current,
    }
}

/// The mistake coach line (HL-02): the pressed key, the expected key and
/// the right finger.
#[must_use]
pub fn coach_line(pressed: Option<&HintView>, expected: &HintView) -> String {
    let want = format!(
        "{}{} ({})",
        if expected.shift { "Shift+" } else { "" },
        expected.key,
        expected.meaning.as_deref().unwrap_or(&expected.output)
    );
    match pressed {
        Some(p) => format!(
            "You pressed {}{} ({}) — expected {} with the {} {} finger.",
            if p.shift { "Shift+" } else { "" },
            p.key,
            p.meaning.as_deref().unwrap_or(&p.output),
            want,
            expected.hand.to_lowercase(),
            expected.finger.to_lowercase()
        ),
        None => format!(
            "Expected {} with the {} {} finger.",
            want,
            expected.hand.to_lowercase(),
            expected.finger.to_lowercase()
        ),
    }
}

/// Progress of one layout.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutProgress {
    /// Highest open step (0-based); steps above it are locked.
    pub unlocked: usize,
    /// Best accuracy per step (key: 1-based step number; TOML keys are
    /// strings).
    pub best: BTreeMap<String, f64>,
    /// Guide mode slug the ramp has reached.
    pub guide: String,
    /// Last accuracies (most recent last, at most 5).
    pub recent: Vec<f64>,
    /// Unit -> (attempts, errors), decayed as the unit is typed cleanly.
    pub weak: BTreeMap<String, (u32, u32)>,
    /// Day (`YYYY-MM-DD`) -> seconds practised.
    pub days: BTreeMap<String, u32>,
}

/// What recording an attempt changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    pub unlocked_next: bool,
    pub guide: GuideMode,
}

impl LayoutProgress {
    /// The ramp's guide mode (Full until the first ramp step).
    #[must_use]
    pub fn guide_mode(&self) -> GuideMode {
        GuideMode::from_slug(&self.guide).unwrap_or_default()
    }

    /// Weak units for repetition: >= 3 attempts and >= 10 % errors, worst
    /// first.
    #[must_use]
    pub fn weak_units(&self, max: usize) -> Vec<String> {
        let mut v: Vec<(&String, f64)> = self
            .weak
            .iter()
            .filter(|(_, &(a, e))| a >= 3 && f64::from(e) * 10.0 >= f64::from(a))
            .map(|(u, &(a, e))| (u, f64::from(e) / f64::from(a.max(1))))
            .collect();
        v.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(y.0)));
        v.into_iter().take(max).map(|(u, _)| u.clone()).collect()
    }

    /// Record one finished attempt (any beginner drill). `step` is the
    /// step index for a step drill, `None` for a trainer.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        step: Option<usize>,
        step_count: usize,
        accuracy: f64,
        chars_typed: usize,
        finished: bool,
        tallies: &[(String, u32, u32)],
        day: &str,
        seconds: u32,
    ) -> StepOutcome {
        let mut unlocked_next = false;
        if let Some(s) = step {
            let best = self.best.entry((s + 1).to_string()).or_insert(0.0);
            *best = best.max(accuracy);
            if unlocks(accuracy, chars_typed, finished) && s == self.unlocked && s + 1 < step_count
            {
                self.unlocked = s + 1;
                unlocked_next = true;
            }
        }
        self.recent.push(accuracy);
        if self.recent.len() > 5 {
            self.recent.remove(0);
        }
        let guide = ramp(self.guide_mode(), &self.recent);
        if guide != self.guide_mode() {
            // A ramp step starts a fresh run of evidence.
            self.recent.clear();
        }
        self.guide = guide.slug().to_owned();
        for (unit, attempts, errors) in tallies {
            if unit.trim().is_empty() {
                continue;
            }
            let entry = self.weak.entry(unit.clone()).or_insert((0, 0));
            if *errors == 0 && *attempts >= 5 {
                // Typed cleanly: the unit fades out of the repetition set.
                entry.1 /= 2;
            }
            entry.0 = entry.0.saturating_add(*attempts);
            entry.1 = entry.1.saturating_add(*errors);
        }
        self.weak.retain(|_, (_, e)| *e > 0);
        *self.days.entry(day.to_owned()).or_insert(0) += seconds;
        if self.days.len() > 30 {
            let first = self.days.keys().next().cloned();
            if let Some(k) = first {
                self.days.remove(&k);
            }
        }
        StepOutcome {
            unlocked_next,
            guide,
        }
    }

    /// Today's 10-minute plan (HL-07): weak keys, the open step, a trainer.
    #[must_use]
    pub fn daily_plan(&self, steps: &[LearnStep], today: &str) -> DailyPlan {
        let done = self.days.get(today).copied().unwrap_or(0) / 60;
        let mut items = Vec::new();
        let weak = self.weak_units(3);
        let mut step_minutes = 6;
        if !weak.is_empty() {
            items.push(format!("2 min weak keys {}", weak.join(" ")));
            step_minutes = 5;
        }
        if let Some(s) = steps.get(self.unlocked) {
            items.push(format!(
                "{step_minutes} min Step {} ({} = {})",
                s.index + 1,
                s.legend,
                s.glyph
            ));
        }
        let trainer = if self.unlocked * 2 < steps.len().max(1) {
            "matra trainer"
        } else {
            "half-letter trainer"
        };
        let weak_minutes = if weak.is_empty() { 0 } else { 2 };
        items.push(format!(
            "{} min {trainer}",
            DAILY_MINUTES - step_minutes - weak_minutes
        ));
        DailyPlan {
            minutes_done: done,
            items,
        }
    }
}

/// The daily plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyPlan {
    pub minutes_done: u32,
    pub items: Vec<String>,
}

impl DailyPlan {
    /// One line for the Exercise screen.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Today {}/{} min · {}",
            self.minutes_done.min(DAILY_MINUTES),
            DAILY_MINUTES,
            self.items.join(" · ")
        )
    }
}

/// Beginner progress of every layout (`hindi-learn.toml`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnProgress {
    pub layouts: BTreeMap<String, LayoutProgress>,
}

impl LearnProgress {
    /// Read the file; a missing or unreadable file is a fresh start.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                log::warn!("Ignoring unreadable {}: {e}", path.display());
                LearnProgress::default()
            }),
            Err(_) => LearnProgress::default(),
        }
    }

    /// Write the file (once per completed drill).
    ///
    /// # Errors
    ///
    /// When it cannot be serialised or written.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string(self)?;
        // Atomic: a power cut mid-write must not wipe the Learn path.
        crate::paths::write_replace(path, text.as_bytes())?;
        Ok(())
    }

    /// One layout's progress (created on first use).
    pub fn layout_mut(&mut self, layout: LayoutId) -> &mut LayoutProgress {
        self.layouts.entry(layout.as_str().to_owned()).or_default()
    }

    /// One layout's progress, read-only.
    #[must_use]
    pub fn layout(&self, layout: LayoutId) -> LayoutProgress {
        self.layouts
            .get(layout.as_str())
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HINDI: [LayoutId; 3] = [
        LayoutId::RemingtonGail,
        LayoutId::RemingtonCbi,
        LayoutId::KrutiDev,
    ];

    #[test]
    fn step_counts_follow_the_prd_sequences() {
        assert_eq!(steps(LayoutId::RemingtonGail).len(), 12);
        assert_eq!(steps(LayoutId::RemingtonCbi).len(), 3);
        assert_eq!(steps(LayoutId::KrutiDev).len(), 8);
        assert!(steps(LayoutId::Qwerty).is_empty());
        let gail = steps(LayoutId::RemingtonGail);
        let legends: String = gail.iter().map(|s| s.legend.as_str()).collect();
        assert_eq!(legends, "CVCVMHAVNCPN");
        assert!(
            gail[0].new_key && gail[1].new_key && !gail[2].new_key,
            "C V then reinforce C"
        );
        assert!(gail
            .iter()
            .all(|s| !s.glyph.is_empty() && !s.finger.is_empty()));
    }

    #[test]
    fn every_drill_is_typeable_on_its_layout() {
        for layout in HINDI {
            let board = load_layout(layout).unwrap();
            for s in steps(layout) {
                let text = step_drill(layout, s.index, &[]);
                assert!(
                    text.chars().count() >= UNLOCK_MIN_CHARS,
                    "{layout:?} step {}",
                    s.index
                );
                assert!(
                    Guide::new(board, &text).untypeable().is_empty(),
                    "{layout:?} step {}: {text}",
                    s.index
                );
            }
            for text in [matra_drill(layout), half_letter_drill(layout)] {
                assert!(!text.is_empty(), "{layout:?}");
                assert!(
                    Guide::new(board, &text).untypeable().is_empty(),
                    "{layout:?}: {text}"
                );
            }
        }
        // Weak units come back in the drill (HL-06).
        let with_weak = step_drill(LayoutId::RemingtonGail, 3, &["क्ष".to_owned()]);
        assert!(with_weak.contains("क्ष"));
        assert_eq!(step_drill(LayoutId::RemingtonCbi, 99, &[]), "");
    }

    #[test]
    fn unlock_needs_accuracy_length_and_a_finish() {
        assert!(unlocks(95.0, 60, true));
        assert!(!unlocks(94.9, 600, true));
        assert!(!unlocks(100.0, 59, true));
        assert!(!unlocks(100.0, 600, false));

        let mut p = LayoutProgress::default();
        let o = p.record(Some(0), 3, 99.0, 80, true, &[], "2026-10-25", 60);
        assert!(o.unlocked_next);
        assert_eq!(p.unlocked, 1);
        let o = p.record(Some(0), 3, 99.0, 80, true, &[], "2026-10-25", 60);
        assert!(
            !o.unlocked_next,
            "replaying an old step unlocks nothing new"
        );
        p.record(Some(1), 3, 90.0, 80, true, &[], "2026-10-25", 60);
        assert_eq!(p.unlocked, 1, "below 95 % stays locked");
        p.record(Some(1), 3, 96.0, 80, true, &[], "2026-10-25", 60);
        p.record(Some(2), 3, 100.0, 80, true, &[], "2026-10-25", 60);
        assert_eq!(p.unlocked, 2, "the last step has nothing after it");
        assert_eq!(p.days.get("2026-10-25"), Some(&300));
    }

    #[test]
    fn guide_ramp_steps_down_and_back_up() {
        assert_eq!(ramp(GuideMode::Full, &[97.0, 98.0]), GuideMode::Smart);
        assert_eq!(ramp(GuideMode::Full, &[98.0]), GuideMode::Full);
        assert_eq!(ramp(GuideMode::Minimal, &[99.0, 85.0]), GuideMode::Smart);
        assert_eq!(ramp(GuideMode::Off, &[97.0, 97.0]), GuideMode::Off);
        let mut p = LayoutProgress::default();
        p.record(None, 12, 98.0, 80, true, &[], "d", 1);
        p.record(None, 12, 98.0, 80, true, &[], "d", 1);
        assert_eq!(p.guide_mode(), GuideMode::Smart);
        assert!(p.recent.is_empty(), "a ramp step resets the evidence");
        p.record(None, 12, 70.0, 80, true, &[], "d", 1);
        assert_eq!(p.guide_mode(), GuideMode::Full);
    }

    #[test]
    fn weak_keys_repeat_until_typed_cleanly() {
        let mut p = LayoutProgress::default();
        let t = vec![
            ("क्ष".to_owned(), 5, 2),
            ("क".to_owned(), 20, 0),
            ("र".to_owned(), 10, 1),
        ];
        p.record(None, 12, 90.0, 80, true, &t, "d", 1);
        assert_eq!(p.weak_units(5), ["क्ष", "र"]);
        p.record(None, 12, 99.0, 80, true, &[("क्ष".to_owned(), 6, 0)], "d", 1);
        p.record(None, 12, 99.0, 80, true, &[("क्ष".to_owned(), 6, 0)], "d", 1);
        assert!(!p.weak_units(5).contains(&"क्ष".to_owned()), "faded out");
    }

    #[test]
    fn daily_plan_fills_ten_minutes_from_progress() {
        let s = steps(LayoutId::RemingtonGail);
        let mut p = LayoutProgress::default();
        let plan = p.daily_plan(&s, "2026-10-25");
        assert_eq!(plan.items.len(), 2);
        assert!(plan.items[0].starts_with("6 min Step 1 (C = "));
        assert_eq!(plan.items[1], "4 min matra trainer");
        p.weak.insert("ि".into(), (4, 2));
        p.days.insert("2026-10-25".into(), 180);
        let plan = p.daily_plan(&s, "2026-10-25");
        assert_eq!(plan.items[0], "2 min weak keys ि");
        assert!(plan.summary().starts_with("Today 3/10 min · "));
        p.unlocked = 8;
        assert!(p
            .daily_plan(&s, "x")
            .items
            .last()
            .unwrap()
            .ends_with("half-letter trainer"));
    }

    #[test]
    fn coach_names_pressed_expected_and_finger() {
        let hint = |key: &str, out: &str, hand, finger| HintView {
            key: key.into(),
            key_id: key.to_lowercase(),
            shift: false,
            altgr: false,
            hand,
            finger,
            output: out.into(),
            meaning: None,
        };
        let expected = hint("D", "क", "Left", "Middle");
        let pressed = hint("J", "र", "Right", "Index");
        assert_eq!(
            coach_line(Some(&pressed), &expected),
            "You pressed J (र) — expected D (क) with the left middle finger."
        );
        assert!(coach_line(None, &expected).starts_with("Expected D (क)"));
    }

    #[test]
    fn progress_round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hindi-learn.toml");
        assert_eq!(LearnProgress::load(&path), LearnProgress::default());
        let mut all = LearnProgress::default();
        all.layout_mut(LayoutId::KrutiDev).record(
            Some(0),
            8,
            99.0,
            70,
            true,
            &[("f".into(), 4, 2)],
            "d",
            30,
        );
        all.save(&path).unwrap();
        assert_eq!(LearnProgress::load(&path), all);
        std::fs::write(&path, "not = [valid").unwrap();
        assert_eq!(LearnProgress::load(&path), LearnProgress::default());
    }
}
