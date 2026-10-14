//! Custom rotors for the screen reader: the named lists an app declares ("Unread", "Errors") beside the standard rotors.
//!
//! The rotor a user cycles through is one of three kinds in this order: "All", the standard rotors (`ax::STANDARD_ROTORS`),
//! then the app's custom rotors in the order the app declared them. [`RotorCycle`] holds the app's rotors, cleaned to the
//! caps the wire format enforces, and answers the index arithmetic. Stepping a custom rotor is `AxCustomRotor::step`; the
//! standard rotors step over the element map (`Map::rotor_step`), so this module leaves them to the caller.

use super::ax::{AxCustomRotor, Rotor, STANDARD_ROTORS};

/// Most custom rotors an app may declare.
pub const MAX_CUSTOM_ROTORS: usize = 8;
/// Most items one custom rotor may list.
pub const MAX_ROTOR_ITEMS: usize = 256;
/// Longest name of a custom rotor, in bytes.
pub const MAX_ROTOR_NAME: usize = 40;

/// Trims the names, drops rotors with no usable name or no items, removes repeated names (the first wins), removes repeated
/// items within a rotor, and applies the caps. The result is what the shell keeps and the screen reader walks.
pub fn clean(rotors: Vec<AxCustomRotor>) -> Vec<AxCustomRotor> {
    let mut out: Vec<AxCustomRotor> = Vec::new();
    for r in rotors {
        if out.len() >= MAX_CUSTOM_ROTORS {
            break;
        }
        let name = truncate(r.name.trim(), MAX_ROTOR_NAME);
        if name.is_empty() || out.iter().any(|o| o.name == name) {
            continue;
        }
        let mut items: Vec<u64> = Vec::new();
        for id in r.items {
            if items.len() >= MAX_ROTOR_ITEMS {
                break;
            }
            if !items.contains(&id) {
                items.push(id);
            }
        }
        if items.is_empty() {
            continue;
        }
        out.push(AxCustomRotor { name: name.to_string(), items });
    }
    out
}

/// The longest prefix of `s` that is at most `max` bytes and ends on a character boundary.
fn truncate(s: &str, max: usize) -> &str {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

/// The rotors the user can pick, in cycle order. Indices wrap, so any number steps round the cycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RotorCycle {
    custom: Vec<AxCustomRotor>,
}

impl RotorCycle {
    /// A cycle over `custom`, cleaned with [`clean`].
    pub fn new(custom: Vec<AxCustomRotor>) -> Self {
        RotorCycle { custom: clean(custom) }
    }

    /// The app's rotors after cleaning, in declared order.
    pub fn custom(&self) -> &[AxCustomRotor] {
        &self.custom
    }

    /// How many rotors the user cycles through: "All", the standard rotors and the custom rotors.
    pub fn count(&self) -> usize {
        1 + STANDARD_ROTORS.len() + self.custom.len()
    }

    /// Reduces `index` to a position in the cycle.
    pub fn wrap(&self, index: usize) -> usize {
        index % self.count()
    }

    /// The announced name of rotor `index` (wrapped).
    pub fn name(&self, index: usize) -> &str {
        let i = self.wrap(index);
        if i == 0 {
            "All"
        } else if i <= STANDARD_ROTORS.len() {
            STANDARD_ROTORS[i - 1].name
        } else {
            &self.custom[i - 1 - STANDARD_ROTORS.len()].name
        }
    }

