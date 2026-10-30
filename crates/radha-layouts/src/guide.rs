//! The guide: which key to press next, with which hand, finger and
//! modifiers, for a target text on one layout.
//!
//! The guide segments the target exactly as the session's matcher does, so
//! the session's cursor indexes the same unit here. Every distinct unit is
//! planned once. The guide is also the layout's [`Composition`] hook: a
//! typed unit is a legitimate intermediate state when it is one of the
//! states of a plan for the expected unit.

use std::collections::HashMap;

use radha_core::akshar::segment;
use radha_core::{Akshar, Composition};

use crate::keys::{Keystroke, Layer, PhysicalKey, SPACE};
use crate::plan::{Plan, PlanOutcome, PlanStep, Planner};
use crate::table::{KeyEntry, TokenKind};
use crate::{Encoding, Finger, Hand, KeyboardLayout};

/// Everything the guide panel shows for one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHint {
    /// The physical key.
    pub key: PhysicalKey,
    /// Its keycap legend (`"D"`, `";"`, `"Space"`).
    pub legend: &'static str,
    /// The layer struck.
    pub layer: Layer,
    /// Whether Shift must be held (Caps Lock taken into account).
    pub hold_shift: bool,
    /// Whether AltGr must be held.
    pub hold_altgr: bool,
    /// The striking hand.
    pub hand: Hand,
    /// The striking finger.
    pub finger: Finger,
    /// The hand that holds Shift (the opposite one), when Shift is held.
    pub shift_hand: Option<Hand>,
    /// What the key emits (raw ASCII on Kruti Dev).
    pub output: &'static str,
    /// Kruti Dev: the Devanagari the font draws.
    pub meaning: Option<&'static str>,
    /// Code points the key emits.
    pub output_len: usize,
}

/// The next thing the typist should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextAction {
    /// Strike a key.
    Key(KeyHint),
    /// Hold Alt and type a numpad code. A gesture, so no finger is given.
    AltCode {
        /// The numpad code.
        code: u16,
        /// The character Windows inserts.
        output: char,
        /// What the font draws.
        meaning: &'static str,
    },
}

/// Hints and plans for one target on one layout.
pub struct Guide {
    layout: &'static dyn KeyboardLayout,
    units: Vec<Box<str>>,
    plans: HashMap<Box<str>, PlanOutcome>,
}

impl std::fmt::Debug for Guide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guide")
            .field("layout", &self.layout.id())
            .field("units", &self.units.len())
            .finish()
    }
}

impl Guide {
    /// Plan every distinct unit of `target`.
    #[must_use]
    pub fn new(layout: &'static dyn KeyboardLayout, target: &str) -> Self {
        let trimmed = target.trim();
        let units: Vec<Box<str>> = match layout.encoding() {
            Encoding::Unicode => segment(trimmed)
                .into_iter()
                .map(String::into_boxed_str)
                .collect(),
            Encoding::Latin | Encoding::Legacy => trimmed
                .chars()
                .map(|c| c.to_string().into_boxed_str())
                .collect(),
        };
        let planner = Planner::new(layout);
        let mut plans = HashMap::new();
        for unit in &units {
            if !plans.contains_key(unit) {
                plans.insert(unit.clone(), planner.plan(unit));
            }
        }
        Guide {
            layout,
            units,
            plans,
        }
    }

    /// Units in the target (equals the session's target unit count).
    #[must_use]
    pub fn unit_count(&self) -> usize {
        self.units.len()
    }

    /// The units, as text.
    #[must_use]
    pub fn units(&self) -> &[Box<str>] {
        &self.units
    }

    /// The full planning outcome for a unit.
    #[must_use]
    pub fn outcome_for(&self, unit: &str) -> Option<&PlanOutcome> {
        self.plans.get(unit)
    }

    /// The shortest plan for a unit.
    #[must_use]
    pub fn plan_for(&self, unit: &str) -> Option<&Plan> {
        self.plans.get(unit).and_then(PlanOutcome::shortest)
    }

    /// Units no key sequence on this layout produces, in target order.
    #[must_use]
    pub fn untypeable(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for unit in &self.units {
            let typeable = self.plan_for(unit).is_some();
            if !typeable && !seen.contains(&&**unit) {
                seen.push(&**unit);
            }
        }
        seen
    }

