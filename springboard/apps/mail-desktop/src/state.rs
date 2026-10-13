//! The mail window's state: the accounts and what is selected, the message rows of the open mailbox, the open message,
//! the sheets (account setup and compose) and the arrival baseline for new-mail notices. Nothing here touches the
//! network, the passwords or the window; `window.rs` runs the jobs and feeds their results in.

use crate::body::Attachment;
use crate::flags::{Flag, Flags};
use crate::message::{self, Draft, Summary};
use crate::search::{Doc, Query};
use crate::session::Listed;
use crate::threads::{self, Node};
use bifrost_app::controls::internet_accounts::{self as shared, Account, Kind, Security, Server};
use std::collections::HashMap;

pub const INBOX: &str = "INBOX";
/// One line of the list: a message, in the threaded view how many messages its conversation holds, and how far it is
/// indented under the conversation's first message (0 when flat).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub row: Row,
    pub thread: usize,
    pub depth: usize,
}

/// The mailbox names Archive and Trash are looked up under, in order (providers name them differently).
pub const ARCHIVE_NAMES: [&str; 4] = ["Archive", "Archives", "All Mail", "[Gmail]/All Mail"];
pub const TRASH_NAMES: [&str; 4] = ["Trash", "Deleted Items", "Deleted Messages", "[Gmail]/Trash"];
/// The IMAP ports that start TLS at once; the other standard ports start with STARTTLS.
const IMAP_TLS_PORT: u16 = 993;
const SMTP_TLS_PORT: u16 = 465;

/// One message in the list: what the list and the reader need, from its header block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub uid: u32,
    pub flags: Flags,
    pub summary: Summary,
}

/// One line of a sheet. `hint` shows while the value is empty; `secret` hides the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub hint: &'static str,
    pub value: String,
    pub secret: bool,
}

impl Field {
    pub fn new(label: &'static str, hint: &'static str, value: &str) -> Field {
        Field { label, hint, value: value.to_string(), secret: false }
    }
}

/// The fields of a sheet and the one that takes typed text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub fields: Vec<Field>,
    pub focus: usize,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Form {
        Form { fields, focus: 0 }
    }

    pub fn type_text(&mut self, text: &str) {
        if let Some(field) = self.fields.get_mut(self.focus) {
            field.value.push_str(text);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(field) = self.fields.get_mut(self.focus) {
            field.value.pop();
        }
    }

    /// Tab: the next field, wrapping to the first.
    pub fn next_field(&mut self) {
        if !self.fields.is_empty() {
            self.focus = (self.focus + 1) % self.fields.len();
        }
    }

    pub fn focus_on(&mut self, index: usize) {
        if index < self.fields.len() {
            self.focus = index;
        }
    }

    pub fn value(&self, label: &str) -> &str {
        self.fields.iter().find(|f| f.label == label).map_or("", |f| f.value.as_str())
    }
}

/// A new message: the form and, for a reply, the threading headers of the message answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compose {
    pub form: Form,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// The sheet over the window, if one is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sheet {
    Setup(Form),
    Compose(Compose),
    /// The Find sheet: one field, the search text.
    Search(Form),
}

/// A message that arrived since the last listing of the inbox, for a notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub account: String,
    pub uid: u32,
    pub from: String,
    pub subject: String,
}

/// The messages of a listing that are newer than `seen`, the highest UID already known. With nothing known yet (the
/// first listing) nothing is new: that listing only sets the baseline.
pub fn arrivals(seen: Option<u32>, listed: &[Listed]) -> Vec<&Listed> {
    match seen {
        None => Vec::new(),
        Some(seen) => listed.iter().filter(|m| m.uid > seen).collect(),
    }
}

/// Listed messages as rows, the newest (highest UID) first.
fn newest_first(messages: Vec<Listed>) -> Vec<Row> {
    let mut rows: Vec<Row> = messages.into_iter().map(|m| Row { uid: m.uid, flags: m.flags, summary: Summary::from_headers(&m.header) }).collect();
    rows.sort_by(|a, b| b.uid.cmp(&a.uid));
    rows
}

