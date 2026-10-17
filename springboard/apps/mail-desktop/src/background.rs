//! The mail check that runs while Mail has no window: the shell's background task scheduler starts the app hidden
//! (`manifest.toml` `[background] tasks`), the app asks Passwords for each mail account's password, lists the newest inbox
//! messages of every account in parallel, and posts a notification for each unread message that arrived since the last
//! check. Then it reports the task done and asks for the next one.
//!
//! The baseline (the highest UID announced for each account) is kept in a file in the app's data directory, so a check
//! in a new process knows what the last one saw. The first check of an account only sets its baseline: nothing that was
//! already in the inbox is announced. A message the person has read, or that sits in a muted conversation, raises no
//! notification, the same as in the window (`state::Mail::apply_listing`). The notification id is the window's
//! (`mail-<account>-<uid>`), so a message the open window announces too is shown once.
//!
//! The shell allows a refresh task 30 s and a few runs a day (`services/background.rs`), and none in Low Power Mode or
//! with Background App Refresh switched off; while the window is open its IDLE watchers announce new mail instead.

use crate::accounts;
use crate::flags::Flag;
use crate::message::Summary;
use crate::mute;
use crate::session::Listed;
use crate::state::{arrivals, Notice, INBOX};
use crate::sync::{self, Event};
use bifrost_app::background_tasks::Request;
use bifrost_app::controls::internet_accounts::Account;
use bifrost_app::passwords;
use bifrost_core::json::Json;
use springboard::deildu::http::Transport;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// The task identifier the manifest declares.
pub const TASK: &str = "is.olibuijr.mail.desktop.refresh";
/// Seconds between a check and the earliest start of the next.
pub const EVERY_SECS: u64 = 900;
/// The baseline file in the app's data directory.
pub const FILE: &str = "mail-baseline.txt";
/// A check gives up this long after it starts, inside the 30 s the shell allows a refresh.
pub const LIMIT_MS: f64 = 24_000.0;
/// How often the app looks for finished jobs while a check runs.
pub const POLL_MS: f64 = 200.0;

/// The request for the next check.
pub fn next_request(now_unix: u64) -> Request {
    Request::refresh(TASK).earliest(now_unix + EVERY_SECS).network()
}

/// The highest UID announced for each account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Baseline(BTreeMap<String, u32>);

impl Baseline {
    /// Reads the file: one `account TAB uid` line each. A missing file is an empty baseline, and a bad line is skipped.
    pub fn load(path: &Path) -> Baseline {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        Baseline(text.lines().filter_map(|l| l.split_once('\t')).filter_map(|(id, uid)| Some((id.to_string(), uid.trim().parse().ok()?))).collect())
    }

    /// Writes the file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text: String = self.0.iter().filter(|(id, _)| !id.contains(['\t', '\r', '\n'])).map(|(id, uid)| format!("{id}\t{uid}\n")).collect();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn get(&self, account: &str) -> Option<u32> {
        self.0.get(account).copied()
    }

    /// Takes one inbox listing of `account`: the notices for the unread messages newer than the baseline, and the
    /// baseline moves up to the newest UID listed. With no baseline yet the listing only sets it.
    pub fn apply(&mut self, account: &str, listed: &[Listed], muted: &HashSet<String>) -> Vec<Notice> {
        let notices = arrivals(self.get(account), listed)
            .into_iter()
            .filter(|m| !m.flags.contains(Flag::Seen))
            .filter_map(|m| {
                let summary = Summary::from_headers(&m.header);
                (!muted.contains(&mute::key(&summary))).then(|| Notice { account: account.to_string(), uid: m.uid, from: summary.from, subject: summary.subject })
            })
            .collect();
        if let Some(top) = listed.iter().map(|m| m.uid).max() {
            let entry = self.0.entry(account.to_string()).or_insert(top);
            *entry = (*entry).max(top);
        }
        notices
    }
}

/// One check: the accounts, their passwords as Passwords answers, the listing jobs, and what they found.
pub struct Check {
    identifier: String,
    accounts: Vec<Account>,
    /// password requests sent to Passwords: request id -> index in `accounts`
    asked: HashMap<u32, usize>,
    /// listing jobs started and not yet answered
    jobs: usize,
    baseline: Baseline,
    muted: HashSet<String>,
    transport: Arc<dyn Transport>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    /// when the check began (ms), set by the first `finished` call
    started: Option<f64>,
    /// accounts that could not be read, for the log
    pub problems: Vec<String>,
}

