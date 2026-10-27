use super::*;

fn account(id: &str) -> Account {
    let mut a = Account::new(Kind::Mail, id, "Work", "olibuijr");
    a.email = "olibuijr@example.com".into();
    a.imap = Some(Server { host: "imap.example.com".into(), port: 993, security: Security::Tls });
    a.smtp = Some(Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls });
    a
}

#[test]
fn sent_addresses_are_suggested_most_recent_first() {
    let mut mail = Mail::new(vec![account("work")]);
    assert!(mail.recipients.is_empty());
    mail.recipients.record("Ana@Example.com", "");
    mail.recipients.record("bo@example.com", "");
    mail.recipients.record("ana@example.com", "");
    let got: Vec<String> = mail.recipients.suggest("", 10).into_iter().map(|r| r.address).collect();
    assert_eq!(got, vec!["ana@example.com", "bo@example.com"]);
    assert_eq!(mail.recipients.suggest("bo", 10).len(), 1);
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
        // the tests set the server fields too
        self.show_manual(None);
        match self.sheet {
            Some(Sheet::Setup(form)) => form,
            _ => unreachable!(),
        }
    }
}

/// Select All in an Add Account field: the old text is selected as a whole, and typing, Backspace or Cut replaces or removes it
/// (the report: Cmd+A did nothing, so the old text stayed when the field was retyped).
#[test]
fn select_all_makes_typing_backspace_and_cut_act_on_the_whole_field() {
    let mut form = setup(&[]);
    let at = form.fields.iter().position(|f| !f.choice && !f.secret).expect("a text field");
    form.focus_on(at);
    assert!(!form.select_all(), "an empty field has nothing to select");
    form.fields[at].value = "imap.old.example".into();
    assert!(!form.selected());
    assert!(form.select_all());
    assert!(form.selected());
    assert_eq!(form.selected_text(), Some("imap.old.example"));
    form.type_text("mail.example.com");
    assert_eq!(form.fields[at].value, "mail.example.com", "typing replaces the selection");
    assert!(!form.selected());
    form.type_text("!");
    assert_eq!(form.fields[at].value, "mail.example.com!", "the next typing appends");
    // Backspace removes the selection, not one character
    form.select_all();
    form.backspace();
    assert_eq!(form.fields[at].value, "");
    // Cut takes the text out and hands it over; nothing selected, nothing cut
    form.fields[at].value = "secret words".into();
    assert_eq!(form.cut(), None);
    form.select_all();
    assert_eq!(form.cut().as_deref(), Some("secret words"));
    assert_eq!(form.fields[at].value, "");
    // moving to another field ends the selection, and it does not come back with the focus
    form.fields[at].value = "abc".into();
    form.select_all();
    form.next_field();
    form.focus_on(at);
    assert!(!form.selected());
    form.select_all();
    form.deselect();
    assert!(!form.selected(), "a press in the field puts the caret at the end");
}

#[test]
fn a_password_is_selected_and_replaced_but_never_copied_or_cut() {
    let mut form = setup(&[]);
    let at = form.fields.iter().position(|f| f.secret).expect("a password field");
    form.focus_on(at);
    form.fields[at].value = "hunter2".into();
    assert!(form.select_all());
    assert_eq!(form.selected_text(), None, "Copy has nothing to copy");
    assert_eq!(form.cut(), None, "Cut is refused");
    assert_eq!(form.fields[at].value, "hunter2");
    form.type_text("new");
    assert_eq!(form.fields[at].value, "new", "typing still replaces the selected password");
}

#[test]
fn option_backspace_deletes_a_word_and_cmd_backspace_the_field() {
    let mut form = setup(&[]);
    let at = form.fields.iter().position(|f| !f.choice && !f.secret).expect("a text field");
    form.focus_on(at);
    for (before, after) in [("smtp.example.com", "smtp.example."), ("hello world  ", "hello "), ("abc.", "abc"), ("abc", ""), ("", ""), ("a b", "a ")] {
        form.fields[at].value = before.into();
        form.delete_word();
        assert_eq!(form.fields[at].value, after, "Option+Backspace on {before:?}");
    }
    form.fields[at].value = "one two".into();
    form.select_all();
    form.delete_word();
    assert_eq!(form.fields[at].value, "", "with a selection it deletes the selection");
    form.fields[at].value = "one two".into();
    form.delete_to_start();
    assert_eq!(form.fields[at].value, "", "Cmd+Backspace clears everything before the caret");
}

#[test]
fn the_first_listing_only_sets_the_baseline() {
    let mut mail = Mail::new(vec![account("work")]);
    let notices = mail.apply_listing("work", INBOX, vec![], vec![listed(4, "old"), listed(9, "older")]);
    assert!(notices.is_empty());
    assert_eq!(mail.baseline.get("work"), Some(&9));
}

#[test]
fn an_empty_inbox_sets_the_baseline_so_the_first_message_is_new() {
    let mut mail = Mail::new(vec![account("work")]);
    assert!(mail.apply_listing("work", INBOX, vec![], vec![]).is_empty());
    assert_eq!(mail.baseline.get("work"), Some(&0));
    let notices = mail.apply_listing("work", INBOX, vec![], vec![listed(1, "first")]);
    assert_eq!(notices.len(), 1);
}

#[test]
fn a_new_uidvalidity_starts_the_baseline_over() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.apply_listing_of("work", INBOX, Some(7), vec![], vec![listed(40, "old")]);
    assert_eq!(mail.baseline.get("work"), Some(&40));
    // the server renumbered the mailbox: nothing is announced for the messages already there, and the baseline is low again
    let notices = mail.apply_listing_of("work", INBOX, Some(8), vec![], vec![listed(1, "a"), listed(2, "b")]);
    assert!(notices.is_empty());
    assert_eq!((mail.baseline.get("work"), mail.validity.get("work")), (Some(&2), Some(&8)));
    let notices = mail.apply_listing_of("work", INBOX, Some(8), vec![], vec![listed(1, "a"), listed(2, "b"), listed(3, "c")]);
    assert_eq!(notices.len(), 1, "mail after the renumbering is announced");
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
fn mark_as_unread_on_the_open_message_holds_until_it_is_opened_again() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.apply_listing("work", INBOX, vec![], vec![listed(3, "x"), listed(4, "y")]);
    mail.open_message(3);
    mail.set_flag_locally(3, Flag::Seen, false);
    // showing it again (a plain click on it, Select All around it) does not mark it read again
    mail.open_message(3);
    mail.select_all_messages();
    assert!(!mail.flags_of(3).unwrap().contains(Flag::Seen), "it stays unread while it is open");
    // another message, then back: it is read again
    mail.open_message(4);
    assert!(mail.flags_of(4).unwrap().contains(Flag::Seen));
    mail.open_message(3);
    assert!(mail.flags_of(3).unwrap().contains(Flag::Seen));
}

#[test]
fn the_text_cache_is_bounded_by_bytes_too() {
    let mut mail = Mail::new(vec![account("work")]);
    let rows: Vec<Listed> = (1..=4).map(|u| listed(u, "x")).collect();
    mail.apply_listing("work", INBOX, vec![], rows);
    assert!(mail.open_message(1));
    let big = "x".repeat(MAX_CACHED_BYTES / 2 + 1);
    for uid in 1..=4 {
        mail.body_arrived("work", INBOX, uid, big.clone());
    }
    assert!(mail.cached_bytes() <= MAX_CACHED_BYTES + big.len(), "at most the open and the newest text beyond the cap");
    assert!(mail.bodies.contains_key(&1) && mail.bodies.contains_key(&4), "the open and the newest text stay");
    assert!(!mail.bodies.contains_key(&2));
}

