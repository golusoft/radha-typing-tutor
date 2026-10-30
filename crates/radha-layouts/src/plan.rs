//! The keystroke planner: target akshar in, key sequence out.
//!
//! Direct layouts (QWERTY, Kruti Dev) have one key per character. For the
//! Devanagari layouts the planner searches over the layout's own composer
//! (iterative deepening, cheapest keys first, like Golu's
//! `KeystrokePlanner`), so a plan is exactly what typing it produces: half
//! forms, conjunct keys, rakar, the reph struck last and GAIL's pre-posed
//! `ि` all come out of the same composer the session uses.
//!
//! Besides the shortest plan it collects every other plan of the same length
//! or one key longer, so the guide can follow a typist on a legitimate
//! alternative path and the matcher can keep those states pending.

use std::collections::HashSet;

use radha_core::akshar::normalize;

use crate::compose::Composer;
use crate::keys::{Keystroke, Layer, SPACE};
use crate::script::VIRAMA;
use crate::table::{KeyEntry, TokenKind};
use crate::KeyboardLayout;

/// Node budget per unit (one node = one composer step).
pub const NODE_BUDGET: usize = 20_000;
/// Most alternative plans collected per unit.
pub const MAX_ALTERNATIVES: usize = 32;
/// Plans may be up to this many keys longer than the unit's code points.
const DEPTH_SLACK: usize = 4;
/// Code points at the end of the buffer that may still be rewritten.
const UNSETTLED_TAIL: usize = 3;

/// One state along a plan: committed text and preedit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanState {
    /// Committed text of the unit so far.
    pub committed: String,
    /// Pending preedit (GAIL's pre-posed sign).
    pub preedit: Option<&'static str>,
}

impl PlanState {
    fn empty() -> Self {
        PlanState {
            committed: String::new(),
            preedit: None,
        }
    }
}

/// One step a typist takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStep {
    /// Strike a table slot.
    Key(&'static KeyEntry),
    /// Strike the space bar.
    Space,
    /// Hold Alt and type a numpad code (Kruti Dev).
    AltCode(u16),
}

impl PlanStep {
    /// The keystroke for a key or Space step.
    #[must_use]
    pub fn keystroke(&self) -> Option<Keystroke> {
        match self {
            PlanStep::Key(entry) => Some(entry.keystroke()),
            PlanStep::Space => Some(Keystroke::new(SPACE, Layer::Base)),
            PlanStep::AltCode(_) => None,
        }
    }
}

/// A key sequence and the states it passes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The steps.
    pub steps: Vec<PlanStep>,
    /// `steps.len() + 1` states; `states[0]` is empty.
    pub states: Vec<PlanState>,
}

/// The result of planning one unit.
#[derive(Debug, Clone)]
pub enum PlanOutcome {
    /// At least one plan exists.
    Planned {
        /// Fewest keys, then fewest modifiers, then board order.
        shortest: Plan,
        /// Other plans of the same length or one key longer.
        alternatives: Vec<Plan>,
        /// Every non-final, non-empty committed state of every plan.
        intermediates: HashSet<Box<str>>,
        /// Whether the search for alternatives ran to completion.
        alternatives_complete: bool,
        /// Composer steps spent.
        nodes: usize,
    },
    /// No key sequence reaches the unit within the depth limit.
    Untypeable {
        /// Composer steps spent.
        nodes: usize,
    },
    /// The budget ran out before any plan was found.
    BudgetExhausted {
        /// Composer steps spent.
        nodes: usize,
    },
}

impl PlanOutcome {
    /// The shortest plan, if any.
    #[must_use]
    pub fn shortest(&self) -> Option<&Plan> {
        match self {
            PlanOutcome::Planned { shortest, .. } => Some(shortest),
            _ => None,
        }
    }

    /// Composer steps spent.
    #[must_use]
    pub fn nodes(&self) -> usize {
        match self {
            PlanOutcome::Planned { nodes, .. }
            | PlanOutcome::Untypeable { nodes }
            | PlanOutcome::BudgetExhausted { nodes } => *nodes,
        }
    }

    fn single(steps: Vec<PlanStep>, states: Vec<PlanState>) -> Self {
        PlanOutcome::Planned {
            shortest: Plan { steps, states },
            alternatives: Vec::new(),
            intermediates: HashSet::new(),
            alternatives_complete: true,
            nodes: 0,
        }
    }
}

