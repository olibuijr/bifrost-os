//! App-facing models of system capabilities: USB device matching, daily limits and Downtime, usage records, drawable
//! sizing, idle and sleep rules, output routing, alarm schedules and app permissions. The models live in
//! `bifrost_ui::os_services` so the shell and the apps share one rule set; this module gives apps the SDK path.
//!
//! Each model is a pure function or value type. The system service owns the state and performs the action.

pub use bifrost_ui::os_services::*;