impl Check {
    /// A check for task `identifier` over `accounts` (the mail accounts with Mail switched on).
    pub fn new(identifier: &str, accounts: Vec<Account>, baseline: Baseline, muted: HashSet<String>, transport: Arc<dyn Transport>) -> Check {
        let (tx, rx) = channel();
        Check { identifier: identifier.to_string(), accounts, asked: HashMap::new(), jobs: 0, baseline, muted, transport, tx, rx, started: None, problems: Vec::new() }
    }

    /// The accounts to read: those with an incoming server, the site and user name Passwords keeps the password under,
    /// by index.
    pub fn logins(&self) -> Vec<(usize, String, String)> {
        self.accounts.iter().enumerate().filter(|(_, a)| a.imap.is_some()).map(|(i, a)| (i, a.password_site(), a.username.clone())).collect()
    }

    /// Records that request `request` asks for the password of account `index`.
    pub fn expect(&mut self, request: u32, index: usize) {
        self.asked.insert(request, index);
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Whether `request` is one of this check's password requests.
    pub fn owns(&self, request: u32) -> bool {
        self.asked.contains_key(&request)
    }

    /// Passwords answered for `request`. With the password, the account's inbox is listed on a thread; without it the
    /// account is skipped. True when the request was this check's.
    pub fn password(&mut self, request: u32, result: &Result<Json, String>) -> bool {
        let Some(index) = self.asked.remove(&request) else { return false };
        let account = self.accounts[index].clone();
        match passwords::decode_password(result) {
            Ok(password) => {
                let transport = Arc::clone(&self.transport);
                self.jobs += 1;
                sync::spawn(self.tx.clone(), None, move || sync::list(transport.as_ref(), &account, &password, INBOX));
            }
            Err(e) => self.problems.push(format!("{}: no password ({e:?})", account.name)),
        }
        true
    }

    /// Takes the listings that have finished; returns the notices for the new unread mail in them.
    pub fn poll(&mut self) -> Vec<Notice> {
        let mut notices = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            if let Event::Listed { account, mailbox, result } = event {
                self.jobs = self.jobs.saturating_sub(1);
                match result {
                    Ok(listing) if mailbox.eq_ignore_ascii_case(INBOX) => notices.extend(self.baseline.apply(&account, &listing.messages, &self.muted)),
                    Ok(_) => {}
                    Err(e) => self.problems.push(format!("{account}: {e}")),
                }
            }
        }
        notices
    }

    /// Whether every password has been answered and every listing has finished, or the time is up at `t` (ms). The
    /// first call starts the clock.
    pub fn finished(&mut self, t: f64) -> bool {
        let started = *self.started.get_or_insert(t);
        (self.asked.is_empty() && self.jobs == 0) || t - started >= LIMIT_MS
    }

    /// The baseline as it stands, to save once the check is done.
    pub fn baseline(&self) -> &Baseline {
        &self.baseline
    }

    /// A listing result as if a job had produced it (tests).
    #[cfg(test)]
    pub fn inject(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    /// Counts a listing job the test injects an answer for (tests).
    #[cfg(test)]
    pub fn expect_job(&mut self) {
        self.jobs += 1;
    }
}

/// The mail accounts to check: those the shared list has with Mail switched on and an incoming server.
pub fn accounts_to_check() -> Vec<Account> {
    accounts::load_mail().unwrap_or_default().into_iter().filter(|a| a.imap.is_some()).collect()
}

/// What a notification for one message says, in the window's wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub thread: String,
}

