//! Location reminders. A reminder with a place fires when the device arrives at the place or leaves it. A place is a
//! circle of `radius_m` metres round a latitude and longitude. Pure: the location fixes come from the app's location
//! manager (while the app runs) and nothing here touches the disk or the network.

use crate::model::{Reminder, MAX_NAME};
use std::collections::HashMap;

/// The smallest and largest radius a place may have, in metres.
pub const MIN_RADIUS_M: u32 = 25;
pub const MAX_RADIUS_M: u32 = 10_000;
/// The longest encoded place (the store reads it back through this limit).
pub const MAX_TEXT: usize = 300;

const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// Which change of state fires the reminder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Arriving,
    Leaving,
}

impl Trigger {
    pub fn label(self) -> &'static str {
        match self {
            Trigger::Arriving => "Arriving",
            Trigger::Leaving => "Leaving",
        }
    }

    fn code(self) -> char {
        match self {
            Trigger::Arriving => 'A',
            Trigger::Leaving => 'L',
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        match code {
            "A" => Some(Trigger::Arriving),
            "L" => Some(Trigger::Leaving),
            _ => None,
        }
    }
}

/// A place a reminder is tied to.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub radius_m: u32,
    pub trigger: Trigger,
}

impl Place {
    /// Whether the place is usable: a finite position on the globe, a radius in range and a name.
    pub fn valid(&self) -> bool {
        let name = self.name.trim();
        self.lat.is_finite()
            && self.lon.is_finite()
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lon)
            && (MIN_RADIUS_M..=MAX_RADIUS_M).contains(&self.radius_m)
            && !name.is_empty()
            && name.chars().count() <= MAX_NAME
            && !name.contains(['\t', '\n', '\r'])
    }

    /// Whether the position `(lat, lon)` is inside the place's circle.
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        distance_m(self.lat, self.lon, lat, lon) <= self.radius_m as f64
    }

    /// The place as one store field: `lat,lon,radius,trigger,name`. The name is last so a comma in it is kept.
    pub fn encode(&self) -> String {
        format!("{:.6},{:.6},{},{},{}", self.lat, self.lon, self.radius_m, self.trigger.code(), self.name)
    }

    /// Reads a place written by `encode`. Anything that does not read back as a valid place is `None`.
    pub fn decode(text: &str) -> Option<Place> {
        let mut parts = text.splitn(5, ',');
        let lat: f64 = parts.next()?.parse().ok()?;
        let lon: f64 = parts.next()?.parse().ok()?;
        let radius_m: u32 = parts.next()?.parse().ok()?;
        let trigger = Trigger::from_code(parts.next()?)?;
        let name = parts.next()?.to_string();
        let place = Place { name, lat, lon, radius_m, trigger };
        place.valid().then_some(place)
    }
}

/// The distance in metres between two positions (haversine on a sphere; within a few metres of the ellipsoid
/// at the scale of a reminder's place).
pub fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()
}

/// Whether a change from `was_inside` to `now_inside` fires a place with `trigger`. The first reading (`was_inside`
/// `None`) never fires, so opening the app inside a place does not alert.
pub fn crossed(trigger: Trigger, was_inside: Option<bool>, now_inside: bool) -> bool {
    match (trigger, was_inside) {
        (_, None) => false,
        (Trigger::Arriving, Some(was)) => !was && now_inside,
        (Trigger::Leaving, Some(was)) => was && !now_inside,
    }
}

/// Remembers, for each reminder with a place, whether the last fix was inside it, and reports the reminders that
/// fired on each new fix. Completed reminders never fire.
#[derive(Debug, Default)]
pub struct Watch {
    inside: HashMap<u64, bool>,
}

