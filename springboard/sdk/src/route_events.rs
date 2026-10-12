//! Route events (`AVCustomRoutingEvent` parity, see `docs/parity/w3-theme-30.md`): an app that offers its own
//! destinations for media is told when the user activates, deactivates or reactivates one of them, and it keeps the
//! state of each destination.
//!
//! The rules follow the framework's event reasons:
//! - `Activate` is the user choosing a route in the picker. It makes the route active and authorizes it.
//! - `Deactivate` is the user clearing a route. The route must be known (it was activated at least once).
//! - `Reactivate` brings back a route the user authorized before. A route that was never activated is refused.
//!
//! A rejected event changes nothing and is not queued. The queue holds at most `capacity` events; when it is full the
//! oldest event is dropped and counted. This is the model only: no picker or network transport produces these events
//! yet, so the matrix row that uses it is partial.

use std::collections::{BTreeMap, VecDeque};

/// `AVCustomRoutingEventReason`: why a route event happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RouteReason {
    Activate,
    Deactivate,
    Reactivate,
}

impl RouteReason {
    /// Stable lower-case name, used in logs and in the control socket.
    pub fn name(self) -> &'static str {
        match self {
            RouteReason::Activate => "activate",
            RouteReason::Deactivate => "deactivate",
            RouteReason::Reactivate => "reactivate",
        }
    }
}

/// `AVCustomDeviceRoute`: a destination the app offers. `id` identifies it; the two optional addresses say how to reach
/// it (a Bluetooth identifier or a local or remote endpoint).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRoute {
    pub id: String,
    pub bluetooth_identifier: Option<String>,
    pub network_endpoint: Option<String>,
}

impl DeviceRoute {
    pub fn new(id: &str) -> Self {
        DeviceRoute { id: id.to_string(), bluetooth_identifier: None, network_endpoint: None }
    }
}

/// `AVCustomRoutingEvent`: one reason for one route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteEvent {
    pub route: DeviceRoute,
    pub reason: RouteReason,
}

/// What the log knows about one route: the last details it was given, whether it is active, and whether the user has
/// ever authorized it (by activating it).
#[derive(Clone, Debug, PartialEq, Eq)]
struct RouteState {
    route: DeviceRoute,
    active: bool,
    authorized: bool,
}

/// Route state plus a bounded queue of the events that were accepted, oldest first.
#[derive(Clone, Debug)]
pub struct RouteEventLog {
    capacity: usize,
    routes: BTreeMap<String, RouteState>,
    queue: VecDeque<RouteEvent>,
    dropped: usize,
}

impl RouteEventLog {
    /// A log that queues up to `capacity` events. A capacity of zero is raised to one.
    pub fn new(capacity: usize) -> Self {
        RouteEventLog { capacity: capacity.max(1), routes: BTreeMap::new(), queue: VecDeque::new(), dropped: 0 }
    }

