//! Parity sdk-complete-31: headless tests for several windows of one app (the scene sessions that the shell opens and
//! focuses) and for the authenticator data of a passkey ceremony. No window is drawn, no shell runs and no authenticator
//! is called.

use bifrost_app::digest::sha256;
use bifrost_app::kit::Image;
use bifrost_app::lazy_image::{filter, LazyImage, RenderContext};
use bifrost_app::passkey_authenticator_data::{assertion_signed_bytes, AuthenticatorData, Error as AuthDataError};
use bifrost_app::passkey_client_data::{Ceremony, ClientData};
use bifrost_app::scene_session::{SceneDelegate, SceneError, SceneEvent, SceneSession, SceneSessions, SceneState};
use bifrost_app::view::{anchors, solve_constraints, Constraints, Item, Spec, ViewInfo};
use bifrost_ui::constraint::Rel;
use bifrost_ui::Rect;

// ---- windows: several scenes of one app (bifrost_app::scene_session) ----

/// Records every callback as (scene identifier, event), in the order they arrive.
#[derive(Default)]
struct EventLog(Vec<(String, SceneEvent)>);

impl SceneDelegate for EventLog {
    fn scene_event(&mut self, scene: &SceneSession, event: SceneEvent) {
        self.0.push((scene.persistent_id.clone(), event));
    }
}

fn state_of(sessions: &SceneSessions, id: &str) -> SceneState {
    sessions.get(id).expect("the window is registered").state
}

#[test]
fn a_window_opened_while_the_app_is_hidden_connects_in_the_background() {
    let mut sessions = SceneSessions::new(4);
    let mut log = EventLog::default();
    let id = sessions.request(&mut log, None, Vec::new()).unwrap();
    assert_eq!(id, "scene-1");
    assert_eq!(state_of(&sessions, &id), SceneState::Background);
    assert_eq!(
        log.0,
        vec![(id.clone(), SceneEvent::WillConnect), (id.clone(), SceneEvent::DidEnterBackground)],
        "a background connection gets no foreground callbacks"
    );
}

#[test]
fn several_windows_of_one_app_are_listed_in_the_order_they_were_requested() {
    let mut sessions = SceneSessions::new(4);
    let mut log = EventLog::default();
    let info = vec![("document".to_string(), "draft.txt".to_string())];
    for _ in 0..3 {
        sessions.request(&mut log, None, info.clone()).unwrap();
    }
    let ids: Vec<&str> = sessions.list().iter().map(|s| s.persistent_id.as_str()).collect();
    assert_eq!(ids, ["scene-1", "scene-2", "scene-3"]);
    assert_eq!(sessions.get("scene-2").unwrap().user_info, info, "each window keeps the information it was opened with");
}

#[test]
fn a_window_restored_after_a_relaunch_keeps_its_saved_identifier() {
    let mut sessions = SceneSessions::new(4);
    let mut log = EventLog::default();
    let restored = sessions.request(&mut log, Some("saved-7"), Vec::new()).unwrap();
    assert_eq!(restored, "saved-7");
    let fresh = sessions.request(&mut log, None, Vec::new()).unwrap();
    assert_eq!(fresh, "scene-1", "a restored identifier does not use up a generated one");
}

#[test]
fn the_front_window_has_focus_while_the_app_is_in_front_and_focus_can_move() {
    let mut sessions = SceneSessions::new(4);
    let mut log = EventLog::default();
    let first = sessions.request(&mut log, None, Vec::new()).unwrap();
    let second = sessions.request(&mut log, None, Vec::new()).unwrap();
    sessions.set_foreground(&mut log, true);
    assert_eq!(state_of(&sessions, &first), SceneState::ForegroundActive);
    assert_eq!(state_of(&sessions, &second), SceneState::ForegroundInactive);
    assert_eq!(sessions.focused().unwrap().persistent_id, first);
    assert!(log.0.contains(&(first.clone(), SceneEvent::DidBecomeActive)));

    sessions.focus(&mut log, &second).unwrap();
    assert_eq!(state_of(&sessions, &second), SceneState::ForegroundActive);
    assert_eq!(state_of(&sessions, &first), SceneState::ForegroundInactive);
    assert!(log.0.contains(&(first.clone(), SceneEvent::WillResignActive)));
}

