//! Parity sdk-gaps-01: headless tests for the HPKE, network attribution, mail handoff, broadcast and content item
//! models. Each test drives the public API of one pure model and asserts concrete values. No test opens a socket,
//! sends a frame or starts a window.

use bifrost_ui::broadcast::{BroadcastConfiguration, BroadcastController, BroadcastHandler, BroadcastPicker, BroadcastService, Error as BroadcastError, State};
use bifrost_ui::compose_handoff::Compose;
use bifrost_ui::content_items::{ContentItem, ContentTree, Error as ContentError, Kind};
use bifrost_ui::hpke::{self, Error as HpkeError};
use bifrost_ui::network_attribution::{is_local_destination, Attribution, Decision, LocalNetwork};
use bifrost_ui::x25519::PrivateKey;

const RECIPIENT_SK: [u8; 32] = [0x11; 32];
const EPHEMERAL_SK: [u8; 32] = [0x22; 32];
const OTHER_SK: [u8; 32] = [0x33; 32];

fn recipient_public() -> [u8; 32] {
    PrivateKey::from_raw(&RECIPIENT_SK).unwrap().public_key().raw()
}

#[test]
fn hpke_one_shot_round_trip_returns_the_plaintext() {
    let (enc, ct) = hpke::seal_base(&recipient_public(), b"info", b"aad", b"hello", &EPHEMERAL_SK).unwrap();
    assert_eq!(enc.len(), 32, "the encapsulated key is one X25519 point");
    assert_eq!(ct.len(), 5 + 16, "the ciphertext is the message plus a 16-byte tag");
    assert_eq!(hpke::open_base(&enc, &RECIPIENT_SK, b"info", b"aad", &ct).unwrap(), b"hello".to_vec());
}

#[test]
fn hpke_a_wrong_key_or_wrong_context_refuses_to_open() {
    let (enc, ct) = hpke::seal_base(&recipient_public(), b"info", b"aad", b"hello", &EPHEMERAL_SK).unwrap();
    assert_eq!(hpke::open_base(&enc, &OTHER_SK, b"info", b"aad", &ct), Err(HpkeError::Open));
    assert_eq!(hpke::open_base(&enc, &RECIPIENT_SK, b"other", b"aad", &ct), Err(HpkeError::Open));
    assert_eq!(hpke::open_base(&enc, &RECIPIENT_SK, b"info", b"other", &ct), Err(HpkeError::Open));
}

#[test]
fn hpke_a_short_public_key_is_refused() {
    assert_eq!(hpke::setup_sender(&[1; 5], b"", &EPHEMERAL_SK).err(), Some(HpkeError::Key));
}

#[test]
fn hpke_messages_in_one_context_open_in_order_only() {
    let (enc, mut sender) = hpke::setup_sender(&recipient_public(), b"", &EPHEMERAL_SK).unwrap();
    let first = sender.seal(b"", b"one").unwrap();
    let second = sender.seal(b"", b"two").unwrap();
    assert_eq!(sender.sequence(), 2);
    let mut receiver = hpke::setup_recipient(&enc, &RECIPIENT_SK, b"").unwrap();
    assert_eq!(receiver.open(b"", &second), Err(HpkeError::Open), "the second message cannot open first");
    assert_eq!(receiver.sequence(), 0, "a failed open does not advance the counter");
    assert_eq!(receiver.open(b"", &first).unwrap(), b"one".to_vec());
    assert_eq!(receiver.open(b"", &second).unwrap(), b"two".to_vec());
}

#[test]
fn hpke_exporter_matches_on_both_sides_and_is_bounded() {
    let (enc, sender) = hpke::setup_sender(&recipient_public(), b"i", &EPHEMERAL_SK).unwrap();
    let receiver = hpke::setup_recipient(&enc, &RECIPIENT_SK, b"i").unwrap();
    let key = sender.export(b"ctx", 32).unwrap();
    assert_eq!(key.len(), 32);
    assert_eq!(key, receiver.export(b"ctx", 32).unwrap());
    assert_eq!(sender.export(b"ctx", 255 * 32 + 1), Err(HpkeError::Export));
}

