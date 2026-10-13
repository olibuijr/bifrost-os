//! Shared-login authentication sessions for apps, shaped like the Safari Services session API. The model lives in
//! `bifrost_ui::safari_auth` (over `webauth_session`) so the shell and apps share one format; this module re-exports it for
//! `bifrost_app`. See `docs/parity/safari-services.md` for the decisions.

pub use bifrost_ui::safari_auth::{AuthSession, Completion, Error};