/// The notification for one message. Its id is the window's, so a message announced twice is shown once.
pub fn notification(notice: &Notice) -> Banner {
    let who = notice.from.split('<').next().unwrap_or("").trim().trim_matches('"');
    let who = if who.is_empty() { notice.from.as_str() } else { who };
    Banner { id: format!("mail-{}-{}", notice.account, notice.uid), title: "New Mail".into(), subtitle: who.into(), body: notice.subject.clone(), thread: notice.account.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::Flags;
    use crate::sync::Listing;
    use bifrost_app::controls::internet_accounts::{Kind, Security, Server};
    use springboard::deildu::http::Stream;

    fn listed(uid: u32, seen: bool, subject: &str) -> Listed {
        let flags = if seen { Flags::empty().with(Flag::Seen, true) } else { Flags::empty() };
        Listed { uid, flags, header: format!("From: \"Gunna\" <gunna@example.is>\r\nSubject: {subject}\r\nMessage-ID: <{uid}@example.is>\r\n\r\n") }
    }

    fn account(id: &str) -> Account {
        let mut a = Account::new(Kind::Mail, id, id, "me");
        a.imap = Some(Server { host: "imap.example.is".into(), port: 993, security: Security::Tls });
        a
    }

    struct Offline;
    impl Transport for Offline {
        fn connect(&self, _: &str, _: u16, _: bool) -> std::io::Result<Box<dyn Stream>> {
            Err(std::io::Error::other("offline"))
        }
    }

    fn check(accounts: Vec<Account>, baseline: Baseline) -> Check {
        Check::new(TASK, accounts, baseline, HashSet::new(), Arc::new(Offline))
    }

    fn answer_with(password: &str) -> Result<Json, String> {
        let hex: String = password.bytes().map(|b| format!("{b:02x}")).collect();
        Ok(Json::Obj(vec![("status".into(), Json::Num(0.0)), ("value".into(), Json::Str(hex))]))
    }

    #[test]
    fn the_first_check_only_sets_the_baseline_and_later_ones_announce_unread_arrivals() {
        let mut b = Baseline::default();
        let none = HashSet::new();
        assert!(b.apply("work", &[listed(5, false, "old"), listed(7, false, "older")], &none).is_empty());
        assert_eq!(b.get("work"), Some(7));
        let n = b.apply("work", &[listed(9, false, "Fundur"), listed(8, true, "read already"), listed(7, false, "older")], &none);
        assert_eq!(n.len(), 1, "a read message and an old one raise nothing");
        assert_eq!((n[0].uid, n[0].subject.as_str(), n[0].account.as_str()), (9, "Fundur", "work"));
        assert_eq!(b.get("work"), Some(9));
        assert!(b.apply("work", &[listed(9, false, "Fundur")], &none).is_empty(), "announced once");
    }

    #[test]
    fn a_muted_conversation_raises_no_notification() {
        let mut b = Baseline::default();
        b.apply("work", &[listed(1, true, "x")], &HashSet::new());
        let muted_key = mute::key(&Summary::from_headers(&listed(2, false, "Quiet").header));
        let muted: HashSet<String> = [muted_key].into_iter().collect();
        assert!(b.apply("work", &[listed(2, false, "Quiet")], &muted).is_empty());
        assert_eq!(b.get("work"), Some(2), "the baseline still moves");
    }

    #[test]
    fn the_baseline_survives_a_restart_in_its_file() {
        let path = std::env::temp_dir().join(format!("bifrost-mail-baseline-{}", std::process::id())).join(FILE);
        assert_eq!(Baseline::load(&path), Baseline::default());
        let mut b = Baseline::default();
        b.apply("work", &[listed(4, false, "a")], &HashSet::new());
        b.apply("home", &[listed(11, false, "b")], &HashSet::new());
        b.save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "home\t11\nwork\t4\n");
        let again = Baseline::load(&path);
        assert_eq!(again, b);
        assert_eq!(again.clone().apply("work", &[listed(5, false, "c")], &HashSet::new()).len(), 1, "a new process knows what the last one saw");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_check_asks_passwords_lists_each_inbox_and_finishes() {
        let mut c = check(vec![account("work"), Account::new(Kind::Mail, "bare", "Bare", "me")], Baseline::default());
        assert_eq!(c.logins().iter().map(|l| l.0).collect::<Vec<_>>(), vec![0], "an account with no incoming server is not read");
        c.expect(40, 0);
        assert!(c.owns(40) && !c.owns(41));
        assert!(!c.finished(100.0), "the password is outstanding");
        assert!(!c.password(41, &answer_with("x")), "not this check's request");
        assert!(c.password(40, &answer_with("hunter2")));
        // the job runs offline and answers with a failure
        for _ in 0..200 {
            if c.finished(100.0) {
                break;
            }
            c.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(c.finished(100.0));
        assert!(c.problems.iter().any(|p| p.contains("work")), "{:?}", c.problems);
    }

    /// A server that greets, accepts the login and lists an inbox holding uid 3 (read) and uid 9 (unread).
    struct Canned;
    impl Transport for Canned {
        fn connect(&self, _: &str, _: u16, _: bool) -> std::io::Result<Box<dyn Stream>> {
            let (old, new) = ("Subject: Old\r\nFrom: A <a@x.is>\r\n\r\n", "Subject: Fundur\r\nFrom: Gunna <gunna@x.is>\r\n\r\n");
            let script = format!(
                "* OK ready\r\nA001 OK done\r\n* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\nA002 OK done\r\nA003 OK done\r\n* SEARCH 3 9\r\nA004 OK done\r\n\
                 * 1 FETCH (UID 3 FLAGS (\\Seen) BODY[HEADER] {{{}}}\r\n{old})\r\n* 2 FETCH (UID 9 FLAGS () BODY[HEADER] {{{}}}\r\n{new})\r\nA005 OK done\r\nA006 OK done\r\n",
                old.len(),
                new.len()
            );
            Ok(Box::new(crate::line::script::Script::new(&script)))
        }
    }

    #[test]
    fn a_check_over_a_server_announces_the_unread_arrival_once() {
        let mut baseline = Baseline::default();
        baseline.apply("work", &[listed(3, true, "x")], &HashSet::new());
        let mut c = Check::new(TASK, vec![account("work")], baseline, HashSet::new(), Arc::new(Canned));
        c.expect(40, 0);
        assert!(c.password(40, &answer_with("pw")));
        let mut notices = Vec::new();
        for _ in 0..400 {
            notices.extend(c.poll());
            if c.finished(0.0) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!((notices[0].uid, notices[0].subject.as_str(), notices[0].from.as_str()), (9, "Fundur", "Gunna <gunna@x.is>"));
        assert_eq!(c.baseline().get("work"), Some(9));
    }

    #[test]
    fn a_missing_password_skips_the_account_and_a_listing_yields_notices() {
        let mut c = check(vec![account("work"), account("home")], Baseline::default());
        c.expect(1, 0);
        c.expect(2, 1);
        c.password(1, &Err("denied".into()));
        assert!(c.problems[0].contains("work"));
        assert!(!c.finished(0.0));
        // home's listing arrives: the first one sets the baseline
        c.expect_job();
        c.asked.remove(&2);
        c.inject(Event::Listed { account: "home".into(), mailbox: "INBOX".into(), result: Ok(Listing { mailboxes: vec![], drafts: None, messages: vec![listed(3, false, "first")] }) });
        assert!(c.poll().is_empty());
        assert!(c.finished(0.0));
        // a later check in the same process sees a newer message
        c.expect_job();
        c.inject(Event::Listed { account: "home".into(), mailbox: "INBOX".into(), result: Ok(Listing { mailboxes: vec![], drafts: None, messages: vec![listed(4, false, "Boð"), listed(3, false, "first")] }) });
        let notices = c.poll();
        assert_eq!(notices.len(), 1);
        assert_eq!(notification(&notices[0]), Banner { id: "mail-home-4".into(), title: "New Mail".into(), subtitle: "Gunna".into(), body: "Boð".into(), thread: "home".into() });
        assert_eq!(c.baseline().get("home"), Some(4));
    }

    #[test]
    fn a_check_gives_up_when_the_time_is_up_and_the_next_one_is_asked_for() {
        let mut c = check(vec![account("work")], Baseline::default());
        c.expect(1, 0);
        assert!(!c.finished(5_000.0), "the first call starts the clock");
        assert!(!c.finished(5_000.0 + LIMIT_MS - 1.0));
        assert!(c.finished(5_000.0 + LIMIT_MS), "the shell ends the task at 30 s");
        let r = next_request(1_000);
        assert_eq!((r.identifier.as_str(), r.earliest_begin, r.requires_network), (TASK, Some(1_000 + EVERY_SECS), true));
    }
}