#[test]
fn the_text_cache_stays_bounded_and_keeps_the_open_message() {
    let mut mail = Mail::new(vec![account("work")]);
    let rows: Vec<Listed> = (1..=(MAX_CACHED_BODIES as u32 + 40)).map(|u| listed(u, "x")).collect();
    mail.apply_listing("work", INBOX, vec![], rows);
    assert!(mail.open_message(1));
    for uid in 1..=(MAX_CACHED_BODIES as u32 + 40) {
        mail.body_arrived("work", INBOX, uid, format!("body {uid}"));
    }
    assert_eq!(mail.bodies.len(), MAX_CACHED_BODIES);
    assert_eq!(mail.attachments.len(), mail.bodies.len(), "attachments leave with their texts");
    assert!(mail.bodies.contains_key(&(MAX_CACHED_BODIES as u32 + 40)), "the newest text is kept");
    assert!(!mail.bodies.contains_key(&2), "the lowest UID went first");
    assert!(mail.open_message(2), "a dropped text is asked for again");
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
fn the_arrows_step_through_the_list_and_stop_at_its_ends() {
    let mut mail = Mail::new(vec![account("work")]);
    assert_eq!(mail.neighbour(true), None, "an empty list");
    mail.apply_listing("work", INBOX, vec![], vec![listed(1, "a"), listed(2, "b"), listed(3, "c")]);
    // newest first: 3, 2, 1
    assert_eq!(mail.neighbour(true), Some((0, 3)), "Down with nothing selected takes the first");
    assert_eq!(mail.neighbour(false), Some((2, 1)), "Up with nothing selected takes the last");
    mail.open_message(2);
    assert_eq!(mail.neighbour(true), Some((2, 1)));
    assert_eq!(mail.neighbour(false), Some((0, 3)));
    mail.open_message(1);
    assert_eq!(mail.neighbour(true), None, "the last message stays selected");
}

/// E2E-3: a click selects one message, Cmd-click toggles one, Shift-click selects the range from the anchor.
#[test]
fn cmd_click_toggles_and_shift_click_extends_from_the_anchor() {
    let mut mail = Mail::new(vec![account("work")]);
    // newest first: 5, 4, 3, 2, 1
    mail.apply_listing("work", INBOX, vec![], (1..=5).map(|uid| listed(uid, "m")).collect());
    mail.open_message(4);
    assert_eq!((mail.selected(), mail.selected_count()), (vec![4], 1));
    mail.toggle_selected(2);
    assert_eq!(mail.selected(), vec![4, 2]);
    assert_eq!(mail.open, Some(2), "the message clicked into the selection is shown");
    assert!(mail.is_selected(4) && mail.is_selected(2) && !mail.is_selected(3));
    assert_eq!(mail.selected_count(), 2);
    // Cmd-click on a selected message takes it out; the one shown before it comes back
    mail.toggle_selected(2);
    assert_eq!((mail.selected(), mail.open), (vec![4], Some(4)));
    // Shift-click from the anchor (2, the last Cmd-clicked) to 5 selects 5, 4, 3, 2 in list order
    mail.toggle_selected(2);
    mail.extend_selection(5);
    assert_eq!(mail.selected(), vec![5, 4, 3, 2]);
    assert_eq!(mail.open, Some(5));
    // another Shift-click redraws the range from the same anchor
    mail.extend_selection(1);
    assert_eq!(mail.selected(), vec![2, 1]);
    // a plain click ends the multiple selection
    mail.open_message(3);
    assert_eq!((mail.selected(), mail.selected_count()), (vec![3], 1));
    assert!(!mail.is_selected(1));
    // the last selected message Cmd-clicked away leaves nothing selected
    mail.toggle_selected(3);
    assert_eq!((mail.selected(), mail.open, mail.selected_count()), (vec![], None, 0));
}

#[test]
fn shift_arrows_grow_and_shrink_the_selection_from_the_anchor() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.apply_listing("work", INBOX, vec![], (1..=4).map(|uid| listed(uid, "m")).collect());
    // 4, 3, 2, 1: start on 3
    mail.open_message(3);
    assert_eq!(mail.extend_by_arrow(true).map(|(i, _)| i), Some(2));
    assert_eq!(mail.selected(), vec![3, 2]);
    mail.extend_by_arrow(true);
    assert_eq!(mail.selected(), vec![3, 2, 1]);
    assert_eq!(mail.extend_by_arrow(true), None, "the end of the list");
    mail.extend_by_arrow(false);
    mail.extend_by_arrow(false);
    assert_eq!((mail.selected(), mail.open), (vec![3], Some(3)), "back at the anchor");
    mail.extend_by_arrow(false);
    assert_eq!(mail.selected(), vec![4, 3], "past the anchor the range grows upward");
}

#[test]
fn select_all_and_moving_messages_keep_the_selection_consistent() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.apply_listing("work", INBOX, vec![], (1..=3).map(|uid| listed(uid, "m")).collect());
    mail.select_all_messages();
    assert_eq!(mail.selected(), vec![3, 2, 1]);
    assert_eq!(mail.open, Some(3), "with nothing open the first line is shown");
    mail.remove_row(3);
    assert_eq!((mail.selected(), mail.open), (vec![2, 1], Some(1)));
    mail.remove_row(2);
    assert_eq!((mail.selected(), mail.selected_count()), (vec![1], 1), "one left is the plain selection");
    mail.remove_row(1);
    assert_eq!((mail.selected(), mail.open), (vec![], None));
    // a listing that no longer has a selected message drops it; another mailbox starts with nothing selected
    mail.apply_listing("work", INBOX, vec![], (1..=3).map(|uid| listed(uid, "m")).collect());
    mail.select_all_messages();
    mail.apply_listing("work", INBOX, vec![], vec![listed(1, "m"), listed(2, "m")]);
    assert_eq!(mail.selected(), vec![2, 1]);
    mail.show_mailbox("Drafts");
    assert_eq!(mail.selected_count(), 0);
}

#[test]
fn each_mailbox_keeps_its_selection() {
    let mut mail = Mail::new(vec![account("work"), account("home")]);
    mail.apply_listing("work", INBOX, vec![], vec![listed(1, "a"), listed(2, "b")]);
    mail.open_message(2);
    mail.show_mailbox("Drafts");
    assert_eq!(mail.open, None);
    mail.apply_listing("work", "Drafts", vec![], vec![listed(7, "d")]);
    assert_eq!(mail.reopen, None, "Drafts had no selection");
    mail.show_mailbox(INBOX);
    assert_eq!(mail.reopen, Some(2), "the inbox's kept rows select its message again at once");
    mail.reopen = None;
    mail.apply_listing("work", INBOX, vec![], vec![listed(1, "a"), listed(2, "b")]);
    assert_eq!(mail.reopen, None, "the listing does not select it a second time");
    // another account's inbox is another mailbox
    mail.open_message(1);
    mail.show_account(1);
    assert_eq!(mail.reopen, None);
    mail.apply_listing("home", INBOX, vec![], vec![listed(1, "a")]);
    assert_eq!(mail.reopen, None);
    mail.show_account(0);
    assert_eq!(mail.reopen, Some(1));
    // a message that is gone is not selected: the inbox is listed without it while Drafts is shown
    mail.reopen = None;
    mail.show_mailbox("Drafts");
    mail.apply_listing("work", INBOX, vec![], vec![listed(2, "b")]);
    mail.show_mailbox(INBOX);
    assert_eq!(mail.reopen, None);
    mail.apply_listing("work", INBOX, vec![], vec![listed(2, "b")]);
    assert_eq!(mail.reopen, None);
}

#[test]
fn a_mailbox_listed_before_shows_its_rows_at_once() {
    let uids = |mail: &Mail| mail.rows.iter().map(|r| r.uid).collect::<Vec<_>>();
    let mut mail = Mail::new(vec![account("work"), account("home")]);
    mail.apply_listing("work", INBOX, vec![], vec![listed(1, "a"), listed(2, "b")]);
    mail.show_mailbox("Sent");
    assert!(mail.rows.is_empty(), "a mailbox never listed starts empty (Loading)");
    mail.apply_listing("work", "Sent", vec![], vec![listed(5, "s")]);
    // the inbox is listed in the background while Sent is shown: the newer listing is kept
    mail.apply_listing("work", INBOX, vec![], vec![listed(1, "a"), listed(2, "b"), listed(3, "c")]);
    assert_eq!(uids(&mail), vec![5], "a listing of another mailbox leaves the shown rows alone");
    mail.show_mailbox(INBOX);
    assert_eq!(uids(&mail), vec![3, 2, 1], "the inbox shows its last listing without waiting");
    mail.apply_listing("work", INBOX, vec![], vec![listed(2, "b"), listed(3, "c")]);
    assert_eq!(uids(&mail), vec![3, 2], "the next listing refreshes it in place");
    mail.show_mailbox("Sent");
    assert_eq!(uids(&mail), vec![5]);
    // each account keeps its own rows, and a forgotten account's are dropped
    mail.show_account(1);
    assert!(mail.rows.is_empty(), "the other account's inbox was never listed");
    mail.apply_listing("home", INBOX, vec![], vec![listed(9, "h")]);
    mail.show_account(0);
    assert_eq!(uids(&mail), vec![3, 2]);
    mail.show_account(1);
    assert_eq!(uids(&mail), vec![9]);
    mail.forget_account("work");
    assert_eq!(uids(&mail), vec![9], "the account left shows its own inbox");
    assert!(mail.listings.keys().all(|(account, _)| account == "home"));
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
fn muting_a_conversation_silences_its_notices_and_toggles_back() {
    let mut mail = Mail::new(vec![account("work")]);
    let root = "Message-ID: <a@x>\r\nSubject: Fundur\r\n\r\n";
    mail.apply_listing("work", INBOX, vec![], vec![Listed { uid: 1, flags: Flags::empty(), header: root.into() }]);
    assert_eq!(mail.toggle_mute(1), Some(true));
    assert!(mail.is_muted(1));
    let reply = "Message-ID: <b@x>\r\nReferences: <a@x>\r\nSubject: Re: Fundur\r\n\r\n";
    let other = "Message-ID: <z@x>\r\nSubject: Annad\r\n\r\n";
    let messages = vec![
        Listed { uid: 1, flags: Flags::empty(), header: root.into() },
        Listed { uid: 2, flags: Flags::empty(), header: reply.into() },
        Listed { uid: 3, flags: Flags::empty(), header: other.into() },
    ];
    let notices = mail.apply_listing("work", INBOX, vec![], messages);
    assert_eq!(notices.iter().map(|n| n.uid).collect::<Vec<_>>(), vec![3]);
    assert_eq!(mail.toggle_mute(1), Some(false));
    assert!(!mail.is_muted(1));
    assert_eq!(mail.toggle_mute(99), None, "an unlisted message cannot be muted");
}

#[test]
fn open_quickly_lists_matching_mailboxes_best_first() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.mailboxes = vec!["INBOX".into(), "Archive/Receipts".into(), "Sent".into(), "Rec".into(), "Drafts".into()];
    assert_eq!(mail.quickly_matches("rec"), vec!["Archive/Receipts".to_string(), "Rec".to_string()]);
    mail.mailboxes = vec!["Old receipts".into(), "Receipts".into()];
    assert_eq!(mail.quickly_matches("receipts"), vec!["Receipts".to_string(), "Old receipts".to_string()], "a name that starts with the text first");
    assert_eq!(mail.quickly_matches("").len(), 2);
    assert!(mail.quickly_matches("zzz").is_empty());
    mail.open_quickly();
    assert!(matches!(mail.sheet, Some(Sheet::Quickly(_))));
}

