//! Accessory hub model: the accessories a home hub keeps (node, endpoint, name, room and the clusters each one serves),
//! the On/Off state the hub last applied to each one, and the routing of a short request such as "turn on the kitchen
//! light" to one On/Off command (std only).
//!
//! Routing: the request's action is "toggle", "off" or "on" (checked in that order). An accessory is a candidate when
//! it serves the On/Off cluster and every word of its name, and of its room when it has one, appears in the request.
//! The candidate with the most matching words wins. Two equally good candidates are an error that names both, so the
//! caller can ask which one was meant. Filler words ("the", "turn", "please") are ignored.
//!
//! The hub sends nothing. A routed request is a [`CommandRequest`] that the caller's controller sends, and the caller
//! records the result with [`Hub::apply`]. Gaps: no discovery or commissioning (the setup codes live in `matter`), no
//! level or colour requests, and no scenes or automations.

use crate::matter_cluster::{OnOffState, ON_OFF};
use crate::matter_interaction::{CommandPath, CommandRequest};
use std::collections::{BTreeMap, BTreeSet};

/// The On/Off cluster command IDs.
const CMD_OFF: u32 = 0x00;
const CMD_ON: u32 = 0x01;
const CMD_TOGGLE: u32 = 0x02;

/// Words that carry no name: articles, verbs of the request and the action words.
const FILLER: &[&str] = &["a", "an", "the", "in", "to", "my", "please", "turn", "set", "switch", "on", "off", "toggle"];

/// One accessory as the hub keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accessory {
    pub node: u64,
    /// The endpoint whose On/Off cluster the hub drives.
    pub endpoint: u32,
    pub name: String,
    /// The room the accessory is in; empty when it has none.
    pub room: String,
    /// The cluster IDs the accessory serves. Sorted and without duplicates after [`Hub::add`].
    pub clusters: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HubError {
    /// The name is empty once trimmed.
    EmptyName,
    /// An accessory with this node ID is already kept.
    Duplicate(u64),
}

/// The On/Off action a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    On,
    Off,
    Toggle,
}

/// A request routed to one accessory: the node, the action and the command to send.
#[derive(Debug)]
pub struct Routed {
    pub node: u64,
    pub action: Action,
    pub command: CommandRequest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// The request names no action (on, off or toggle).
    NoAction,
    /// No accessory that serves On/Off matches the request's words.
    NoMatch,
    /// Several accessories match equally well; the nodes are listed in node order.
    Ambiguous(Vec<u64>),
}

/// The accessories a hub keeps, and the On/Off state it last applied to each.
#[derive(Default)]
pub struct Hub {
    accessories: BTreeMap<u64, Accessory>,
    states: BTreeMap<u64, OnOffState>,
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// Keeps an accessory. The name and room are trimmed and the clusters sorted; a node that is already kept is refused.
    pub fn add(&mut self, accessory: Accessory) -> Result<(), HubError> {
        let name = accessory.name.trim().to_string();
        if name.is_empty() {
            return Err(HubError::EmptyName);
        }
        if self.accessories.contains_key(&accessory.node) {
            return Err(HubError::Duplicate(accessory.node));
        }
        let mut clusters = accessory.clusters;
        clusters.sort_unstable();
        clusters.dedup();
        let room = accessory.room.trim().to_string();
        let node = accessory.node;
        self.accessories.insert(node, Accessory { node, endpoint: accessory.endpoint, name, room, clusters });
        Ok(())
    }

    /// Drops an accessory and the state kept for it. Returns `false` when the node is not kept.
    pub fn remove(&mut self, node: u64) -> bool {
        self.states.remove(&node);
        self.accessories.remove(&node).is_some()
    }

    pub fn get(&self, node: u64) -> Option<&Accessory> {
        self.accessories.get(&node)
    }

    /// Every kept accessory, in node order.
    pub fn accessories(&self) -> impl Iterator<Item = &Accessory> {
        self.accessories.values()
    }

    /// The distinct non-empty rooms, sorted.
    pub fn rooms(&self) -> Vec<String> {
        let rooms: BTreeSet<&str> = self.accessories.values().map(|a| a.room.as_str()).filter(|r| !r.is_empty()).collect();
        rooms.into_iter().map(str::to_string).collect()
    }

    /// Applies an On/Off action to an accessory that serves the On/Off cluster and returns the new state. `None` when the
    /// node is not kept or does not serve On/Off; nothing changes in that case.
    pub fn apply(&mut self, node: u64, action: Action) -> Option<bool> {
        let accessory = self.accessories.get(&node)?;
        if !accessory.clusters.contains(&ON_OFF) {
            return None;
        }
        let command = match action {
            Action::On => CMD_ON,
            Action::Off => CMD_OFF,
            Action::Toggle => CMD_TOGGLE,
        };
        let state = self.states.entry(node).or_default();
        state.apply(command);
        Some(state.on)
    }

    /// The On/Off state the hub last applied to a node; `None` when it has applied none.
    pub fn is_on(&self, node: u64) -> Option<bool> {
        self.states.get(&node).map(|s| s.on)
    }