/// Plans units for one layout.
#[derive(Clone)]
pub struct Planner {
    layout: &'static dyn KeyboardLayout,
    budget: usize,
}

impl std::fmt::Debug for Planner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Planner")
            .field("layout", &self.layout.id())
            .field("budget", &self.budget)
            .finish()
    }
}

/// Search bookkeeping for one unit.
struct Search<'a> {
    target: &'a str,
    target_cps: Vec<char>,
    allowed: HashSet<char>,
    candidates: Vec<&'static KeyEntry>,
    nodes: usize,
    budget: usize,
    exhausted: bool,
    found: Vec<Plan>,
}

impl Planner {
    /// A planner with the default budget.
    #[must_use]
    pub fn new(layout: &'static dyn KeyboardLayout) -> Self {
        Planner::with_budget(layout, NODE_BUDGET)
    }

    /// A planner with an explicit node budget (tests).
    #[must_use]
    pub fn with_budget(layout: &'static dyn KeyboardLayout, budget: usize) -> Self {
        Planner { layout, budget }
    }

    /// Plan one unit (one akshar, one character, or a space).
    #[must_use]
    pub fn plan(&self, unit: &str) -> PlanOutcome {
        if unit == " " {
            return PlanOutcome::single(
                vec![PlanStep::Space],
                vec![
                    PlanState::empty(),
                    PlanState {
                        committed: " ".to_owned(),
                        preedit: None,
                    },
                ],
            );
        }
        if unit.is_empty() || unit.chars().any(char::is_whitespace) {
            return PlanOutcome::Untypeable { nodes: 0 };
        }
        if self.layout.encoding() != crate::Encoding::Unicode {
            return self.plan_direct(unit);
        }
        let target = normalize(unit);
        match self.plan_search(&target) {
            PlanOutcome::Untypeable { nodes } => match self.plan_direct(&target) {
                // A character no key reaches but an Alt code does (`?` is
                // Alt+63 on GAIL/CBI).
                planned @ PlanOutcome::Planned { .. } => planned,
                _ => PlanOutcome::Untypeable { nodes },
            },
            outcome => outcome,
        }
    }

