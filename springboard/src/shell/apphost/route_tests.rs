use super::*;

fn cfg() -> Configure {
    Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false }
}

#[test]
fn the_route_is_direct_only_under_bifrost_wm_with_the_switch_on() {
    assert_eq!(route_for(Some("1"), Some("bifrost-wm")), Route::Direct);
    assert_eq!(route_for(Some(" 1\n"), Some("bifrost-wm\n")), Route::Direct);
    assert_eq!(route_for(Some("1"), Some("other-wm")), Route::Shm, "any other compositor keeps the shared-memory path");
    assert_eq!(route_for(None, Some("bifrost-wm")), Route::Shm, "no switch, no direct path");
    assert_eq!(route_for(Some("0"), Some("bifrost-wm")), Route::Shm);
    assert_eq!(route_for(Some("1"), None), Route::Shm, "no /etc/bifrost/compositor, no direct path");
    assert_eq!(route_for(Some("true"), Some("bifrost-wm")), Route::Shm, "only the literal 1 switches it on");
}

#[test]
fn a_direct_app_that_sends_shared_memory_buffers_falls_back() {
    assert_eq!(route_after_message(Route::Direct, true), Route::Shm);
    assert_eq!(route_after_message(Route::Direct, false), Route::Direct, "a frame or any other message keeps the route");
    assert_eq!(route_after_message(Route::Shm, true), Route::Shm);
    assert_eq!(route_after_message(Route::Shm, false), Route::Shm);
}

#[test]
fn only_the_shared_memory_route_expects_frames() {
    assert!(Route::Shm.expects_frames());
    assert!(!Route::Direct.expects_frames());
}

#[test]
fn the_window_of_an_app_is_its_first_toplevel_with_that_app_id() {
    let windows = [(3, "is.a"), (7, "is.b"), (9, "is.b")];
    assert_eq!(foreign_window_of(windows, "is.b"), Some(7), "the first listed toplevel wins");
    assert_eq!(foreign_window_of(windows, "is.a"), Some(3));
    assert_eq!(foreign_window_of(windows, "is.c"), None, "no window while the app is still starting");
    assert_eq!(foreign_window_of([(1, "is.bb")], "is.b"), None, "app ids match whole, not by prefix");
    assert_eq!(foreign_window_of(Vec::<(u32, &str)>::new(), "is.a"), None);
}

#[test]
fn a_direct_app_thumbnail_is_a_capture_once_ready_and_its_icon_before() {
    assert_eq!(thumbnail(Route::Shm, true, false), Thumb::Frame);
    assert_eq!(thumbnail(Route::Shm, true, true), Thumb::Frame, "the shared-memory frame is kept by the shell");
    assert_eq!(thumbnail(Route::Shm, false, true), Thumb::Icon, "no frame yet");
    assert_eq!(thumbnail(Route::Direct, true, true), Thumb::Capture);
    assert_eq!(thumbnail(Route::Direct, true, false), Thumb::Icon, "the shell has no frame for a direct app");
    assert_eq!(thumbnail(Route::Direct, false, false), Thumb::Icon);
}

#[test]
fn only_the_answer_to_a_close_request_on_the_direct_route_quits_the_app() {
    assert!(order_out_quits(Route::Direct, true));
    assert!(!order_out_quits(Route::Direct, false), "File > Close of the last document keeps the app running");
    assert!(!order_out_quits(Route::Shm, true), "the shell drops the window itself on the shared-memory route");
}

#[test]
fn a_direct_host_is_demoted_by_buffers_once_and_ignores_its_frames() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    host.route = Route::Direct;
    let mut out = Pumped::default();
    host.handle(ToShell::Frame { buf: 0, damage: vec![] }, Vec::new(), &mut out);
    assert_eq!(host.route(), Route::Direct, "a frame from a direct app does not demote it");
    assert!(!host.take_fallback());
    host.handle(ToShell::Buffers { w: 4, h: 3, generation: 1 }, Vec::new(), &mut out);
    assert_eq!(host.route(), Route::Shm);
    assert!(host.take_fallback(), "the fallback is reported once");
    assert!(!host.take_fallback());
    host.handle(ToShell::Buffers { w: 4, h: 3, generation: 1 }, Vec::new(), &mut out);
    assert!(!host.take_fallback(), "a shared-memory app is never demoted again");
}

pub(super) fn fixture_host() -> (AppHost, Channel) {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    (AppHost::with_channel("fixture", channel, None, cfg), peer)
}

#[test]
fn short_memfds_are_refused_and_good_ones_sealed() {
    let (mut host, _peer) = fixture_host();
    let mut out = Pumped::default();
    host.handle(ToShell::Buffers { w: 8192, h: 8192, generation: 1 }, vec![sys::shm_create("s1", 4096).unwrap(), sys::shm_create("s2", 4096).unwrap()], &mut out);
    assert!(host.bufs.is_empty(), "4 KiB files cannot back 8192 x 8192");
    let a = sys::shm_create("g1", 48).unwrap();
    let b = sys::shm_create("g2", 48).unwrap();
    host.handle(ToShell::Buffers { w: 4, h: 3, generation: 7 }, vec![a, b], &mut out);
    assert_eq!(host.bufs.len(), 2);
    assert_eq!(host.generation, 7);
}

