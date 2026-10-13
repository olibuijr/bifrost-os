//! Mail core: accounts, the IMAP and SMTP wire formats, messages and threads, flags, search and the pane layout.
//! Nothing here touches the network or the window; the app's connection and UI layers call into it.

pub mod account;
pub mod codec;
pub mod flags;
pub mod html;
pub mod imap;
pub mod layout;
pub mod message;
pub mod search;
pub mod smtp;
pub mod threads;