#[test]
fn a_flag_colour_shows_at_once_and_clearing_the_flag_clears_the_colour() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.apply_listing("work", INBOX, vec![], vec![listed(3, "x")]);
    mail.set_color_locally(3, Some(FlagColor::Purple));
    assert_eq!(mail.flags_of(3).and_then(Flags::color), Some(FlagColor::Purple));
    mail.set_color_locally(3, Some(FlagColor::Blue));
    assert_eq!(mail.flags_of(3).and_then(Flags::color), Some(FlagColor::Blue));
    mail.set_flag_locally(3, Flag::Flagged, false);
    assert_eq!(mail.flags_of(3), Some(Flags::empty()));
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

fn found() -> Discovered {
    Discovered {
        imap: Server { host: "mail.example.com".into(), port: 993, security: Security::Tls },
        smtp: Server { host: "mail.example.com".into(), port: 587, security: Security::StartTls },
        username: Some("ana".into()),
        trusted: true,
    }
}

#[test]
fn add_account_asks_for_a_name_an_address_and_a_password_only() {
    let mut mail = Mail::new(vec![]);
    mail.open_setup();
    let labels: Vec<&str> = mail.sheet.as_ref().unwrap().form().fields.iter().map(|f| f.label).collect();
    assert_eq!(labels, ["Name", "Email", "Password", "Configure Manually"]);
    assert!(!is_yes(mail.sheet.as_ref().unwrap().form().value(MANUAL)));
}

#[test]
fn configure_manually_shows_and_hides_the_server_fields() {
    let mut mail = Mail::new(vec![]);
    mail.open_setup();
    let at = mail.sheet.as_ref().unwrap().form().fields.iter().position(|f| f.label == MANUAL).unwrap();
    assert!(mail.sheet.as_mut().unwrap().form_mut().flip(at));
    mail.sync_manual();
    let form = mail.sheet.as_ref().unwrap().form().clone();
    let labels: Vec<&str> = form.fields.iter().map(|f| f.label).collect();
    assert_eq!(labels, ["Name", "Email", "Password", "Configure Manually", "User name", "Incoming server", "Incoming port", "Outgoing server", "Outgoing port"]);
    assert_eq!((form.value("Incoming port"), form.value("Outgoing port")), ("993", "587"));
    // typed values stay while the fields are shown, and a second sync changes nothing
    mail.sheet.as_mut().unwrap().form_mut().fields[5].value = "mail.example.com".into();
    mail.sync_manual();
    assert_eq!(mail.sheet.as_ref().unwrap().form().value("Incoming server"), "mail.example.com");
    mail.sheet.as_mut().unwrap().form_mut().focus = 8;
    assert!(mail.sheet.as_mut().unwrap().form_mut().flip(at));
    mail.sync_manual();
    let form = mail.sheet.as_ref().unwrap().form();
    assert_eq!(form.fields.len(), 4);
    assert_eq!(form.focus, 3, "the focus does not point past the fields");
}

#[test]
fn only_a_yes_or_no_field_flips() {
    let mut form = Form::new(vec![Field::new("Name", "", "x"), Field::yes_no("Choice", false)]);
    assert!(!form.flip(0) && !form.flip(9));
    assert!(form.flip(1) && form.value("Choice") == "yes");
    assert!(form.flip(1) && form.value("Choice") == "no");
}

#[test]
fn found_servers_fill_the_form_when_it_leaves_them_empty() {
    let mail = Mail::new(vec![]);
    let mut form = setup(&[("Email", "ana@example.com"), ("Password", "secret")]);
    form.fields.retain(|f| f.label != "Incoming port" && f.label != "Outgoing port");
    let (account, password) = mail.account_from_form_found(&form, &[], Some(&found())).unwrap();
    assert_eq!(password, "secret");
    assert_eq!(account.username, "ana");
    assert_eq!(account.imap, Some(Server { host: "mail.example.com".into(), port: 993, security: Security::Tls }));
    assert_eq!(account.smtp, Some(Server { host: "mail.example.com".into(), port: 587, security: Security::StartTls }));
    // without a result the old guess stands
    let (guessed, _) = mail.account_from_form(&form, &[]).unwrap();
    assert_eq!(guessed.imap.map(|s| s.host), Some("imap.example.com".to_string()));
    assert_eq!(guessed.username, "ana@example.com");
}

#[test]
fn what_is_typed_wins_over_what_was_found() {
    let mail = Mail::new(vec![]);
    let form = setup(&[
        ("Email", "ana@example.com"),
        ("Password", "p"),
        ("User name", "ana2"),
        ("Incoming server", "other.example.com"),
        ("Incoming port", "143"),
        ("Outgoing port", "465"),
    ]);
    let (account, _) = mail.account_from_form_found(&form, &[], Some(&found())).unwrap();
    assert_eq!(account.username, "ana2");
    assert_eq!(account.imap, Some(Server { host: "other.example.com".into(), port: 143, security: Security::StartTls }));
    assert_eq!(account.smtp, Some(Server { host: "mail.example.com".into(), port: 465, security: Security::Tls }));
}

#[test]
fn showing_the_found_servers_prefills_the_manual_fields() {
    let mut mail = Mail::new(vec![]);
    mail.open_setup();
    mail.show_manual(Some(&found()));
    let form = mail.sheet.as_ref().unwrap().form();
    assert_eq!(form.value(MANUAL), "yes");
    assert_eq!((form.value("User name"), form.value("Incoming server"), form.value("Incoming port")), ("ana", "mail.example.com", "993"));
    assert_eq!((form.value("Outgoing server"), form.value("Outgoing port")), ("mail.example.com", "587"));
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
    let draft = mail.draft_from(&Compose::new(form, None, vec![], Priority::Normal, vec![], String::new())).unwrap();
    assert_eq!(draft.to, vec!["ana@example.com", "bo@example.com"]);
    assert_eq!(draft.from, "olibuijr@example.com");
    let empty = Form::new(vec![Field::new("To", "", " ")]);
    assert!(mail.draft_from(&Compose::new(empty, None, vec![], Priority::Normal, vec![], String::new())).is_err());
}

fn person(given: &str, family: &str, address: &str) -> Contact {
    use bifrost_app::controls::contact_book::LabeledValue;
    Contact { given_name: given.into(), family_name: family.into(), email_addresses: vec![LabeledValue::new(None, address.to_string())], ..Contact::default() }
}

fn type_in_focus(mail: &mut Mail, text: &str) {
    mail.sheet.as_mut().unwrap().form_mut().type_text(text);
    mail.sync_recipient_field();
}

#[test]
fn the_recipient_tail_is_what_is_being_typed() {
    assert_eq!(split_last_recipient(""), ("", ""));
    assert_eq!(split_last_recipient("an"), ("", "an"));
    assert_eq!(split_last_recipient("ana@example.com, bo"), ("ana@example.com,", " bo"));
    assert_eq!(split_last_recipient("Ana <ana@example.com>, "), ("Ana <ana@example.com>,", " "));
    assert_eq!(split_last_recipient("\"Smith, Ana\" <a@x.is>"), ("", "\"Smith, Ana\" <a@x.is>"), "a comma in quotes does not separate");
}

