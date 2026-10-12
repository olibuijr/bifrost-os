//! Credential exchange for apps: the document model that moves password records between credential managers.
//! The model lives in `bifrost_ui::credential_exchange` so the shell and apps share one format; this module re-exports it
//! for `bifrost_app`. See `docs/parity/w9-authentication-services.md` for the format and the decisions.

pub use bifrost_ui::credential_exchange::{escape, export, import, unescape, ExchangeError, ImportReport, PasswordRecord, HEADER};
