//! Local service discovery parity: the app-facing names for `bifrost_core::dns_browse`.
//!
//! A `ServiceType` such as `_http._tcp` is checked when it is parsed. A `Browser` keeps the
//! instances of one type in one domain. The app feeds it browse replies (added or removed) and
//! resolve replies (host, port and TXT record), and reads the instances back sorted by name.
//! The browser sends no query itself.

pub use bifrost_core::dns_browse::{
    BrowseError, Browser, Change, Instance, ServiceType, ServiceTypeError,
};