#[test]
fn typing_in_to_suggests_contacts_and_recents_and_a_choice_fills_the_field() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.recipients.record("bob@example.com", "Bob Recent");
    mail.set_address_book(vec![person("Ana", "Lovelace", "ana@example.com"), person("Anders", "Holm", "anders@example.com")]);
    mail.open_compose(None);
    // an untouched To field offers the addresses used last
    assert_eq!(mail.recipient_suggestions().iter().map(|s| s.address.as_str()).collect::<Vec<_>>(), ["bob@example.com"]);
    type_in_focus(&mut mail, "an");
    let got: Vec<String> = mail.recipient_suggestions().iter().map(|s| s.address.clone()).collect();
    assert_eq!(got, ["ana@example.com", "anders@example.com"], "both contacts match at a word start, in display-name order");
    assert!(mail.move_recipient_highlight(true) && mail.move_recipient_highlight(true));
    assert_eq!(mail.recipient_field.highlighted(), Some(1));
    assert!(mail.move_recipient_highlight(false));
    assert_eq!(mail.recipient_field.highlighted(), Some(0));
    assert!(mail.accept_recipient(None));
    assert_eq!(compose_of(&mail).form.value("To"), "Ana Lovelace <ana@example.com>, ");
    assert!(mail.recipient_suggestions().is_empty(), "nothing is offered after a finished recipient");
    // the recipient is split off for the draft as an address with its name
    assert_eq!(message::split_addresses(compose_of(&mail).form.value("To")), ["Ana Lovelace <ana@example.com>"]);
    // the next recipient does not offer the one already in the field
    type_in_focus(&mut mail, "ana");
    assert!(mail.recipient_suggestions().is_empty());
    // "ana" is wiped and the next name typed whole (typing "ders" after it would spell "anaders")
    for _ in 0..3 {
        mail.sheet.as_mut().unwrap().form_mut().backspace();
    }
    type_in_focus(&mut mail, "anders");
    assert_eq!(mail.recipient_suggestions().len(), 1);
    assert!(mail.accept_recipient(Some(0)));
    assert_eq!(compose_of(&mail).form.value("To"), "Ana Lovelace <ana@example.com>, Anders Holm <anders@example.com>, ");
}

#[test]
fn the_message_field_and_a_closed_list_offer_nothing() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.set_address_book(vec![person("Ana", "Lovelace", "ana@example.com")]);
    mail.open_compose(None);
    type_in_focus(&mut mail, "ana");
    assert_eq!(mail.recipient_suggestions().len(), 1);
    assert!(mail.close_recipient_list(), "Escape closes the list first");
    assert!(mail.recipient_suggestions().is_empty());
    assert!(!mail.close_recipient_list(), "with no list showing Escape belongs to the sheet");
    assert!(!mail.accept_recipient(None), "nothing is highlighted");
    let sheet = mail.sheet.as_mut().unwrap().form_mut();
    sheet.focus_on(sheet.fields.len() - 1);
    mail.sync_recipient_field();
    assert!(mail.recipient_suggestions().is_empty(), "the message body has no suggestions");
}

#[test]
fn a_name_with_a_comma_is_kept_out_of_the_token() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.set_address_book(vec![person("Lovelace, Dr", "Ada", "ada@example.com")]);
    mail.open_compose(None);
    type_in_focus(&mut mail, "ada");
    assert!(mail.move_recipient_highlight(true) && mail.accept_recipient(None));
    assert_eq!(compose_of(&mail).form.value("To"), "ada@example.com, ", "the bare address, so the list still splits cleanly");
}

fn read_message(mail: &mut Mail, uid: u32, from: &str, to: &str, body: &str) {
    let summary = Summary { subject: "Plans".into(), from: from.into(), to: to.into(), ..Summary::default() };
    mail.rows.push(Row { uid, flags: Flags::default(), summary });
    mail.open = Some(uid);
    mail.bodies.insert(uid, format!("From: {from}\r\nSubject: Plans\r\n\r\n{body}\r\n"));
}

#[test]
fn a_read_message_donates_its_people_once_unless_the_setting_is_off() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.rows.push(Row { uid: 4, flags: Flags::default(), summary: Summary { from: "Bo <bo@example.com>".into(), to: "olibuijr@example.com, Cy <cy@example.com>".into(), ..Summary::default() } });
    mail.open = Some(4);
    assert!(mail.people_to_donate().is_empty(), "the text has not arrived");
    mail.bodies.insert(4, "Subject: x\r\n\r\nHi\r\n".into());
    let seen = mail.people_to_donate();
    let got: Vec<(&str, &str)> = seen.iter().map(|s| (s.name.as_str(), s.address.as_str())).collect();
    assert_eq!(got, [("Bo", "bo@example.com"), ("Cy", "cy@example.com")], "the person's own address is left out");
    assert!(mail.people_to_donate().is_empty(), "a message is donated once");
    mail.prefs.donate_people = false;
    mail.rows.push(Row { uid: 5, flags: Flags::default(), summary: Summary { from: "Di <di@example.com>".into(), ..Summary::default() } });
    mail.open = Some(5);
    mail.bodies.insert(5, "Subject: y\r\n\r\nHi\r\n".into());
    assert!(mail.people_to_donate().is_empty(), "donating is off");
}

#[test]
fn the_open_message_offers_what_it_mentions_and_a_turned_down_one_stays_down() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.rows.push(Row { uid: 9, flags: Flags::default(), summary: Summary { subject: "Plans".into(), ..Summary::default() } });
    mail.open = Some(9);
    mail.refresh_found_in("IS");
    assert!(mail.found_current().is_none(), "no text, no banner");
    mail.bodies.insert(9, "Subject: Plans\r\n\r\nLunch tomorrow at 1pm\r\nWrite to ana@example.com\r\n".into());
    mail.refresh_found_in("IS");
    assert_eq!(mail.found_current().map(|f| f.kind), Some(found_in::SuggestionKind::Event));
    assert_eq!(mail.found_current().map(|f| f.found_in()).as_deref(), Some("Found in Mail"));
    assert_eq!(mail.found_label().as_deref(), Some("1 of 2"));
    assert!(mail.found_step(true) && !mail.found_step(true), "the second page is the last");
    assert_eq!(mail.found_current().map(|f| f.title.as_str()), Some("ana@example.com"));
    assert!(mail.found_step(false));
    assert!(mail.found_dismiss(), "the event is turned down");
    assert_eq!(mail.found_label(), None, "one left needs no pager");
    match mail.found_accept() {
        Some(Action::AddContact { email, phone }) => assert_eq!((email.as_deref(), phone), (Some("ana@example.com"), None)),
        other => panic!("{other:?}"),
    }
    assert!(mail.found_current().is_none());
    // the same message again: nothing that was answered comes back
    mail.open = None;
    mail.refresh_found_in("IS");
    mail.open = Some(9);
    mail.refresh_found_in("IS");
    assert!(mail.found_current().is_none());
    // another message clears the banner of the first
    read_message(&mut mail, 10, "a@example.com", "", "Call me on ana@example.org");
    mail.refresh_found_in("IS");
    assert_eq!(mail.found_current().map(|f| f.title.as_str()), Some("ana@example.org"));
    mail.open = Some(9);
    assert!(mail.found_current().is_none(), "a banner belongs to its message");
}

fn compose_of(mail: &Mail) -> &Compose {
    match &mail.sheet {
        Some(Sheet::Compose(c)) => c,
        other => panic!("expected a compose sheet, got {other:?}"),
    }
}

fn labels(compose: &Compose) -> Vec<&'static str> {
    compose.form.fields.iter().map(|f| f.label).collect()
}

#[test]
fn cc_bcc_and_reply_to_fields_follow_the_view_menu_and_reach_the_draft() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.open_compose(None);
    assert_eq!(labels(compose_of(&mail)), vec!["To", "Cc", "Subject", "Message"]);
    mail.set_address_field("Bcc", true);
    mail.set_address_field("Reply-To", true);
    assert_eq!(labels(compose_of(&mail)), vec!["To", "Cc", "Bcc", "Reply-To", "Subject", "Message"]);
    if let Some(Sheet::Compose(c)) = &mut mail.sheet {
        c.form.focus_on(2);
        c.form.type_text("hidden@example.com");
        c.form.focus_on(0);
        c.form.type_text("ana@example.com");
        c.form.focus_on(3);
        c.form.type_text("back@example.com");
        c.priority = Priority::Low;
    }
    let draft = mail.draft_from(compose_of(&mail)).unwrap();
    assert_eq!((draft.bcc, draft.reply_to, draft.priority), (vec!["hidden@example.com".to_string()], vec!["back@example.com".to_string()], Priority::Low));
    mail.set_address_field("Cc", false);
    mail.set_address_field("Bcc", false);
    assert_eq!(labels(compose_of(&mail)), vec!["To", "Reply-To", "Subject", "Message"]);
    mail.open_compose(None);
    assert_eq!(labels(compose_of(&mail)), vec!["To", "Reply-To", "Subject", "Message"], "the choice is kept for the next message");
}

