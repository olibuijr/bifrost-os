//! Parity app-find-my: headless tests of the locator model. No test reads a clock, a network or a device.

use bifrost_locate::devices::{Command, CommandError, Device, Devices, Fix};
use bifrost_locate::friends::{distance_m, distance_text, fires, friend_distance, inside, rule_fires, Friend, NotifyRule, Trigger};
use bifrost_locate::links::{device_link, directions, parse, Target};
use bifrost_locate::share::{valid_address, Duration, ShareError, Shares};

const REYKJAVIK: (f64, f64) = (64.1466, -21.9426);

fn fix(lat: f64, lon: f64, at_unix: i64) -> Fix {
    Fix { lat, lon, accuracy_m: 5.0, at_unix }
}

#[test]
fn a_new_device_has_no_position_battery_lost_mode_or_commands() {
    let d = Device::new(" dev-1 ", " Desk ");
    assert_eq!(d.id, "dev-1");
    assert_eq!(d.name, "Desk");
    assert_eq!(d.position, None);
    assert_eq!(d.battery_label(), None);
    assert!(d.lost.is_none());
    assert!(d.pending().is_empty());
}

#[test]
fn a_newer_fix_replaces_the_held_one_and_an_older_or_equal_one_is_ignored() {
    let mut d = Device::new("dev-1", "Desk");
    assert!(d.report(fix(64.0, -21.0, 100)));
    assert!(!d.report(fix(65.0, -20.0, 50)), "an older fix is not stored");
    assert!(!d.report(fix(65.0, -20.0, 100)), "a fix with the same time is not stored");
    assert!(d.report(fix(65.0, -20.0, 200)));
    assert_eq!(d.position.map(|p| p.lat), Some(65.0));
}

#[test]
fn the_battery_label_shows_the_level_and_clamps_to_one_hundred_percent() {
    let mut d = Device::new("dev-1", "Desk");
    d.set_battery(80);
    assert_eq!(d.battery_label().as_deref(), Some("Battery 80%"));
    d.set_battery(140);
    assert_eq!(d.battery_label().as_deref(), Some("Battery 100%"));
}

#[test]
fn the_device_list_finds_a_device_by_id_and_updates_it_in_place() {
    let mut list = Devices::new(vec![Device::new("a", "Laptop"), Device::new("b", "Tablet")]);
    assert_eq!(list.list().len(), 2);
    assert!(list.get("c").is_none());
    list.get_mut("b").unwrap().set_battery(42);
    assert_eq!(list.get("b").unwrap().battery, Some(42));
}

#[test]
fn marking_a_device_lost_keeps_the_message_and_number_and_queues_a_lock() {
    let mut d = Device::new("dev-1", "Phone");
    d.mark_lost("  Call the office  ", " 555 0100 ");
    let lost = d.lost.clone().expect("lost mode is set");
    assert_eq!(lost.message, "Call the office");
    assert_eq!(lost.phone, "555 0100");
    assert_eq!(d.pending(), [Command::Lock].as_slice());
    d.clear_lost();
    assert!(d.lost.is_none());
}

#[test]
fn erase_needs_confirmation_and_a_waiting_command_is_not_queued_twice() {
    let mut d = Device::new("dev-1", "Phone");
    assert_eq!(d.request(Command::Erase, false), Err(CommandError::NeedsConfirmation));
    assert!(d.pending().is_empty());
    assert_eq!(d.request(Command::PlaySound, false), Ok(()));
    assert_eq!(d.request(Command::PlaySound, false), Ok(()));
    assert_eq!(d.pending(), [Command::PlaySound].as_slice());
    assert_eq!(d.request(Command::Erase, true), Ok(()));
    assert_eq!(d.take_commands(), vec![Command::PlaySound, Command::Erase]);
    assert!(d.pending().is_empty(), "taking the commands empties the queue");
}

#[test]
fn an_address_needs_a_local_part_and_a_dotted_domain_without_spaces() {
    assert!(valid_address("olibuijr@olibuijr.com"));
    assert!(!valid_address("olibuijr"));
    assert!(!valid_address("@olibuijr.com"));
    assert!(!valid_address("a@b"));
    assert!(!valid_address("a b@c.is"));
    assert!(!valid_address("a@b@c.is"));
}