#[test]
fn leaving_the_app_sends_every_window_to_the_background() {
    let mut sessions = SceneSessions::new(4);
    let mut log = EventLog::default();
    sessions.request(&mut log, None, Vec::new()).unwrap();
    sessions.request(&mut log, None, Vec::new()).unwrap();
    sessions.set_foreground(&mut log, true);
    sessions.set_foreground(&mut log, false);
    assert!(sessions.list().iter().all(|s| s.state == SceneState::Background));
    assert!(sessions.focused().is_none(), "a background app has no focused window");
}

#[test]
fn the_registry_refuses_a_window_beyond_its_limit() {
    let mut sessions = SceneSessions::new(2);
    let mut log = EventLog::default();
    sessions.request(&mut log, None, Vec::new()).unwrap();
    sessions.request(&mut log, None, Vec::new()).unwrap();
    let refused = sessions.request(&mut log, None, Vec::new());
    assert_eq!(refused, Err(SceneError::TooManyScenes(2)));
    assert_eq!(SceneError::TooManyScenes(2).to_string(), "an app holds at most 2 windows");
    assert_eq!(sessions.list().len(), 2);
}

#[test]
fn closing_the_focused_window_moves_focus_to_the_last_window_and_disconnects_it() {
    let mut sessions = SceneSessions::new(4);
    let mut log = EventLog::default();
    for _ in 0..3 {
        sessions.request(&mut log, None, Vec::new()).unwrap();
    }
    sessions.set_foreground(&mut log, true);
    let closed = sessions.close(&mut log, "scene-1").unwrap();
    assert_eq!(closed.persistent_id, "scene-1");
    assert_eq!(closed.state, SceneState::Unattached);
    assert_eq!(sessions.focused().unwrap().persistent_id, "scene-3");
    assert!(log.0.contains(&("scene-1".to_string(), SceneEvent::DidDisconnect)));
    assert_eq!(sessions.list().len(), 2);
}

#[test]
fn focusing_or_closing_an_unknown_window_changes_nothing() {
    let mut sessions = SceneSessions::new(4);
    let mut log = EventLog::default();
    sessions.request(&mut log, None, Vec::new()).unwrap();
    assert_eq!(sessions.focus(&mut log, "missing"), Err(SceneError::UnknownScene("missing".to_string())));
    assert_eq!(sessions.close(&mut log, "missing").unwrap_err(), SceneError::UnknownScene("missing".to_string()));
    assert_eq!(sessions.list().len(), 1);
}

// ---- passkeys: authenticator data (bifrost_app::passkey_authenticator_data) ----

/// Authenticator data with the given flags and counter and no credential data.
fn authenticator_bytes(rp_id: &str, flags: u8, counter: u32) -> Vec<u8> {
    let mut out = sha256(rp_id.as_bytes()).to_vec();
    out.push(flags);
    out.extend_from_slice(&counter.to_be_bytes());
    out
}

#[test]
fn an_assertion_reads_back_its_relying_party_flags_and_counter() {
    let bytes = authenticator_bytes("example.com", 0x05, 42);
    let parsed = AuthenticatorData::parse(&bytes).unwrap();
    assert!(parsed.rp_id_matches("example.com"));
    assert!(parsed.user_present && parsed.user_verified);
    assert_eq!(parsed.sign_count, 42);
    assert!(parsed.attested.is_none());
}

#[test]
fn the_signature_base_of_an_assertion_covers_the_client_data_it_answers() {
    let challenge = [7u8; 16];
    let client = ClientData::new(Ceremony::Get, &challenge, "https://example.com", false).unwrap();
    let json = client.to_json();
    let auth = authenticator_bytes("example.com", 0x05, 1);
    let signed = assertion_signed_bytes(&auth, json.as_bytes());
    assert_eq!(&signed[..auth.len()], auth.as_slice());
    assert_eq!(&signed[auth.len()..], &sha256(json.as_bytes())[..]);
    assert!(AuthenticatorData::parse(&signed[..auth.len()]).unwrap().rp_id_matches("example.com"));
}

#[test]
fn a_truncated_or_padded_authenticator_data_is_refused() {
    let mut padded = authenticator_bytes("example.com", 0x01, 0);
    padded.push(0);
    assert_eq!(AuthenticatorData::parse(&padded), Err(AuthDataError::TrailingBytes));
    let short = authenticator_bytes("example.com", 0x01, 0);
    assert_eq!(AuthenticatorData::parse(&short[..20]), Err(AuthDataError::TooShort));
}

// ---- layout: priorities and layout guides of Auto Layout (bifrost_app::view autolayout) ----