    /// The standard rotor at `index` (wrapped), or `None` for "All" and the custom rotors.
    pub fn standard(&self, index: usize) -> Option<&'static Rotor> {
        let i = self.wrap(index);
        (i >= 1 && i <= STANDARD_ROTORS.len()).then(|| &STANDARD_ROTORS[i - 1])
    }

    /// The custom rotor at `index` (wrapped), or `None` for "All" and the standard rotors.
    pub fn custom_at(&self, index: usize) -> Option<&AxCustomRotor> {
        let i = self.wrap(index);
        i.checked_sub(1 + STANDARD_ROTORS.len()).and_then(|k| self.custom.get(k))
    }

    /// The next (`forward`) or previous item of custom rotor `index` after `from`. None when `index` is not a custom rotor.
    pub fn step_custom(&self, index: usize, from: Option<u64>, forward: bool) -> Option<u64> {
        self.custom_at(index)?.step(from, forward)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::STANDARD_ROTORS;

    fn rot(name: &str, items: &[u64]) -> AxCustomRotor {
        AxCustomRotor::new(name, items.to_vec())
    }

    #[test]
    fn clean_trims_drops_and_dedupes() {
        let out = clean(vec![rot("  Unread ", &[3, 1, 3, 2]), rot("", &[9]), rot("Errors", &[]), rot("Unread", &[7]), rot("Links", &[5])]);
        assert_eq!(out, vec![rot("Unread", &[3, 1, 2]), rot("Links", &[5])]);
    }

    #[test]
    fn clean_applies_the_caps() {
        let many: Vec<AxCustomRotor> = (0..12).map(|i| rot(&format!("r{i}"), &[i as u64])).collect();
        assert_eq!(clean(many).len(), MAX_CUSTOM_ROTORS);
        let ids: Vec<u64> = (0..(MAX_ROTOR_ITEMS as u64 + 50)).collect();
        assert_eq!(clean(vec![rot("Big", &ids)])[0].items.len(), MAX_ROTOR_ITEMS);
        let long = "n".repeat(MAX_ROTOR_NAME + 9);
        assert_eq!(clean(vec![rot(&long, &[1])])[0].name.len(), MAX_ROTOR_NAME);
    }

    #[test]
    fn clean_keeps_names_on_a_character_boundary() {
        let name = format!("{}\u{f0}", "a".repeat(MAX_ROTOR_NAME - 1));
        let out = clean(vec![rot(&name, &[1])]);
        assert!(out[0].name.len() <= MAX_ROTOR_NAME);
        assert!(out[0].name.is_char_boundary(out[0].name.len()));
    }

    #[test]
    fn the_cycle_is_all_then_standard_then_custom() {
        let c = RotorCycle::new(vec![rot("Unread", &[1]), rot("Errors", &[2])]);
        assert_eq!(c.count(), 1 + STANDARD_ROTORS.len() + 2);
        assert_eq!(c.name(0), "All");
        assert_eq!(c.name(1), STANDARD_ROTORS[0].name);
        let first_custom = STANDARD_ROTORS.len() + 1;
        assert_eq!(c.name(first_custom), "Unread");
        assert_eq!(c.name(first_custom + 1), "Errors");
        assert_eq!(c.name(c.count()), "All", "the index wraps after the last rotor");
    }

    #[test]
    fn kinds_are_told_apart() {
        let c = RotorCycle::new(vec![rot("Unread", &[1])]);
        assert!(c.standard(0).is_none() && c.custom_at(0).is_none());
        assert!(c.standard(1).is_some() && c.custom_at(1).is_none());
        let custom = STANDARD_ROTORS.len() + 1;
        assert!(c.standard(custom).is_none());
        assert_eq!(c.custom_at(custom).map(|r| r.name.as_str()), Some("Unread"));
    }

    #[test]
    fn stepping_a_custom_rotor_wraps_and_ignores_other_kinds() {
        let c = RotorCycle::new(vec![rot("Unread", &[10, 20, 30])]);
        let idx = STANDARD_ROTORS.len() + 1;
        assert_eq!(c.step_custom(idx, None, true), Some(10));
        assert_eq!(c.step_custom(idx, Some(30), true), Some(10), "forward wraps at the end");
        assert_eq!(c.step_custom(idx, Some(10), false), Some(30), "backward wraps at the start");
        assert_eq!(c.step_custom(0, None, true), None, "All is not a custom rotor");
        assert_eq!(RotorCycle::default().count(), 1 + STANDARD_ROTORS.len(), "no custom rotors: only All and the standard ones");
    }
}