pub struct Mail {
    pub accounts: Vec<Account>,
    /// The index of the account shown in the sidebar.
    pub account: usize,
    pub mailboxes: Vec<String>,
    pub mailbox: String,
    /// The messages of the shown mailbox, newest first.
    pub rows: Vec<Row>,
    /// The UID of the message open in the reader.
    pub open: Option<u32>,
    /// Message text by UID, for the shown mailbox only.
    pub bodies: HashMap<u32, String>,
    /// Passwords read from the Passwords group for this run. They are never written to the shared account list.
    pub passwords: HashMap<String, String>,
    /// The highest inbox UID seen per account id. Absent until the first inbox listing of the account.
    pub baseline: HashMap<String, u32>,
    pub sheet: Option<Sheet>,
    pub status: String,
    /// The search text as applied. Empty shows every message.
    pub search: String,
    /// The threaded view: the list groups conversations together.
    pub threaded: bool,
    /// The files of each message loaded for the shown mailbox, by UID.
    pub attachments: HashMap<u32, Vec<Attachment>>,
    /// The messages the server found for the applied search in the shown mailbox (`SEARCH` on the server), newest
    /// first. None until the server answers; the list then shows the loaded rows that match.
    pub found: Option<Vec<Row>>,
}

impl Mail {
    pub fn new(accounts: Vec<Account>) -> Mail {
        Mail {
            accounts,
            account: 0,
            mailboxes: vec![INBOX.to_string()],
            mailbox: INBOX.to_string(),
            rows: Vec::new(),
            open: None,
            bodies: HashMap::new(),
            passwords: HashMap::new(),
            baseline: HashMap::new(),
            sheet: None,
            status: String::new(),
            search: String::new(),
            threaded: false,
            attachments: HashMap::new(),
            found: None,
        }
    }

    pub fn current(&self) -> Option<&Account> {
        self.accounts.get(self.account)
    }

    pub fn password(&self, account: &Account) -> Option<&str> {
        self.passwords.get(&account.id).map(String::as_str)
    }

    /// Shows another account: its inbox is selected and the list starts empty until its listing arrives.
    pub fn show_account(&mut self, index: usize) {
        if index >= self.accounts.len() {
            return;
        }
        self.account = index;
        self.mailboxes = vec![INBOX.to_string()];
        self.show_mailbox(INBOX);
    }

    pub fn show_mailbox(&mut self, name: &str) {
        self.mailbox = name.to_string();
        self.rows.clear();
        self.bodies.clear();
        self.attachments.clear();
        self.found = None;
        self.open = None;
    }

    /// Takes the result of a listing. Returns the notices for new inbox messages. The rows change only when the listing
    /// is for the shown account and mailbox.
    pub fn apply_listing(&mut self, account_id: &str, mailbox: &str, mailboxes: Vec<String>, messages: Vec<Listed>) -> Vec<Notice> {
        let mut notices = Vec::new();
        if mailbox.eq_ignore_ascii_case(INBOX) {
            let seen = self.baseline.get(account_id).copied();
            for m in arrivals(seen, &messages) {
                let summary = Summary::from_headers(&m.header);
                notices.push(Notice { account: account_id.to_string(), uid: m.uid, from: summary.from.clone(), subject: summary.subject.clone() });
            }
            if let Some(top) = messages.iter().map(|m| m.uid).max() {
                let entry = self.baseline.entry(account_id.to_string()).or_insert(top);
                *entry = (*entry).max(top);
            }
        }
        let shown = self.current().is_some_and(|a| a.id == account_id) && self.mailbox == mailbox;
        if shown {
            if !mailboxes.is_empty() {
                self.mailboxes = mailboxes;
            }
            self.rows = newest_first(messages);
        }
        notices
    }

