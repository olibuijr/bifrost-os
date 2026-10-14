//! App-facing models of capabilities that the operating system and its daemons provide.
//!
//! Each module holds the types, validation and decisions an app needs to ask about one capability. The system
//! service keeps the state and performs the action (opens a device, locks the screen, routes audio). Nothing here
//! touches a device, a file or a clock: callers pass the values in, so every rule is testable without a machine.

pub mod alarm;
pub mod drawable;
pub mod idle;
pub mod output_route;
pub mod permissions;
pub mod time_limits;
pub mod usage_record;
pub mod usb;