    /// Routes a request to one accessory. See the module notes for the matching rules.
    pub fn route(&self, request: &str) -> Result<Routed, RouteError> {
        let tokens = tokens(request);
        let action = if tokens.iter().any(|t| t == "toggle") {
            Action::Toggle
        } else if tokens.iter().any(|t| t == "off") {
            Action::Off
        } else if tokens.iter().any(|t| t == "on") {
            Action::On
        } else {
            return Err(RouteError::NoAction);
        };
        let wanted = words(request);
        let mut best: Vec<&Accessory> = Vec::new();
        let mut best_score = 0;
        for accessory in self.accessories.values() {
            if !accessory.clusters.contains(&ON_OFF) {
                continue;
            }
            let name = words(&accessory.name);
            let room = words(&accessory.room);
            if name.is_empty() || !name.iter().all(|w| wanted.contains(w)) || !room.iter().all(|w| wanted.contains(w)) {
                continue;
            }
            let score = name.len() + room.len();
            if score > best_score {
                best_score = score;
                best.clear();
            }
            if score == best_score {
                best.push(accessory);
            }
        }
        match best.as_slice() {
            [] => Err(RouteError::NoMatch),
            [one] => {
                let command = match action {
                    Action::On => CMD_ON,
                    Action::Off => CMD_OFF,
                    Action::Toggle => CMD_TOGGLE,
                };
                let path = CommandPath { endpoint: one.endpoint, cluster: ON_OFF, command };
                let command = CommandRequest::new(path, Vec::new()).ok_or(RouteError::NoMatch)?;
                Ok(Routed { node: one.node, action, command })
            }
            many => Err(RouteError::Ambiguous(many.iter().map(|a| a.node).collect())),
        }
    }
}

/// Lower-case words split on anything that is not a letter or digit.
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// The tokens that name something: the tokens without filler and action words.
fn words(text: &str) -> Vec<String> {
    tokens(text).into_iter().filter(|w| !FILLER.contains(&w.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matter_cluster::DESCRIPTOR;

    fn light(node: u64, endpoint: u32, name: &str, room: &str) -> Accessory {
        Accessory { node, endpoint, name: name.to_string(), room: room.to_string(), clusters: vec![ON_OFF, DESCRIPTOR] }
    }

    fn hub() -> Hub {
        let mut hub = Hub::new();
        hub.add(light(1, 1, "Light", "Kitchen")).unwrap();
        hub.add(light(2, 1, "Light", "Bedroom")).unwrap();
        hub.add(Accessory { node: 3, endpoint: 1, name: "Thermostat".into(), room: String::new(), clusters: vec![DESCRIPTOR] }).unwrap();
        hub
    }

    #[test]
    fn add_refuses_empty_names_and_duplicate_nodes() {
        let mut hub = Hub::new();
        assert_eq!(hub.add(light(1, 1, "   ", "")), Err(HubError::EmptyName));
        hub.add(light(1, 1, "Lamp", "")).unwrap();
        assert_eq!(hub.add(light(1, 2, "Other", "")), Err(HubError::Duplicate(1)));
        assert_eq!(hub.get(1).unwrap().name, "Lamp");
    }

    #[test]
    fn add_sorts_and_dedups_clusters() {
        let mut hub = Hub::new();
        hub.add(Accessory { node: 9, endpoint: 1, name: " Fan ".into(), room: " Hall ".into(), clusters: vec![ON_OFF, DESCRIPTOR, ON_OFF] }).unwrap();
        let fan = hub.get(9).unwrap();
        assert_eq!(fan.name, "Fan");
        assert_eq!(fan.room, "Hall");
        assert_eq!(fan.clusters, vec![DESCRIPTOR, ON_OFF]);
        assert_eq!(hub.rooms(), vec!["Hall".to_string()]);
    }

    #[test]
    fn the_room_picks_between_equal_names() {
        let hub = hub();
        let routed = hub.route("turn on the kitchen light").unwrap();
        assert_eq!(routed.node, 1);
        assert_eq!(routed.action, Action::On);
        assert_eq!(routed.command.path, CommandPath { endpoint: 1, cluster: ON_OFF, command: CMD_ON });
    }

    #[test]
    fn equal_matches_are_ambiguous_and_the_room_is_required() {
        let hub = hub();
        assert_eq!(hub.route("switch off the light").unwrap_err(), RouteError::NoMatch);
        let mut plain = Hub::new();
        plain.add(light(4, 1, "Lamp", "")).unwrap();
        plain.add(light(5, 1, "Lamp", "")).unwrap();
        assert_eq!(plain.route("toggle lamp").unwrap_err(), RouteError::Ambiguous(vec![4, 5]));
    }

    #[test]
    fn actions_are_read_from_the_request() {
        let hub = hub();
        assert_eq!(hub.route("toggle bedroom light").unwrap().action, Action::Toggle);
        assert_eq!(hub.route("turn off bedroom light").unwrap().action, Action::Off);
        assert_eq!(hub.route("bedroom light").unwrap_err(), RouteError::NoAction);
    }

    #[test]
    fn accessories_without_on_off_are_not_routed() {
        let hub = hub();
        assert_eq!(hub.route("turn on the thermostat").unwrap_err(), RouteError::NoMatch);
    }

    #[test]
    fn apply_tracks_state_and_refuses_accessories_without_on_off() {
        let mut hub = hub();
        assert_eq!(hub.is_on(1), None);
        assert_eq!(hub.apply(1, Action::On), Some(true));
        assert_eq!(hub.apply(1, Action::Toggle), Some(false));
        assert_eq!(hub.is_on(1), Some(false));
        assert_eq!(hub.apply(3, Action::On), None);
        assert_eq!(hub.apply(42, Action::On), None);
    }

    #[test]
    fn remove_drops_the_accessory_and_its_state() {
        let mut hub = hub();
        hub.apply(2, Action::On);
        assert!(hub.remove(2));
        assert!(!hub.remove(2));
        assert_eq!(hub.is_on(2), None);
        assert_eq!(hub.accessories().count(), 2);
        assert_eq!(hub.rooms(), vec!["Kitchen".to_string()]);
    }
}