#[test]
fn reply_all_and_forward_open_compose_sheets() {
    let mut mail = Mail::new(vec![account("work")]);
    let original = Summary::from_headers("Message-ID: <a@x>\r\nSubject: Plan\r\nFrom: Ana <ana@example.com>\r\nTo: olibuijr@example.com, bo@example.com\r\nCc: cy@example.com\r\n\r\n");
    mail.open_reply_all(&original);
    let c = compose_of(&mail);
    assert_eq!(c.form.value("To"), "ana@example.com, bo@example.com");
    assert_eq!(c.form.value("Cc"), "cy@example.com");
    assert_eq!(c.in_reply_to.as_deref(), Some("<a@x>"));
    mail.open_forward(&original, "Hello");
    let c = compose_of(&mail);
    assert_eq!(c.form.value("Subject"), "Fwd: Plan");
    assert_eq!(c.form.focus, 0, "a forward starts at the recipient");
    assert!(c.form.value("Message").ends_with("Hello"));
    assert_eq!(c.in_reply_to, None);
}

fn mail_with_mixed_rows() -> Mail {
    let mut mail = Mail::new(vec![account("work")]);
    let header = |subject: &str, from: &str| format!("Subject: {subject}\r\nFrom: {from}\r\n\r\n");
    let rows = vec![
        Listed { uid: 1, flags: Flags::empty().with(Flag::Seen, true), header: header("Banana", "Cy <cy@example.com>") },
        Listed { uid: 2, flags: Flags::empty().with(Flag::Flagged, true), header: header("avocado", "Bo <bo@example.com>") },
        Listed { uid: 3, flags: Flags::empty().with(Flag::Seen, true).with(Flag::Answered, true), header: header("Cherry", "Ana <ana@example.com>") },
    ];
    mail.apply_listing("work", INBOX, vec![], rows);
    mail
}

fn uids(mail: &Mail) -> Vec<u32> {
    mail.visible_rows().iter().map(|r| r.uid).collect()
}

#[test]
fn the_list_sorts_by_key_and_direction() {
    let mut mail = mail_with_mixed_rows();
    assert_eq!(uids(&mail), vec![3, 2, 1], "newest first by default");
    mail.set_ascending(true);
    assert_eq!(uids(&mail), vec![1, 2, 3]);
    mail.sort_by(SortKey::Subject);
    assert_eq!(uids(&mail), vec![2, 1, 3], "A to Z without case");
    mail.set_ascending(false);
    assert_eq!(uids(&mail), vec![3, 1, 2]);
    mail.sort_by(SortKey::From);
    assert_eq!(uids(&mail), vec![3, 2, 1], "a new key starts A to Z: Ana, Bo, Cy");
    mail.set_ascending(false);
    assert_eq!(uids(&mail), vec![1, 2, 3]);
}

#[test]
fn flagged_and_unread_messages_can_be_gathered_by_sorting() {
    let mut mail = mail_with_mixed_rows();
    mail.sort_by(SortKey::Flagged);
    assert_eq!(uids(&mail), vec![2, 1, 3], "flagged first, the rest in the order they were: oldest first when ascending");
    mail.set_ascending(false);
    assert_eq!(uids(&mail), vec![3, 1, 2], "flagged last");
    mail.sort_by(SortKey::Unread);
    mail.set_ascending(true);
    assert_eq!(uids(&mail), vec![2, 1, 3], "the unread message first");
    assert_eq!(SortKey::ALL.len(), 6);
    assert_eq!(SortKey::Unread.label(), "Read Status");
}

#[test]
fn the_filter_keeps_unread_flagged_or_replied_messages() {
    let mut mail = mail_with_mixed_rows();
    mail.filter = Filter::Unread;
    assert_eq!(uids(&mail), vec![2]);
    mail.filter = Filter::Flagged;
    assert_eq!(uids(&mail), vec![2]);
    mail.filter = Filter::Replied;
    assert_eq!(uids(&mail), vec![3]);
    mail.filter = Filter::All;
    assert_eq!(uids(&mail).len(), 3);
}

#[test]
fn conversations_fold_and_unfold() {
    let mut mail = Mail::new(vec![account("work")]);
    let plan = "Message-ID: <a@x>\r\nSubject: Plan\r\nFrom: Ana <ana@example.com>\r\n\r\n";
    let reply = "Message-ID: <b@x>\r\nIn-Reply-To: <a@x>\r\nSubject: Re: Plan\r\nFrom: Bo <bo@example.com>\r\n\r\n";
    let lone = "Message-ID: <c@x>\r\nSubject: Lunch\r\nFrom: Cy <cy@example.com>\r\n\r\n";
    let rows = vec![(1, plan), (2, lone), (3, reply)].into_iter().map(|(uid, h)| Listed { uid, flags: Flags::empty(), header: h.to_string() }).collect();
    mail.apply_listing("work", INBOX, vec![], rows);
    mail.threaded = true;
    let shown = |m: &Mail| m.visible_lines().iter().map(|l| l.row.uid).collect::<Vec<_>>();
    assert_eq!(shown(&mail), vec![1, 3, 2]);
    mail.toggle_conversation(3);
    assert_eq!(shown(&mail), vec![1, 2], "only the first message of the folded conversation");
    assert_eq!(mail.visible_lines()[0].thread, 2, "the count still says two");
    assert!(mail.is_folded(3) && !mail.is_folded(2));
    mail.toggle_conversation(2);
    assert_eq!(shown(&mail), vec![1, 2], "a message alone has nothing to fold");
    mail.expand_all();
    assert_eq!(shown(&mail), vec![1, 3, 2]);
    mail.collapse_all();
    assert_eq!(shown(&mail), vec![1, 2]);
}

#[test]
fn mailbox_sheets_check_the_name() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.mailboxes = vec!["INBOX".into(), "Sent".into(), "Work".into()];
    mail.show_mailbox("Work");
    assert_eq!(mail.move_targets(), vec!["INBOX", "Sent"]);
    mail.open_new_mailbox();
    let Some(Sheet::Mailbox(sheet)) = mail.sheet.clone() else { panic!("expected the mailbox sheet") };
    assert!(mail.new_mailbox_name(&sheet.form, None).is_err(), "empty");
    let named = |name: &str| Form::new(vec![Field::new("Name", "", name)]);
    assert!(mail.new_mailbox_name(&named("sent"), None).is_err(), "taken, without case");
    assert_eq!(mail.new_mailbox_name(&named(" Bills "), None).unwrap(), "Bills");
    assert_eq!(mail.new_mailbox_name(&named("Work"), Some("Work")).unwrap(), "Work", "renaming to itself is allowed");
    assert!(mail.open_rename_mailbox().is_some());
    mail.show_mailbox(INBOX);
    assert!(mail.open_rename_mailbox().is_none(), "the inbox keeps its name");
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

fn file(name: &str, size: usize) -> Attachment {
    Attachment { name: name.into(), mime: "application/octet-stream".into(), bytes: vec![1; size] }
}

fn prompt_form(mail: &Mail) -> &Form {
    match &mail.sheet {
        Some(Sheet::Prompt(prompt)) => &prompt.form,
        other => panic!("expected a prompt, got {other:?}"),
    }
}

fn filled(mut form: Form, values: &[(&str, &str)]) -> Form {
    for (label, value) in values {
        form.fields.iter_mut().find(|f| f.label == *label).unwrap().value = value.to_string();
    }
    form
}

#[test]
fn a_message_takes_a_limited_number_of_files_and_bytes() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.open_compose(None);
    let Some(Sheet::Compose(compose)) = &mut mail.sheet else { panic!("no compose sheet") };
    for n in 0..MAX_ATTACHMENTS {
        compose.attach(file(&format!("f{n}"), 10)).unwrap();
    }
    assert_eq!(compose.attach(file("one more", 10)).unwrap_err(), format!("A message carries at most {MAX_ATTACHMENTS} files"));
    compose.detach(0);
    compose.detach(99);
    assert_eq!(compose.attachments.len(), MAX_ATTACHMENTS - 1);
    compose.attachments.clear();
    assert!(compose.attach(file("big", MAX_ATTACHMENT_BYTES)).is_ok());
    assert!(compose.attach(file("too much", 1)).unwrap_err().contains("MB a message may carry"));
}

#[test]
fn the_signature_follows_the_message_and_stays_out_of_the_text_field() {
    assert_eq!(with_signature("Hello", "Ana\nOlafsson"), "Hello\n\n-- \nAna\nOlafsson");
    assert_eq!(with_signature("Hello\n\n", "Ana\n"), "Hello\n\n-- \nAna");
    assert_eq!(with_signature("", "Ana"), "\n\n-- \nAna");
    assert_eq!(with_signature("Hello", "  \n"), "Hello");
    assert_eq!(with_signature_above_quote("Thanks\n\nOn Mon, Ana wrote:\n> hi", "Ana"), "Thanks\n\n-- \nAna\n\nOn Mon, Ana wrote:\n> hi");
    assert_eq!(with_signature_above_quote("Hello", "Ana"), "Hello\n\n-- \nAna", "no quote: the end");
    let mut mail = Mail::new(vec![account("work")]);
    mail.prefs.set_signature("work", "Ana");
    mail.open_compose(None);
    assert_eq!(compose_of(&mail).form.value("Message"), "", "the text field starts empty");
    let mut compose = compose_of(&mail).clone();
    compose.form.fields.iter_mut().find(|f| f.label == "To").unwrap().value = "bo@example.com".into();
    compose.form.fields.iter_mut().find(|f| f.label == "Message").unwrap().value = "Hi".into();
    assert_eq!(mail.draft_from(&compose).unwrap().body, "Hi\n\n-- \nAna");
}

