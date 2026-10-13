//! Shared-login authentication sessions for apps, shaped like the system web-session API. The model lives in
//! `bifrost_ui::web_auth` (over `webauth_session`) so the shell and apps share one format; this module re-exports it for
//! `bifrost_app`. See `docs/bifrost-ui.md` for the decisions.

pub use bifrost_ui::web_auth::{AuthSession, Completion, Error};
