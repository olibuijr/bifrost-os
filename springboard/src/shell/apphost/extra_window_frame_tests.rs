use super::*;
use crate::shell::{desk, AppId, Screen};

/// Every plain touch the shell has queued for the fixture app so far, with what produced it.
fn touch_sources(peer: &Channel) -> Vec<(TouchKind, bifrost_proto::TouchSource)> {
    let mut out = Vec::new();
    while let Recv::Packet(p, _) = peer.recv(false).unwrap() {
        if let ToApp::Touch { kind, source, .. } = ToApp::decode(&p).unwrap() {
            out.push((kind, source));
        }
    }
    out
}

/// A mouse press, drag and release reach the app as the pointer's (so its lists do not pan on the desktop); the drag
/// the compositor emulates for a wheel notch the app did not take reaches it as a finger's, so the notch still
/// scrolls, and the next press is the pointer's again.
#[test]
fn a_mouse_drag_is_the_pointer_and_the_drag_emulated_for_a_wheel_notch_is_a_finger() {
    use bifrost_proto::TouchSource::{Finger, Pointer};
    let mut sh = crate::shell::tests::desktop_shell();
    let peer = crate::shell::tests::fixture_app(&mut sh);
    sh.ext_touch(0, Event::Down(10.0, 10.0), 1000.0);
    sh.ext_touch(0, Event::Move(10.0, 60.0), 1016.0);
    sh.ext_touch(0, Event::Up(10.0, 60.0), 1032.0);
    assert_eq!(touch_sources(&peer), vec![(TouchKind::Down, Pointer), (TouchKind::Move, Pointer), (TouchKind::Up, Pointer)]);
    // a wheel notch the app did not ask for (`pointer_wheel`), then the down, move and up emulated for it
    sh.ext.get_mut(0).unwrap().as_mut().unwrap().keep_wheel_drag(2000.0);
    sh.ext_touch(0, Event::Down(10.0, 10.0), 2000.0);
    sh.ext_touch(0, Event::Move(10.0, 40.0), 2016.0);
    sh.ext_touch(0, Event::Up(10.0, 40.0), 2032.0);
    sh.ext_touch(0, Event::Down(10.0, 10.0), 3000.0);
    assert_eq!(touch_sources(&peer), vec![(TouchKind::Down, Finger), (TouchKind::Move, Finger), (TouchKind::Up, Finger), (TouchKind::Down, Pointer)]);
}

/// The colour the app paints into every window.
const FILL: u32 = 0xff33_6699;

/// Two buffers of `w` by `h` filled with `FILL`, handed to the host as a window's (or, with no id, the main window's).
fn buffers(host: &mut AppHost, window: Option<u32>, w: usize, h: usize) {
    let bytes = w * h * 4;
    let fds = vec![sys::shm_create("extra-window-a", bytes).unwrap(), sys::shm_create("extra-window-b", bytes).unwrap()];
    for fd in &fds {
        Map::new(fd, bytes).unwrap().pixels_mut().fill(FILL);
    }
    let (w, h) = (w as u32, h as u32);
    let msg = match window {
        Some(id) => ToShell::WindowBuffers { id, w, h, generation: 1 },
        None => ToShell::Buffers { w, h, generation: 1 },
    };
    host.handle(msg, fds, &mut Pumped::default());
}

#[test]
fn a_second_window_is_drawn_with_pixels_above_its_app_and_closing_it_takes_it_off_the_desk() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.wayland = true;
    sh.screen = Screen::Home;
    sh.anim = None;
    let (mine, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 64, h: 48, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", mine, None, cfg);
    let mut out = Pumped::default();
    // the order the app sends them in: its window, then the window's buffers and first frame
    buffers(&mut host, None, 64, 48);
    host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 64, 48]] }, Vec::new(), &mut out);
    host.handle(ToShell::WindowOpen { id: 1, title: "New Message".into(), w: 40.0, h: 30.0, position: None, flags: 0 }, Vec::new(), &mut out);
    buffers(&mut host, Some(1), 40, 30);
    host.handle(ToShell::WindowFrame { id: 1, buf: 0, damage: vec![[0, 0, 40, 30]] }, Vec::new(), &mut out);
    sh.ext.push(Some(host));
    let m = sh.desk_metrics();
    sh.desk.as_mut().unwrap().open(&m, AppId::External(0), (600, 400));
    sh.desk_sync_extra_windows();
    let (main, extra) = (AppId::External(0), AppId::ExternalWindow(0, 1));
    assert!(sh.desk.as_ref().unwrap().index_of(extra).is_some(), "the app's window is on the desk");
    assert!(!sh.desk_window_unmapped(extra), "the app committed a frame to it");
    let (w, h) = (sh.w as usize, sh.h as usize);
    let mut px = vec![0u32; w * h];
    sh.dirty = true;
    sh.draw_desk_windows(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1000.0);
    let frames = sh.take_wayland_windows();
    // an app that is not installed has no catalogue entry, so its main window has no id of its own: the desk draws it as 0
    let id_of = |app: AppId| desk::win_id(app).unwrap_or(0);
    let at = |app: AppId| frames.iter().position(|f| f.id == id_of(app));
    let (parent, window) = (at(main).expect("the app's own window is drawn"), at(extra).expect("a frame is drawn for the second window"));
    assert!(window > parent, "the second window is above its app's window");
    assert!(frames[window].pixels.as_ref().is_some_and(|p| p.iter().any(|&c| c == FILL)), "with the app's pixels in it");
    // closed: it is off the desk, the app is told, and the next frame has no window for it
    sh.desk_close_extra_window(0, 1);
    assert!(sh.desk.as_ref().unwrap().index_of(extra).is_none());
    assert!(sh.ext[0].as_ref().unwrap().windows.of_app("fixture").is_empty(), "the app's window table dropped it");
    sh.dirty = true;
    sh.draw_desk_windows(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 2000.0);
    let after = sh.take_wayland_windows();
    assert!(after.iter().all(|f| f.id != id_of(extra)), "no frame for the closed window");
    assert!(after.iter().any(|f| f.id == id_of(main)), "the app's own window stays");
}

