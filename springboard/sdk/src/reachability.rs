//! Reachability for apps: whether the machine has a default route, and whether a host accepts a TCP connection on a
//! port. The model and the probe are in `bifrost_core::reachability`.
//!
//! ```ignore
//! if !reachability::default_route_present() { /* offline: show the offline state */ }
//! // a blocking probe: run it on a worker thread, not the main loop
//! match reachability::probe("updates.example.org", 443, Duration::from_secs(3)) {
//!     Reach::Reachable(addr) => { /* the host answers */ }
//!     Reach::Refused | Reach::TimedOut | Reach::NoAddress | Reach::Failed(_) => { /* not reachable now */ }
//! }
//! ```
//! The route flags for a host or address (reachable, connection required, local, cellular) are in
//! [`reachability_flags`], computed from the network path with no probe. Gap: the interface a route uses and the change
//! events the system's reachability observers give are not reported.
pub use bifrost_core::reachability::{classify, default_route_present, has_default_route, probe, Reach, ROUTE_TABLE};
