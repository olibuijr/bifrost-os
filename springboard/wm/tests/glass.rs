//! End to end: a client asks for blur behind a fully transparent window over a striped background layer; the
//! compositor's frame must show the stripes flattened inside the window and sharp outside it.

use std::{io::Write, os::fd::AsFd, os::unix::net::UnixStream, path::PathBuf, sync::mpsc, time::Duration};

use bifrost_wm::headless::Compositor;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_compositor, wl_region, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::{
    ext::background_effect::v1::client::{ext_background_effect_manager_v1 as bem, ext_background_effect_surface_v1 as bes},
    xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1 as layer_shell, zwlr_layer_surface_v1 as layer_surface};

#[derive(Default)]
struct Client {
    window_configured: bool,
    layer_size: Option<(u32, u32)>,
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
impl Dispatch<layer_surface::ZwlrLayerSurfaceV1, ()> for Client {
    fn event(s: &mut Self, ls: &layer_surface::ZwlrLayerSurfaceV1, e: layer_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let layer_surface::Event::Configure { serial, width, height } = e {
            ls.ack_configure(serial);
            s.layer_size = Some((width, height));
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
        xdg_toplevel::XdgToplevel, layer_shell::ZwlrLayerShellV1, bem::ExtBackgroundEffectManagerV1, bes::ExtBackgroundEffectSurfaceV1);

/// A `w` x `h` ARGB8888 shm buffer filled by `px(x, y)`.
fn buffer(shm: &wl_shm::WlShm, qh: &QueueHandle<Client>, w: i32, h: i32, px: impl Fn(i32, i32) -> u32) -> wl_buffer::WlBuffer {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).expect("XDG_RUNTIME_DIR");
    let mut file = tempfile::tempfile_in(dir).expect("shm file");
    let mut bytes = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            bytes.extend_from_slice(&px(x, y).to_le_bytes());
        }
    }
    file.write_all(&bytes).expect("write shm");
    let pool = shm.create_pool(file.as_fd(), w * h * 4, qh, ());
    let b = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    b
}

fn luma(px: &[u8], w: usize, x: usize, y: usize) -> i32 {
    let i = (y * w + x) * 4;
    (px[i] as i32 * 2126 + px[i + 1] as i32 * 7152 + px[i + 2] as i32 * 722) / 10_000
}

/// Keep a missing wl_display.sync reply from hanging the entire integration-test process forever.
fn bounded_registry_sync(socket: &str) {
    let socket = socket.to_owned();
    let (done, wait) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).ok_or("XDG_RUNTIME_DIR")?;
            let conn = Connection::from_socket(UnixStream::connect(dir.join(socket)).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            registry_queue_init::<Client>(&conn).map(|_| ()).map_err(|e| e.to_string())
        })();
        let _ = done.send(result);
    });
    match wait.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("bounded Wayland registry/sync failed: {e}"),
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("Wayland registry/sync received no reply within 10 seconds"),
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("Wayland registry/sync worker exited without a result"),
    }
}

#[test]
fn glass_blurs_what_is_behind_the_window_and_nothing_else() {
    let (w, h) = (800, 600);
    let wm = Compositor::start("wayland-bfwm-glass-test", w, h).expect("headless compositor");
    bounded_registry_sync(&wm.socket);
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap();
    let conn = Connection::from_socket(UnixStream::connect(dir.join(&wm.socket)).expect("connect")).expect("connection");
    let (globals, mut queue) = registry_queue_init::<Client>(&conn).expect("registry");
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
    let layers: layer_shell::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).expect("layer shell");
    let effects: bem::ExtBackgroundEffectManagerV1 = globals.bind(&qh, 1..=1, ()).expect("ext_background_effect_manager_v1");
    let mut client = Client::default();

    // the background: one-pixel black and white columns, the worst case for a blur to leave untouched
    let bg = compositor.create_surface(&qh, ());
    let bg_layer = layers.get_layer_surface(&bg, None, layer_shell::Layer::Background, "test-background".into(), &qh, ());
    bg_layer.set_anchor(layer_surface::Anchor::all());
    bg_layer.set_exclusive_zone(-1);
    bg.commit();
    // a fully transparent 400x300 window that asks for blur over all of it
    let win = compositor.create_surface(&qh, ());
    let xs = wm_base.get_xdg_surface(&win, &qh, ());
    let top = xs.get_toplevel(&qh, ());
    top.set_title("glass".into());
    let effect = effects.get_background_effect(&win, &qh, ());
    let region = compositor.create_region(&qh, ());
    region.add(0, 0, 400, 300);
    effect.set_blur_region(Some(&region));
    win.commit();

    for _ in 0..50 {
        queue.roundtrip(&mut client).expect("roundtrip");
        if client.window_configured && client.layer_size.is_some() {
            break;
        }
    }
    let (lw, lh) = client.layer_size.expect("layer configured");
    assert_eq!((lw, lh), (w as u32, h as u32), "the background fills the output");
    let stripes = buffer(&shm, &qh, w, h, |x, _| if x % 2 == 0 { 0xff00_0000 } else { 0xffff_ffff });
    bg.attach(Some(&stripes), 0, 0);
    bg.damage_buffer(0, 0, w, h);
    bg.commit();
    let clear = buffer(&shm, &qh, 400, 300, |_, _| 0);
    win.attach(Some(&clear), 0, 0);
    win.damage_buffer(0, 0, 400, 300);
    win.commit();
    for _ in 0..10 {
        queue.roundtrip(&mut client).expect("roundtrip");
        std::thread::sleep(Duration::from_millis(40));
    }

    let frame = wm.run(|state, b| {
        b.render(state).expect("render");
        let rect = state.wm.rect(state.wm.focused().expect("the window is mapped")).expect("frame");
        (rect, b.pixels().expect("pixels"))
    });
    let (rect, px) = frame;
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (200, 150, 400, 300), "centred");
    let (wx, wy) = ((rect.x + rect.w / 2) as usize, (rect.y + rect.h / 2) as usize);
    let inside: Vec<i32> = (0..40).map(|i| luma(&px, w as usize, wx - 20 + i, wy)).collect();
    let outside: Vec<i32> = (0..40).map(|i| luma(&px, w as usize, 40 + i, 40)).collect();
    let spread = |v: &[i32]| v.iter().max().unwrap() - v.iter().min().unwrap();
    assert!(spread(&outside) > 200, "the background stripes are sharp outside the window: {outside:?}");
    assert!(spread(&inside) < 40, "the window's glass flattens them: {inside:?}");
    let mean = inside.iter().sum::<i32>() / inside.len() as i32;
    assert!((60..200).contains(&mean), "black and white blur to a grey, not to black: {mean}");
}
