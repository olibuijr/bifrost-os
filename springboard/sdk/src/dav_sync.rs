//! Internet Accounts sync for the apps that read them: Calendar reads the CalDAV accounts with the Calendars service on,
//! Contacts reads the CardDAV accounts with the Contacts service on. An app holds one [`Sync`] and calls it from its
//! lifecycle:
//!
//! - `poll` from `tick`: returns a finished sync ([`Done`]), and starts the next account when one is due. An account
//!   syncs when the app starts and then every [`EVERY_MS`];
//! - `reply` from `service_reply`: takes the password the passwords store answered with (the sync asks for it through
//!   [`passwords::fill`](crate::passwords::fill)). Returns true when the answer was that password.
//!
//! The network walk runs on a background thread ([`dav_client`](crate::dav_client)); the app imports its result on
//! the next `poll`. The password never reaches the account list or a log.

use crate::controls::internet_accounts::{self as shared, Account, Kind, Service};
use crate::dav_client;
use crate::passwords;
use crate::url::URLRequest;
use crate::Ctx;
use bifrost_core::json::Json;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{SystemTime, UNIX_EPOCH};

/// How often an account syncs while the app runs, in the app's clock (milliseconds).
pub const EVERY_MS: f64 = 15.0 * 60.0 * 1000.0;
/// Calendar events are read this many days before and after now.
pub const DAYS_BEFORE: i64 = 31;
pub const DAYS_AFTER: i64 = 366;

/// One finished sync: the account's name and what it read. Calendar gets iCalendar texts, Contacts vCard texts.
#[derive(Debug)]
pub struct Done {
    pub account: String,
    pub result: Result<Vec<String>, String>,
}

/// Seconds since the Unix epoch, from the system clock.
pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The first account that switches `service` on and is a DAV account of the kind that service reads.
pub fn first_account(all: &[Account], service: Service) -> Option<Account> {
    let wanted = match service {
        Service::Calendars => Kind::CalDav,
        Service::Contacts => Kind::CardDav,
        _ => return None,
    };
    shared::serving(all, service).into_iter().find(|a| a.kind == wanted).cloned()
}

/// One local edit that waits to reach the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    /// Create (`etag` is `None`) or replace the calendar object or card at `url`. `etag` is the version the edit was
    /// made on, so a replace only lands when the server still has that version.
    Put { url: String, body: String, etag: Option<String> },
    /// Delete the item at `url`, only when it still has `etag`.
    Delete { url: String, etag: Option<String> },
}

/// The local edits not yet on the server, sent in the order they were made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outbox {
    queue: std::collections::VecDeque<Write>,
}

/// What a [`Outbox::flush`] did. Writes that landed leave the outbox; a write the server refused is dropped and listed
/// so the app can re-sync that item; a conflict (the server has a newer version) is dropped and listed too, because
/// the server's version must be read before the edit is made again. A transport failure keeps the rest queued.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flush {
    pub written: usize,
    /// URLs of writes that hit a conflict (412), in order.
    pub conflicts: Vec<String>,
    /// URLs and status codes of writes the server refused.
    pub refused: Vec<(String, u16)>,
    /// The transport error that stopped the flush, when one did. The writes after it stay queued.
    pub stopped: Option<String>,
}

impl Outbox {
    pub fn new() -> Outbox {
        Outbox::default()
    }

    /// Queues an edit at the end.
    pub fn push(&mut self, write: Write) {
        self.queue.push_back(write);
    }

    /// The number of edits still waiting.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Sends the queued edits in order with the DAV write calls. `auth` is the Basic authorisation header value and
    /// `kind` picks the content type of a PUT.
    pub fn flush(&mut self, send: &mut dav_client::SendEtag<'_>, auth: &str, kind: &Kind) -> Flush {
        use crate::dav_client::{Precondition, WriteError};
        let mut out = Flush::default();
        while let Some(write) = self.queue.front().cloned() {
            let (url, result) = match &write {
                Write::Put { url, body, etag } => {
                    let precondition = match etag {
                        Some(tag) => Precondition::Match(tag.as_str()),
                        None => Precondition::Create,
                    };
                    (url, dav_client::put_resource(send, url, auth, kind, body, precondition).map(|_| ()))
                }
                Write::Delete { url, etag } => (url, dav_client::delete_resource(send, url, auth, etag.as_deref())),
            };
            let url = url.clone();
            self.queue.pop_front();
            match result {
                Ok(()) => out.written += 1,
                Err(WriteError::Conflict) => out.conflicts.push(url),
                Err(WriteError::Refused(status)) => out.refused.push((url, status)),
                Err(WriteError::Transport(e)) => {
                    // the failed write stays queued at the front, so the next flush retries it
                    self.queue.push_front(write);
                    out.stopped = Some(e);
                    break;
                }
            }
        }
        out
    }
}

