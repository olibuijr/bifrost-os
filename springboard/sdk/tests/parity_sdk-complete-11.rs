//! Parity sdk-complete-11: headless tests for the preview panel keys, the embedded preview view, preview items,
//! the thumbnail generation cancel and the MIDI network session ends. No test needs a window, a device or a socket.

use bifrost_app::midi_network::{Command, ConnectionPolicy, Host, NetworkSession, SessionPacket};
use bifrost_app::peek::{ItemState, PanelKey, PanelKeyAction, PreviewItem, PreviewPanel, PreviewView, ViewStyle};
use bifrost_app::thumbnails::{Generation, GeneratorRequest, RepresentationType, Reply, State};

fn item(title: &str) -> PreviewItem {
    PreviewItem::new(format!("file:///docs/{title}"), title)
}

fn panel_with(titles: &[&str]) -> PreviewPanel {
    let mut panel = PreviewPanel::new();
    panel.set_items(titles.iter().map(|t| item(t)).collect());
    panel
}

#[test]
fn the_arrow_keys_step_through_the_list_without_wrapping() {
    let mut p = panel_with(&["a.txt", "b.txt", "c.txt"]);
    assert_eq!(p.handle_key(PanelKey::Right), PanelKeyAction::Stepped);
    assert_eq!(p.current_preview_item_index(), 1);
    p.handle_key(PanelKey::Right);
    p.handle_key(PanelKey::Right);
    assert_eq!(p.current_preview_item_index(), 2, "the last item holds at the end");
    p.handle_key(PanelKey::Left);
    p.handle_key(PanelKey::Left);
    p.handle_key(PanelKey::Left);
    assert_eq!(p.current_preview_item_index(), 0, "the first item holds at the start");
}

#[test]
fn space_and_escape_are_handed_back_without_moving_the_index() {
    let mut p = panel_with(&["a.txt", "b.txt"]);
    p.set_current_preview_item_index(1);
    assert_eq!(p.handle_key(PanelKey::Space), PanelKeyAction::Toggle);
    assert_eq!(p.handle_key(PanelKey::Escape), PanelKeyAction::Close);
    assert_eq!(p.current_preview_item_index(), 1);
    assert!(!p.is_visible(), "the app decides visibility through toggle and close");
}

#[test]
fn a_panel_pulls_its_items_from_a_source_and_keeps_states_per_item() {
    let mut p = PreviewPanel::new();
    p.load_from(&vec![item("a.txt"), item("b.txt")]);
    assert_eq!(p.items().len(), 2);
    assert!(p.set_display_state(1, ItemState::Loading));
    assert_eq!(p.display_state(1), Some(ItemState::Loading));
    assert_eq!(p.display_state(0), Some(ItemState::Ready));
}

#[test]
fn a_file_item_names_its_file_and_other_urls_name_none() {
    assert_eq!(PreviewItem::new("file:///home/u/notes.txt", "Notes").file_name(), Some("notes.txt"));
    assert_eq!(PreviewItem::new("/home/u/todo.md", "").file_name(), Some("todo.md"));
    assert_eq!(PreviewItem::new("file:///home/u/", "").file_name(), None);
    assert_eq!(PreviewItem::new("https://example.com/a.pdf", "").file_name(), None);
}

#[test]
fn changing_the_style_of_an_embedded_view_keeps_its_item_and_state() {
    let mut v = PreviewView::new(ViewStyle::Normal);
    v.set_preview_item(Some(item("a.txt")));
    v.set_display_state(ItemState::Failed);
    v.set_style(ViewStyle::Compact);
    assert_eq!(v.style(), ViewStyle::Compact);
    assert_eq!(v.display_state(), ItemState::Failed);
    assert_eq!(v.preview_item().map(|i| i.title.as_str()), Some("a.txt"));
}

fn answer(kind: RepresentationType, px: u32) -> Reply {
    let state = match kind {
        RepresentationType::Icon => State::Icon { type_id: "public.png".into(), badge: Some("PNG".into()) },
        _ => State::Pixels { image: "/cache/photo.raw".into(), w: 64, h: 64 },
    };
    Reply { file: "/docs/photo.png".into(), px, state }
}

