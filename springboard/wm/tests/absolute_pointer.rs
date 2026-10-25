//! End to end (E2E7 d01-d04): an absolute pointer (a tablet, VNC, an absolute uinput device) moving across one surface
//! reaches it as `wl_pointer.motion`, as a relative pointer does, so the hover of a Dock-like surface follows each move
//! and not only the first landing (the enter).

use std::{io::Write, os::fd::AsFd, os::unix::net::UnixStream, path::PathBuf, time::Duration};

use bifrost_wm::headless::Compositor;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1 as layer_shell, zwlr_layer_surface_v1 as layer_surface};

#[derive(Default)]
struct Client {
    configured: Option<(u32, u32)>,
    /// surface-local points of each enter
    enters: Vec<(f64, f64)>,
    /// surface-local points of each motion
    motions: Vec<(f64, f64)>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<layer_surface::ZwlrLayerSurfaceV1, ()> for Client {
    fn event(s: &mut Self, ls: &layer_surface::ZwlrLayerSurfaceV1, e: layer_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let layer_surface::Event::Configure { serial, width, height } = e {
            ls.ack_configure(serial);
            s.configured = Some((width, height));
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(s: &mut Self, _: &wl_pointer::WlPointer, e: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => s.enters.push((surface_x, surface_y)),
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => s.motions.push((surface_x, surface_y)),
            _ => {}
        }
    }
}
macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for Client {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(wl_compositor::WlCompositor, wl_surface::WlSurface, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, wl_seat::WlSeat,
        layer_shell::ZwlrLayerShellV1);

/// A `w` x `h` ARGB8888 shm buffer of one colour.
fn buffer(shm: &wl_shm::WlShm, qh: &QueueHandle<Client>, w: i32, h: i32, px: u32) -> wl_buffer::WlBuffer {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).expect("XDG_RUNTIME_DIR");
    let mut file = tempfile::tempfile_in(dir).expect("shm file");
    let bytes = px.to_le_bytes().repeat((w * h) as usize);
    file.write_all(&bytes).expect("write shm");
    let pool = shm.create_pool(file.as_fd(), w * h * 4, qh, ());
    let b = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    b
}

#[test]
fn absolute_moves_across_a_surface_reach_it_as_motion() {
    let (w, h) = (800, 600);
    let wm = Compositor::start("wayland-bfwm-absolute-pointer-test", w, h).expect("headless compositor");
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap();
    let conn = Connection::from_socket(UnixStream::connect(dir.join(&wm.socket)).expect("connect")).expect("connection");
    let (globals, mut queue) = registry_queue_init::<Client>(&conn).expect("registry");
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let layers: layer_shell::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).expect("layer shell");
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("wl_seat");
    let _pointer = seat.get_pointer(&qh, ());
    let mut client = Client::default();

    // a 400x80 Dock-like strip on the bottom edge, centred: (200, 520) to (600, 600)
    let dock = compositor.create_surface(&qh, ());
    let dock_layer = layers.get_layer_surface(&dock, None, layer_shell::Layer::Top, "test-dock".into(), &qh, ());
    dock_layer.set_anchor(layer_surface::Anchor::Bottom);
    dock_layer.set_exclusive_zone(-1);
    dock_layer.set_size(400, 80);
    dock.commit();
    for _ in 0..50 {
        queue.roundtrip(&mut client).expect("roundtrip");
        if client.configured.is_some() {
            break;
        }
    }
    assert_eq!(client.configured, Some((400, 80)), "the strip keeps its size");
    dock.attach(Some(&buffer(&shm, &qh, 400, 80, 0xff80_8080)), 0, 0);
    dock.damage_buffer(0, 0, 400, 80);
    dock.commit();
    for _ in 0..10 {
        queue.roundtrip(&mut client).expect("roundtrip");
        std::thread::sleep(Duration::from_millis(40));
    }

    // three absolute moves over three "icons" of the strip, as the remote pointer's uinput device sends them
    let (fw, fh) = (w as f64, h as f64);
    wm.run(move |state, _| {
        for (i, x) in [300.0, 400.0, 500.0].into_iter().enumerate() {
            state.pointer_motion_absolute((x / fw, 560.0 / fh), 10 + i as u32);
        }
        let _ = state.dh.flush_clients();
    });
    for _ in 0..5 {
        queue.roundtrip(&mut client).expect("roundtrip");
    }
    assert_eq!(client.enters.len(), 1, "one enter, on the first landing: {:?}", client.enters);
    assert!((client.enters[0].0 - 100.0).abs() < 2.0, "the enter is where the pointer landed: {:?}", client.enters);
    assert_eq!(client.motions.len(), 2, "each later move inside the strip is a motion: {:?}", client.motions);
    assert!((client.motions[0].0 - 200.0).abs() < 2.0 && (client.motions[1].0 - 300.0).abs() < 2.0, "{:?}", client.motions);
    assert!(client.motions.iter().all(|m| (m.1 - 40.0).abs() < 2.0), "{:?}", client.motions);
}