/// The sync state of one app for one service.
pub struct Sync {
    service: Service,
    /// The account whose password was asked for: request id, account and the time the sync is for.
    asking: Option<(u32, Account, i64)>,
    /// A sync on a background thread, with the account's name.
    running: Option<(Receiver<Result<Vec<String>, String>>, String)>,
    /// A result that is ready for the next `poll` (a missing password is reported this way).
    ready: Option<Done>,
    /// The app clock when the last sync started.
    last: Option<f64>,
}

impl Sync {
    pub fn new(service: Service) -> Sync {
        Sync { service, asking: None, running: None, ready: None, last: None }
    }

    /// Call from `tick` with the app clock `t` (milliseconds) and the Unix time `now`. Returns a finished sync, if one
    /// just ended, and starts the next one when it is due.
    pub fn poll(&mut self, ctx: &Ctx, t: f64, now: i64) -> Option<Done> {
        let done = self.collect();
        let due = self.last.map_or(true, |last| t - last >= EVERY_MS);
        if due && self.asking.is_none() && self.running.is_none() && self.ready.is_none() {
            self.start(ctx, t, now);
        }
        done
    }

    /// Asks for a sync on the next `poll`, not waiting for the interval (View > Refresh Calendars). A sync that is
    /// already running finishes first and the next one starts after it.
    pub fn refresh(&mut self) {
        self.last = None;
    }

    /// Call from `service_reply`. True when `request` was the password this sync asked for.
    pub fn reply(&mut self, request: u32, result: &Result<Json, String>) -> bool {
        match &self.asking {
            Some((id, _, _)) if *id == request => {}
            _ => return false,
        }
        let Some((_, account, now)) = self.asking.take() else { return false };
        match passwords::decode_password(result) {
            Ok(password) => self.spawn(account, password, now),
            Err(_) => {
                self.ready = Some(Done { account: account.name.clone(), result: Err(format!("No password is saved for {} in Passwords.", account.username)) })
            }
        }
        true
    }

    fn collect(&mut self) -> Option<Done> {
        if let Some(done) = self.ready.take() {
            return Some(done);
        }
        let outcome = {
            let (rx, _) = self.running.as_ref()?;
            rx.try_recv()
        };
        let name = self.running.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
        match outcome {
            Ok(result) => {
                self.running = None;
                Some(Done { account: name, result })
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.running = None;
                Some(Done { account: name, result: Err("The sync stopped before it finished.".into()) })
            }
        }
    }

    fn start(&mut self, ctx: &Ctx, t: f64, now: i64) {
        let Some(path) = shared::store_path() else { return };
        let Ok(all) = shared::load(&path) else { return };
        let Some(account) = first_account(&all, self.service) else { return };
        self.last = Some(t);
        let id = passwords::fill(ctx, &account.password_site(), &account.username);
        self.asking = Some((id, account, now));
    }