#[test]
fn a_generation_delivers_each_wanted_representation_and_finishes_at_the_best() {
    let req = GeneratorRequest::new("/docs/photo.png", 64, 2.0, &[]).unwrap();
    let plan = req.plan();
    let mut g = Generation::new(&req);
    let mut delivered = Vec::new();
    for (kind, _, px) in &plan {
        let rep = g.accept(answer(*kind, *px)).expect("each wanted kind is delivered once");
        delivered.push(rep.kind);
    }
    assert_eq!(
        delivered,
        vec![RepresentationType::Icon, RepresentationType::LowQualityThumbnail, RepresentationType::Thumbnail]
    );
    assert!(g.is_finished());
}

#[test]
fn a_pending_answer_delivers_nothing_and_does_not_finish() {
    let req = GeneratorRequest::new("/docs/photo.png", 64, 2.0, &[]).unwrap();
    let mut g = Generation::new(&req);
    let px = req.plan()[0].2;
    let pending = Reply { file: "/docs/photo.png".into(), px, state: State::Pending };
    assert!(g.accept(pending).is_none());
    assert!(!g.is_finished());
}

#[test]
fn a_failed_answer_ends_the_generation_without_a_picture() {
    let req = GeneratorRequest::new("/docs/photo.png", 64, 2.0, &[]).unwrap();
    let mut g = Generation::new(&req);
    let px = req.plan()[0].2;
    let failed = Reply { file: "/docs/photo.png".into(), px, state: State::Failed("unreadable".into()) };
    assert!(g.accept(failed).is_none());
    assert!(g.is_finished());
}

#[test]
fn cancelling_a_generation_refuses_every_later_answer() {
    let req = GeneratorRequest::new("/docs/photo.png", 64, 2.0, &[]).unwrap();
    let plan = req.plan();
    let mut g = Generation::new(&req);
    g.cancel();
    assert!(g.is_finished());
    assert!(g.accept(answer(plan[0].0, plan[0].2)).is_none());
}

fn enabled_session(name: &str, ssrc: u32) -> NetworkSession {
    let mut s = NetworkSession::new(name, ssrc);
    s.enabled = true;
    s
}

/// Makes `me` and a host that accepts any invitation into a connection, using the invitation with `token`.
fn connect(me: &mut NetworkSession, host: &Host, token: u32) {
    let mut peer = enabled_session(&host.name, 9);
    peer.connection_policy = ConnectionPolicy::Anyone;
    let invite = me.invite(host, token).expect("an enabled session invites");
    let reply = peer.handle(&invite).expect("the peer answers an invitation");
    me.handle(&reply);
}

#[test]
fn leaving_a_connection_sends_an_end_packet_for_its_invitation() {
    let host = Host::new("Studio", "10.0.0.5", 5004);
    let mut me = enabled_session("Me", 1);
    connect(&mut me, &host, 42);
    assert_eq!(me.connections().len(), 1);
    let bytes = me.disconnect(&host).expect("the accepted invitation has an end packet");
    assert_eq!(
        SessionPacket::decode(&bytes),
        Ok(SessionPacket { command: Command::EndSession, initiator_token: 42, ssrc: 1, name: String::new() })
    );
    assert!(me.connections().is_empty());
    assert!(me.disconnect(&host).is_none(), "a second leave finds no connection");
}

#[test]
fn disabling_the_session_ends_every_connection_and_stops_invitations() {
    let mut me = enabled_session("Me", 1);
    let studio = Host::new("Studio", "10.0.0.5", 5004);
    let laptop = Host::new("Laptop", "10.0.0.6", 5004);
    connect(&mut me, &studio, 1);
    connect(&mut me, &laptop, 2);
    let packets = me.disable();
    assert_eq!(packets.len(), 2);
    assert!(packets.iter().all(|p| SessionPacket::decode(p).map(|s| s.command) == Ok(Command::EndSession)));
    assert!(!me.enabled);
    assert!(me.connections().is_empty());
    assert!(me.invite(&studio, 3).is_none(), "a disabled session sends no invitation");
}

#[test]
fn an_app_reads_a_csv_table_through_the_sdk() {
    let table = bifrost_app::csv::parse_table("symbol,close\nAAPL,190.5\n").unwrap();
    assert_eq!(table.value(0, "close"), Some("190.5"));
    assert_eq!(table.value(0, "symbol"), Some("AAPL"));
}