    /// Takes the server's answer to the applied search. It is kept only while the same search is applied to the same
    /// account and mailbox; an answer for an older search is dropped.
    pub fn apply_found(&mut self, account_id: &str, mailbox: &str, query: &str, messages: Vec<Listed>) {
        let current = self.current().is_some_and(|a| a.id == account_id) && self.mailbox == mailbox && self.search == query;
        if current {
            self.found = Some(newest_first(messages));
        }
    }

    /// The row of a message the list shows: from the server's answer when a search is applied, else from the listing.
    pub fn row(&self, uid: u32) -> Option<&Row> {
        self.found.iter().flatten().chain(self.rows.iter()).find(|r| r.uid == uid)
    }

    /// Opens a message in the reader. Returns true when its text is not loaded yet.
    pub fn open_message(&mut self, uid: u32) -> bool {
        self.open = Some(uid);
        self.mark_seen_locally(uid);
        !self.bodies.contains_key(&uid)
    }

    pub fn mark_seen_locally(&mut self, uid: u32) {
        self.set_flag_locally(uid, Flag::Seen, true);
    }

    /// Sets a flag on a row at once, before the server answers.
    pub fn set_flag_locally(&mut self, uid: u32, flag: Flag, on: bool) {
        for row in self.rows.iter_mut().chain(self.found.iter_mut().flatten()).filter(|r| r.uid == uid) {
            row.flags = row.flags.with(flag, on);
        }
    }

    /// The flags of a row in the shown mailbox, when it is listed.
    pub fn flags_of(&self, uid: u32) -> Option<Flags> {
        self.row(uid).map(|r| r.flags)
    }

    /// Takes a message out of the shown list, because it moved to another mailbox. An open message is closed.
    pub fn remove_row(&mut self, uid: u32) {
        self.rows.retain(|r| r.uid != uid);
        if let Some(found) = &mut self.found {
            found.retain(|r| r.uid != uid);
        }
        self.bodies.remove(&uid);
        self.attachments.remove(&uid);
        if self.open == Some(uid) {
            self.open = None;
        }
    }

    /// The lines the list shows. Threaded, the matching messages are grouped by conversation (`threads::group`): each
    /// conversation is listed together, the newest conversation first, in tree order inside it (`threads::arrange`):
    /// a reply sits under the message it answers, indented by its depth.
    pub fn visible_lines(&self) -> Vec<Line> {
        let rows = self.visible_rows();
        if !self.threaded {
            return rows.into_iter().map(|row| Line { row, thread: 1, depth: 0 }).collect();
        }
        let nodes: Vec<Node> = rows
            .iter()
            .map(|r| Node {
                message_id: r.summary.message_id.clone(),
                in_reply_to: r.summary.in_reply_to.clone(),
                references: r.summary.references.clone(),
                subject: r.summary.subject.clone(),
            })
            .collect();
        let mut lines = Vec::with_capacity(rows.len());
        for group in threads::group(&nodes) {
            for (i, depth) in threads::arrange(&nodes, &group) {
                lines.push(Line { row: rows[i].clone(), thread: group.len(), depth });
            }
        }
        lines
    }

    /// The first of `candidates` the account has as a mailbox (names match without case), for Archive and Trash.
    pub fn find_mailbox(&self, candidates: &[&str]) -> Option<String> {
        candidates.iter().find_map(|c| self.mailboxes.iter().find(|m| m.eq_ignore_ascii_case(c)).cloned())
    }

    /// The text of a message arrived. It is kept only when it belongs to the shown account and mailbox.
    pub fn body_arrived(&mut self, account_id: &str, mailbox: &str, uid: u32, text: String) {
        if self.current().is_some_and(|a| a.id == account_id) && self.mailbox == mailbox {
            self.attachments.insert(uid, crate::body::attachments(&text));
            self.bodies.insert(uid, text);
        }
    }

