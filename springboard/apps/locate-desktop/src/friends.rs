//! Friends: the people who share their location with the person, the distance from each to a place, and the rules that
//! notify when a friend arrives at or leaves a place. Distances use the great-circle metric of the maps app.

use crate::devices::Fix;
use bifrost_maps::route::haversine_m;

#[derive(Clone, Debug, PartialEq)]
pub struct Friend {
    pub address: String,
    pub name: String,
    pub position: Option<Fix>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Arrives,
    Leaves,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotifyRule {
    pub address: String,
    pub trigger: Trigger,
    pub place: (f64, f64),
    pub radius_m: f64,
}

/// Metres from a fix to a place.
pub fn distance_m(fix: &Fix, place: (f64, f64)) -> f64 {
    haversine_m((fix.lat, fix.lon), place)
}

/// The friend's distance from a place, when a position is known.
pub fn friend_distance(friend: &Friend, place: (f64, f64)) -> Option<f64> {
    friend.position.as_ref().map(|fix| distance_m(fix, place))
}

/// Whether a fix lies within the radius of a place.
pub fn inside(fix: &Fix, place: (f64, f64), radius_m: f64) -> bool {
    distance_m(fix, place) <= radius_m
}

/// Whether a trigger fires when the inside state changes from `previous` to `now`. The first state seen fires
/// nothing, so a friend who is already at a place does not notify on the first update.
pub fn fires(trigger: Trigger, previous: Option<bool>, now: bool) -> bool {
    matches!(
        (trigger, previous, now),
        (Trigger::Arrives, Some(false), true) | (Trigger::Leaves, Some(true), false)
    )
}

/// Whether a rule fires for a new fix, given whether the friend was inside its place before.
pub fn rule_fires(rule: &NotifyRule, previous: Option<bool>, fix: &Fix) -> bool {
    fires(rule.trigger, previous, inside(fix, rule.place, rule.radius_m))
}

/// A distance as the list shows it: whole metres under a kilometre, then kilometres with one decimal.
pub fn distance_text(metres: f64) -> String {
    if metres.round() < 1_000.0 {
        format!("{} m", metres.round() as i64)
    } else {
        format!("{:.1} km", metres / 1_000.0)
    }
}
