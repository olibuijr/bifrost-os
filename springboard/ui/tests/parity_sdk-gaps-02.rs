//! Parity sdk-gaps-02: headless tests for route addresses, encrypted models, remote media sessions, push registration,
//! map snapshot geometry and model prediction. No test opens a socket, draws a frame or touches a device.

use bifrost_ui::features::{DictionaryProvider, FeatureProvider, FeatureValue};
use bifrost_ui::map_snapshot::{self, Placed, Snapshot};
use bifrost_ui::ml_model::{Model, ModelError};
use bifrost_ui::ml_tensor::Tensor;
use bifrost_ui::model_seal::{self, OpenError};
use bifrost_ui::push_registration::{self, Refusal, Registration, Registry, Subscription, MAX_PAYLOAD};
use bifrost_ui::remote_session::{MediaDevice, RemoteMediaSession, RemoteSessionError};
use bifrost_ui::route_address::{KnownRoutes, RouteAddress};
use bifrost_ui::sealed_box::{self, Error as SealError};
use std::net::IpAddr;

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

#[test]
fn a_full_route_address_matches_only_itself() {
    let address = RouteAddress::parse("192.168.1.12").unwrap();
    assert!(address.matches(ip("192.168.1.12")));
    assert!(!address.matches(ip("192.168.1.13")));
    assert_eq!(RouteAddress::parse("fe80::1"), Some(RouteAddress::Full(ip("fe80::1"))));
}

#[test]
fn a_partial_route_address_clears_host_bits_and_matches_its_network() {
    let address = RouteAddress::parse("192.168.1.77/24").unwrap();
    assert_eq!(address, RouteAddress::parse("192.168.1.0/24").unwrap());
    assert!(address.matches(ip("192.168.1.200")));
    assert!(!address.matches(ip("192.168.2.1")));
}

#[test]
fn a_zero_prefix_matches_every_address_of_its_family_and_no_other() {
    let any_v4 = RouteAddress::parse("0.0.0.0/0").unwrap();
    assert!(any_v4.matches(ip("8.8.8.8")));
    assert!(!any_v4.matches(ip("fe80::1")), "an IPv4 network never holds an IPv6 address");
}

#[test]
fn an_ipv6_network_matches_by_its_prefix() {
    let network = RouteAddress::parse("fe80::/64").unwrap();
    assert!(network.matches(ip("fe80::abcd")));
    assert!(!network.matches(ip("fe81::1")));
}

#[test]
fn route_addresses_refuse_text_and_prefixes_longer_than_the_address() {
    assert_eq!(RouteAddress::parse("192.168.1.0/33"), None);
    assert_eq!(RouteAddress::parse("fe80::/129"), None);
    assert_eq!(RouteAddress::parse("192.168.1.1/x"), None);
    assert_eq!(RouteAddress::parse("not an address"), None);
}

#[test]
fn known_routes_hold_each_address_once_and_report_the_bad_entry() {
    let routes = KnownRoutes::from_texts(["192.168.1.0/24", "192.168.1.9/24", "10.0.0.5"]).unwrap();
    assert_eq!(routes.len(), 2, "the two spellings of one network are one entry");
    assert!(routes.contains(ip("192.168.1.50")));
    assert!(routes.contains(ip("10.0.0.5")));
    assert!(!routes.contains(ip("10.0.0.6")));
    assert_eq!(KnownRoutes::from_texts(["10.0.0.5", "bad"]), Err("bad".to_string()));
}

#[test]
fn known_routes_add_and_remove_report_whether_they_changed_anything() {
    let mut routes = KnownRoutes::default();
    assert!(routes.is_empty());
    let address = RouteAddress::parse("10.1.2.3").unwrap();
    assert!(routes.add(address));
    assert!(!routes.add(address));
    assert!(routes.remove(address));
    assert!(!routes.remove(address));
    assert!(!routes.contains(ip("10.1.2.3")));
}

const MODEL_TEXT: &str = "input x 2\ndense 2 1,2,3,4 0.5,-1\nrelu\noutput y\n";
const SALT: [u8; 16] = [3; 16];
const NONCE: [u8; 12] = [5; 12];

