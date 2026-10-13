//! Background work for the mail window. Each job runs on its own thread with its own connection, and sends one [`Event`]
//! back. A byte written to the wake pipe tells the window's loop to read the channel, so nothing waits on a network call.

use crate::account::Account;
use crate::message::Draft;
use crate::session::{self, Listed, LIST_LIMIT};
use crate::submit;
use springboard::deildu::http::Transport;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// What a mailbox listing read: the mailboxes of the account and the newest messages of the one asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub mailboxes: Vec<String>,
    pub messages: Vec<Listed>,
}

/// The answer of one job. Each carries the account id so the window can tell which account it belongs to.
#[derive(Debug)]
pub enum Event {
    Listed { account: String, mailbox: String, result: Result<Listing, String> },
    Body { account: String, mailbox: String, uid: u32, result: Result<String, String> },
    Sent { account: String, result: Result<(), String> },
}

/// Runs `work` on a new thread and sends its event. The wake pipe, when there is one, is written after the event is
/// queued.
pub fn spawn(tx: Sender<Event>, wake: Option<UnixStream>, work: impl FnOnce() -> Event + Send + 'static) {
    std::thread::spawn(move || {
        let event = work();
        if tx.send(event).is_ok() {
            if let Some(mut wake) = wake {
                let _ = wake.write(&[1]);
            }
        }
    });
}

/// The mailboxes of `account` and the newest messages of `mailbox`.
pub fn list(transport: &dyn Transport, account: &Account, password: &str, mailbox: &str) -> Event {
    let result = fetch_listing(transport, account, password, mailbox);
    Event::Listed { account: account.id.clone(), mailbox: mailbox.to_string(), result }
}

fn fetch_listing(transport: &dyn Transport, account: &Account, password: &str, mailbox: &str) -> Result<Listing, String> {
    let mut session = session::connect(transport, &account.imap)?;
    session.login(&account.username, password)?;
    let mailboxes: Vec<String> = session.list()?.into_iter().map(|m| m.name).collect();
    session.select(mailbox)?;
    let uids = session.uids()?;
    let newest: Vec<u32> = uids.iter().rev().take(LIST_LIMIT).copied().collect();
    let messages = session.headers(&newest)?;
    session.logout();
    Ok(Listing { mailboxes, messages })
}

/// The whole text of one message. Opening it sets the Seen flag on the server.
pub fn body(transport: &dyn Transport, account: &Account, password: &str, mailbox: &str, uid: u32) -> Event {
    let result = fetch_body(transport, account, password, mailbox, uid);
    Event::Body { account: account.id.clone(), mailbox: mailbox.to_string(), uid, result }
}

fn fetch_body(transport: &dyn Transport, account: &Account, password: &str, mailbox: &str, uid: u32) -> Result<String, String> {
    let mut session = session::connect(transport, &account.imap)?;
    session.login(&account.username, password)?;
    session.select(mailbox)?;
    let text = session.message(uid)?;
    let _ = session.mark_seen(uid);
    session.logout();
    Ok(text)
}

/// Sends a rendered message (`text`, from [`Draft::render`]) to every recipient of `draft`.
pub fn send(transport: Arc<dyn Transport>, account: Account, password: String, draft: Draft, text: String) -> Event {
    let recipients: Vec<String> = draft.to.iter().chain(draft.cc.iter()).cloned().collect();
    let result = submit::send(transport.as_ref(), &account.smtp, &account.username, &password, &draft.from, &recipients, &text);
    Event::Sent { account: account.id, result }
}