    fn plan_direct(&self, unit: &str) -> PlanOutcome {
        let done = |committed: &str| {
            vec![
                PlanState::empty(),
                PlanState {
                    committed: committed.to_owned(),
                    preedit: None,
                },
            ]
        };
        if let Some(entry) = self.layout.table().producing(unit).into_iter().next() {
            return PlanOutcome::single(vec![PlanStep::Key(entry)], done(unit));
        }
        let mut chars = unit.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            for code in 0..=u16::from(u8::MAX) {
                if let Some(alt) = self.layout.alt_code(code) {
                    if alt.ansi == c {
                        return PlanOutcome::single(vec![PlanStep::AltCode(code)], done(unit));
                    }
                }
            }
        }
        PlanOutcome::Untypeable { nodes: 0 }
    }

    fn plan_search(&self, target: &str) -> PlanOutcome {
        let target_cps: Vec<char> = target.chars().collect();
        let allowed = self.allowed_chars(target);
        let candidates: Vec<&'static KeyEntry> = self
            .layout
            .table()
            .cheapest_first()
            .into_iter()
            .filter(|e| {
                e.kind == TokenKind::Completer || e.output.chars().all(|c| allowed.contains(&c))
            })
            .collect();
        let mut search = Search {
            target,
            target_cps,
            allowed,
            candidates,
            nodes: 0,
            budget: self.budget,
            exhausted: false,
            found: Vec::new(),
        };
        let max_depth = search.target_cps.len() + DEPTH_SLACK;
        let mut shortest_depth: Option<usize> = None;
        let mut capped = false;
        for depth in 1..=max_depth {
            if shortest_depth.is_some_and(|d| depth > d + 1) {
                break;
            }
            let mut steps = Vec::new();
            let mut states = vec![PlanState::empty()];
            self.dfs(
                &mut search,
                &Composer::new(self.layout),
                "",
                depth,
                &mut steps,
                &mut states,
            );
            if shortest_depth.is_none() && !search.found.is_empty() {
                shortest_depth = Some(depth);
            }
            if search.found.len() > MAX_ALTERNATIVES {
                capped = true;
                break;
            }
            if search.exhausted {
                break;
            }
        }
        let nodes = search.nodes;
        // Among the shortest plans, teach the one with the fewest malformed
        // intermediate states (a sign right after a virama, as in `थ्ी`);
        // ties keep discovery order (fewest modifiers, then board order).
        let min_len = search.found.iter().map(|p| p.steps.len()).min();
        let best = search
            .found
            .iter()
            .enumerate()
            .filter(|(_, p)| Some(p.steps.len()) == min_len)
            .min_by_key(|(_, p)| (late_preposed(p), malformed_states(p)))
            .map(|(i, _)| i);
        if let Some(i) = best {
            if i > 0 && i < search.found.len() {
                let chosen = search.found.remove(i);
                search.found.insert(0, chosen);
            }
        }
        let mut found = search.found.into_iter();
        let Some(shortest) = found.next() else {
            if let Some(outcome) = self.plan_trailing_sign(target, nodes) {
                return outcome;
            }
            if search.exhausted {
                log::debug!("planner budget exhausted for {target:?} after {nodes} nodes");
                return PlanOutcome::BudgetExhausted { nodes };
            }
            return PlanOutcome::Untypeable { nodes };
        };
        let alternatives: Vec<Plan> = found.take(MAX_ALTERNATIVES).collect();
        let mut intermediates = HashSet::new();
        for plan in std::iter::once(&shortest).chain(alternatives.iter()) {
            let last = plan.states.len().saturating_sub(1);
            for state in plan.states.iter().take(last).skip(1) {
                if !state.committed.is_empty() {
                    intermediates.insert(state.committed.clone().into_boxed_str());
                }
            }
        }
        let alternatives_complete = !search.exhausted && !capped;
        if !alternatives_complete {
            log::debug!("planner kept a partial alternative set for {target:?}");
        }
        PlanOutcome::Planned {
            shortest,
            alternatives,
            intermediates,
            alternatives_complete,
            nodes,
        }
    }

    /// Fallback for a conjunct that ends in a nasal or visarga sign
    /// (`र्षों`, `क्ष्यों`, `ह्नों`): the search space of the whole unit can
    /// run past the depth or node budget, so plan the unit without its final
    /// sign, then strike the sign's key at the first position (latest first,
    /// so a reph struck last stays last) whose replay through the layout's
    /// own composer yields exactly the unit. Nothing is guessed: a plan is
    /// only returned when the replay reproduces the target.
    fn plan_trailing_sign(&self, target: &str, nodes: usize) -> Option<PlanOutcome> {
        let mut cps: Vec<char> = target.chars().collect();
        let sign = *cps.last()?;
        if cps.len() < 2 || !matches!(sign, '\u{0901}' | '\u{0902}' | '\u{0903}') {
            return None;
        }
        cps.pop();
        let prefix: String = cps.into_iter().collect();
        let PlanOutcome::Planned {
            shortest,
            nodes: prefix_nodes,
            ..
        } = self.plan_search(&prefix)
        else {
            return None;
        };
        for entry in self.layout.table().producing(&sign.to_string()) {
            for at in (0..=shortest.steps.len()).rev() {
                let mut steps = shortest.steps.clone();
                steps.insert(at, PlanStep::Key(entry));
                let Some(states) = self.replay(&steps) else {
                    continue;
                };
                let reached = states
                    .last()
                    .is_some_and(|s| s.committed == target && s.preedit.is_none());
                if !reached {
                    continue;
                }
                let last = states.len().saturating_sub(1);
                let intermediates = states
                    .iter()
                    .take(last)
                    .skip(1)
                    .filter(|s| !s.committed.is_empty())
                    .map(|s| s.committed.clone().into_boxed_str())
                    .collect();
                return Some(PlanOutcome::Planned {
                    shortest: Plan { steps, states },
                    alternatives: Vec::new(),
                    intermediates,
                    alternatives_complete: false,
                    nodes: nodes + prefix_nodes,
                });
            }
        }
        None
    }

    /// The states a key sequence passes through on this layout's composer,
    /// from an empty unit. `None` when a step is unmapped.
    fn replay(&self, steps: &[PlanStep]) -> Option<Vec<PlanState>> {
        let mut composer = Composer::new(self.layout);
        let mut committed = String::new();
        let mut states = vec![PlanState::empty()];
        for step in steps {
            let out = composer.press(step.keystroke()?, &committed)?;
            out.edit.apply_to(&mut committed);
            states.push(PlanState {
                committed: committed.clone(),
                preedit: composer.preedit(),
            });
        }
        Some(states)
    }

    /// Code points a plan may ever commit: the target's, the parts of any
    /// ligature that can reach them (fixed point), and the virama.
    fn allowed_chars(&self, target: &str) -> HashSet<char> {
        let mut allowed: HashSet<char> = target.chars().collect();
        allowed.insert(VIRAMA);
        if let Some(rules) = self.layout.rules() {
            loop {
                let before = allowed.len();
                for lig in rules.ligatures() {
                    // Parts of a reachable result, and results built only
                    // from reachable parts (`आ` on the way to `औ`).
                    if lig.result.chars().all(|c| allowed.contains(&c)) {
                        allowed.extend(lig.joined().chars());
                    }
                    if lig.joined().chars().all(|c| allowed.contains(&c)) {
                        allowed.extend(lig.result.chars());
                    }
                }
                if allowed.len() == before {
                    break;
                }
            }
        }
        allowed
    }

    fn dfs(
        &self,
        search: &mut Search<'_>,
        composer: &Composer,
        committed: &str,
        depth: usize,
        steps: &mut Vec<PlanStep>,
        states: &mut Vec<PlanState>,
    ) {
        for i in 0..search.candidates.len() {
            let Some(&entry) = search.candidates.get(i) else {
                break;
            };
            if search.nodes >= search.budget {
                search.exhausted = true;
                return;
            }
            search.nodes += 1;
            let mut next_composer = composer.clone();
            let Some(out) = next_composer.press(entry.keystroke(), committed) else {
                continue;
            };
            let mut next = committed.to_owned();
            out.edit.apply_to(&mut next);
            let preedit = next_composer.preedit();
            steps.push(PlanStep::Key(entry));
            states.push(PlanState {
                committed: next.clone(),
                preedit,
            });
            if next == search.target && preedit.is_none() {
                if steps.len() == depth && !search.found.iter().any(|p| p.steps == *steps) {
                    search.found.push(Plan {
                        steps: steps.clone(),
                        states: states.clone(),
                    });
                }
            } else if steps.len() < depth && viable(search, &next, preedit) {
                self.dfs(search, &next_composer, &next, depth, steps, states);
            }
            steps.pop();
            states.pop();
            if search.exhausted || search.found.len() > MAX_ALTERNATIVES {
                return;
            }
        }
    }
}

