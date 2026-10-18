//! Messages: Matrix chat as a BifrOSt desktop app. [`protocol`] is the wire format (URLs, bodies and answers, parsed from
//! JSON), [`api`] makes the calls over the shell's HTTPS client, [`accounts`] keeps the homeserver logins in the shared
//! passwords group, [`model`] holds the rooms and timelines, [`sync`] runs jobs on utility dispatch queues, and [`window`]
//! and [`draw`] are the live window.

pub mod accounts;
pub mod api;
pub mod attachment;
pub mod crypto;
pub mod draw;
pub mod model;
pub mod protocol;
pub mod state_file;
pub mod store;
pub mod sync;
pub mod window;
