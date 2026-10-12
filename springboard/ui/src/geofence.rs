//! Circular geographic regions and an enter and exit monitor for them.
//!
//! The types follow the official region-monitoring model (a region with an identifier, a centre and a radius, and a
//! monitor that reports when the user's position enters or leaves it) in outline. The monitor has no thread, no
//! timer and no position source. A caller feeds it [`LocationFix`] values in time order and gets [`RegionEvent`]
//! values back.
//!
//! Each fix is judged with its own horizontal accuracy, so noise near the edge does not make a region flicker. A fix
//! counts as inside when its distance to the centre plus its accuracy is within the radius, and as outside when its
//! distance minus its accuracy is beyond the radius. A fix between those two limits keeps the previous state. The
//! accuracy margin is this module's own rule, not part of the official model.

use crate::geo::Coordinate;
use crate::location::LocationFix;

/// A circular area on the Earth's surface.
#[derive(Clone, Debug, PartialEq)]
pub struct CircularRegion {
    /// Name that identifies the region to its owner. Unique within a [`RegionMonitor`].
    pub identifier: String,
    pub center: Coordinate,
    /// Radius in metres, always positive.
    pub radius_m: f64,
    /// Whether entering the region produces an event.
    pub notify_on_entry: bool,
    /// Whether leaving the region produces an event.
    pub notify_on_exit: bool,
}

impl CircularRegion {
    /// A region that reports both entry and exit. `None` when the identifier is empty or the radius is not a finite,
    /// positive number.
    pub fn new(identifier: &str, center: Coordinate, radius_m: f64) -> Option<Self> {
        (!identifier.is_empty() && radius_m.is_finite() && radius_m > 0.0).then(|| CircularRegion {
            identifier: identifier.to_string(),
            center,
            radius_m,
            notify_on_entry: true,
            notify_on_exit: true,
        })
    }

    /// True when `point` lies within the radius, the boundary included.
    pub fn contains(&self, point: Coordinate) -> bool {
        self.center.distance_m(point) <= self.radius_m
    }
}

/// What a monitor knows about the user's position relative to a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionState {
    /// No fix has decided the position yet.
    Unknown,
    Inside,
    Outside,
}

/// The kind of change a region reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionEventKind {
    Entered,
    Exited,
}

/// A change of state of one region, stamped with the time of the fix that caused it.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionEvent {
    pub identifier: String,
    pub kind: RegionEventKind,
    pub timestamp: f64,
}

#[derive(Clone, Debug)]
struct Watched {
    region: CircularRegion,
    state: RegionState,
}

/// Watches a set of regions and turns a stream of fixes into enter and exit events.
#[derive(Clone, Debug, Default)]
pub struct RegionMonitor {
    watched: Vec<Watched>,
}

impl RegionMonitor {
    /// A monitor that watches no regions.
    pub fn new() -> Self {
        RegionMonitor::default()
    }

    /// Starts watching `region`. Returns `false`, and changes nothing, when a region with the same identifier is
    /// already watched.
    pub fn add(&mut self, region: CircularRegion) -> bool {
        if self.watched.iter().any(|w| w.region.identifier == region.identifier) {
            return false;
        }
        self.watched.push(Watched { region, state: RegionState::Unknown });
        true
    }

    /// Stops watching the region called `identifier`. Returns whether one was removed.
    pub fn remove(&mut self, identifier: &str) -> bool {
        let before = self.watched.len();
        self.watched.retain(|w| w.region.identifier != identifier);
        self.watched.len() != before
    }

    /// The number of regions being watched.
    pub fn len(&self) -> usize {
        self.watched.len()
    }

    pub fn is_empty(&self) -> bool {
        self.watched.is_empty()
    }

    /// The last known state of the region called `identifier`. `None` when it is not watched.
    pub fn state(&self, identifier: &str) -> Option<RegionState> {
        self.watched.iter().find(|w| w.region.identifier == identifier).map(|w| w.state)
    }

    /// Judges one fix against every watched region and returns the events it caused, in the order the regions were
    /// added. Invalid fixes (see [`LocationFix::is_valid`]) are ignored.
    ///
    /// The first decision for a region only sets its state: the user's starting position is not reported as an entry
    /// or an exit. After that, only a change of state produces an event, and only when the region asks for that kind
    /// of notification.
    pub fn observe(&mut self, fix: &LocationFix) -> Vec<RegionEvent> {
        if !fix.is_valid() {
            return Vec::new();
        }
        let accuracy = fix.horizontal_accuracy_m;
        let mut events = Vec::new();
        for w in &mut self.watched {
            let distance = w.region.center.distance_m(fix.coordinate);
            let next = if distance + accuracy <= w.region.radius_m {
                RegionState::Inside
            } else if distance - accuracy > w.region.radius_m {
                RegionState::Outside
            } else {
                w.state
            };
            if next == w.state {
                continue;
            }
            let previous = w.state;
            w.state = next;
            if previous == RegionState::Unknown {
                continue;
            }
            let (kind, wanted) = match next {
                RegionState::Inside => (RegionEventKind::Entered, w.region.notify_on_entry),
                _ => (RegionEventKind::Exited, w.region.notify_on_exit),
            };
            if wanted {
                events.push(RegionEvent { identifier: w.region.identifier.clone(), kind, timestamp: fix.timestamp });
            }
        }
        events
    }
}