#[test]
fn a_stale_release_never_reaches_the_new_set() {
    let (mut host, peer) = fixture_host();
    let mut out = Pumped::default();
    let set = |n: &str| vec![sys::shm_create(n, 48).unwrap(), sys::shm_create(n, 48).unwrap()];
    host.handle(ToShell::Buffers { w: 4, h: 3, generation: 1 }, set("a"), &mut out);
    host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 4, 3]] }, vec![], &mut out);
    host.handle(ToShell::Frame { buf: 1, damage: vec![[0, 0, 4, 3]] }, vec![], &mut out);
    host.handle(ToShell::Buffers { w: 4, h: 3, generation: 2 }, set("b"), &mut out);
    host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 4, 3]] }, vec![], &mut out);
    let mut px = vec![0; 12];
    host.blit(&mut Surface { w: 4, h: 3, stride: 4, px: &mut px, y0: 0 }, true);
    while let Ok(Recv::Packet(p, _)) = peer.recv(false) {
        if let Ok(ToApp::Release { generation, .. }) = ToApp::decode(&p) {
            assert_eq!(generation, 2, "only releases of the current set go out");
        }
    }
}

#[test]
fn an_app_that_never_reads_does_not_block_the_shell() {
    let (mut host, _peer) = fixture_host();
    for _ in 0..100_000 {
        host.send(&ToApp::Resume);
        if !host.running() {
            break;
        }
    }
    assert!(host.backlog.len() <= MAX_BACKLOG);
}

#[test]
fn a_flooding_app_is_read_in_bounded_passes() {
    let (mut host, peer) = fixture_host();
    for i in 0..10u32 {
        peer.send(&ToShell::Notify { title: i.to_string(), body: String::new() }.encode(), &[]).unwrap();
    }
    let first = host.pump_up_to(3);
    assert_eq!(first.requests.len(), 3, "one pass reads at most its budget");
    let second = host.pump_up_to(3);
    assert_eq!(second.requests.len(), 3);
    let rest = host.pump_up_to(usize::MAX);
    assert_eq!(rest.requests.len(), 4, "the rest of the flood is read on the next passes, in order");
    assert_eq!(rest.requests.first(), Some(&ToShell::Notify { title: "6".into(), body: String::new() }));
    assert!(matches!(peer.recv(false), Ok(Recv::Empty)));
}

#[test]
fn a_window_level_of_an_extra_window_is_kept_in_the_table() {
    let (mut host, _peer) = fixture_host();
    let mut out = Pumped::default();
    host.handle(ToShell::WindowOpen { id: 2, title: "CPU".into(), w: 300.0, h: 200.0, position: None, flags: 0 }, Vec::new(), &mut out);
    host.handle(ToShell::WindowLevelOf { id: 2, level: bifrost_proto::WindowLevel::Floating }, Vec::new(), &mut out);
    assert_eq!(host.windows.floating_ids(&host.id), vec![2]);
    assert_eq!(host.window_level, bifrost_proto::WindowLevel::Normal, "the main window keeps its level");
    host.handle(ToShell::WindowLevelOf { id: 0, level: bifrost_proto::WindowLevel::Floating }, Vec::new(), &mut out);
    assert!(host.windows.floating_ids(&host.id).contains(&2));
    host.kill();
}

#[test]
fn a_window_level_request_is_kept_for_the_app() {
    let (mut host, _peer) = fixture_host();
    assert_eq!(host.window_level, bifrost_proto::WindowLevel::Normal);
    let mut out = Pumped::default();
    host.handle(ToShell::WindowLevel(bifrost_proto::WindowLevel::Floating), Vec::new(), &mut out);
    assert_eq!(host.window_level, bifrost_proto::WindowLevel::Floating);
    assert!(out.requests.is_empty(), "the level is not passed on to the shell's requests");
    host.handle(ToShell::WindowLevel(bifrost_proto::WindowLevel::Normal), Vec::new(), &mut out);
    assert_eq!(host.window_level, bifrost_proto::WindowLevel::Normal);
    host.kill();
}

/// An app asks for the main window's close control with `CloseRequests`; until it does, the shell closes as before.
#[test]
fn an_app_opts_in_to_close_requests_with_the_message() {
    let (mut host, _peer) = fixture_host();
    let mut out = Pumped::default();
    assert!(!host.close_requests, "no opt-in until the app says so");
    host.handle(ToShell::CloseRequests { on: true }, Vec::new(), &mut out);
    assert!(host.close_requests);
    host.handle(ToShell::CloseRequests { on: false }, Vec::new(), &mut out);
    assert!(!host.close_requests);
    host.kill();
}

#[test]
fn quitting_never_blocks_on_an_app_that_stopped_reading() {
    let (mut host, _peer) = fixture_host();
    // fill the socket: nothing reads the peer end
    while host.chan.try_send(&[0u8; 64], &[]).is_ok() {}
    host.sudden = false;
    let _ = host.begin_quit(0.0);
    host.kill();
}
