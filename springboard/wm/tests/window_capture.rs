//! End to end: `bifrost_window_capture_v1` on the headless window server.
//!
//! - A client that is not the shell does not see the global at all (the test client runs as the same non-root user as
//!   the window server, which is how an app looks to the window server).
//! - With the shell's uid named (`BIFROST_WM_SHELL_UID`, the development rule) the client binds it, captures its own
//!   toplevel scaled into a smaller buffer, hears `unchanged` for a picture it already holds and `ready` again after
//!   the window committed new content.
//!
//! One test function: the shell rule is read from the environment when the window server starts, so the two parts run in
//! order inside one process.

use std::{io::Write, os::fd::AsFd, os::unix::fs::{FileExt, MetadataExt}, os::unix::net::UnixStream, path::PathBuf, time::Duration};

use bifrost_wm::headless::Compositor;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

#[allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables, non_upper_case_globals, non_snake_case, unused_imports, missing_docs, clippy::all)]
mod capture_client {
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols_wlr::foreign_toplevel::v1::client::*;

    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols_wlr::foreign_toplevel::v1::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocol/bifrost-window-capture-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocol/bifrost-window-capture-v1.xml");
}
use capture_client::{bifrost_window_capture_frame_v1 as frame_mod, bifrost_window_capture_manager_v1 as manager_mod};

#[derive(Debug, PartialEq)]
enum Answer {
    Ready { width: u32, height: u32, serial: u32 },
    Unchanged { serial: u32 },
    Failed { reason: u32 },
}

#[derive(Default)]
struct Client {
    configured: bool,
    answers: Vec<Answer>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(_: &mut Self, base: &xdg_wm_base::XdgWmBase, e: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = e {
            base.pong(serial);
        }
    }
}
impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(s: &mut Self, xs: &xdg_surface::XdgSurface, e: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = e {
            xs.ack_configure(serial);
            s.configured = true;
        }
    }
}
impl Dispatch<frame_mod::BifrostWindowCaptureFrameV1, ()> for Client {
    fn event(s: &mut Self, _: &frame_mod::BifrostWindowCaptureFrameV1, e: frame_mod::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        s.answers.push(match e {
            frame_mod::Event::Ready { width, height, serial } => Answer::Ready { width, height, serial },
            frame_mod::Event::Unchanged { serial } => Answer::Unchanged { serial },
            frame_mod::Event::Failed { reason } => Answer::Failed { reason },
            _ => return,
        });
    }
}
macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for Client {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(wl_compositor::WlCompositor, wl_surface::WlSurface, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, xdg_toplevel::XdgToplevel,
        manager_mod::BifrostWindowCaptureManagerV1);

/// A `w` x `h` ARGB8888 shm buffer filled with `px`, and the file behind it to read back.
fn buffer(shm: &wl_shm::WlShm, qh: &QueueHandle<Client>, w: i32, h: i32, px: u32) -> (wl_buffer::WlBuffer, std::fs::File) {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).expect("XDG_RUNTIME_DIR");
    let mut file = tempfile::tempfile_in(dir).expect("shm file");
    let bytes: Vec<u8> = (0..w * h).flat_map(|_| px.to_le_bytes()).collect();
    file.write_all(&bytes).expect("write shm");
    let pool = shm.create_pool(file.as_fd(), w * h * 4, qh, ());
    let b = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    (b, file)
}

fn connect(socket: &str) -> Connection {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).expect("XDG_RUNTIME_DIR");
    Connection::from_socket(UnixStream::connect(dir.join(socket)).expect("connect")).expect("connection")
}

fn lists_capture_global(socket: &str) -> bool {
    let conn = connect(socket);
    let (globals, _queue) = registry_queue_init::<Client>(&conn).expect("registry");
    globals.contents().with_list(|list| list.iter().any(|g| g.interface == "bifrost_window_capture_manager_v1"))
}