#[test]
fn other_addresses_add_a_from_field_that_must_name_one_of_them() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.prefs.set_aliases("work", vec!["info@example.com".into()]);
    mail.open_compose(None);
    assert_eq!(labels(compose_of(&mail))[..2], ["From", "To"]);
    assert_eq!(compose_of(&mail).form.value("From"), "olibuijr@example.com");
    assert_eq!(compose_of(&mail).form.focus, 1, "the cursor starts at the recipient");
    assert_eq!(mail.from_choices(), vec!["olibuijr@example.com", "info@example.com"]);
    let Some(Sheet::Compose(compose)) = &mut mail.sheet else { panic!("no compose sheet") };
    compose.form.fields.iter_mut().find(|f| f.label == "To").unwrap().value = "bo@example.com".into();
    let choices = vec!["olibuijr@example.com".to_string(), "info@example.com".to_string()];
    compose.cycle_from(&choices);
    assert_eq!(compose.form.value("From"), "info@example.com");
    compose.cycle_from(&choices);
    assert_eq!(compose.form.value("From"), "olibuijr@example.com", "the choices wrap");
    compose.cycle_from(&choices);
    let chosen = compose.clone();
    assert_eq!(mail.draft_from(&chosen).unwrap().from, "info@example.com");
    let mut other = chosen;
    other.form.fields.iter_mut().find(|f| f.label == "From").unwrap().value = "someone@else.example".into();
    assert!(mail.draft_from(&other).unwrap_err().contains("is not one of this account's addresses"));
}

#[test]
fn send_later_adds_and_removes_its_field_without_losing_the_text() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.open_compose(None);
    mail.set_send_later(true);
    assert!(mail.send_later_shown());
    assert_eq!(labels(compose_of(&mail)), vec!["To", "Cc", "Subject", "Send in", "Message"]);
    if let Some(Sheet::Compose(c)) = &mut mail.sheet {
        c.form.focus_on(3);
        c.form.type_text("2h");
    }
    mail.set_send_later(false);
    assert!(!mail.send_later_shown());
    assert_eq!(labels(compose_of(&mail)), vec!["To", "Cc", "Subject", "Message"]);
}

#[test]
fn a_new_sheet_may_not_replace_a_message_being_written() {
    let mut mail = Mail::new(vec![account("work")]);
    assert!(mail.sheet_is_free());
    mail.open_compose(None);
    assert!(!mail.sheet_is_free());
    mail.close_sheet();
    mail.open_search();
    assert!(mail.sheet_is_free());
}

#[test]
fn the_attachments_filter_uses_loaded_files_and_the_message_type() {
    let mut mail = Mail::new(vec![account("work")]);
    let rows = vec![
        Listed { uid: 1, flags: Flags::empty(), header: "Subject: a\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n".into() },
        Listed { uid: 2, flags: Flags::empty(), header: "Subject: b\r\nContent-Type: text/plain\r\n\r\n".into() },
        Listed { uid: 3, flags: Flags::empty(), header: "Subject: c\r\n\r\n".into() },
    ];
    mail.apply_listing("work", INBOX, vec![], rows);
    mail.filter = Filter::Attachments;
    assert_eq!(uids(&mail), vec![1]);
    mail.attachments.insert(3, vec![file("late.pdf", 3)]);
    assert_eq!(uids(&mail), vec![3, 1], "a loaded message is judged by its files");
}

#[test]
fn the_category_keeps_only_that_kind_of_mail() {
    let mut mail = Mail::new(vec![account("work")]);
    let rows = vec![
        Listed { uid: 1, flags: Flags::empty(), header: "Subject: Lunch?\r\nFrom: Ana <ana@example.com>\r\n\r\n".into() },
        Listed { uid: 2, flags: Flags::empty(), header: "Subject: Your receipt\r\nFrom: shop@store.example\r\n\r\n".into() },
        Listed { uid: 3, flags: Flags::empty(), header: "Subject: Hello\r\nFrom: news@store.example\r\nList-Unsubscribe: <mailto:u@store.example>\r\n\r\n".into() },
    ];
    mail.apply_listing("work", INBOX, vec![], rows);
    assert_eq!(uids(&mail).len(), 3);
    mail.category = Some(Category::Primary);
    assert_eq!(uids(&mail), vec![1]);
    mail.category = Some(Category::Transactions);
    assert_eq!(uids(&mail), vec![2]);
    mail.category = Some(Category::Promotions);
    assert_eq!(uids(&mail), vec![3]);
    mail.category = Some(Category::Updates);
    assert!(uids(&mail).is_empty());
}

#[test]
fn a_smart_mailbox_applies_its_search_and_a_new_search_leaves_it() {
    let mut mail = mail_with_mixed_rows();
    mail.prefs.smart.push(SmartMailbox { name: "From Ana".into(), query: "from:ana".into() });
    assert!(mail.apply_smart(0));
    assert_eq!(mail.smart_active, Some(0));
    assert_eq!(uids(&mail), vec![3]);
    assert!(!mail.apply_smart(5));
    mail.apply_search("bo");
    assert_eq!(mail.smart_active, None);
}

#[test]
fn the_listing_remembers_which_messages_just_arrived_for_the_rules() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old")]);
    assert!(mail.arrived.is_empty());
    mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old"), listed(12, "Fundur"), listed(13, "Annað")]);
    let arrived: Vec<(&str, u32)> = mail.arrived.iter().map(|(a, r)| (a.as_str(), r.uid)).collect();
    assert_eq!(arrived, vec![("work", 12), ("work", 13)]);
    mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old"), listed(12, "Fundur"), listed(13, "Annað")]);
    assert!(mail.arrived.is_empty(), "only the newest listing's arrivals are kept");
}

#[test]
fn the_rule_form_checks_its_name_criteria_actions_and_mailbox() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.mailboxes = vec![INBOX.into(), "Receipts".into()];
    mail.open_rule();
    let form = prompt_form(&mail).clone();
    assert_eq!(mail.rule_from_form(&form).unwrap_err(), "Name the rule");
    let form = filled(form, &[("Name", "Shops")]);
    assert!(mail.rule_from_form(&form).unwrap_err().contains("something to look for"));
    let form = filled(form, &[("From contains", "shop.example")]);
    assert!(mail.rule_from_form(&form).unwrap_err().contains("Choose what the rule does"));
    let form = filled(form, &[("Move to mailbox", "receipts"), ("Mark as read", "yes")]);
    let rule = mail.rule_from_form(&form).unwrap();
    assert_eq!(rule.actions, vec![rules::Action::MoveTo("Receipts".into()), rules::Action::MarkRead], "the mailbox is spelled as the account has it");
    let missing = filled(form.clone(), &[("Move to mailbox", "Nowhere")]);
    assert_eq!(mail.rule_from_form(&missing).unwrap_err(), "There is no mailbox named Nowhere");
    mail.prefs.rules.push(rule);
    assert!(mail.rule_from_form(&form).unwrap_err().contains("already a rule named Shops"));
}

#[test]
fn the_smart_mailbox_form_needs_a_name_and_a_search() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.apply_search("from:ana");
    mail.open_smart_mailbox();
    let form = prompt_form(&mail).clone();
    assert_eq!(form.value("Search"), "from:ana", "it starts from the search applied now");
    assert_eq!(mail.smart_from_form(&form).unwrap_err(), "Name the smart mailbox");
    let named = filled(form.clone(), &[("Name", "Ana")]);
    assert_eq!(mail.smart_from_form(&named).unwrap(), SmartMailbox { name: "Ana".into(), query: "from:ana".into() });
    assert!(mail.smart_from_form(&filled(named.clone(), &[("Search", "  ")])).unwrap_err().contains("searches for"));
    mail.prefs.smart.push(SmartMailbox { name: "ana".into(), query: "x".into() });
    assert!(mail.smart_from_form(&named).unwrap_err().contains("already a smart mailbox"));
}

#[test]
fn mail_settings_set_the_signature_other_addresses_and_undo_delay() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.open_settings().unwrap();
    let form = prompt_form(&mail).clone();
    let set = filled(form.clone(), &[("Signature", "Ana\nOlafsson"), ("Other addresses", "Info <info@example.com>, info@example.com, ana@example.org"), ("Undo send (seconds)", "10")]);
    mail.apply_settings(&set).unwrap();
    assert_eq!(mail.prefs.signature("work"), "Ana\nOlafsson");
    assert_eq!(mail.prefs.aliases_of("work"), ["info@example.com", "ana@example.org"], "bare addresses, each once");
    assert_eq!(mail.prefs.undo_delay, 10);
    mail.open_settings().unwrap();
    assert_eq!(prompt_form(&mail).value("Other addresses"), "info@example.com, ana@example.org");
    assert_eq!(prompt_form(&mail).value("Undo send (seconds)"), "10");
    assert!(mail.apply_settings(&filled(form.clone(), &[("Other addresses", "not an address")])).is_err());
    assert!(mail.apply_settings(&filled(form.clone(), &[("Undo send (seconds)", "soon")])).unwrap_err().contains("number of seconds"));
    assert!(mail.apply_settings(&filled(form, &[("Undo send (seconds)", "31")])).unwrap_err().contains("at most 30"));
    assert!(Mail::new(vec![]).open_settings().is_err());
}

