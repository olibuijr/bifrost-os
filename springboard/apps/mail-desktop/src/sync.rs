//! Background work for the mail window. Each job runs on its own thread with its own connection, and sends one [`Event`]
//! back. A byte written to the wake pipe tells the window's loop to read the channel, so nothing waits on a network call.

use crate::accounts;
use crate::flags::{Flag, FlagColor};
use crate::message::Draft;
use crate::state::INBOX;
use bifrost_app::controls::internet_accounts::Account;
use crate::session::{self, Listed, LIST_LIMIT};
use crate::submit;
use springboard::deildu::http::Transport;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

/// What a mailbox listing read: the mailboxes of the account and the newest messages of the one asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub mailboxes: Vec<String>,
    pub messages: Vec<Listed>,
}

/// A change to one message on the server, made from the reader or the toolbar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Sets or clears one flag (Seen, Flagged, ...).
    Flag { flag: Flag, on: bool },
    /// Flags the message in a colour, or clears its flag and colour (Message > Flag).
    Color { color: Option<FlagColor> },
    /// Moves the message to another mailbox of the account (Archive, Trash, Junk, Move To).
    Move { to: String },
    /// Copies the message to another mailbox of the account (Copy To).
    Copy { to: String },
    /// Erases every message of the mailbox (Erase Deleted Items, Erase Junk Mail). The job's UID is 0.
    EraseAll,
    /// Creates the mailbox named `name`. The job's UID is 0 and its mailbox is not used.
    Create { name: String },
    /// Renames the job's mailbox to `to` (UID 0).
    Rename { to: String },
    /// Deletes the job's mailbox with the messages in it (UID 0).
    Delete,
}

/// The answer of one job. Each carries the account id so the window can tell which account it belongs to.
#[derive(Debug)]
pub enum Event {
    Listed { account: String, mailbox: String, result: Result<Listing, String> },
    Body { account: String, mailbox: String, uid: u32, result: Result<String, String> },
    Sent { account: String, result: Result<(), String> },
    Changed { account: String, mailbox: String, uid: u32, result: Result<(), String> },
    /// The server's matches for the search `query` in `mailbox`: the newest matches' headers.
    Found { account: String, mailbox: String, query: String, result: Result<Vec<Listed>, String> },
    /// The inbox watcher's connection state: `live` while IDLE is waiting for the server.
    Watch { account: String, live: bool },
    /// The server reported a new message in the inbox (from IDLE). The window lists the inbox again.
    Arrived { account: String },
}

impl Event {
    /// True for the answer of a job the window counts as busy. The watcher's reports are not jobs.
    pub fn is_job(&self) -> bool {
        !matches!(self, Event::Watch { .. } | Event::Arrived { .. })
    }
}

/// How long a watcher waits before it opens its connection again after the connection was lost.
const WATCH_RETRY: Duration = Duration::from_secs(60);

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
    let mut session = session::connect(transport, accounts::incoming(account)?)?;
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
    let mut session = session::connect(transport, accounts::incoming(account)?)?;
    session.login(&account.username, password)?;
    session.select(mailbox)?;
    let text = session.message(uid)?;
    let _ = session.mark_seen(uid);
    session.logout();
    Ok(text)
}

/// Applies `action` to one message of `mailbox` on the server.
pub fn change(transport: &dyn Transport, account: &Account, password: &str, mailbox: &str, uid: u32, action: &Change) -> Event {
    let result = apply_change(transport, account, password, mailbox, uid, action);
    Event::Changed { account: account.id.clone(), mailbox: mailbox.to_string(), uid, result }
}

fn apply_change(transport: &dyn Transport, account: &Account, password: &str, mailbox: &str, uid: u32, action: &Change) -> Result<(), String> {
    let mut session = session::connect(transport, accounts::incoming(account)?)?;
    session.login(&account.username, password)?;
    match action {
        Change::Create { name } => session.create_mailbox(name)?,
        Change::Rename { to } => session.rename_mailbox(mailbox, to)?,
        Change::Delete => session.delete_mailbox(mailbox)?,
        _ => {
            session.select(mailbox)?;
            match action {
                Change::Flag { flag, on } => session.set_flag(uid, *flag, *on)?,
                Change::Color { color } => session.set_color(uid, *color)?,
                Change::Move { to } => session.move_to(uid, to)?,
                Change::Copy { to } => session.copy_to(uid, to)?,
                Change::EraseAll => session.erase_all()?,
                Change::Create { .. } | Change::Rename { .. } | Change::Delete => {}
            }
        }
    }
    session.logout();
    Ok(())
}