fn tensor_input(values: &[f32]) -> DictionaryProvider {
    let t = Tensor::new(&[values.len()], values.to_vec()).unwrap();
    DictionaryProvider::new().with("x", FeatureValue::Tensor(t))
}

fn output_values(out: &DictionaryProvider, name: &str) -> Option<Vec<f32>> {
    out.value(name).and_then(FeatureValue::as_tensor).map(|t| t.values().to_vec())
}

#[test]
fn a_model_text_predicts_its_output_from_a_tensor_input() {
    let model = Model::parse(MODEL_TEXT).unwrap();
    assert_eq!((model.input_name(), model.input_len(), model.output_name()), ("x", 2, Some("y")));
    let out = model.predict(&tensor_input(&[1.0, 2.0])).unwrap();
    assert_eq!(output_values(&out, "y"), Some(vec![7.5, 9.0]));
}

#[test]
fn a_softmax_model_gives_equal_shares_for_equal_inputs() {
    let model = Model::parse("input x 3\nsoftmax\noutput p\n").unwrap();
    let out = model.predict(&tensor_input(&[0.0, 0.0, 0.0])).unwrap();
    for v in output_values(&out, "p").unwrap() {
        assert!((v - 1.0 / 3.0).abs() < 1e-6, "{v}");
    }
}

#[test]
fn a_model_refuses_an_input_of_the_wrong_length() {
    let model = Model::parse(MODEL_TEXT).unwrap();
    assert_eq!(model.predict(&tensor_input(&[1.0, 2.0, 3.0])).err(), Some(ModelError::InputLength { expected: 2, found: 3 }));
}

#[test]
fn an_encrypted_model_opens_with_its_password_and_predicts() {
    let sealed = model_seal::seal_with_password(MODEL_TEXT, "pass", &SALT, &NONCE, 3);
    assert!(sealed_box::is_sealed(&sealed));
    assert!(!sealed.windows(5).any(|w| w == b"input"), "the model text is not visible in the container");
    let model = model_seal::open_with_password(&sealed, "pass").unwrap();
    let out = model.predict(&tensor_input(&[1.0, 2.0])).unwrap();
    assert_eq!(output_values(&out, "y"), Some(vec![7.5, 9.0]));
}

#[test]
fn an_encrypted_model_refuses_a_wrong_password_and_changed_bytes() {
    let mut sealed = model_seal::seal_with_password(MODEL_TEXT, "pass", &SALT, &NONCE, 3);
    assert_eq!(model_seal::open_with_password(&sealed, "other").err(), Some(OpenError::Seal(SealError::Authentication)));
    let last = sealed.len() - 1;
    sealed[last] ^= 1;
    assert_eq!(model_seal::open_with_password(&sealed, "pass").err(), Some(OpenError::Seal(SealError::Authentication)));
}

#[test]
fn an_encrypted_model_under_a_raw_key_needs_the_same_key_and_context() {
    let key = [7u8; 32];
    let sealed = model_seal::seal_with_key(MODEL_TEXT, &key, &NONCE, b"model:v1").unwrap();
    assert!(model_seal::open_with_key(&sealed, &key, b"model:v1").is_ok());
    assert_eq!(model_seal::open_with_key(&sealed, &[8u8; 32], b"model:v1").err(), Some(OpenError::Seal(SealError::Authentication)));
    assert_eq!(model_seal::open_with_key(&sealed, &key, b"model:v2").err(), Some(OpenError::Seal(SealError::Authentication)));
}

#[test]
fn an_encrypted_payload_that_is_not_model_text_is_refused() {
    let binary = sealed_box::seal_with_password(&[0xff, 0xfe], "pass", &SALT, &NONCE, 3);
    assert_eq!(model_seal::open_with_password(&binary, "pass").err(), Some(OpenError::NotText));
    let broken = model_seal::seal_with_password("input x 2\nbogus\n", "pass", &SALT, &NONCE, 3);
    assert!(matches!(model_seal::open_with_password(&broken, "pass"), Err(OpenError::Model(_))));
}