    pub fn open_setup(&mut self) {
        let hint_email = "name@example.com";
        self.sheet = Some(Sheet::Setup(Form::new(vec![
            Field::new("Name", "Shown in the sidebar", ""),
            Field::new("Email", hint_email, ""),
            Field::new("User name", "Same as the email when empty", ""),
            Field { secret: true, ..Field::new("Password", "Stored in Passwords", "") },
            Field::new("Incoming server", "imap.<domain> when empty", ""),
            Field::new("Incoming port", "", &IMAP_TLS_PORT.to_string()),
            Field::new("Outgoing server", "smtp.<domain> when empty", ""),
            Field::new("Outgoing port", "", "587"),
        ])));
    }

    /// A compose sheet: empty for a new message, or the reply to `reply`, with the original's sender as recipient.
    /// The cursor goes to the recipient field when there is none yet, otherwise to the message body.
    pub fn open_compose(&mut self, reply: Option<&Summary>) {
        let from = self.current().map(|a| a.email.clone()).unwrap_or_default();
        let draft = match reply {
            Some(original) => message::reply_draft(original, &from),
            None => Draft { from, ..Draft::default() },
        };
        let mut form = Form::new(vec![
            Field::new("To", "name@example.com, other@example.com", &draft.to.join(", ")),
            Field::new("Subject", "", &draft.subject),
            Field::new("Message", "", &draft.body),
        ]);
        form.focus_on(if draft.to.is_empty() { 0 } else { 2 });
        self.sheet = Some(Sheet::Compose(Compose { form, in_reply_to: draft.in_reply_to, references: draft.references }));
    }

    /// The Find sheet, with the search applied so far.
    pub fn open_search(&mut self) {
        let text = self.search.clone();
        self.sheet = Some(Sheet::Search(Form::new(vec![Field::new("Search", "from:ana subject:fundur \"exact phrase\"", &text)])));
    }

    /// Applies a search text. Empty (or blank) shows every message again.
    pub fn apply_search(&mut self, text: &str) {
        self.search = text.trim().to_string();
        self.found = None;
    }

    /// The rows the list shows. With a search applied and the server's answer in, that answer (the server searched
    /// every message of the mailbox). Otherwise the loaded rows that match the search, newest first: bodies are searched
    /// when they are loaded, and a message whose body is not loaded matches on its headers only.
    pub fn visible_rows(&self) -> Vec<Row> {
        if let Some(found) = &self.found {
            return found.clone();
        }
        let query = Query::parse(&self.search);
        self.rows
            .iter()
            .filter(|row| {
                let body = self.bodies.get(&row.uid).map_or("", String::as_str);
                let doc = Doc {
                    from: row.summary.from.as_str(),
                    to: row.summary.to.as_str(),
                    subject: row.summary.subject.as_str(),
                    body,
                };
                query.matches(&doc)
            })
            .cloned()
            .collect()
    }

    pub fn close_sheet(&mut self) {
        self.sheet = None;
    }

    /// Checks the setup form and builds the mail account it describes, with its password. Hosts and ports fall back to
    /// the domain's `imap.` and `smtp.` names and the standard ports. `taken` is the whole shared list, so the new id
    /// is unique among every account.
    pub fn account_from_form(&self, form: &Form, taken: &[Account]) -> Result<(Account, String), String> {
        let email = form.value("Email").trim().to_string();
        if !shared::valid_address(&email) {
            return Err("Enter an email address, for example name@example.com".into());
        }
        let domain = email.rsplit_once('@').map_or("", |(_, d)| d).to_string();
        let password = form.value("Password").to_string();
        if password.is_empty() {
            return Err("Enter the password for this account".into());
        }
        let name = match form.value("Name").trim() {
            "" => email.clone(),
            name => name.to_string(),
        };
        let username = match form.value("User name").trim() {
            "" => email.clone(),
            user => user.to_string(),
        };
        let imap_port = port(form.value("Incoming port"), IMAP_TLS_PORT)?;
        let smtp_port = port(form.value("Outgoing port"), 587)?;
        let imap_host = host(form.value("Incoming server"), &format!("imap.{domain}"));
        let smtp_host = host(form.value("Outgoing server"), &format!("smtp.{domain}"));
        let mut account = Account::new(Kind::Mail, &account_id(&email, taken), &name, &username);
        account.email = email;
        account.imap = Some(Server { host: imap_host, port: imap_port, security: security(imap_port, IMAP_TLS_PORT) });
        account.smtp = Some(Server { host: smtp_host, port: smtp_port, security: security(smtp_port, SMTP_TLS_PORT) });
        account.validate().map_err(|e| e.to_string())?;
        Ok((account, password))
    }