/// E2E F4: a cascaded window is cascaded only when it first opens; one the person moved comes back where they left
/// it when the desk shows it again, instead of jumping back to a cascade slot.
#[test]
fn a_moved_cascaded_window_keeps_its_position_when_shown_again() {
    let _lock = crate::test_support::global_lock();
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let (mine, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 64, h: 48, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", mine, None, cfg);
    let flags = bifrost_proto::window_flags::CASCADE;
    host.handle(ToShell::WindowOpen { id: 1, title: "Viewer".into(), w: 40.0, h: 30.0, position: None, flags }, Vec::new(), &mut Pumped::default());
    sh.ext.push(Some(host));
    let m = sh.desk_metrics();
    sh.desk.as_mut().unwrap().open(&m, AppId::External(0), (600, 400));
    sh.desk_sync_extra_windows();
    let extra = AppId::ExternalWindow(0, 1);
    let cascades = |sh: &crate::shell::Shell| sh.ext[0].as_ref().unwrap().windows.of_app("fixture").iter().any(|w| w.id == 1 && w.cascade);
    assert!(!cascades(&sh), "the cascade flag is cleared once the window is placed");
    // the person drags it to the work area's top left corner
    let a = desk::Desk::area(&m);
    let moved = (a.x + 5, a.y + 7);
    {
        let d = sh.desk.as_mut().unwrap();
        let i = d.index_of(extra).expect("the viewer is on the desk");
        assert_ne!((d.wins[i].r.x, d.wins[i].r.y), moved, "the cascade put it somewhere else");
        d.wins[i].r.x = moved.0;
        d.wins[i].r.y = moved.1;
        // hidden: the desk drops it while the app's table keeps it
        d.close(i);
    }
    sh.desk_sync_extra_windows();
    let d = sh.desk.as_ref().unwrap();
    let i = d.index_of(extra).expect("shown again");
    assert_eq!((d.wins[i].r.x, d.wins[i].r.y), moved, "it comes back where the person left it");
}

/// E2E-11/12: an app that reallocates its buffers (every step of a live resize) keeps its window on the desk until it
/// draws into them: the window is not unmapped and its frame (last pixels, last size) is still sent, so the toplevel
/// is not destroyed under the pointer (which ended the resize) and the window behind does not take the focus.
#[test]
fn an_app_between_buffer_sets_keeps_its_window() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.wayland = true;
    sh.screen = Screen::Home;
    sh.anim = None;
    let (mine, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 64, h: 48, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", mine, None, cfg);
    assert!(!host.between_buffers(), "an app that never drew is not between buffers");
    buffers(&mut host, None, 64, 48);
    host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 64, 48]] }, Vec::new(), &mut Pumped::default());
    sh.ext.push(Some(host));
    let m = sh.desk_metrics();
    let main = AppId::External(0);
    sh.desk.as_mut().unwrap().open(&m, main, (600, 400));
    let (w, h) = (sh.w as usize, sh.h as usize);
    let mut px = vec![0u32; w * h];
    sh.dirty = true;
    sh.draw_desk_windows(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1000.0);
    let id = desk::win_id(main).unwrap_or(0);
    let first = sh.take_wayland_windows();
    let drawn = first.iter().find(|f| f.id == id).expect("the window is drawn");
    let size = (drawn.width, drawn.height);
    // the window was resized: the app sends a new buffer set and has not drawn into it yet
    let r = sh.desk.as_ref().unwrap().wins[0].r;
    sh.desk.as_mut().unwrap().wins[0].r.w = r.w - 40;
    buffers(sh.ext[0].as_mut().unwrap(), None, 80, 60);
    assert!(sh.ext[0].as_ref().unwrap().between_buffers());
    assert!(!sh.desk_window_unmapped(main), "the window stays mapped between buffer sets");
    sh.dirty = true;
    sh.draw_desk_windows(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1100.0);
    let next = sh.take_wayland_windows();
    let kept = next.iter().find(|f| f.id == id).expect("a frame is still sent: the toplevel is kept");
    assert_eq!((kept.width, kept.height), size, "at the size of the pixels it last presented");
    assert!(kept.pixels.is_none(), "the window server keeps showing the last pixels");
}
