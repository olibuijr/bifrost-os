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
}