#[test]
fn local_network_destinations_need_the_permission_and_external_ones_do_not() {
    assert!(is_local_destination("192.168.1.9"));
    assert!(is_local_destination("printer.local"));
    assert!(!is_local_destination("203.0.113.7"));
    let mut net = LocalNetwork::default();
    assert_eq!(net.check("scanner", "192.168.1.9"), Decision::Ask);
    assert_eq!(net.check("scanner", "203.0.113.7"), Decision::Allow, "an external host never asks");
    net.answer("scanner", true);
    assert_eq!(net.check("scanner", "192.168.1.9"), Decision::Allow);
    net.answer("scanner", false);
    assert_eq!(net.check("scanner", "10.0.0.2"), Decision::Deny);
    net.reset("scanner");
    assert_eq!(net.check("scanner", "10.0.0.2"), Decision::Ask);
}

#[test]
fn network_attribution_counts_connections_per_app_and_host() {
    let mut log = Attribution::default();
    log.record("browser", "Example.com");
    log.record("browser", "example.com");
    log.record("browser", "cdn.example.net");
    log.record("mail", "example.com");
    assert_eq!(log.top_hosts("browser", 1), vec![("example.com".to_string(), 2)]);
    assert_eq!(log.apps_for("EXAMPLE.COM"), vec!["browser".to_string(), "mail".to_string()]);
    assert!(log.top_hosts("nobody", 5).is_empty());
}

#[test]
fn mail_handoff_round_trips_recipients_subject_and_body() {
    let draft = Compose { to: vec!["ann@example.is".into()], subject: "Notes & plans".into(), body: "a\nb".into(), ..Compose::default() };
    let url = draft.to_url();
    assert_eq!(url, "mailto:ann@example.is?subject=Notes%20%26%20plans&body=a%0Ab");
    assert_eq!(Compose::from_url(&url), Some(draft));
}

#[test]
fn mail_handoff_keeps_attachments_out_of_the_url() {
    let mut draft = Compose::default();
    assert!(draft.attach("/home/ann/report.pdf"));
    assert!(!draft.attach("/home/ann/report.pdf"), "a file is attached once");
    assert!(!draft.attach("report.pdf"), "attachments are absolute paths");
    assert_eq!(draft.to_url(), "mailto:");
    assert!(!draft.is_empty());
}

#[derive(Default)]
struct Log {
    events: Vec<String>,
}

impl BroadcastHandler for Log {
    fn started(&mut self, configuration: &BroadcastConfiguration) {
        self.events.push(format!("start {}", configuration.service_name()));
    }
    fn paused(&mut self) {
        self.events.push("pause".to_string());
    }
    fn resumed(&mut self) {
        self.events.push("resume".to_string());
    }
    fn finished(&mut self) {
        self.events.push("finish".to_string());
    }
}

fn stream_hall() -> BroadcastConfiguration {
    BroadcastConfiguration::new("Stream Hall", "rtmps://ingest.example.is/live/key").unwrap()
}

#[test]
fn broadcast_lifecycle_calls_the_handler_in_order() {
    let mut controller = BroadcastController::new(Log::default());
    assert_eq!(controller.state(), State::Idle);
    controller.start(stream_hall()).unwrap();
    controller.pause().unwrap();
    controller.resume().unwrap();
    controller.finish().unwrap();
    assert_eq!(controller.state(), State::Finished);
    assert_eq!(controller.handler().events, vec!["start Stream Hall", "pause", "resume", "finish"]);
}

#[test]
fn broadcast_refuses_actions_out_of_order() {
    let mut controller = BroadcastController::new(Log::default());
    assert_eq!(controller.pause(), Err(BroadcastError::NotBroadcasting));
    assert_eq!(controller.finish(), Err(BroadcastError::NotBroadcasting));
    controller.start(stream_hall()).unwrap();
    assert_eq!(controller.start(stream_hall()), Err(BroadcastError::AlreadyBroadcasting));
    assert_eq!(controller.resume(), Err(BroadcastError::NotPaused));
    assert_eq!(controller.handler().events, vec!["start Stream Hall"]);
}

#[test]
fn a_finished_broadcast_can_start_again_and_keeps_the_last_configuration() {
    let mut controller = BroadcastController::new(Log::default());
    controller.start(stream_hall()).unwrap();
    controller.finish().unwrap();
    let other = BroadcastConfiguration::new("Second", "https://second.example.is/in").unwrap();
    controller.start(other).unwrap();
    assert_eq!(controller.state(), State::Broadcasting);
    assert_eq!(controller.configuration().map(|c| c.url()), Some("https://second.example.is/in"));
}