/// Pre-posed keys struck after the unit already has text (the typewriter
/// order strikes `ि` first; the composer tolerates it late, but the guide
/// must not teach that).
fn late_preposed(plan: &Plan) -> usize {
    plan.steps
        .iter()
        .zip(plan.states.iter())
        .filter(|(step, before)| {
            matches!(step, PlanStep::Key(e) if e.kind == TokenKind::Preposed)
                && !before.committed.is_empty()
        })
        .count()
}

/// States of a plan where a sign directly follows a virama.
fn malformed_states(plan: &Plan) -> usize {
    plan.states
        .iter()
        .filter(|s| {
            let cps: Vec<char> = s.committed.chars().collect();
            cps.windows(2).any(|w| {
                w.first() == Some(&VIRAMA) && w.get(1).copied().is_some_and(crate::script::is_mark)
            })
        })
        .count()
}

/// Whether a state can still reach the target.
fn viable(search: &Search<'_>, committed: &str, preedit: Option<&str>) -> bool {
    let cps: Vec<char> = committed.chars().collect();
    if cps.len() > search.target_cps.len() + UNSETTLED_TAIL {
        return false;
    }
    if !cps.iter().all(|c| search.allowed.contains(c)) {
        return false;
    }
    let settled = cps.len().saturating_sub(UNSETTLED_TAIL);
    if !search
        .target_cps
        .starts_with(cps.get(..settled).unwrap_or(&[]))
    {
        return false;
    }
    preedit.is_none_or(|p| search.target.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::load_layout;
    use crate::LayoutId;

    fn keys(plan: &Plan) -> Vec<(char, Layer)> {
        plan.steps
            .iter()
            .filter_map(|s| match s {
                PlanStep::Key(e) => Some((e.key.base_char(), e.layer)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn long_ii_has_the_altgr_key_first_and_the_reph_sequence_as_an_alternative() {
        let plan = |id| {
            let layout = load_layout(id).unwrap();
            let PlanOutcome::Planned {
                shortest,
                alternatives,
                alternatives_complete,
                ..
            } = Planner::new(layout).plan("ई")
            else {
                panic!("ई must be typeable on {id:?}");
            };
            assert!(alternatives_complete);
            (
                keys(&shortest),
                alternatives.iter().map(keys).collect::<Vec<_>>(),
            )
        };
        // GAIL: AltGr+B (GAIL-MAP-122), b + Shift+Z as the alternative.
        let (shortest, alternatives) = plan(LayoutId::RemingtonGail);
        assert_eq!(shortest, [('b', Layer::AltGr)]);
        assert!(alternatives.contains(&vec![('b', Layer::Base), ('z', Layer::Shift)]));
        // CBI has no AltGr layer (book 03): b + Shift+Z.
        let (shortest, _) = plan(LayoutId::RemingtonCbi);
        assert_eq!(shortest, [('b', Layer::Base), ('z', Layer::Shift)]);
    }

    #[test]
    fn a_character_only_an_alt_code_reaches_is_planned_as_that_code() {
        // GAIL-MAP-158: `?` is Alt+63.
        let gail = load_layout(LayoutId::RemingtonGail).unwrap();
        let outcome = Planner::new(gail).plan("?");
        let steps = outcome
            .shortest()
            .map(|p| p.steps.clone())
            .unwrap_or_default();
        assert_eq!(steps, [PlanStep::AltCode(63)]);
    }

    #[test]
    fn a_tiny_budget_is_reported_not_guessed() {
        let layout = load_layout(LayoutId::RemingtonGail).unwrap();
        assert!(matches!(
            Planner::with_budget(layout, 1).plan("क्षे"),
            PlanOutcome::BudgetExhausted { .. }
        ));
        let full = Planner::new(layout).plan("क्षे");
        let needed = full.nodes();
        let partial = Planner::with_budget(layout, needed.saturating_sub(1).max(1)).plan("क्षे");
        if let PlanOutcome::Planned {
            alternatives_complete,
            ..
        } = partial
        {
            assert!(!alternatives_complete);
        }
    }

    /// Replay a plan through a fresh composer and return the committed text.
    fn typed_by(layout: &'static dyn KeyboardLayout, plan: &Plan) -> String {
        let mut composer = Composer::new(layout);
        let mut text = String::new();
        for step in &plan.steps {
            let out = composer
                .press(step.keystroke().expect("key step"), &text)
                .expect("mapped key");
            out.edit.apply_to(&mut text);
        }
        assert!(composer.preedit().is_none());
        text
    }

    #[test]
    fn conjuncts_with_a_final_nasal_sign_are_planned_and_replay_exactly() {
        // Importer report (P04): these akshars had no plan, so the guide
        // showed "—". Every plan must type exactly the unit.
        let cases = [
            (LayoutId::RemingtonGail, "र्षों"),
            (LayoutId::RemingtonGail, "क्ष्यों"),
            (LayoutId::RemingtonCbi, "र्षों"),
            (LayoutId::RemingtonCbi, "क्ष्यों"),
            (LayoutId::RemingtonCbi, "ह्नों"),
            (LayoutId::RemingtonCbi, "र्थों"),
        ];
        for (id, unit) in cases {
            let layout = load_layout(id).unwrap();
            let outcome = Planner::new(layout).plan(unit);
            let plan = outcome
                .shortest()
                .unwrap_or_else(|| panic!("{id:?}: {unit} must be planned"));
            assert_eq!(typed_by(layout, plan), normalize(unit), "{id:?} {unit}");
            assert_eq!(
                plan.states.last().map(|s| s.committed.as_str()),
                Some(normalize(unit).as_str())
            );
        }
    }

    #[test]
    fn untypeable_characters_have_no_plan() {
        // Book 01: ॐ and ₹ are not supported by GAIL ("?" became Alt+63,
        // GAIL-MAP-158, see the test above).
        let gail = load_layout(LayoutId::RemingtonGail).unwrap();
        assert!(matches!(
            Planner::new(gail).plan("ॐ"),
            PlanOutcome::Untypeable { .. }
        ));
        assert!(matches!(
            Planner::new(gail).plan("₹"),
            PlanOutcome::Untypeable { .. }
        ));
        assert!(matches!(
            Planner::new(gail).plan("५"),
            PlanOutcome::Untypeable { .. }
        ));
        let qwerty = load_layout(LayoutId::Qwerty).unwrap();
        assert!(matches!(
            Planner::new(qwerty).plan("क"),
            PlanOutcome::Untypeable { .. }
        ));
    }
}
