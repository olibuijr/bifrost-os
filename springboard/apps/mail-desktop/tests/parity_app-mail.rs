//! Parity app-mail: headless tests of the Mail models for signatures, mailbox files, the body find, categories and
//! Remind Me times. No test opens a window, reads a mailbox or touches the network.

use bifrost_mail::mbox;
use bifrost_mail::organize::{categorize, due_reminders, due_sends, find_in_body, remind_time, Category, Remind, Scheduled};
use bifrost_mail::signature;

#[test]
fn an_appended_signature_follows_a_delimiter_line() {
    let body = signature::append("Hello", "Ann\nShop");
    assert_eq!(body, "Hello\n\n-- \nAnn\nShop");
    assert_eq!(signature::extract(&body).as_deref(), Some("Ann\nShop"));
    assert_eq!(signature::strip(&body), "Hello");
}

#[test]
fn a_dashes_line_without_a_space_counts_as_a_signature_delimiter() {
    let body = "Hi\n--\nAnn";
    assert!(signature::has_signature(body));
    assert_eq!(signature::strip(body), "Hi");
    assert_eq!(signature::extract(body).as_deref(), Some("Ann"));
}

#[test]
fn a_reply_puts_the_signature_above_the_quoted_text() {
    assert_eq!(signature::reply_body("> hi", "Ann"), "\n\n-- \nAnn\n\n> hi");
    assert_eq!(signature::reply_body("> hi", "   "), "\n\n> hi", "a blank signature adds nothing");
}

#[test]
fn a_signature_is_not_appended_twice() {
    let once = signature::append("Hello", "Ann");
    assert_eq!(signature::append(&once, "Bob"), once);
}

#[test]
fn a_mailbox_file_reads_back_to_the_same_message() {
    let raw = "From: a@x.is\r\nSubject: hi\r\n\r\nBody\r\n".to_string();
    let file = mbox::write(std::slice::from_ref(&raw), 0);
    assert!(file.lines().next().unwrap().starts_with("From "), "each message starts with a separator line");
    assert_eq!(mbox::read(&file), vec![raw]);
}

#[test]
fn a_body_line_that_starts_with_from_is_escaped_and_restored() {
    let raw = "From: a@x.is\r\n\r\nHi\r\nFrom here\r\n".to_string();
    let file = mbox::write(std::slice::from_ref(&raw), 0);
    assert!(file.contains("\n>From here\n"), "the body line gets one more quote mark");
    assert_eq!(mbox::read(&file), vec![raw]);
}

#[test]
fn a_mailbox_file_keeps_the_order_of_its_messages() {
    let first = "From: a@x.is\r\nSubject: one\r\n\r\nFirst\r\n".to_string();
    let second = "From: b@x.is\r\nSubject: two\r\n\r\nSecond\r\n".to_string();
    let file = mbox::write(&[first.clone(), second.clone()], 0);
    assert_eq!(mbox::read(&file), vec![first, second]);
}

#[test]
fn an_exported_mailbox_takes_its_file_name_from_the_last_part() {
    assert_eq!(mbox::file_name("INBOX"), "Inbox.mbox");
    assert_eq!(mbox::file_name("Work/Clients"), "Clients.mbox");
}

#[test]
fn an_imported_file_names_its_mailbox_after_the_file() {
    assert_eq!(mbox::mailbox_name("Old mail.mbox"), "Old mail");
    assert_eq!(mbox::mailbox_name(""), "Imported");
}

#[test]
fn due_reminders_come_earliest_first_and_ties_keep_their_order() {
    let list = vec![
        Remind { message_id: "<a>".into(), due: 300 },
        Remind { message_id: "<b>".into(), due: 100 },
        Remind { message_id: "<c>".into(), due: 100 },
    ];
    let ids: Vec<&str> = due_reminders(&list, 400).iter().map(|r| r.message_id.as_str()).collect();
    assert_eq!(ids, ["<b>", "<c>", "<a>"]);
    assert!(due_reminders(&list, 99).is_empty(), "nothing is due before its time");
}

#[test]
fn a_send_later_message_waits_for_its_time() {
    let queue = vec![Scheduled { id: 1, due: 50, raw: "x".into() }];
    assert!(due_sends(&queue, 49).is_empty());
    let due = due_sends(&queue, 50);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, 1);
}

#[test]
fn remind_me_times_are_later_today_tonight_tomorrow_and_next_week() {
    // 2026-10-15 (Thursday) 12:00 UTC
    let now = 1_792_065_600;
    assert_eq!(remind_time("later_today", now, 0), Some(now + 10_800));
    assert_eq!(remind_time("tonight", now, 0), Some(now + 8 * 3600));
    assert_eq!(remind_time("tomorrow", now, 0), Some(now + 20 * 3600));
    assert_eq!(remind_time("next_week", now, 0), Some(now + 4 * 86_400 - 4 * 3600));
    assert_eq!(remind_time("someday", now, 0), None, "an unknown choice has no time");
}

#[test]
fn tonight_is_never_in_the_past() {
    // 2026-10-15 21:00 UTC: the 20:00 of today has passed, so the reminder falls one minute on
    let now = 1_792_065_600 + 9 * 3600;
    assert_eq!(remind_time("tonight", now, 0), Some(now + 60));
}

#[test]
fn the_body_find_matches_icelandic_capitals_case_insensitively() {
    // Þ and þ are both two bytes in UTF-8, so the ranges line up with the original text
    assert_eq!(find_in_body("Þing þing", "ÞING"), vec![(0, 5), (6, 11)]);
}

#[test]
fn the_body_find_trims_the_needle_and_returns_nothing_for_an_empty_one() {
    assert_eq!(find_in_body("a b", "  b "), vec![(2, 3)]);
    assert!(find_in_body("abc", "").is_empty());
}

#[test]
fn a_reply_stays_primary_even_with_a_transaction_subject() {
    let raw = "Subject: Re: receipt\nFrom: shop@x.is\nIn-Reply-To: <1@x.is>\n";
    assert_eq!(categorize(raw), Category::Primary);
}

#[test]
fn bulk_mail_with_a_sale_subject_is_a_promotion() {
    let raw = "Subject: 50% off today\nFrom: news@x.is\nList-Unsubscribe: <mailto:u@x.is>\n";
    assert_eq!(categorize(raw), Category::Promotions);
}