/// Which crossings of a region an alarm reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proximity {
    /// Only entering the region.
    Enter,
    /// Only leaving the region.
    Leave,
    /// Both entering and leaving.
    Both,
}

impl CircularRegion {
    /// Sets which crossings the region reports.
    pub fn with_proximity(mut self, proximity: Proximity) -> Self {
        self.notify_on_entry = matches!(proximity, Proximity::Enter | Proximity::Both);
        self.notify_on_exit = matches!(proximity, Proximity::Leave | Proximity::Both);
        self
    }
}

/// A named place with a trigger radius, used as the location of a calendar item's alarm. The alarm fires when the
/// user comes within `radius_m` of `geo_location`, so the place is the circle a [`RegionMonitor`] watches.
#[derive(Clone, Debug, PartialEq)]
pub struct StructuredLocation {
    /// Shown to the user, for example "Office". Never blank.
    pub title: String,
    pub geo_location: Coordinate,
    /// Trigger distance in metres, always positive.
    pub radius_m: f64,
}

impl StructuredLocation {
    /// `None` when the title is blank or the radius is not a finite, positive number.
    pub fn new(title: &str, geo_location: Coordinate, radius_m: f64) -> Option<Self> {
        (!title.trim().is_empty() && radius_m.is_finite() && radius_m > 0.0).then(|| StructuredLocation {
            title: title.to_string(),
            geo_location,
            radius_m,
        })
    }

    /// The circle this location triggers on, under the identifier `identifier`.
    pub fn region(&self, identifier: &str) -> Option<CircularRegion> {
        CircularRegion::new(identifier, self.geo_location, self.radius_m)
    }
}