/// Sends a rendered message (`text`, from [`Draft::render`]) to every recipient of `draft`.
pub fn send(transport: Arc<dyn Transport>, account: Account, password: String, draft: Draft, text: String) -> Event {
    let recipients: Vec<String> = draft.envelope_recipients().into_iter().map(str::to_string).collect();
    let result = accounts::outgoing(&account)
        .and_then(|server| submit::send(transport.as_ref(), server, &account.username, &password, &draft.from, &recipients, &text));
    Event::Sent { account: account.id, result }
}

/// Searches `mailbox` on the server with `criteria` (IMAP search keys, see `Query::imap_criteria`) and reads the headers
/// of the newest matches. `query` is the search text the criteria came from; the window keeps the answer only while
/// that text is still the applied search.
pub fn search(transport: &dyn Transport, account: &Account, password: &str, mailbox: &str, query: &str, criteria: &str) -> Event {
    let result = fetch_matches(transport, account, password, mailbox, criteria);
    Event::Found { account: account.id.clone(), mailbox: mailbox.to_string(), query: query.to_string(), result }
}

fn fetch_matches(transport: &dyn Transport, account: &Account, password: &str, mailbox: &str, criteria: &str) -> Result<Vec<Listed>, String> {
    let mut session = session::connect(transport, accounts::incoming(account)?)?;
    session.login(&account.username, password)?;
    session.select(mailbox)?;
    let uids = session.search(criteria)?;
    let newest: Vec<u32> = uids.iter().rev().take(LIST_LIMIT).copied().collect();
    let messages = session.headers(&newest)?;
    session.logout();
    Ok(messages)
}

/// What ended a watcher's connection: `Stop` is final (the server has no IDLE, or the password is refused); `Retry`
/// opens the connection again after a pause.
enum WatchEnd {
    Stop,
    Retry,
}

/// Keeps an IDLE connection (RFC 2177) open on the inbox of `account` and reports each new message as
/// `Event::Arrived`, with the connection's state as `Event::Watch`. Runs on its own thread until a `Stop`. The poll
/// covers an account whose watcher has stopped or is between connections.
pub fn watch(transport: Arc<dyn Transport>, account: Account, password: String, tx: Sender<Event>, wake: Option<UnixStream>) {
    std::thread::spawn(move || loop {
        let end = watch_inbox(transport.as_ref(), &account, &password, &tx, &wake);
        report(&tx, &wake, Event::Watch { account: account.id.clone(), live: false });
        match end {
            WatchEnd::Stop => return,
            WatchEnd::Retry => std::thread::sleep(WATCH_RETRY),
        }
    });
}

fn watch_inbox(transport: &dyn Transport, account: &Account, password: &str, tx: &Sender<Event>, wake: &Option<UnixStream>) -> WatchEnd {
    let Ok(server) = accounts::incoming(account) else { return WatchEnd::Stop };
    let Ok(mut session) = session::connect(transport, server) else { return WatchEnd::Retry };
    if session.login(&account.username, password).is_err() {
        return WatchEnd::Stop;
    }
    if session.select(INBOX).is_err() {
        return WatchEnd::Retry;
    }
    match session.capabilities() {
        Ok(names) if names.iter().any(|name| name.eq_ignore_ascii_case("IDLE")) => {}
        Ok(_) => return WatchEnd::Stop,
        Err(_) => return WatchEnd::Retry,
    }
    report(tx, wake, Event::Watch { account: account.id.clone(), live: true });
    loop {
        if session.idle().is_err() {
            return WatchEnd::Retry;
        }
        report(tx, wake, Event::Arrived { account: account.id.clone() });
    }
}

/// Queues a watcher's event and wakes the window's loop, as [`spawn`] does for a job.
fn report(tx: &Sender<Event>, wake: &Option<UnixStream>, event: Event) {
    if tx.send(event).is_ok() {
        if let Some(mut wake) = wake.as_ref().and_then(|w| w.try_clone().ok()) {
            let _ = wake.write(&[1]);
        }
    }
}