fn device(id: &str, name: &str) -> MediaDevice {
    MediaDevice { id: id.into(), name: name.into() }
}

#[test]
fn a_remote_session_needs_an_app_and_a_title() {
    assert_eq!(RemoteMediaSession::new(" ", "Song").unwrap_err(), RemoteSessionError::Incomplete);
    assert_eq!(RemoteMediaSession::new("is.app", "").unwrap_err(), RemoteSessionError::Incomplete);
    let session = RemoteMediaSession::new(" is.app ", " Song ").unwrap();
    assert_eq!((session.app_id.as_str(), session.title.as_str()), ("is.app", "Song"));
}

#[test]
fn a_remote_session_keeps_each_device_once_and_drops_empty_ids() {
    let mut session = RemoteMediaSession::new("is.app", "Song").unwrap();
    session.set_devices(vec![device("a", "Kitchen"), device("", "Nameless"), device("a", "Again"), device("b", "Den")]);
    let names: Vec<&str> = session.devices().iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Kitchen", "Den"]);
}

#[test]
fn connecting_a_remote_session_moves_playback_between_listed_devices() {
    let mut session = RemoteMediaSession::new("is.app", "Song").unwrap();
    session.set_devices(vec![device("a", "Kitchen"), device("b", "Den")]);
    assert!(session.connected().is_none());
    assert_eq!(session.connect("a").unwrap().name.as_str(), "Kitchen");
    assert_eq!(session.connected().map(|d| d.id.as_str()), Some("a"));
    session.connect("b").unwrap();
    assert_eq!(session.connected().map(|d| d.name.as_str()), Some("Den"));
    assert_eq!(session.connect("missing").err(), Some(RemoteSessionError::UnknownDevice));
}

#[test]
fn disconnecting_returns_the_device_and_a_second_disconnect_is_refused() {
    let mut session = RemoteMediaSession::new("is.app", "Song").unwrap();
    session.set_devices(vec![device("a", "Kitchen")]);
    session.connect("a").unwrap();
    assert_eq!(session.disconnect(), Ok(device("a", "Kitchen")));
    assert_eq!(session.disconnect(), Err(RemoteSessionError::NotConnected));
    assert!(session.connected().is_none());
}

#[test]
fn losing_the_connected_device_disconnects_a_remote_session() {
    let mut session = RemoteMediaSession::new("is.app", "Song").unwrap();
    session.set_devices(vec![device("a", "Kitchen")]);
    session.connect("a").unwrap();
    session.set_devices(vec![device("b", "Den")]);
    assert!(session.connected().is_none());
    assert_eq!(session.disconnect(), Err(RemoteSessionError::NotConnected));
}

fn token(c: char) -> String {
    std::iter::repeat(c).take(64).collect()
}

fn registration(app: &str, topic: &str, token: String, expires_at: i64) -> Registration {
    Registration { app_id: app.into(), topic: topic.into(), token, expires_at }
}

#[test]
fn a_push_is_routed_to_the_app_that_registered_its_token() {
    let mut registry = Registry::default();
    assert!(registry.register(registration("is.app", "news", token('a'), 1000)));
    assert_eq!(registry.route(&token('a'), "news", 100, 500), Ok("is.app"));
    assert_eq!(registry.route(&token('b'), "news", 100, 500), Err(Refusal::UnknownToken));
    assert_eq!(registry.route(&token('a'), "news", 100, 1000), Err(Refusal::Expired));
    assert_eq!(registry.route(&token('a'), "other", 100, 500), Err(Refusal::TopicMismatch));
    assert_eq!(registry.route(&token('a'), "news", MAX_PAYLOAD + 1, 500), Err(Refusal::TooLarge));
}

#[test]
fn a_registration_replaces_the_token_of_the_same_app_and_topic() {
    let mut registry = Registry::default();
    registry.register(registration("is.app", "news", token('a'), 1000));
    registry.register(registration("is.app", "news", token('b'), 1000));
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.route(&token('a'), "news", 1, 0), Err(Refusal::UnknownToken));
    assert!(!registry.register(registration("", "news", token('c'), 1000)), "an app identifier is required");
    assert!(!registry.register(registration("is.app", "news", "abcd".into(), 1000)), "a token is 32 to 64 bytes of hex");
    assert!(push_registration::valid_token(&token('f')));
    assert!(!push_registration::valid_token(&token('g')));
}