    fn spawn(&mut self, account: Account, password: String, now: i64) {
        let (tx, rx) = channel();
        let name = account.name.clone();
        let service = self.service;
        let base = account.url.clone();
        let auth = dav_client::basic_auth(&account.username, &password);
        std::thread::spawn(move || {
            let mut send = |req: &URLRequest| dav_client::session_send(req);
            let result = match service {
                Service::Contacts => dav_client::card_texts(&mut send, &base, &auth),
                _ => dav_client::calendar_texts(&mut send, &base, &auth, now - DAYS_BEFORE * 86_400, now + DAYS_AFTER * 86_400),
            };
            let _ = tx.send(result);
        });
        self.running = Some((rx, name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dav(kind: Kind, id: &str, on: bool) -> Account {
        let mut a = Account::new(kind, id, id, "olibuijr");
        a.url = "https://dav.example.com/".into();
        if !on {
            for s in Service::ALL {
                a.set_service(s, false);
            }
        }
        a
    }

    #[test]
    fn calendars_read_caldav_and_contacts_read_carddav() {
        let all = [dav(Kind::CardDav, "book", true), dav(Kind::CalDav, "cal", true)];
        assert_eq!(first_account(&all, Service::Calendars).unwrap().id, "cal");
        assert_eq!(first_account(&all, Service::Contacts).unwrap().id, "book");
        assert!(first_account(&all, Service::Mail).is_none());
    }

    #[test]
    fn an_account_with_its_service_off_is_skipped() {
        let all = [dav(Kind::CalDav, "off", false), dav(Kind::CalDav, "on", true)];
        assert_eq!(first_account(&all, Service::Calendars).unwrap().id, "on");
        assert!(first_account(&[dav(Kind::CalDav, "off", false)], Service::Calendars).is_none());
    }

    #[test]
    fn a_reply_for_another_request_is_not_the_password() {
        let mut s = Sync::new(Service::Contacts);
        assert!(!s.reply(7, &Err("x".into())));
    }

    #[test]
    fn refresh_makes_the_next_sync_due_at_once() {
        let mut s = Sync::new(Service::Calendars);
        s.last = Some(1_000_000.0);
        s.refresh();
        assert!(s.last.is_none(), "the interval no longer holds the next sync back");
    }

    #[test]
    fn the_unix_clock_is_after_2020() {
        assert!(unix_now() > 1_577_836_800);
    }

    fn put(url: &str, etag: Option<&str>) -> Write {
        Write::Put { url: url.into(), body: "BEGIN:VCARD\r\nEND:VCARD".into(), etag: etag.map(str::to_string) }
    }

    #[test]
    fn a_flush_sends_the_edits_in_order_and_empties_the_outbox() {
        let mut ob = Outbox::new();
        ob.push(put("https://dav.example.com/a.vcf", None));
        ob.push(put("https://dav.example.com/b.vcf", Some("\"v1\"")));
        ob.push(Write::Delete { url: "https://dav.example.com/c.vcf".into(), etag: Some("\"v2\"".into()) });
        let mut seen: Vec<(String, String, Option<String>)> = Vec::new();
        let mut send = |req: &URLRequest| -> Result<(u16, Option<String>), String> {
            seen.push((req.method.clone(), req.url.clone(), req.headers.iter().find(|h| h.0 == "If-Match" || h.0 == "If-None-Match").map(|h| h.1.clone())));
            Ok((200, None))
        };
        let out = ob.flush(&mut send, "Basic x", &Kind::CardDav);
        assert_eq!(out, Flush { written: 3, ..Flush::default() });
        assert!(ob.is_empty());
        assert_eq!(
            seen,
            vec![
                ("PUT".to_string(), "https://dav.example.com/a.vcf".to_string(), Some("*".to_string())),
                ("PUT".to_string(), "https://dav.example.com/b.vcf".to_string(), Some("\"v1\"".to_string())),
                ("DELETE".to_string(), "https://dav.example.com/c.vcf".to_string(), Some("\"v2\"".to_string())),
            ]
        );
    }

    #[test]
    fn a_conflict_or_refusal_is_listed_and_the_rest_still_goes() {
        let mut ob = Outbox::new();
        ob.push(put("https://dav.example.com/a.vcf", Some("\"old\"")));
        ob.push(put("https://dav.example.com/b.vcf", None));
        ob.push(Write::Delete { url: "https://dav.example.com/c.vcf".into(), etag: None });
        let mut answers = vec![412u16, 403, 204].into_iter();
        let mut send = |_: &URLRequest| -> Result<(u16, Option<String>), String> { Ok((answers.next().unwrap(), None)) };
        let out = ob.flush(&mut send, "Basic x", &Kind::CardDav);
        assert_eq!(out.written, 1);
        assert_eq!(out.conflicts, vec!["https://dav.example.com/a.vcf".to_string()]);
        assert_eq!(out.refused, vec![("https://dav.example.com/b.vcf".to_string(), 403)]);
        assert!(out.stopped.is_none());
        assert!(ob.is_empty());
    }

    #[test]
    fn a_transport_failure_keeps_the_failed_write_and_the_rest_queued() {
        let mut ob = Outbox::new();
        ob.push(put("https://dav.example.com/a.vcf", None));
        ob.push(put("https://dav.example.com/b.vcf", None));
        let mut send = |_: &URLRequest| Err::<(u16, Option<String>), String>("offline".into());
        let out = ob.flush(&mut send, "Basic x", &Kind::CardDav);
        assert_eq!(out, Flush { stopped: Some("offline".into()), ..Flush::default() });
        assert_eq!(ob.len(), 2, "nothing was sent, so both edits wait for the next flush");
        assert_eq!(ob.queue.front(), Some(&put("https://dav.example.com/a.vcf", None)));
    }
}