    /// The draft a compose sheet describes, from the shown account. Recipients are split on commas and semicolons.
    pub fn draft_from(&self, compose: &Compose) -> Result<Draft, String> {
        let from = self.current().map(|a| a.email.clone()).ok_or("Add an account first")?;
        let to: Vec<String> = compose
            .form
            .value("To")
            .split([',', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if to.is_empty() {
            return Err("Add at least one recipient".into());
        }
        Ok(Draft {
            from,
            to,
            cc: Vec::new(),
            subject: compose.form.value("Subject").to_string(),
            body: compose.form.value("Message").to_string(),
            in_reply_to: compose.in_reply_to.clone(),
            references: compose.references.clone(),
        })
    }

    /// Adds an account the user set up, once it is in the shared list and its password is saved. It becomes the shown
    /// account.
    pub fn add_account(&mut self, account: Account, password: String) {
        self.passwords.insert(account.id.clone(), password);
        self.accounts.push(account);
        self.show_account(self.accounts.len() - 1);
    }
}

fn security(port: u16, tls_port: u16) -> Security {
    if port == tls_port {
        Security::Tls
    } else {
        Security::StartTls
    }
}

fn host(typed: &str, fallback: &str) -> String {
    match typed.trim() {
        "" => fallback.to_string(),
        h => h.to_string(),
    }
}

fn port(typed: &str, fallback: u16) -> Result<u16, String> {
    match typed.trim() {
        "" => Ok(fallback),
        p => p.parse::<u16>().ok().filter(|&n| n != 0).ok_or_else(|| format!("{p} is not a port number")),
    }
}

/// A stable account id from the address: lower-case letters, digits and dashes, unique among `taken`.
pub fn account_id(email: &str, taken: &[Account]) -> String {
    let mut base: String = email
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    base = base.trim_matches('-').to_string();
    if base.is_empty() {
        base = "account".to_string();
    }
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|a| a.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str) -> Account {
        let mut a = Account::new(Kind::Mail, id, "Work", "olibuijr");
        a.email = "olibuijr@example.com".into();
        a.imap = Some(Server { host: "imap.example.com".into(), port: 993, security: Security::Tls });
        a.smtp = Some(Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls });
        a
    }

    fn listed(uid: u32, subject: &str) -> Listed {
        Listed { uid, flags: Flags::empty(), header: format!("Subject: {subject}\r\nFrom: Ana <ana@example.com>\r\n\r\n") }
    }

    fn setup(fields: &[(&str, &str)]) -> Form {
        let mut form = Mail::new(vec![]).sheet_form_for_test();
        for (label, value) in fields {
            let field = form.fields.iter_mut().find(|f| f.label == *label).unwrap();
            field.value = value.to_string();
        }
        form
    }