#[test]
fn a_reply_comes_from_the_alias_it_was_addressed_to_and_quotes_the_original() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.prefs.set_aliases("work", vec!["info@example.com".into()]);
    let original = Summary { from: "Ana <ana@example.org>".into(), to: "Info <INFO@example.com>".into(), subject: "Hi".into(), ..Summary::default() };
    assert_eq!(mail.sent_from(&original), "info@example.com");
    let own = Summary { to: "olibuijr@example.com".into(), ..original.clone() };
    assert_eq!(mail.sent_from(&own), "olibuijr@example.com", "the account's own address when it was addressed");
    mail.open_reply(&original, Some("first line\nsecond"));
    let form = match &mail.sheet {
        Some(Sheet::Compose(compose)) => compose.form.clone(),
        _ => panic!("a compose sheet"),
    };
    assert_eq!(form.value("From"), "info@example.com");
    assert_eq!(form.value("To"), "ana@example.org");
    assert!(form.value("Message").contains("Ana wrote:"), "{}", form.value("Message"));
    assert!(form.value("Message").contains("> first line\n> second"));
}

#[test]
fn reply_all_is_offered_only_when_someone_else_is_on_the_message() {
    let mail = Mail::new(vec![account("work")]);
    let alone = Summary { from: "Ana <ana@example.org>".into(), to: "olibuijr@example.com".into(), ..Summary::default() };
    assert!(!mail.has_other_recipients(&alone));
    let group = Summary { from: "Ana <ana@example.org>".into(), to: "olibuijr@example.com, Bo <bo@example.org>".into(), ..Summary::default() };
    assert!(mail.has_other_recipients(&group));
    let copied = Summary { from: "ana@example.org".into(), cc: "carl@example.org".into(), ..Summary::default() };
    assert!(mail.has_other_recipients(&copied));
}

#[test]
fn forwarding_keeps_the_files_of_the_original_within_the_message_limits() {
    let mut mail = Mail::new(vec![account("work")]);
    let original = Summary { subject: "Plan".into(), from: "Ana <ana@example.org>".into(), ..Summary::default() };
    let file = |name: &str, size: usize| Attachment { name: name.into(), mime: "application/pdf".into(), bytes: vec![0; size] };
    let files: Vec<Attachment> = (0..10).map(|n| file(&format!("f{n}.pdf"), 1000)).collect();
    mail.open_forward_with(&original, "text", files);
    match &mail.sheet {
        Some(Sheet::Compose(compose)) => assert_eq!(compose.attachments.len(), MAX_ATTACHMENTS),
        _ => panic!("a compose sheet"),
    }
    let kept = within_limits(vec![file("big", MAX_ATTACHMENT_BYTES), file("more", 1)]);
    assert_eq!(kept.len(), 1, "the second file would pass the size limit");
    assert!(within_limits(Vec::new()).is_empty());
}

#[test]
fn the_settings_toggle_quoting_and_plain_text() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.open_settings().unwrap();
    let form = prompt_form(&mail).clone();
    assert_eq!(form.value("Quote the original in replies"), "yes");
    assert_eq!(form.value("Plain text messages only"), "no");
    assert!(form.fields.iter().filter(|f| f.label.starts_with("Quote") || f.label.starts_with("Plain")).all(|f| f.choice));
    mail.apply_settings(&filled(form.clone(), &[("Quote the original in replies", "no"), ("Plain text messages only", "yes")])).unwrap();
    assert!(!mail.prefs.quote_replies && mail.prefs.plain_text);
    mail.open_settings().unwrap();
    assert_eq!(prompt_form(&mail).value("Plain text messages only"), "yes");
}

#[test]
fn editing_an_account_keeps_its_id_and_asks_for_the_password_only_when_it_moves() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.open_edit_account().unwrap();
    let form = prompt_form(&mail).clone();
    assert_eq!(form.value("Incoming server"), "imap.example.com");
    assert_eq!(form.value("Outgoing port"), "587");
    let old = mail.accounts[0].clone();
    let renamed = filled(form.clone(), &[("Name", "Office"), ("Outgoing server", "smtp.office.example")]);
    let (edited, password) = mail.edited_account(&renamed, &old).unwrap();
    assert_eq!((edited.id.as_str(), edited.name.as_str(), password), ("work", "Office", None));
    assert_eq!(edited.smtp.as_ref().map(|s| s.host.as_str()), Some("smtp.office.example"));
    assert_eq!(edited.services, old.services);
    let moved = filled(form.clone(), &[("Incoming server", "imap.other.example")]);
    assert!(mail.edited_account(&moved, &old).unwrap_err().contains("Enter the password again"));
    let (_, password) = mail.edited_account(&filled(moved, &[("Password", "new")]), &old).unwrap();
    assert_eq!(password.as_deref(), Some("new"));
    mail.replace_account(edited, Some("kept".into()));
    assert_eq!(mail.accounts[0].name, "Office");
    assert_eq!(mail.passwords.get("work").map(String::as_str), Some("kept"));
}

#[test]
fn removing_an_account_forgets_everything_kept_for_it() {
    let mut mail = Mail::new(vec![account("work"), account("home")]);
    mail.passwords.insert("work".into(), "pw".into());
    mail.baseline.insert("work".into(), 5);
    mail.prefs.set_signature("work", "Ana");
    mail.prefs.toggle_favorite("work", "INBOX");
    mail.prefs.toggle_favorite("home", "INBOX");
    mail.forget_account("work");
    assert_eq!(mail.accounts.len(), 1);
    assert_eq!(mail.accounts[0].id, "home");
    assert!(mail.passwords.is_empty() && mail.baseline.is_empty());
    assert_eq!(mail.prefs.signature("work"), "");
    assert_eq!(mail.prefs.favorites.len(), 1);
    assert_eq!(mail.account, 0);
    mail.forget_account("home");
    assert!(mail.accounts.is_empty());
    mail.forget_account("nobody");
}

#[test]
fn remind_me_needs_an_open_message_with_an_id() {
    let mut mail = Mail::new(vec![account("work")]);
    assert_eq!(mail.open_remind().unwrap_err(), "Open a message first");
    let rows = vec![
        Listed { uid: 1, flags: Flags::empty(), header: "Message-ID: <a@x>\r\nSubject: with\r\n\r\n".into() },
        Listed { uid: 2, flags: Flags::empty(), header: "Subject: without\r\n\r\n".into() },
    ];
    mail.apply_listing("work", INBOX, vec![], rows);
    mail.open_message(2);
    assert!(mail.open_remind().unwrap_err().contains("no Message-ID"));
    mail.open_message(1);
    mail.open_remind().unwrap();
    assert_eq!(mail.sheet.as_ref().map(Sheet::title), Some("Remind Me"));
}

#[test]
fn every_sheet_has_a_title_and_an_action() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.open_setup();
    assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("Add Account", "Add Account"));
    mail.open_compose(None);
    assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("New Message", "Send"));
    mail.open_redirect();
    assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("Redirect", "Redirect"));
    mail.open_remove_account().unwrap();
    assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("Remove Account", "Remove"));
    assert!(is_multiline("Message") && is_multiline("Signature") && !is_multiline("Subject"));
}

fn message_compose(text: &str) -> Compose {
    let form = Form::new(vec![Field::new("To", "", "a@example.com"), Field::new("Message", "", text)]);
    let mut compose = Compose::new(form, None, vec![], Priority::Normal, vec![], String::new());
    compose.form.focus_on(1);
    compose
}

#[test]
fn typed_text_takes_the_style_in_force_and_a_deleted_character_takes_its_style_with_it() {
    let mut compose = message_compose("ab");
    assert_eq!(compose.styles, vec![Style::default(); 2]);
    compose.toggle_bold();
    compose.edit(|form| form.type_text("cd"));
    compose.toggle_bold();
    compose.toggle_italic();
    compose.edit(|form| form.type_text("e"));
    assert_eq!(compose.message(), "abcde");
    let looks: Vec<(bool, bool)> = compose.styles.iter().map(|s| (s.bold, s.italic)).collect();
    assert_eq!(looks, vec![(false, false), (false, false), (true, false), (true, false), (false, true)]);
    compose.edit(|form| {
        form.backspace();
        form.backspace();
    });
    assert_eq!(compose.styles.len(), 3);
    compose.edit(|form| form.type_text("é"));
    assert_eq!(compose.styles.len(), 4, "styles count characters, not bytes");
    assert!(!compose.styles[3].bold && compose.styles[3].italic);
}