impl RegionMonitor {
    /// Starts watching the circle of `location` under `identifier`. Returns `false` when the identifier is already
    /// watched or the location cannot form a region.
    pub fn add_location(&mut self, identifier: &str, location: &StructuredLocation) -> bool {
        location.region(identifier).map_or(false, |region| self.add(region))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::LocationFix;

    fn c(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate::new(latitude, longitude).expect("valid test coordinate")
    }

    fn fix(point: Coordinate, accuracy_m: f64, timestamp: f64) -> LocationFix {
        let mut f = LocationFix::new(point, timestamp);
        f.horizontal_accuracy_m = accuracy_m;
        f
    }

    fn region(identifier: &str, radius_m: f64) -> CircularRegion {
        CircularRegion::new(identifier, c(0.0, 0.0), radius_m).expect("valid test region")
    }

    #[test]
    fn regions_reject_an_empty_identifier_and_bad_radii() {
        assert!(CircularRegion::new("", c(0.0, 0.0), 100.0).is_none());
        assert!(CircularRegion::new("home", c(0.0, 0.0), 0.0).is_none());
        assert!(CircularRegion::new("home", c(0.0, 0.0), -5.0).is_none());
        assert!(CircularRegion::new("home", c(0.0, 0.0), f64::NAN).is_none());
        assert!(CircularRegion::new("home", c(0.0, 0.0), 100.0).is_some());
    }

    #[test]
    fn containment_includes_the_boundary() {
        let r = region("office", 1_000.0);
        let inside = c(0.0, 0.0).destination(90.0, 999.0).expect("valid point");
        let outside = c(0.0, 0.0).destination(90.0, 1_001.0).expect("valid point");
        assert!(r.contains(inside));
        assert!(!r.contains(outside));
    }

    #[test]
    fn entry_and_exit_are_reported_once_per_change() {
        let mut m = RegionMonitor::new();
        assert!(m.add(region("office", 500.0)));
        let far = c(0.0, 1.0);
        let near = c(0.0, 0.0);

        // The first fix only sets the starting state.
        assert!(m.observe(&fix(far, 10.0, 1.0)).is_empty());
        assert_eq!(m.state("office"), Some(RegionState::Outside));

        let entered = m.observe(&fix(near, 10.0, 2.0));
        assert_eq!(entered, vec![RegionEvent { identifier: "office".into(), kind: RegionEventKind::Entered, timestamp: 2.0 }]);
        assert!(m.observe(&fix(near, 10.0, 3.0)).is_empty());

        let exited = m.observe(&fix(far, 10.0, 4.0));
        assert_eq!(exited.len(), 1);
        assert_eq!(exited[0].kind, RegionEventKind::Exited);
        assert_eq!(exited[0].timestamp, 4.0);
    }

    #[test]
    fn an_ambiguous_fix_near_the_edge_keeps_the_previous_state() {
        let mut m = RegionMonitor::new();
        m.add(region("office", 500.0));
        m.observe(&fix(c(0.0, 0.0), 10.0, 1.0));
        assert_eq!(m.state("office"), Some(RegionState::Inside));

        // 500 m from the centre with 100 m of accuracy: it might be inside or outside, so nothing changes.
        let edge = c(0.0, 0.0).destination(90.0, 500.0).expect("valid point");
        assert!(m.observe(&fix(edge, 100.0, 2.0)).is_empty());
        assert_eq!(m.state("office"), Some(RegionState::Inside));
    }

    #[test]
    fn invalid_fixes_are_ignored() {
        let mut m = RegionMonitor::new();
        m.add(region("office", 500.0));
        // A fix built with LocationFix::new has no accuracy yet, so it is not valid.
        let unknown_accuracy = LocationFix::new(c(0.0, 0.0), 1.0);
        assert!(m.observe(&unknown_accuracy).is_empty());
        assert_eq!(m.state("office"), Some(RegionState::Unknown));
    }

    #[test]
    fn notification_flags_silence_one_direction_but_keep_the_state() {
        let mut quiet_entry = region("gate", 500.0);
        quiet_entry.notify_on_entry = false;
        let mut m = RegionMonitor::new();
        m.add(quiet_entry);
        m.observe(&fix(c(0.0, 1.0), 10.0, 1.0));
        assert!(m.observe(&fix(c(0.0, 0.0), 10.0, 2.0)).is_empty());
        assert_eq!(m.state("gate"), Some(RegionState::Inside));
        let left = m.observe(&fix(c(0.0, 1.0), 10.0, 3.0));
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].kind, RegionEventKind::Exited);
    }

    #[test]
    fn add_refuses_duplicates_and_remove_forgets_the_region() {
        let mut m = RegionMonitor::new();
        assert!(m.add(region("home", 100.0)));
        assert!(!m.add(region("home", 900.0)));
        assert_eq!(m.len(), 1);
        assert!(m.remove("home"));
        assert!(!m.remove("home"));
        assert!(m.is_empty());
        assert_eq!(m.state("home"), None);
    }

    #[test]
    fn structured_locations_reject_a_blank_title_and_bad_radii() {
        assert!(StructuredLocation::new("  ", c(0.0, 0.0), 100.0).is_none());
        assert!(StructuredLocation::new("Office", c(0.0, 0.0), 0.0).is_none());
        assert!(StructuredLocation::new("Office", c(0.0, 0.0), f64::INFINITY).is_none());
        let office = StructuredLocation::new("Office", c(0.0, 0.0), 200.0).expect("valid location");
        assert_eq!(office.title, "Office");
        assert_eq!(office.region("office").map(|r| r.radius_m), Some(200.0));
    }

    #[test]
    fn a_location_alarm_fires_on_arrival_and_departure() {
        let office = StructuredLocation::new("Office", c(0.0, 0.0), 500.0).expect("valid location");
        let mut m = RegionMonitor::new();
        assert!(m.add_location("alarm-1", &office));
        assert!(!m.add_location("alarm-1", &office), "one watch per identifier");
        // Hand-worked: 1 degree of longitude on the equator is about 111 km, far outside 500 m.
        m.observe(&fix(c(0.0, 1.0), 10.0, 1.0));
        let arrived = m.observe(&fix(c(0.0, 0.0), 10.0, 2.0));
        assert_eq!(arrived.len(), 1);
        assert_eq!(arrived[0].kind, RegionEventKind::Entered);
        let left = m.observe(&fix(c(0.0, 1.0), 10.0, 3.0));
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].identifier, "alarm-1");
    }

    #[test]
    fn proximity_limits_the_crossings_a_region_reports() {
        let arrive_only = region("arrive", 500.0).with_proximity(Proximity::Enter);
        assert!(arrive_only.notify_on_entry && !arrive_only.notify_on_exit);
        let leave_only = region("leave", 500.0).with_proximity(Proximity::Leave);
        assert!(!leave_only.notify_on_entry && leave_only.notify_on_exit);
        let mut m = RegionMonitor::new();
        m.add(arrive_only);
        m.observe(&fix(c(0.0, 1.0), 10.0, 1.0));
        assert_eq!(m.observe(&fix(c(0.0, 0.0), 10.0, 2.0)).len(), 1);
        assert!(m.observe(&fix(c(0.0, 1.0), 10.0, 3.0)).is_empty(), "exit is silent for an arrive-only region");
        assert_eq!(m.state("arrive"), Some(RegionState::Outside), "the state still follows the user");
    }
}