impl Watch {
    /// Feeds one location fix to the watch and returns the ids of the reminders that fired on it.
    pub fn fix(&mut self, items: &[Reminder], lat: f64, lon: f64) -> Vec<u64> {
        let mut fired = Vec::new();
        for r in items.iter().filter(|r| !r.completed) {
            let Some(place) = &r.place else { continue };
            let now = place.contains(lat, lon);
            let was = self.inside.insert(r.id, now);
            if crossed(place.trigger, was, now) {
                fired.push(r.id);
            }
        }
        // forget reminders that no longer have a place
        self.inside.retain(|id, _| items.iter().any(|r| r.id == *id && r.place.is_some()));
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn office() -> Place {
        Place { name: "Office, Reykjavik".into(), lat: 64.1466, lon: -21.9426, radius_m: 100, trigger: Trigger::Arriving }
    }

    #[test]
    fn distance_is_zero_for_the_same_point_and_about_111_km_per_degree_of_latitude() {
        assert!(distance_m(64.0, -21.0, 64.0, -21.0) < 1e-6);
        let d = distance_m(0.0, 0.0, 1.0, 0.0);
        assert!((d - 111_195.0).abs() < 100.0, "{d}");
    }

    #[test]
    fn a_place_contains_the_points_inside_its_radius_only() {
        let p = office();
        assert!(p.contains(64.1466, -21.9426));
        assert!(p.contains(64.1466 + 0.0005, -21.9426)); // about 56 m north
        assert!(!p.contains(64.1466 + 0.002, -21.9426)); // about 220 m north
    }

    #[test]
    fn a_place_round_trips_through_its_store_field_with_commas_in_the_name() {
        let p = office();
        let back = Place::decode(&p.encode()).unwrap();
        assert_eq!(back.name, "Office, Reykjavik");
        assert_eq!((back.radius_m, back.trigger), (100, Trigger::Arriving));
        assert!((back.lat - p.lat).abs() < 1e-6 && (back.lon - p.lon).abs() < 1e-6);
    }

    #[test]
    fn a_bad_place_field_is_refused() {
        assert!(Place::decode("95.0,0.0,100,A,Pole").is_none(), "latitude out of range");
        assert!(Place::decode("64.0,-21.0,5,A,Too small").is_none(), "radius below the minimum");
        assert!(Place::decode("64.0,-21.0,100,X,Bad trigger").is_none());
        assert!(Place::decode("64.0,-21.0,100,A,").is_none(), "no name");
        assert!(Place::decode("nan,-21.0,100,A,Nan").is_none());
        assert!(Place::decode("64.0,-21.0,100").is_none(), "too few fields");
    }

    #[test]
    fn arriving_fires_once_on_the_change_into_the_place_and_never_on_the_first_fix() {
        assert!(!crossed(Trigger::Arriving, None, true), "the first fix inside does not alert");
        assert!(crossed(Trigger::Arriving, Some(false), true));
        assert!(!crossed(Trigger::Arriving, Some(true), true), "already inside");
        assert!(!crossed(Trigger::Arriving, Some(true), false), "leaving does not fire an arrival");
    }

    #[test]
    fn leaving_fires_on_the_change_out_of_the_place() {
        assert!(crossed(Trigger::Leaving, Some(true), false));
        assert!(!crossed(Trigger::Leaving, Some(false), false));
        assert!(!crossed(Trigger::Leaving, None, false));
    }

    #[test]
    fn the_watch_fires_an_arrival_from_outside_and_skips_completed_reminders() {
        let mut far = Reminder::new(1, "Buy milk");
        far.place = Some(office());
        let mut done = Reminder::new(2, "Done");
        done.place = Some(office());
        done.completed = true;
        let items = vec![far, done];
        let mut watch = Watch::default();
        assert!(watch.fix(&items, 64.0, -21.0).is_empty(), "first fix, outside");
        assert_eq!(watch.fix(&items, 64.1466, -21.9426), vec![1], "walks in: fires for the open one only");
        assert!(watch.fix(&items, 64.1466, -21.9426).is_empty(), "stays inside: no repeat");
    }

    #[test]
    fn a_place_that_is_removed_is_forgotten_by_the_watch() {
        let mut r = Reminder::new(3, "Gym");
        r.place = Some(Place { name: "Gym".into(), lat: 64.0, lon: -21.0, radius_m: 50, trigger: Trigger::Leaving });
        let mut watch = Watch::default();
        watch.fix(std::slice::from_ref(&r), 64.0, -21.0);
        r.place = None;
        watch.fix(std::slice::from_ref(&r), 64.0, -21.0);
        assert!(watch.inside.is_empty());
    }
}