    /// The next action at `cursor` (the session's target cursor).
    /// `pending_tail` is the last typed unit's text while the session reports
    /// it pending; `preedit` is the composer's preedit. `None` past the end
    /// or for a unit with no plan (never an invented key).
    #[must_use]
    pub fn next_action(
        &self,
        cursor: usize,
        pending_tail: Option<&str>,
        preedit: Option<&str>,
        caps_lock: bool,
    ) -> Option<NextAction> {
        let unit = self.units.get(cursor)?;
        if let Some(entry) = self.lone_preposed(unit) {
            // An isolated pre-posed sign (GAIL `ि` at a word start): strike
            // its key; it is held as preedit and the next key (usually the
            // space) commits it, so once it waits the guide shows that key.
            if preedit == Some(entry.output) {
                let next = self
                    .units
                    .get(cursor + 1)
                    .and_then(|u| self.plan_for(u))
                    .and_then(|p| p.steps.first().copied())
                    .unwrap_or(PlanStep::Space);
                return self.action_for(next, caps_lock);
            }
            return Some(NextAction::Key(self.entry_hint(entry, caps_lock)));
        }
        let PlanOutcome::Planned {
            shortest,
            alternatives,
            ..
        } = self.plans.get(unit)?
        else {
            return None;
        };
        let tail = pending_tail.unwrap_or("");
        let step = std::iter::once(shortest)
            .chain(alternatives.iter())
            .find_map(|plan| {
                let i = plan
                    .states
                    .iter()
                    .rposition(|s| s.committed == tail && s.preedit == preedit)?;
                plan.steps.get(i).copied()
            })
            .or_else(|| shortest.steps.first().copied())?;
        self.action_for(step, caps_lock)
    }

    /// The key of a unit that is a lone pre-posed sign with no plan of its
    /// own (the planner only plans units it can commit; this one is
    /// committed by the key after it).
    #[must_use]
    pub fn lone_preposed(&self, unit: &str) -> Option<&'static KeyEntry> {
        if self.plan_for(unit).is_some() || unit.chars().count() != 1 {
            return None;
        }
        self.layout
            .table()
            .producing(unit)
            .into_iter()
            .find(|e| e.kind == TokenKind::Preposed)
    }

    /// What a pressed key is (for a mistake coach).
    #[must_use]
    pub fn hint_for(&self, stroke: Keystroke, caps_lock: bool) -> Option<KeyHint> {
        if stroke.key == SPACE {
            return Some(self.space_hint());
        }
        self.layout
            .table()
            .get(stroke)
            .map(|e| self.entry_hint(e, caps_lock))
    }

    fn action_for(&self, step: PlanStep, caps_lock: bool) -> Option<NextAction> {
        match step {
            PlanStep::Key(entry) => Some(NextAction::Key(self.entry_hint(entry, caps_lock))),
            PlanStep::Space => Some(NextAction::Key(self.space_hint())),
            PlanStep::AltCode(code) => {
                let alt = self.layout.alt_code(code)?;
                Some(NextAction::AltCode {
                    code,
                    output: alt.ansi,
                    meaning: alt.meaning,
                })
            }
        }
    }

    fn entry_hint(&self, entry: &'static KeyEntry, caps_lock: bool) -> KeyHint {
        let (hold_shift, hold_altgr) = entry.keystroke().hold_modifiers(caps_lock);
        let (hand, finger) = self.layout.finger_for(entry.key);
        KeyHint {
            key: entry.key,
            legend: entry.key.legend(),
            layer: entry.layer,
            hold_shift,
            hold_altgr,
            hand,
            finger,
            shift_hand: hold_shift.then(|| hand.opposite()),
            output: entry.output,
            meaning: entry.meaning,
            output_len: entry.output.chars().count(),
        }
    }

    fn space_hint(&self) -> KeyHint {
        let (hand, finger) = self.layout.finger_for(SPACE);
        KeyHint {
            key: SPACE,
            legend: SPACE.legend(),
            layer: Layer::Base,
            hold_shift: false,
            hold_altgr: false,
            hand,
            finger,
            shift_hand: None,
            output: " ",
            meaning: None,
            output_len: 1,
        }
    }
}

impl Composition<Akshar> for Guide {
    fn is_intermediate(&self, typed: &Akshar, expected: &Akshar) -> bool {
        matches!(
            self.plans.get(expected.as_str()),
            Some(PlanOutcome::Planned { intermediates, .. }) if intermediates.contains(typed.as_str())
        )
    }
}