#[test]
fn purging_drops_expired_registrations_and_unregistering_drops_an_app() {
    let mut registry = Registry::default();
    registry.register(registration("a", "t", token('a'), 100));
    registry.register(registration("b", "t", token('b'), 900));
    assert_eq!(registry.purge(500), 1);
    assert_eq!(registry.unregister("b"), 1);
    assert_eq!(registry.len(), 0);
}

#[test]
fn a_web_push_subscription_needs_https_endpoint_and_origin_and_both_keys() {
    let subscription = Subscription {
        origin: "https://news.example".into(),
        endpoint: "https://push.example/abc".into(),
        p256dh: "key".into(),
        auth: "secret".into(),
    };
    assert!(subscription.is_valid());
    assert!(!Subscription { endpoint: "http://push.example/abc".into(), ..subscription.clone() }.is_valid());
    assert!(!Subscription { origin: "http://news.example".into(), ..subscription.clone() }.is_valid());
    assert!(!Subscription { auth: String::new(), ..subscription }.is_valid());
}

#[test]
fn a_snapshot_at_zoom_one_covers_the_world_with_four_tiles() {
    let snapshot = Snapshot { lat: 0.0, lon: 0.0, zoom: 1, width: 256, height: 256 };
    let tiles = snapshot.tiles();
    assert_eq!(tiles.len(), 4);
    let at = |x: i64, y: i64| tiles.iter().find(|p| p.tile_x == x && p.tile_y == y).unwrap();
    assert!((at(0, 0).dest_x + 128.0).abs() < 1e-6, "the tile sits left of centre by half the snapshot");
    assert!((at(1, 1).dest_y - 128.0).abs() < 1e-6);
}

#[test]
fn a_snapshot_places_its_centre_in_the_middle_of_the_image() {
    let snapshot = Snapshot { lat: 64.1466, lon: -21.9426, zoom: 12, width: 512, height: 300 };
    let (x, y) = snapshot.point(64.1466, -21.9426);
    assert!((x - 256.0).abs() < 1e-6 && (y - 150.0).abs() < 1e-6, "{x} {y}");
    let (world_x, world_y) = map_snapshot::world_px(64.1466, -21.9426, 12);
    let (lat, lon) = map_snapshot::lat_lon(world_x, world_y, 12);
    assert!((lat - 64.1466).abs() < 1e-6 && (lon + 21.9426).abs() < 1e-6);
}

#[test]
fn snapshot_tiles_wrap_across_the_antimeridian_and_stay_inside_the_world() {
    let snapshot = Snapshot { lat: 0.0, lon: 179.9, zoom: 2, width: 600, height: 300 };
    let tiles = snapshot.tiles();
    assert!(tiles.iter().all(|p| (0..4).contains(&p.tile_x) && (0..4).contains(&p.tile_y)));
    let polar = Snapshot { lat: 85.0, lon: 0.0, zoom: 3, width: 512, height: 512 };
    assert!(polar.tiles().iter().all(|p| p.tile_y >= 0));
}

#[test]
fn fit_zoom_is_smaller_for_pins_spread_further_apart() {
    let near = [(64.14, -21.94), (64.15, -21.93)];
    let far = [(64.14, -21.94), (66.0, -18.0)];
    let near_zoom = Snapshot::fit_zoom(&near, 512, 512, 20, 18);
    let far_zoom = Snapshot::fit_zoom(&far, 512, 512, 20, 18);
    assert!(near_zoom > far_zoom, "{near_zoom} {far_zoom}");
    assert_eq!(Snapshot::fit_zoom(&[], 100, 100, 0, 9), 9);
}

#[test]
fn a_tile_path_uses_zoom_then_column_then_row() {
    let placed = Placed { tile_x: 3, tile_y: 4, zoom: 5, dest_x: 0.0, dest_y: 0.0 };
    assert_eq!(map_snapshot::tile_path(&placed), "5/3/4");
}
