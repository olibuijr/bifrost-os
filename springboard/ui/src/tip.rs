//! Popover tip content and display rules. A tip has a title, a message and an optional action button. Its rules say
//! when it may show: after enough donated events, and at most a set number of times. The store counts the donated
//! events and the displays, keeps invalidated tips hidden for good, and presents the first eligible tip of a list,
//! so one tip shows at a time. The popover itself is placed by `split::place_popover`; this module draws nothing.

use std::collections::BTreeMap;

/// The button a tip offers, such as "Show Me". The action id is what the app receives when the button is pressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TipAction {
    pub id: String,
    pub title: String,
}

/// Why a tip stopped being shown for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalidation {
    /// The user used the feature the tip describes.
    ActionPerformed,
    /// The user closed the tip.
    Dismissed,
}

/// When a tip may show. `required_events` lists event names with the count each must reach before the tip shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub max_displays: Option<u32>,
    pub required_events: Vec<(String, u32)>,
}

/// One tip: the words shown in the popover, the optional action and the display rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tip {
    pub id: String,
    pub title: String,
    pub message: String,
    pub action: Option<TipAction>,
    pub rules: Rules,
}

impl Tip {
    /// A tip with no action and no rules: it may show every time it is offered.
    pub fn new(id: &str, title: &str, message: &str) -> Self {
        Tip { id: id.to_string(), title: title.to_string(), message: message.to_string(), action: None, rules: Rules::default() }
    }

    /// Adds the button the tip offers.
    pub fn with_action(mut self, id: &str, title: &str) -> Self {
        self.action = Some(TipAction { id: id.to_string(), title: title.to_string() });
        self
    }

    /// Replaces the display rules.
    pub fn with_rules(mut self, rules: Rules) -> Self {
        self.rules = rules;
        self
    }
}

/// Donated events and display history for the tips of one app. Counts saturate instead of wrapping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TipStore {
    events: BTreeMap<String, u32>,
    displays: BTreeMap<String, u32>,
    invalidated: BTreeMap<String, Invalidation>,
}

impl TipStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that the app saw `event` once.
    pub fn donate(&mut self, event: &str) {
        bump(&mut self.events, event);
    }

    /// How many times `event` was donated.
    pub fn event_count(&self, event: &str) -> u32 {
        self.events.get(event).copied().unwrap_or(0)
    }

    /// How many times the tip with `tip_id` was presented.
    pub fn display_count(&self, tip_id: &str) -> u32 {
        self.displays.get(tip_id).copied().unwrap_or(0)
    }

    /// The reason the tip with `tip_id` was invalidated, if it was.
    pub fn invalidation(&self, tip_id: &str) -> Option<Invalidation> {
        self.invalidated.get(tip_id).copied()
    }

    /// Hides the tip for good. The first reason stays: a dismissed tip that is later acted on keeps `Dismissed`.
    pub fn invalidate(&mut self, tip_id: &str, reason: Invalidation) {
        self.invalidated.entry(tip_id.to_string()).or_insert(reason);
    }

    /// True when the tip is not invalidated, is under its display limit and has its required events.
    pub fn is_eligible(&self, tip: &Tip) -> bool {
        if self.invalidated.contains_key(&tip.id) {
            return false;
        }
        if tip.rules.max_displays.is_some_and(|max| self.display_count(&tip.id) >= max) {
            return false;
        }
        tip.rules.required_events.iter().all(|(event, min)| self.event_count(event) >= *min)
    }

    /// Presents the first eligible tip of `tips` and counts the display. Returns `None` when no tip is eligible.
    pub fn present<'a>(&mut self, tips: &'a [Tip]) -> Option<&'a Tip> {
        let tip = tips.iter().find(|tip| self.is_eligible(tip))?;
        bump(&mut self.displays, &tip.id);
        Some(tip)
    }
}

fn bump(counts: &mut BTreeMap<String, u32>, key: &str) {
    let count = counts.entry(key.to_string()).or_insert(0);
    *count = count.saturating_add(1);
}
