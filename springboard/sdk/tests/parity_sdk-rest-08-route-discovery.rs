//! Parity sdk-rest-08: headless tests for the app-facing route-discovery model. The feed does not open sockets or run
//! an extension process.

use bifrost_app::route_discovery::{discovery_point, DiscoveryError, Receiver, ReceiverEvent, ReceiverFeed, DISCOVERY_SUFFIX};
use bifrost_ui::route_auth::RouteAuth;

fn receiver(id: &str, name: &str, at: u64) -> Receiver {
    Receiver { id: id.into(), name: name.into(), host: "192.0.2.10".into(), port: 7000, last_seen_ms: at }
}

#[test]
fn a_valid_owner_gets_a_discovery_point_in_its_namespace() {
    let point = discovery_point("com.example.player").unwrap();
    assert_eq!(point.identifier, format!("com.example.player.{DISCOVERY_SUFFIX}"));
    assert_eq!(point.owner, "com.example.player");
    assert_eq!(discovery_point("player"), Err(DiscoveryError::InvalidOwner));
}

#[test]
fn the_feed_requires_start_and_reports_new_receivers_once() {
    let mut feed = ReceiverFeed::new(1_000).unwrap();
    assert!(!feed.is_running());
    assert_eq!(feed.report(receiver("tv", "Living room", 10)), Ok(None));
    assert_eq!(feed.receivers().count(), 0);

    feed.start();
    assert!(feed.is_running());
    let found = Receiver { id: "tv".into(), name: "Living room".into(), ..receiver("tv", "Living room", 10) };
    assert_eq!(feed.report(receiver(" tv ", " Living room ", 10)), Ok(Some(ReceiverEvent::Found(found))));
    assert_eq!(feed.report(receiver("tv", "Living room", 20)), Ok(None), "refreshes do not make duplicate Found events");
    let current = feed.receivers().next().unwrap();
    assert_eq!(current.last_seen_ms, 20);
}

#[test]
fn invalid_reports_and_a_zero_ttl_are_refused() {
    assert_eq!(ReceiverFeed::new(0).err(), Some(DiscoveryError::ZeroTimeToLive));
    let mut feed = ReceiverFeed::new(100).unwrap();
    feed.start();
    assert_eq!(feed.report(receiver(" ", "TV", 0)), Err(DiscoveryError::Receiver(bifrost_ui::network_audio_browser::BrowseError::EmptyId)));
    assert_eq!(feed.report(receiver("tv", " ", 0)), Err(DiscoveryError::Receiver(bifrost_ui::network_audio_browser::BrowseError::EmptyName)));
    let mut no_port = receiver("tv", "TV", 0);
    no_port.port = 0;
    assert_eq!(feed.report(no_port), Err(DiscoveryError::Receiver(bifrost_ui::network_audio_browser::BrowseError::ZeroPort)));
}

#[test]
fn expiry_keeps_the_exact_ttl_boundary_then_reports_lost() {
    let mut feed = ReceiverFeed::new(100).unwrap();
    feed.start();
    feed.report(receiver("tv", "TV", 100)).unwrap();
    assert!(feed.expire(200).is_empty());
    assert_eq!(feed.expire(201), vec![ReceiverEvent::Lost(receiver("tv", "TV", 100))]);
    assert_eq!(feed.receivers().count(), 0);
}

#[test]
fn destinations_are_sorted_and_mark_only_authorized_active_routes() {
    let mut feed = ReceiverFeed::with_default_ttl();
    feed.start();
    feed.report(receiver("kitchen", "kitchen", 0)).unwrap();
    feed.report(receiver("attic", "Attic", 0)).unwrap();
    let mut auth = RouteAuth::new();
    auth.authorize("kitchen");
    auth.set_active("kitchen", true).unwrap();
    let destinations = feed.destinations(&auth);
    assert_eq!(destinations.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["Attic", "kitchen"]);
    assert!(!destinations[0].active);
    assert!(destinations[1].active);
}

#[test]
fn stopping_keeps_known_receivers_and_suppresses_expiry() {
    let mut feed = ReceiverFeed::new(10).unwrap();
    feed.start();
    feed.report(receiver("tv", "TV", 0)).unwrap();
    feed.stop();
    assert!(!feed.is_running());
    assert!(feed.expire(100).is_empty());
    assert_eq!(feed.receivers().count(), 1);
}
