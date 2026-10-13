//! Mail: accounts, the IMAP and SMTP wire formats, messages and threads, flags, search, the pane layout, and the live
//! window. The protocols run over the shell's TLS transport (`transport`); the window (`window`, `draw`) drives them
//! through background jobs (`sync`) and the keychain.

pub mod account;
pub mod body;
pub mod codec;
pub mod draw;
pub mod flags;
pub mod html;
pub mod imap;
pub mod layout;
pub mod line;
pub mod message;
pub mod search;
pub mod session;
pub mod smtp;
pub mod state;
pub mod store;
pub mod submit;
pub mod sync;
pub mod threads;
pub mod transport;
pub mod window;