    /// Applies an event to the route state and queues it. Returns the rejection reason for an event that does not fit
    /// the route's state; a rejected event is not queued and leaves the state as it was.
    pub fn record(&mut self, event: RouteEvent) -> Result<(), &'static str> {
        let id = event.route.id.clone();
        match event.reason {
            RouteReason::Activate => {
                self.routes.insert(id, RouteState { route: event.route.clone(), active: true, authorized: true });
            }
            RouteReason::Deactivate => {
                let state = self.routes.get_mut(&id).ok_or("route is unknown")?;
                state.route = event.route.clone();
                state.active = false;
            }
            RouteReason::Reactivate => {
                let state = self.routes.get_mut(&id).ok_or("route was never activated")?;
                if !state.authorized {
                    return Err("route was never activated");
                }
                state.route = event.route.clone();
                state.active = true;
            }
        }
        if self.queue.len() == self.capacity {
            self.queue.pop_front();
            self.dropped += 1;
        }
        self.queue.push_back(event);
        Ok(())
    }

    /// Whether the route is active now. An unknown route is not active.
    pub fn is_active(&self, route_id: &str) -> bool {
        self.routes.get(route_id).map_or(false, |s| s.active)
    }

    /// The ids of the active routes, in id order.
    pub fn active_routes(&self) -> Vec<&str> {
        self.routes.iter().filter(|(_, s)| s.active).map(|(id, _)| id.as_str()).collect()
    }

    /// The last details given for a route.
    pub fn route(&self, route_id: &str) -> Option<&DeviceRoute> {
        self.routes.get(route_id).map(|s| &s.route)
    }

    /// Takes every queued event, oldest first. The queue is empty afterwards.
    pub fn take_events(&mut self) -> Vec<RouteEvent> {
        self.queue.drain(..).collect()
    }

    /// Events waiting to be taken.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Events dropped because the queue was full.
    pub fn dropped(&self) -> usize {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, reason: RouteReason) -> RouteEvent {
        RouteEvent { route: DeviceRoute::new(id), reason }
    }

    #[test]
    fn activate_makes_a_route_active_and_queues_the_event() {
        let mut log = RouteEventLog::new(8);
        log.record(event("tv", RouteReason::Activate)).unwrap();
        assert!(log.is_active("tv"));
        assert_eq!(log.active_routes(), vec!["tv"]);
        assert_eq!(log.pending(), 1);
    }

    #[test]
    fn deactivate_needs_a_known_route() {
        let mut log = RouteEventLog::new(8);
        assert_eq!(log.record(event("speaker", RouteReason::Deactivate)), Err("route is unknown"));
        assert_eq!(log.pending(), 0);

        log.record(event("speaker", RouteReason::Activate)).unwrap();
        log.record(event("speaker", RouteReason::Deactivate)).unwrap();
        assert!(!log.is_active("speaker"));
        assert!(log.active_routes().is_empty());
    }

    #[test]
    fn reactivate_brings_back_an_authorized_route_only() {
        let mut log = RouteEventLog::new(8);
        assert_eq!(log.record(event("lamp", RouteReason::Reactivate)), Err("route was never activated"));
        assert!(!log.is_active("lamp"));

        log.record(event("lamp", RouteReason::Activate)).unwrap();
        log.record(event("lamp", RouteReason::Deactivate)).unwrap();
        log.record(event("lamp", RouteReason::Reactivate)).unwrap();
        assert!(log.is_active("lamp"));
    }

    #[test]
    fn a_full_queue_drops_the_oldest_event_and_counts_it() {
        let mut log = RouteEventLog::new(2);
        log.record(event("a", RouteReason::Activate)).unwrap();
        log.record(event("b", RouteReason::Activate)).unwrap();
        log.record(event("c", RouteReason::Activate)).unwrap();
        assert_eq!(log.dropped(), 1);
        assert_eq!(log.pending(), 2);

        let ids: Vec<String> = log.take_events().into_iter().map(|e| e.route.id).collect();
        assert_eq!(ids, vec!["b", "c"]);
    }

    #[test]
    fn take_events_drains_the_queue_but_keeps_the_route_state() {
        let mut log = RouteEventLog::new(8);
        log.record(event("tv", RouteReason::Activate)).unwrap();
        assert_eq!(log.take_events().len(), 1);
        assert_eq!(log.pending(), 0);
        assert!(log.take_events().is_empty());
        assert!(log.is_active("tv"));
    }

    #[test]
    fn a_rejected_event_is_not_queued_and_changes_nothing() {
        let mut log = RouteEventLog::new(8);
        log.record(event("tv", RouteReason::Activate)).unwrap();
        let before = log.pending();
        assert!(log.record(event("ghost", RouteReason::Deactivate)).is_err());
        assert_eq!(log.pending(), before);
        assert_eq!(log.active_routes(), vec!["tv"]);
    }

    #[test]
    fn zero_capacity_is_raised_to_one_and_the_latest_route_details_are_kept() {
        let mut log = RouteEventLog::new(0);
        let mut first = DeviceRoute::new("box");
        first.network_endpoint = Some("192.168.1.9:7000".to_string());
        log.record(RouteEvent { route: first, reason: RouteReason::Activate }).unwrap();
        log.record(event("box", RouteReason::Activate)).unwrap();
        assert_eq!(log.pending(), 1);
        assert_eq!(log.dropped(), 1);
        assert_eq!(log.route("box").unwrap().network_endpoint, None);
    }
}
