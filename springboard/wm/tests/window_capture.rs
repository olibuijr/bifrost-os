//! End to end: the capture globals are for privileged clients only, and `bifrost_window_capture_v1` on the headless
//! window server.
//!
//! - A client that connected through the ordinary socket (an app) sees none of the capture globals: window capture,
//!   wlr-screencopy, ext-image-copy-capture and its output capture source.
//! - A client that connected through the privileged socket (the shell and its helpers) sees all of them, binds window
//!   capture, captures its own toplevel scaled into a smaller buffer, hears `unchanged` for a picture it already
//!   holds and `ready` again after the window committed new content, and can capture a minimized window.
//!
//! One test function: the privileged socket is named by the environment when the window server starts.

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

/// Every global that can read pixels of the screen or of a window, or act on every window or on the display (the
/// window list, output management, output power).
const CAPTURE_GLOBALS: [&str; 7] = [
    "zwlr_foreign_toplevel_manager_v1",
    "zwlr_output_manager_v1",
    "zwlr_output_power_manager_v1",
    "bifrost_window_capture_manager_v1",
    "zwlr_screencopy_manager_v1",
    "ext_image_copy_capture_manager_v1",
    "ext_output_image_capture_source_manager_v1",
];

/// The capture globals the registry of `conn` lists.
fn listed_capture_globals(conn: &Connection) -> Vec<String> {
    let (globals, _queue) = registry_queue_init::<Client>(conn).expect("registry");
    globals.contents().with_list(|list| list.iter().filter(|g| CAPTURE_GLOBALS.contains(&g.interface.as_str())).map(|g| g.interface.clone()).collect())
}

fn connect_path(path: &std::path::Path) -> Connection {
    Connection::from_socket(UnixStream::connect(path).expect("connect to the privileged socket")).expect("connection")
}

fn roundtrips(queue: &mut wayland_client::EventQueue<Client>, client: &mut Client, n: usize) {
    for _ in 0..n {
        queue.roundtrip(client).expect("roundtrip");
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn only_privileged_clients_see_the_capture_globals_and_a_capture_fits_the_window_into_the_buffer() {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).expect("XDG_RUNTIME_DIR");
    let privileged = dir.join("bfwm-wcap-privileged");
    std::env::set_var("BIFROST_WM_PRIVILEGED_SOCKET", &privileged);
    let wm = Compositor::start("wayland-bfwm-wcap", 800, 600).expect("headless compositor");

    // an app (the ordinary socket) sees none of the capture globals
    let app_conn = connect(&wm.socket);
    assert_eq!(listed_capture_globals(&app_conn), Vec::<String>::new(), "an ordinary client must not see any capture global");
    // the privileged socket is the owner's alone
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&privileged).expect("the privileged socket exists").permissions().mode();
        assert_eq!(mode & 0o077, 0, "no access for group or others: {mode:o}");
    }
    // the shell (the privileged socket) sees every one of them
    let conn = connect_path(&privileged);
    let mut seen = listed_capture_globals(&conn);
    seen.sort();
    let mut want: Vec<String> = CAPTURE_GLOBALS.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(seen, want, "a privileged client sees all the capture globals");

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

    std::env::remove_var("BIFROST_WM_PRIVILEGED_SOCKET");
}
