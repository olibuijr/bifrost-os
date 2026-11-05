//! End to end (E2E R1): an overlay layer surface takes the pointer by its input region, not by its size. A menu
//! exported with room for its shadow takes a press on the menu wherever it reaches, past the window under it too, even
//! when a newer overlay surface with an empty input region (a desktop switch's sweep) covers the whole screen; its
//! shadow lets the press through to the window.

use std::{io::Write, os::fd::AsFd, os::unix::net::UnixStream, path::PathBuf, time::Duration};

use bifrost_wm::headless::Compositor;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_compositor, wl_region, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1 as layer_shell, zwlr_layer_surface_v1 as layer_surface};

#[derive(Default)]
struct Client {
    window_configured: bool,
    /// the configured size of each layer surface, by its user data (0 the menu, 1 the sweep)
    layers: [Option<(u32, u32)>; 2],
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
            s.window_configured = true;
        }
    }
}
impl Dispatch<layer_surface::ZwlrLayerSurfaceV1, usize> for Client {
    fn event(s: &mut Self, ls: &layer_surface::ZwlrLayerSurfaceV1, e: layer_surface::Event, which: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let layer_surface::Event::Configure { serial, width, height } = e {
            ls.ack_configure(serial);
            s.layers[*which] = Some((width, height));
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
ignore!(wl_compositor::WlCompositor, wl_surface::WlSurface, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, wl_region::WlRegion,
        xdg_toplevel::XdgToplevel, layer_shell::ZwlrLayerShellV1);

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
fn an_overlay_menu_takes_the_pointer_over_its_input_region_wherever_it_reaches() {
    let (w, h) = (800, 600);
    let wm = Compositor::start("wayland-bfwm-layer-input-test", w, h).expect("headless compositor");
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap();
    let conn = Connection::from_socket(UnixStream::connect(dir.join(&wm.socket)).expect("connect")).expect("connection");
    let (globals, mut queue) = registry_queue_init::<Client>(&conn).expect("registry");
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
    let layers: layer_shell::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).expect("layer shell");
    let mut client = Client::default();

    // a 400x300 window, centred: (200, 150) to (600, 450)
    let win = compositor.create_surface(&qh, ());
    let xs = wm_base.get_xdg_surface(&win, &qh, ());
    let top = xs.get_toplevel(&qh, ());
    top.set_title("window".into());
    win.commit();
    // a 300x200 menu at (500, 200): it reaches past the window's right edge. Only the menu inside a 12 px shadow
    // margin takes the pointer, (512, 212) to (788, 388)
    let menu = compositor.create_surface(&qh, ());
    let menu_layer = layers.get_layer_surface(&menu, None, layer_shell::Layer::Overlay, "test-menu".into(), &qh, 0usize);
    menu_layer.set_anchor(layer_surface::Anchor::Top | layer_surface::Anchor::Left);
    menu_layer.set_margin(200, 0, 0, 500);
    menu_layer.set_exclusive_zone(-1);
    menu_layer.set_size(300, 200);
    let region = compositor.create_region(&qh, ());
    region.add(12, 12, 276, 176);
    menu.set_input_region(Some(&region));
    menu.commit();
    // a newer overlay surface over the whole screen that takes no pointer at all
    let sweep = compositor.create_surface(&qh, ());
    let sweep_layer = layers.get_layer_surface(&sweep, None, layer_shell::Layer::Overlay, "test-sweep".into(), &qh, 1usize);
    sweep_layer.set_anchor(layer_surface::Anchor::all());
    sweep_layer.set_exclusive_zone(-1);
    let empty = compositor.create_region(&qh, ());
    sweep.set_input_region(Some(&empty));
    sweep.commit();

    for _ in 0..50 {
        queue.roundtrip(&mut client).expect("roundtrip");
        if client.window_configured && client.layers.iter().all(Option::is_some) {
            break;
        }
    }
    assert_eq!(client.layers[0], Some((300, 200)), "the menu keeps its size");
    let (sw, sh) = client.layers[1].expect("the sweep is configured");
    for (surface, (bw, bh)) in [(&win, (400, 300)), (&menu, (300, 200)), (&sweep, (sw as i32, sh as i32))] {
        surface.attach(Some(&buffer(&shm, &qh, bw, bh, 0xff80_8080)), 0, 0);
        surface.damage_buffer(0, 0, bw, bh);
        surface.commit();
    }
    for _ in 0..10 {
        queue.roundtrip(&mut client).expect("roundtrip");
        std::thread::sleep(Duration::from_millis(40));
    }

    let (rect, menu_inside, menu_outside, shadow, desktop) = wm.run(|state, _| {
        let rect = state.wm.rect(state.wm.focused().expect("the window is mapped")).expect("frame");
        let menu_inside = state.layer_above_windows_at((550.0, 250.0).into());
        let menu_outside = state.layer_above_windows_at((700.0, 300.0).into());
        let shadow = state.layer_above_windows_at((505.0, 205.0).into());
        let desktop = state.layer_above_windows_at((50.0, 50.0).into());
        (rect, menu_inside, menu_outside, shadow, desktop)
    });
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (200, 150, 400, 300), "centred");
    assert!(menu_inside, "the menu takes a press over the window");
    assert!(menu_outside, "and past the window's edge: the sweep above it takes no pointer");
    assert!(!shadow, "the menu's shadow lets the press through to the window");
    assert!(!desktop, "nothing above the windows takes the desktop's corner");
}