#[test]
fn a_timed_share_ends_after_its_duration_and_an_indefinite_share_does_not() {
    let now = 1_000;
    let mut s = Shares::default();
    s.share("friend@example.is", Duration::Hour, now).unwrap();
    s.share("family@example.is", Duration::Indefinite, now).unwrap();
    assert_eq!(s.active(now + 3_599).len(), 2);
    let later = s.active(now + 3_600);
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].to, "family@example.is");
    assert_eq!(s.active(now + 10 * 365 * 86_400).len(), 1);
}

#[test]
fn sharing_again_replaces_the_grant_and_stopping_removes_it() {
    let mut s = Shares::default();
    s.share("Friend@Example.is", Duration::Hour, 0).unwrap();
    s.share("friend@example.is ", Duration::Day, 0).unwrap();
    assert_eq!(s.active(0).len(), 1);
    assert_eq!(s.active(0)[0].until_unix, Some(86_400));
    s.stop("FRIEND@example.is");
    assert!(s.active(0).is_empty());
    assert_eq!(s.share("not an address", Duration::Day, 0), Err(ShareError::InvalidAddress));
}

#[test]
fn a_fix_within_the_radius_is_inside_and_a_distant_fix_is_not() {
    let near = fix(64.1470, -21.9430, 1);
    let far = fix(64.2, -21.9, 1);
    assert!(inside(&near, REYKJAVIK, 100.0));
    assert!(!inside(&far, REYKJAVIK, 100.0));
    assert!(distance_m(&near, REYKJAVIK) < 100.0);
}

#[test]
fn an_arrival_fires_only_after_the_friend_was_seen_outside() {
    assert!(fires(Trigger::Arrives, Some(false), true));
    assert!(!fires(Trigger::Arrives, None, true), "the first state seen fires nothing");
    assert!(!fires(Trigger::Arrives, Some(true), true));
    assert!(fires(Trigger::Leaves, Some(true), false));
    assert!(!fires(Trigger::Leaves, Some(false), false));
}

#[test]
fn a_rule_fires_when_the_friend_crosses_into_its_place() {
    let rule = NotifyRule { address: "friend@example.is".into(), trigger: Trigger::Arrives, place: REYKJAVIK, radius_m: 100.0 };
    let home = fix(64.1470, -21.9430, 2);
    let away = fix(64.2, -21.9, 3);
    assert!(rule_fires(&rule, Some(false), &home));
    assert!(!rule_fires(&rule, Some(true), &home));
    assert!(!rule_fires(&rule, Some(false), &away));
}

#[test]
fn a_friend_without_a_position_has_no_distance_and_one_with_a_position_has_one() {
    let unknown = Friend { address: "a@example.is".into(), name: "A".into(), position: None };
    assert_eq!(friend_distance(&unknown, REYKJAVIK), None);
    let known = Friend { address: "b@example.is".into(), name: "B".into(), position: Some(fix(64.2, -21.9, 1)) };
    let metres = friend_distance(&known, REYKJAVIK).expect("a distance");
    assert!((metres - distance_m(&fix(64.2, -21.9, 1), REYKJAVIK)).abs() < 1e-6);
}

#[test]
fn distances_read_in_metres_then_kilometres() {
    assert_eq!(distance_text(850.4), "850 m");
    assert_eq!(distance_text(999.6), "1.0 km");
    assert_eq!(distance_text(2_400.0), "2.4 km");
}

#[test]
fn a_locate_link_opens_a_device_or_a_person_and_nothing_else() {
    assert_eq!(parse("locate:device/dev-1"), Some(Target::Device("dev-1".into())));
    assert_eq!(parse("locate://person/friend%40example.is"), Some(Target::Person("friend@example.is".into())));
    assert_eq!(parse("locate:device/"), None);
    assert_eq!(parse("maps:?q=x"), None);
    assert_eq!(parse("locate:vehicle/dev-1"), None);
}

#[test]
fn a_device_link_round_trips_through_the_parser() {
    let link = device_link("dev-1");
    assert_eq!(link, "locate:device/dev-1");
    assert_eq!(parse(&link), Some(Target::Device("dev-1".into())));
}

#[test]
fn directions_to_a_device_are_a_driving_route_in_the_maps_link_form() {
    let link = directions((64.0, -21.0), (64.1466, -21.9426));
    assert_eq!(link, "maps:?saddr=64.00000,-21.00000&daddr=64.14660,-21.94260&dirflg=d");
}