    impl Mail {
        fn sheet_form_for_test(mut self) -> Form {
            self.open_setup();
            match self.sheet {
                Some(Sheet::Setup(form)) => form,
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn the_first_listing_only_sets_the_baseline() {
        let mut mail = Mail::new(vec![account("work")]);
        let notices = mail.apply_listing("work", INBOX, vec![], vec![listed(4, "old"), listed(9, "older")]);
        assert!(notices.is_empty());
        assert_eq!(mail.baseline.get("work"), Some(&9));
    }

    #[test]
    fn later_listings_notify_for_newer_inbox_messages_only() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old")]);
        let notices = mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old"), listed(12, "Fundur")]);
        assert_eq!(notices, vec![Notice { account: "work".into(), uid: 12, from: "Ana <ana@example.com>".into(), subject: "Fundur".into() }]);
        assert_eq!(mail.baseline.get("work"), Some(&12));
    }

    #[test]
    fn rows_show_the_newest_first_for_the_shown_mailbox_only() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", "Sent", vec!["INBOX".into(), "Sent".into()], vec![listed(1, "sent")]);
        assert!(mail.rows.is_empty(), "another mailbox does not fill the list");
        mail.apply_listing("work", INBOX, vec!["INBOX".into(), "Sent".into()], vec![listed(2, "a"), listed(7, "b")]);
        let uids: Vec<u32> = mail.rows.iter().map(|r| r.uid).collect();
        assert_eq!(uids, vec![7, 2]);
        assert_eq!(mail.mailboxes, vec!["INBOX", "Sent"]);
    }