fn view(id: u64) -> ViewInfo {
    ViewInfo { id, frame: Rect::default(), intrinsic: (0.0, 0.0), hugging: (250, 250), compression: (750, 750) }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn a_required_width_wins_over_a_lower_priority_width_without_being_reported() {
    let mut cs = Constraints::new();
    cs.add(Spec::fixed(anchors(1).width(), Rel::Eq, 120.0));
    cs.add(Spec::fixed(anchors(1).width(), Rel::Eq, 200.0).priority(10));
    let solution = solve_constraints(&cs, Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 }, 0.0, 20.0, &[view(1)]);
    assert!(near(solution.frames[&1].w, 120.0), "{:?}", solution.frames[&1]);
    assert!(solution.broken.is_empty(), "a lower priority preference gives way silently");
}

#[test]
fn a_priority_is_kept_between_one_and_the_required_value() {
    assert_eq!(Spec::fixed(anchors(1).width(), Rel::Eq, 1.0).priority(0).priority, 1);
    assert_eq!(Spec::fixed(anchors(1).width(), Rel::Eq, 1.0).priority(5000).priority, 1000);
    assert_eq!(Spec::fixed(anchors(1).width(), Rel::Eq, 1.0).priority, 1000, "a fixed constraint is required by default");
}

#[test]
fn a_layout_guide_spaces_two_views_apart_and_they_share_the_rest_of_the_width() {
    let mut cs = Constraints::new();
    let gap = cs.guide();
    cs.add(Spec::eq(anchors(1).leading(), anchors(Item::Window).leading()));
    cs.add(Spec::eq(anchors(2).trailing(), anchors(Item::Window).trailing()));
    cs.add(Spec::eq(anchors(1).trailing(), anchors(gap).leading()));
    cs.add(Spec::eq(anchors(2).leading(), anchors(gap).trailing()));
    cs.add(Spec::fixed(anchors(gap).width(), Rel::Eq, 20.0));
    cs.add(Spec::eq(anchors(1).width(), anchors(2).width()));
    cs.add(Spec::fixed(anchors(1).height(), Rel::Eq, 30.0));
    cs.add(Spec::fixed(anchors(2).height(), Rel::Eq, 30.0));
    let bounds = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
    let solution = solve_constraints(&cs, bounds, 0.0, 20.0, &[view(1), view(2)]);
    let (left, right) = (solution.frames[&1], solution.frames[&2]);
    assert!(near(left.w, 190.0) && near(right.w, 190.0), "{left:?} {right:?}");
    assert!(near(left.x, 0.0) && near(right.x, 210.0), "{left:?} {right:?}");
    assert!(solution.broken.is_empty());
}

// ---- core image: lazily evaluated images and their render context (bifrost_app::lazy_image) ----

/// One pixel, alpha 255, red 200, green 100, blue 52 (packed red in the low byte).
const PIXEL: u32 = 0xff34_64c8;

#[test]
fn an_app_image_without_filters_renders_the_source_pixels() {
    let source = Image { w: 2, h: 1, px: vec![PIXEL, 0] };
    let image = LazyImage::new(source.clone());
    assert_eq!(image.filter_count(), 0);
    assert_eq!(image.extent(), (2, 1));
    assert_eq!(RenderContext::new().render(&image), Some(source));
}

#[test]
fn a_named_exposure_filter_with_its_parameter_darkens_the_render() {
    let exposure = filter("exposure", &[("ev", -1.0)]).expect("exposure takes an ev parameter");
    let image = LazyImage::new(Image { w: 1, h: 1, px: vec![PIXEL] }).apply(exposure);
    // 200, 100 and 52 halve to 100, 50 and 26.
    assert_eq!(RenderContext::new().render(&image).unwrap().px, vec![0xff1a_3264]);
}

#[test]
fn an_unknown_filter_or_parameter_is_not_built() {
    assert!(filter("no-such-filter", &[]).is_none());
    assert!(filter("exposure", &[("no-such-parameter", 1.0)]).is_none());
}

#[test]
fn an_edited_copy_leaves_the_image_it_came_from_unchanged() {
    let context = RenderContext::new();
    let base = LazyImage::new(Image { w: 1, h: 1, px: vec![PIXEL] });
    let darker = base.clone().apply(filter("exposure", &[("ev", -1.0)]).unwrap());
    assert_eq!(base.filter_count(), 0);
    assert_eq!(darker.filter_count(), 1);
    assert_eq!(context.render(&base).unwrap().px, vec![PIXEL]);
}

#[test]
fn a_pixel_buffer_of_the_wrong_length_does_not_render() {
    let short = LazyImage::new(Image { w: 2, h: 2, px: vec![0; 3] });
    assert!(RenderContext::new().render(&short).is_none());
}