fn roundtrips(queue: &mut wayland_client::EventQueue<Client>, client: &mut Client, n: usize) {
    for _ in 0..n {
        queue.roundtrip(client).expect("roundtrip");
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn only_the_shell_sees_the_global_and_a_capture_fits_the_window_into_the_buffer() {
    // part 1: an app (same user, not root) does not see the global
    let apps = Compositor::start("wayland-bfwm-wcap-app", 800, 600).expect("headless compositor");
    assert!(!lists_capture_global(&apps.socket), "a client that is not the shell must not see bifrost_window_capture_manager_v1");
    drop(apps);

    // part 2: the shell is named by uid, as in a development session where both run as one user
    let own = std::fs::metadata("/proc/self").expect("/proc/self").uid();
    std::env::set_var("BIFROST_WM_SHELL_UID", own.to_string());
    let wm = Compositor::start("wayland-bfwm-wcap-shell", 800, 600).expect("headless compositor");
    assert!(lists_capture_global(&wm.socket), "the shell sees the global");

    let conn = connect(&wm.socket);
    let (globals, mut queue) = registry_queue_init::<Client>(&conn).expect("registry");
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
    let capture: manager_mod::BifrostWindowCaptureManagerV1 = globals.bind(&qh, 1..=1, ()).expect("bifrost_window_capture_manager_v1");
    let mut client = Client::default();

    // a red 400x200 window
    let win = compositor.create_surface(&qh, ());
    let xs = wm_base.get_xdg_surface(&win, &qh, ());
    let top = xs.get_toplevel(&qh, ());
    top.set_title("capture".into());
    win.commit();
    for _ in 0..50 {
        queue.roundtrip(&mut client).expect("roundtrip");
        if client.configured {
            break;
        }
    }
    assert!(client.configured, "the window was configured");
    let (red, _red_file) = buffer(&shm, &qh, 400, 200, 0xffff_0000);
    win.attach(Some(&red), 0, 0);
    win.damage_buffer(0, 0, 400, 200);
    win.commit();
    roundtrips(&mut queue, &mut client, 6);

    // a capture buffer of 100x100: the 400x200 window comes back as 100x50, from the top-left corner
    let (small, small_file) = buffer(&shm, &qh, 100, 100, 0);
    let _frame = capture.capture_surface(&win, &small, 0, &qh, ());
    roundtrips(&mut queue, &mut client, 2);
    let first = match client.answers.as_slice() {
        [Answer::Ready { width: 100, height: 50, serial }] => *serial,
        other => panic!("expected one ready 100x50, got {other:?}"),
    };
    let mut px = [0u8; 4];
    small_file.read_exact_at(&mut px, 0).expect("read back");
    assert_eq!(px, [0, 0, 255, 255], "red, opaque, in ARGB8888 memory order (B, G, R, A)");
    small_file.read_exact_at(&mut px, 100 * 49 * 4).expect("read back the last picture row");
    assert_eq!(px, [0, 0, 255, 255], "the picture fills its 100x50 area");
    small_file.read_exact_at(&mut px, 100 * 50 * 4).expect("read back below the picture");
    assert_eq!(px, [0, 0, 0, 0], "the rest of the buffer is not touched");

    // the shell holds that picture: nothing is drawn
    client.answers.clear();
    let _frame = capture.capture_surface(&win, &small, first, &qh, ());
    roundtrips(&mut queue, &mut client, 2);
    assert_eq!(client.answers, vec![Answer::Unchanged { serial: first }]);

    // the window commits new content: the serial moves and the picture is drawn again
    client.answers.clear();
    let (blue, _blue_file) = buffer(&shm, &qh, 400, 200, 0xff00_00ff);
    win.attach(Some(&blue), 0, 0);
    win.damage_buffer(0, 0, 400, 200);
    win.commit();
    roundtrips(&mut queue, &mut client, 4);
    let _frame = capture.capture_surface(&win, &small, first, &qh, ());
    roundtrips(&mut queue, &mut client, 2);
    match client.answers.as_slice() {
        [Answer::Ready { width: 100, height: 50, serial }] => assert_ne!(*serial, first, "a new commit is a new serial"),
        other => panic!("expected a new picture, got {other:?}"),
    }
    small_file.read_exact_at(&mut px, 0).expect("read back");
    assert_eq!(px, [255, 0, 0, 255], "blue now");

    // the smallest buffer: a 1x1 buffer gets a 1x1 picture
    client.answers.clear();
    let (zero_rows, _f) = buffer(&shm, &qh, 1, 1, 0);
    let _frame = capture.capture_surface(&win, &zero_rows, 0, &qh, ());
    roundtrips(&mut queue, &mut client, 2);
    assert!(matches!(client.answers.as_slice(), [Answer::Ready { width: 1, height: 1, .. }]), "a 1x1 buffer gets a 1x1 picture: {:?}", client.answers);

    // minimized: the window leaves the space but the window server still holds it, and it can still be captured
    client.answers.clear();
    top.set_minimized();
    roundtrips(&mut queue, &mut client, 4);
    assert_eq!(wm.run(|state, _| (state.hidden.len(), state.space.elements().count())), (1, 0), "held hidden, out of the space");
    let _frame = capture.capture_surface(&win, &small, 0, &qh, ());
    roundtrips(&mut queue, &mut client, 2);
    assert!(matches!(client.answers.as_slice(), [Answer::Ready { width: 100, height: 50, .. }]), "a minimized window is captured: {:?}", client.answers);

    // closing it frees what was held for it
    top.destroy();
    xs.destroy();
    win.destroy();
    roundtrips(&mut queue, &mut client, 4);
    assert_eq!(wm.run(|state, _| (state.hidden.len(), state.window_capture.tracked())), (0, 0), "nothing is kept for a closed window");

    std::env::remove_var("BIFROST_WM_SHELL_UID");
}
