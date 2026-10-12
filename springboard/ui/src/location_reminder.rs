//! Location-based reminders: a named place with a radius, and an alarm that fires when the user arrives at it or
//! leaves it.
//!
//! The place is a structured location (a title, a coordinate and a radius in metres measured from the coordinate, as
//! the official structured location does). The alarm chooses arriving or leaving. [`LocationAlarm::region`] turns it
//! into a [`CircularRegion`] for the region monitor in `geofence`, and [`LocationAlarm::fires`] picks out the enter or
//! exit event that the alarm waits for. The region monitor applies the notify flags of each region, so an alarm for
//! arriving never reports leaving. This module stores no reminders, reads no position and shows no notification. The
//! official structured location can also be made from a title alone, with no coordinate. This model needs one, so
//! that it can watch a region.

use crate::geo::Coordinate;
use crate::geofence::{CircularRegion, RegionEvent, RegionEventKind};

/// A named place and the distance from it that counts as being there.
#[derive(Clone, Debug, PartialEq)]
pub struct StructuredLocation {
    /// Trimmed. Never empty.
    pub title: String,
    pub coordinate: Coordinate,
    /// Metres from the coordinate, always finite and positive.
    pub radius_m: f64,
}

impl StructuredLocation {
    /// `None` when the title is blank after trimming, or the radius is not a finite, positive number.
    pub fn new(title: &str, coordinate: Coordinate, radius_m: f64) -> Option<Self> {
        let title = title.trim();
        (!title.is_empty() && radius_m.is_finite() && radius_m > 0.0).then(|| StructuredLocation {
            title: title.to_string(),
            coordinate,
            radius_m,
        })
    }
}

/// Which change of position sounds the alarm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proximity {
    Arriving,
    Leaving,
}

/// An alarm that belongs to one reminder. The identifier names the region the alarm watches.
#[derive(Clone, Debug, PartialEq)]
pub struct LocationAlarm {
    pub identifier: String,
    pub location: StructuredLocation,
    pub proximity: Proximity,
}

impl LocationAlarm {
    /// The region to hand the monitor. Only the enter notification is on for `Arriving`, and only the exit
    /// notification for `Leaving`. `None` when the identifier is empty.
    pub fn region(&self) -> Option<CircularRegion> {
        let mut region = CircularRegion::new(&self.identifier, self.location.coordinate, self.location.radius_m)?;
        region.notify_on_entry = self.proximity == Proximity::Arriving;
        region.notify_on_exit = self.proximity == Proximity::Leaving;
        Some(region)
    }

    /// True when `event` is for this alarm's region and is the change it waits for: an entry for `Arriving`, an exit
    /// for `Leaving`.
    pub fn fires(&self, event: &RegionEvent) -> bool {
        let wanted = match self.proximity {
            Proximity::Arriving => RegionEventKind::Entered,
            Proximity::Leaving => RegionEventKind::Exited,
        };
        event.identifier == self.identifier && event.kind == wanted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn centre() -> Coordinate {
        Coordinate::new(64.0, -21.0).unwrap()
    }

    fn alarm(proximity: Proximity) -> LocationAlarm {
        LocationAlarm {
            identifier: "reminder-7".into(),
            location: StructuredLocation::new("  Office ", centre(), 100.0).unwrap(),
            proximity,
        }
    }

    fn event(identifier: &str, kind: RegionEventKind) -> RegionEvent {
        RegionEvent { identifier: identifier.into(), kind, timestamp: 0.0 }
    }

    #[test]
    fn structured_location_trims_title_and_checks_radius() {
        assert_eq!(StructuredLocation::new("  Office ", centre(), 50.0).unwrap().title, "Office");
        assert!(StructuredLocation::new("   ", centre(), 50.0).is_none());
        assert!(StructuredLocation::new("Office", centre(), 0.0).is_none());
        assert!(StructuredLocation::new("Office", centre(), f64::NAN).is_none());
        assert!(StructuredLocation::new("Office", centre(), -5.0).is_none());
    }

    #[test]
    fn arriving_alarm_region_reports_only_entry() {
        let region = alarm(Proximity::Arriving).region().unwrap();
        assert_eq!(region.identifier, "reminder-7");
        assert_eq!(region.radius_m, 100.0);
        assert!(region.notify_on_entry);
        assert!(!region.notify_on_exit);
    }

    #[test]
    fn leaving_alarm_region_reports_only_exit() {
        let region = alarm(Proximity::Leaving).region().unwrap();
        assert!(!region.notify_on_entry);
        assert!(region.notify_on_exit);
    }

    #[test]
    fn region_is_refused_for_an_empty_identifier() {
        let mut a = alarm(Proximity::Arriving);
        a.identifier = String::new();
        assert!(a.region().is_none());
    }

    #[test]
    fn fires_only_on_the_matching_change_for_the_same_region() {
        let arriving = alarm(Proximity::Arriving);
        assert!(arriving.fires(&event("reminder-7", RegionEventKind::Entered)));
        assert!(!arriving.fires(&event("reminder-7", RegionEventKind::Exited)));
        assert!(!arriving.fires(&event("reminder-8", RegionEventKind::Entered)));

        let leaving = alarm(Proximity::Leaving);
        assert!(leaving.fires(&event("reminder-7", RegionEventKind::Exited)));
        assert!(!leaving.fires(&event("reminder-7", RegionEventKind::Entered)));
    }

    #[test]
    fn radius_boundary_follows_the_distance_from_the_centre() {
        // One degree of latitude is R * pi / 180 metres, about 111195.08 m with R = 6371008.8 m. So 0.0008 degrees
        // is about 88.96 m (inside 100 m) and 0.0009 degrees is about 100.08 m (outside).
        let region = alarm(Proximity::Arriving).region().unwrap();
        assert!(region.contains(Coordinate::new(64.0008, -21.0).unwrap()));
        assert!(!region.contains(Coordinate::new(64.0009, -21.0).unwrap()));
    }
}