#[test]
fn broadcast_configuration_checks_the_name_and_the_ingest_address() {
    assert_eq!(BroadcastConfiguration::new("  ", "https://a.example.is/x").err(), Some(BroadcastError::BadName));
    assert_eq!(BroadcastConfiguration::new(&"n".repeat(65), "https://a.example.is/x").err(), Some(BroadcastError::BadName));
    assert_eq!(BroadcastConfiguration::new("Hall", "ftp://a.example.is/x").err(), Some(BroadcastError::BadUrl));
    assert_eq!(BroadcastConfiguration::new("Hall", "rtmp:///live").err(), Some(BroadcastError::BadUrl));
    assert_eq!(BroadcastConfiguration::new("Hall", "https://a.example.is/x y").err(), Some(BroadcastError::BadUrl));
    let ok = BroadcastConfiguration::new(" Hall ", " rtmp://a.example.is:1935/live ").unwrap();
    assert_eq!(ok.service_name(), "Hall");
    assert_eq!(ok.url(), "rtmp://a.example.is:1935/live");
}

#[test]
fn broadcast_picker_confirms_only_a_valid_selected_service() {
    let mut picker = BroadcastPicker::new(vec![
        BroadcastService { name: "Hall".into(), url: "https://hall.example.is/in".into() },
        BroadcastService { name: "Bad".into(), url: "gopher://x".into() },
    ]);
    assert_eq!(picker.confirm().err(), Some(BroadcastError::NoSelection));
    assert!(!picker.select(5), "an index past the end is refused");
    assert_eq!(picker.confirm().err(), Some(BroadcastError::NoSelection));
    assert!(picker.select(0));
    assert_eq!(picker.confirm().unwrap().service_name(), "Hall");
    assert!(picker.select(1));
    assert_eq!(picker.confirm().err(), Some(BroadcastError::BadUrl));
    assert_eq!(picker.services().len(), 2);
}

fn library() -> ContentTree {
    let mut tree = ContentTree::default();
    tree.add(None, ContentItem::new("albums", "Albums", Kind::Container)).unwrap();
    tree.add(Some("albums"), ContentItem::new("a1", "First", Kind::Playable).streaming(true)).unwrap();
    tree.add(Some("albums"), ContentItem::new("a2", "Second", Kind::Playable).explicit(true)).unwrap();
    tree.add(None, ContentItem::new("radio", "Radio", Kind::Playable)).unwrap();
    tree
}

#[test]
fn content_tree_lists_roots_and_children_in_insertion_order() {
    let tree = library();
    let roots: Vec<String> = tree.roots().iter().map(|i| i.identifier().to_string()).collect();
    assert_eq!(roots, vec!["albums", "radio"]);
    let children: Vec<String> = tree.children("albums").iter().map(|i| i.title().to_string()).collect();
    assert_eq!(children, vec!["First", "Second"]);
    assert!(tree.children("radio").is_empty(), "a playable item has no children");
    assert!(tree.children("missing").is_empty());
}

#[test]
fn content_tree_refuses_duplicates_and_bad_parents() {
    let mut tree = library();
    assert_eq!(tree.add(None, ContentItem::new("radio", "Again", Kind::Playable)), Err(ContentError::Duplicate));
    assert_eq!(tree.add(Some("nope"), ContentItem::new("x", "X", Kind::Playable)), Err(ContentError::UnknownParent));
    assert_eq!(tree.add(Some("radio"), ContentItem::new("x", "X", Kind::Playable)), Err(ContentError::NotAContainer));
    assert!(tree.item("x").is_none());
}

#[test]
fn content_playback_resolves_only_playable_items() {
    let tree = library();
    assert_eq!(tree.resolve_playback("a1").unwrap().title(), "First");
    assert_eq!(tree.resolve_playback("albums").err(), Some(ContentError::NotPlayable));
    assert_eq!(tree.resolve_playback("zzz").err(), Some(ContentError::UnknownItem));
}

#[test]
fn content_item_flags_and_progress_are_kept_and_clamped() {
    let mut tree = library();
    assert!(tree.item("a1").unwrap().is_streaming());
    assert!(!tree.item("a2").unwrap().is_streaming());
    assert!(tree.item("a2").unwrap().is_explicit());
    assert_eq!(ContentItem::new("p", "P", Kind::Playable).progress(1.7).playback_progress(), 1.0);
    assert_eq!(ContentItem::new("p", "P", Kind::Playable).progress(-0.2).playback_progress(), 0.0);
    assert_eq!(ContentItem::new("p", "P", Kind::Playable).progress(f64::NAN).playback_progress(), 0.0);
    tree.set_progress("a1", 0.25).unwrap();
    assert_eq!(tree.item("a1").unwrap().playback_progress(), 0.25);
    assert_eq!(tree.set_progress("missing", 0.5), Err(ContentError::UnknownItem));
}
