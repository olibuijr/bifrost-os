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
//! Gap: this is a TCP probe of one host and port. It does not report the system's route flags for a named host, the
//! interface a route uses, or the change events the system's reachability observers give.
pub use bifrost_core::reachability::{classify, default_route_present, has_default_route, probe, Reach, ROUTE_TABLE};