    #[test]
    fn opening_a_message_marks_it_read_and_asks_for_its_text_once() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "x")]);
        assert!(mail.open_message(3));
        assert!(mail.rows[0].flags.contains(Flag::Seen));
        mail.body_arrived("work", INBOX, 3, "text".into());
        assert!(!mail.open_message(3));
        mail.body_arrived("other", INBOX, 4, "not shown".into());
        assert!(!mail.bodies.contains_key(&4));
    }

    #[test]
    fn the_search_filters_the_rows_and_an_empty_search_shows_all() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "Fundur"), listed(4, "Reikningur")]);
        assert_eq!(mail.visible_rows().len(), 2);
        mail.apply_search("  subject:fundur ");
        assert_eq!(mail.search, "subject:fundur");
        assert_eq!(mail.visible_rows().iter().map(|r| r.uid).collect::<Vec<_>>(), vec![3]);
        mail.body_arrived("work", INBOX, 4, "Fundur um reikninginn".into());
        mail.apply_search("body:fundur");
        assert_eq!(mail.visible_rows().iter().map(|r| r.uid).collect::<Vec<_>>(), vec![4]);
        mail.apply_search("");
        assert_eq!(mail.visible_rows().len(), 2);
    }

    #[test]
    fn the_threaded_view_lists_each_conversation_together() {
        let mut mail = Mail::new(vec![account("work")]);
        let reply = "Message-ID: <b@x>\r\nIn-Reply-To: <a@x>\r\nSubject: Re: Plan\r\nFrom: Ana <ana@example.com>\r\n\r\n";
        let plan = "Message-ID: <a@x>\r\nSubject: Plan\r\nFrom: Ana <ana@example.com>\r\n\r\n";
        let lone = "Message-ID: <c@x>\r\nSubject: Lunch\r\nFrom: Bo <bo@example.com>\r\n\r\n";
        let listed_threaded = |uid, header: &str| Listed { uid, flags: Flags::empty(), header: header.to_string() };
        mail.apply_listing("work", INBOX, vec![], vec![listed_threaded(1, plan), listed_threaded(2, lone), listed_threaded(3, reply)]);
        assert_eq!(mail.visible_lines().iter().map(|l| l.row.uid).collect::<Vec<_>>(), vec![3, 2, 1], "flat, newest first");
        assert!(mail.visible_lines().iter().all(|l| l.thread == 1));
        mail.threaded = true;
        let lines = mail.visible_lines();
        assert_eq!(lines.iter().map(|l| l.row.uid).collect::<Vec<_>>(), vec![1, 3, 2], "the reply sits under its parent");
        assert_eq!(lines.iter().map(|l| l.thread).collect::<Vec<_>>(), vec![2, 2, 1]);
        assert_eq!(lines.iter().map(|l| l.depth).collect::<Vec<_>>(), vec![0, 1, 0]);
    }

    #[test]
    fn a_server_search_answer_replaces_the_rows_only_for_the_same_search() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "Fundur"), listed(4, "Reikningur")]);
        mail.apply_search("subject:reikn");
        assert_eq!(mail.visible_rows().iter().map(|r| r.uid).collect::<Vec<_>>(), vec![4], "the loaded rows until the server answers");
        mail.apply_found("work", INBOX, "subject:other", vec![listed(9, "x")]);
        assert_eq!(mail.found, None, "an answer for another search is dropped");
        mail.apply_found("work", INBOX, "subject:reikn", vec![listed(2, "old"), listed(9, "Reikningur 2")]);
        assert_eq!(mail.visible_rows().iter().map(|r| r.uid).collect::<Vec<_>>(), vec![9, 2]);
        assert!(mail.row(9).is_some() && mail.row(3).is_some());
        mail.apply_search("");
        assert_eq!(mail.visible_rows().len(), 2);
        assert_eq!(mail.found, None);
    }

    #[test]
    fn a_loaded_message_keeps_its_files_until_the_mailbox_changes() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "Skjöl")]);
        let raw = "Content-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nSee attached\r\n--B\r\nContent-Type: text/csv; name=\"tolur.csv\"\r\nContent-Disposition: attachment; filename=\"tolur.csv\"\r\n\r\na,b\r\n--B--\r\n";
        mail.body_arrived("work", INBOX, 3, raw.to_string());
        let files = mail.attachments.get(&3).expect("the files of the loaded message");
        assert_eq!(files.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["tolur.csv"]);
        mail.show_mailbox("Sent");
        assert!(mail.attachments.is_empty());
    }

    #[test]
    fn the_find_sheet_starts_from_the_applied_search() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_search("bo");
        mail.open_search();
        match &mail.sheet {
            Some(Sheet::Search(form)) => assert_eq!(form.value("Search"), "bo"),
            other => panic!("expected the Find sheet, got {other:?}"),
        }
    }

    #[test]
    fn flags_change_at_once_and_a_moved_message_leaves_the_list() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "a"), listed(4, "b")]);
        mail.open_message(3);
        mail.set_flag_locally(3, Flag::Flagged, true);
        assert!(mail.flags_of(3).is_some_and(|f| f.contains(Flag::Flagged)));
        mail.remove_row(3);
        assert_eq!(mail.open, None, "the open message closes when it moves");
        assert_eq!(mail.rows.iter().map(|r| r.uid).collect::<Vec<_>>(), vec![4]);
        assert_eq!(mail.flags_of(3), None);
    }

    #[test]
    fn archive_and_trash_are_found_under_the_names_providers_use() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.mailboxes = vec!["INBOX".into(), "[Gmail]/All Mail".into(), "Trash".into()];
        assert_eq!(mail.find_mailbox(&ARCHIVE_NAMES).as_deref(), Some("[Gmail]/All Mail"));
        assert_eq!(mail.find_mailbox(&TRASH_NAMES).as_deref(), Some("Trash"));
        mail.mailboxes = vec!["INBOX".into()];
        assert_eq!(mail.find_mailbox(&ARCHIVE_NAMES), None);
        mail.mailboxes = vec!["archive".into()];
        assert_eq!(mail.find_mailbox(&ARCHIVE_NAMES).as_deref(), Some("archive"));
    }

    #[test]
    fn switching_mailboxes_clears_the_open_message() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_message(3);
        mail.show_mailbox("Sent");
        assert_eq!(mail.open, None);
        assert_eq!(mail.mailbox, "Sent");
    }

    #[test]
    fn a_setup_form_builds_an_account_with_defaults() {
        let mail = Mail::new(vec![]);
        let form = setup(&[("Email", "ana@example.com"), ("Password", "secret")]);
        let (account, password) = mail.account_from_form(&form, &[]).unwrap();
        assert_eq!(password, "secret");
        assert_eq!(account.kind, Kind::Mail);
        assert_eq!(account.id, "ana-example-com");
        assert_eq!(account.username, "ana@example.com");
        assert_eq!(account.imap, Some(Server { host: "imap.example.com".into(), port: 993, security: Security::Tls }));
        assert_eq!(account.smtp, Some(Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls }));
        assert_eq!(account.password_site(), "imap.example.com");
    }

    #[test]
    fn a_setup_form_needs_an_address_and_a_password() {
        let mail = Mail::new(vec![]);
        assert!(mail.account_from_form(&setup(&[("Email", "nope"), ("Password", "x")]), &[]).is_err());
        assert!(mail.account_from_form(&setup(&[("Email", "ana@example.com")]), &[]).is_err());
        assert!(mail.account_from_form(&setup(&[("Email", "ana@example.com"), ("Password", "x"), ("Incoming port", "0")]), &[]).is_err());
    }

    #[test]
    fn plain_imap_ports_start_with_starttls() {
        let mail = Mail::new(vec![]);
        let form = setup(&[("Email", "ana@example.com"), ("Password", "p"), ("Incoming port", "143")]);
        let (account, _) = mail.account_from_form(&form, &[]).unwrap();
        assert_eq!(account.imap.map(|s| s.security), Some(Security::StartTls));
    }

    #[test]
    fn a_new_account_id_avoids_every_account_in_the_shared_list() {
        let mail = Mail::new(vec![]);
        let shared_list = vec![account("ana-example-com")];
        let form = setup(&[("Email", "ana@example.com"), ("Password", "p")]);
        let (account, _) = mail.account_from_form(&form, &shared_list).unwrap();
        assert_eq!(account.id, "ana-example-com-2");
    }

    #[test]
    fn account_ids_are_unique() {
        let taken = vec![account("ana-example-com")];
        assert_eq!(account_id("ana@example.com", &taken), "ana-example-com-2");
        assert_eq!(account_id("Ana+x@Example.com", &[]), "ana-x-example-com");
    }

    #[test]
    fn a_compose_sheet_becomes_a_draft_with_its_recipients() {
        let mail = Mail::new(vec![account("work")]);
        let form = Form::new(vec![
            Field::new("To", "", "ana@example.com; bo@example.com,  "),
            Field::new("Subject", "", "Hi"),
            Field::new("Message", "", "Body"),
        ]);
        let draft = mail.draft_from(&Compose { form, in_reply_to: None, references: vec![] }).unwrap();
        assert_eq!(draft.to, vec!["ana@example.com", "bo@example.com"]);
        assert_eq!(draft.from, "olibuijr@example.com");
        let empty = Form::new(vec![Field::new("To", "", " ")]);
        assert!(mail.draft_from(&Compose { form: empty, in_reply_to: None, references: vec![] }).is_err());
    }

    #[test]
    fn a_reply_sheet_carries_the_threading_headers() {
        let mut mail = Mail::new(vec![account("work")]);
        let original = Summary::from_headers("Message-ID: <abc@x>\r\nSubject: Plan\r\nFrom: Ana <ana@example.com>\r\n\r\n");
        mail.open_compose(Some(&original));
        match &mail.sheet {
            Some(Sheet::Compose(c)) => {
                assert_eq!(c.form.value("To"), "ana@example.com");
                assert_eq!(c.form.value("Subject"), "Re: Plan");
                assert_eq!(c.in_reply_to.as_deref(), Some("<abc@x>"));
            }
            other => panic!("expected a compose sheet, got {other:?}"),
        }
    }

    #[test]
    fn arrivals_need_a_baseline() {
        let listed_now = vec![listed(5, "a")];
        assert!(arrivals(None, &listed_now).is_empty());
        assert_eq!(arrivals(Some(4), &listed_now).len(), 1);
        assert!(arrivals(Some(5), &listed_now).is_empty());
    }

    #[test]
    fn form_editing_moves_and_types() {
        let mut form = Form::new(vec![Field::new("A", "", ""), Field::new("B", "", "")]);
        form.type_text("hi");
        form.backspace();
        form.type_text("yo");
        form.next_field();
        form.type_text("x");
        assert_eq!(form.value("A"), "hyo");
        assert_eq!(form.value("B"), "x");
        form.next_field();
        assert_eq!(form.focus, 0);
    }
}