#[test]
fn typing_over_the_selected_message_starts_with_the_style_being_typed_in_not_the_old_runs() {
    let mut compose = message_compose("");
    compose.toggle_bold();
    compose.edit(|form| form.type_text("abc"));
    assert!(compose.styles.iter().all(|s| s.bold));
    compose.toggle_bold();
    compose.edit(|form| {
        form.select_all();
    });
    assert_eq!(compose.styles.len(), 3, "selecting changes no style");
    assert!(compose.styles.iter().all(|s| s.bold));
    compose.edit(|form| form.type_text("wxyz"));
    assert_eq!(compose.message(), "wxyz");
    assert_eq!(compose.styles, vec![Style::default(); 4], "no bold carries over from the replaced text");
}

#[test]
fn an_edit_of_another_field_leaves_the_message_styles_alone() {
    let mut compose = message_compose("ab");
    compose.toggle_bold();
    compose.form.focus_on(0);
    compose.edit(|form| form.type_text("x"));
    assert_eq!(compose.form.value("To"), "a@example.comx");
    assert_eq!(compose.styles.len(), 2);
    assert!(compose.styles.iter().all(|s| s.is_plain()));
}

#[test]
fn colour_and_underline_follow_the_typed_text_and_clearing_makes_it_plain_again() {
    let mut compose = message_compose("");
    compose.toggle_underline();
    compose.set_text_color(Some(TextColor::Blue));
    compose.edit(|form| form.type_text("link"));
    assert!(compose.styles.iter().all(|s| s.underline && s.color == Some(TextColor::Blue)));
    compose.clear_formatting();
    assert!(compose.styles.iter().all(|s| s.is_plain()) && compose.typing.is_plain());
    compose.set_text_color(None);
    assert!(compose.typing.is_plain());
}

#[test]
fn a_bullet_and_an_indent_go_at_the_start_of_the_last_line_and_come_off_again() {
    let mut compose = message_compose("first\nsecond");
    compose.toggle_bullet();
    assert_eq!(compose.message(), "first\n\u{2022} second");
    assert_eq!(compose.styles.len(), compose.message().chars().count());
    compose.indent(true);
    assert_eq!(compose.message(), "first\n    \u{2022} second");
    compose.indent(false);
    compose.toggle_bullet();
    assert_eq!(compose.message(), "first\nsecond");
    compose.indent(false);
    assert_eq!(compose.message(), "first\nsecond", "nothing to take off");
    let mut single = message_compose("one");
    single.toggle_bullet();
    assert_eq!(single.message(), "\u{2022} one");
}

#[test]
fn a_bulleted_line_keeps_the_styles_of_its_text() {
    let mut compose = message_compose("");
    compose.toggle_bold();
    compose.edit(|form| form.type_text("bold"));
    compose.toggle_bullet();
    assert_eq!(compose.message(), "\u{2022} bold");
    assert!(compose.styles[0].is_plain() && compose.styles[1].is_plain());
    assert!(compose.styles[2..].iter().all(|s| s.bold));
}

#[test]
fn a_styled_message_gets_an_html_part_in_the_draft_and_a_plain_one_does_not() {
    let mail = Mail::new(vec![account("work")]);
    let mut compose = message_compose("");
    assert_eq!(mail.draft_from(&compose).unwrap().html, None);
    compose.toggle_bold();
    compose.edit(|form| form.type_text("hi"));
    let draft = mail.draft_from(&compose).unwrap();
    assert!(draft.html.as_deref().is_some_and(|html| html.contains("<span style=\"font-weight: bold\">hi</span>")));
    assert_eq!(draft.body, "hi");
}

#[test]
fn spelling_marks_step_around_and_a_correction_replaces_the_word_in_its_style() {
    let mut compose = message_compose("");
    compose.toggle_bold();
    compose.edit(|form| form.type_text("hello wrold and teh end"));
    compose.set_misspelled(vec![6..11, 16..19]);
    assert_eq!(compose.current_misspelling(), Some((6..11, "wrold".to_string())));
    compose.next_misspelling();
    assert_eq!(compose.current_misspelling(), Some((16..19, "teh".to_string())));
    compose.next_misspelling();
    assert_eq!(compose.misspelled_at, 0, "the marks wrap");
    assert!(compose.correct_misspelling("world"));
    assert_eq!(compose.message(), "hello world and teh end");
    assert!(compose.misspelled.is_empty(), "the text changed, so the marks are cleared");
    assert_eq!(compose.styles.len(), compose.message().chars().count());
    assert!(compose.styles.iter().all(|s| s.bold));
    assert!(!compose.correct_misspelling("x"), "no word in view");
    compose.set_misspelled(vec![16..19]);
    compose.edit(|form| form.type_text("!"));
    assert!(compose.misspelled.is_empty(), "typing clears the marks");
}

#[test]
fn proofreading_marks_the_slips_and_accepting_edits_the_message_in_its_style() {
    let mut compose = message_compose("");
    compose.toggle_bold();
    compose.edit(|form| form.type_text("Say it it again, red,green"));
    assert_eq!(compose.start_proofread(), 2);
    let (marks, at) = compose.proofread_marks();
    assert_eq!((marks.len(), at), (2, 0));
    assert!(compose.proofread_line().unwrap().starts_with("Suggestion 1 of 2."), "{:?}", compose.proofread_line());
    assert_eq!(compose.accept_suggestion(false), 1);
    assert_eq!(compose.message(), "Say it again, red,green");
    assert_eq!(compose.styles.len(), compose.message().chars().count());
    assert!(compose.styles.iter().all(|s| s.bold));
    assert_eq!(compose.proofread.as_ref().map(Panel::len), Some(1), "the other suggestion moved with the edit");
    compose.skip_suggestion();
    assert!(compose.proofread.is_none() && compose.proofread_line().is_none());
    // all at once, and typing drops the suggestions
    let mut again = message_compose("the the  cat red,green");
    assert_eq!(again.start_proofread(), 3);
    assert_eq!(again.accept_suggestion(true), 3);
    assert_eq!(again.message(), "the cat red, green");
    assert!(again.proofread.is_none());
    let mut typed = message_compose("so so");
    assert_eq!(typed.start_proofread(), 1);
    typed.edit(|form| form.type_text("!"));
    assert!(typed.proofread.is_none(), "the suggestions no longer fit the text");
    assert_eq!(message_compose("").start_proofread(), 0);
}

#[test]
fn a_correction_longer_than_the_word_keeps_the_text_after_it_aligned_with_its_styles() {
    let mut compose = message_compose("ab cd");
    compose.styles[3].italic = true;
    compose.set_misspelled(vec![0..2]);
    assert!(compose.correct_misspelling("abcdef"));
    assert_eq!(compose.message(), "abcdef cd");
    assert_eq!(compose.styles.len(), 9);
    assert!(compose.styles[7].italic, "the italic 'c' moved to index 7");
    assert!(!compose.styles[3].italic);
}

#[test]
fn the_activity_list_keeps_the_newest_lines_and_opens_as_a_sheet() {
    let mut mail = Mail::new(vec![account("work")]);
    mail.open_activity();
    assert_eq!(prompt_form(&mail).value("Activity"), "Nothing has happened yet");
    for n in 0..MAX_ACTIVITY + 5 {
        mail.log(format!("line {n}"));
    }
    assert_eq!(mail.activity.len(), MAX_ACTIVITY);
    assert_eq!(mail.activity[0], "line 5");
    mail.open_activity();
    let text = prompt_form(&mail).value("Activity").to_string();
    assert!(text.starts_with("line 5\n") && text.ends_with(&format!("line {}", MAX_ACTIVITY + 4)));
    assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("Activity", "Done"));
}

#[test]
fn only_the_first_line_of_a_conversation_is_its_head() {
    let mut mail = Mail::new(vec![account("work")]);
    let rows = vec![
        Listed { uid: 1, flags: Flags::empty(), header: "Message-ID: <a@x>\r\nSubject: Plan\r\n\r\n".into() },
        Listed { uid: 2, flags: Flags::empty(), header: "Message-ID: <b@x>\r\nIn-Reply-To: <a@x>\r\nReferences: <a@x>\r\nSubject: Re: Plan\r\n\r\n".into() },
        Listed { uid: 3, flags: Flags::empty(), header: "Message-ID: <c@x>\r\nSubject: Other\r\n\r\n".into() },
    ];
    mail.apply_listing("work", INBOX, vec![], rows);
    assert!(mail.visible_lines().iter().all(|l| l.head && l.thread == 1), "the flat list has no conversations");
    mail.threaded = true;
    let lines = mail.visible_lines();
    let heads: Vec<(u32, bool)> = lines.iter().map(|l| (l.row.uid, l.head)).collect();
    let plan: Vec<&(u32, bool)> = heads.iter().filter(|(uid, _)| *uid != 3).collect();
    assert_eq!(plan.iter().filter(|(_, head)| *head).count(), 1, "one head in the two-message conversation");
    assert!(heads.contains(&(3, true)));
}
