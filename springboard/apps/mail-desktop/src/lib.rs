//! Mail: accounts, the IMAP and SMTP wire formats, messages and threads, flags, search, the pane layout, and the live
//! window. The protocols run over the shell's TLS transport (`transport`); the window (`window`, `draw`) drives them
//! through background jobs (`sync`) and Passwords.

pub mod accounts;
pub mod attachments;
pub mod body;
pub mod category;
pub mod codec;
pub mod draw;
pub mod export;
pub mod mute;
pub mod people;
pub mod prefs;
pub mod findbar;
pub mod flags;
pub mod html;
pub mod imap;
pub mod layout;
pub mod line;
pub mod message;
pub mod migrate;
pub mod mbox;
pub mod pdf;
pub mod mime;
pub mod richtext;
pub mod rules;
pub mod schedule;
pub mod search;
pub mod session;
pub mod smtp;
pub mod spelling;
pub mod state;
pub mod submit;
pub mod sync;
pub mod threads;
pub mod transport;
pub mod window;
pub mod signature;
pub mod organize;
