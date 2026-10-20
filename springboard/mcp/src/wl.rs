//! One long-lived Wayland connection to the compositor (bifrost-wm): screen and window capture,
//! the window list, virtual pointer and keyboard. Everything is synchronous; each call
//! dispatches until its reply arrives or a deadline passes.

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1 as ext_handle, ext_foreign_toplevel_list_v1 as ext_list,
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1 as tl_source_mgr, ext_image_capture_source_v1 as source,
    ext_output_image_capture_source_manager_v1 as out_source_mgr,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1 as cc_frame, ext_image_copy_capture_manager_v1 as cc_mgr,
    ext_image_copy_capture_session_v1 as cc_session,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1 as vk_mgr, zwp_virtual_keyboard_v1 as vk,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1 as wlr_handle, zwlr_foreign_toplevel_manager_v1 as wlr_mgr,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1 as sc_frame, zwlr_screencopy_manager_v1 as sc_mgr,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1 as vp_mgr, zwlr_virtual_pointer_v1 as vp,
};

/// An RGBA8 image, top row first.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Window {
    pub title: String,
    pub app_id: String,
    pub activated: bool,
    pub maximized: bool,
    pub minimized: bool,
    pub fullscreen: bool,
}

struct Toplevel {
    wlr: wlr_handle::ZwlrForeignToplevelHandleV1,
    info: Window,
    closed: bool,
}

struct ExtToplevel {
    handle: ext_handle::ExtForeignToplevelHandleV1,
    title: String,
    app_id: String,
    closed: bool,
}

#[derive(Default)]
struct Output {
    out: Option<wl_output::WlOutput>,
    w: i32,
    h: i32,
    name: String,
}

#[derive(Default)]
struct Shot {
    format: Option<(u32, u32, u32, u32)>, // format, w, h, stride
    y_invert: bool,
    buffer_done: bool,
    ready: bool,
    failed: bool,
}

#[derive(Default)]
struct Session {
    size: Option<(u32, u32)>,
    formats: Vec<u32>,
    done: bool,
    stopped: bool,
    ready: bool,
    failed: Option<u32>,
}

#[derive(Default)]
pub struct State {
    pub globals: Vec<(u32, String, u32)>,
    outputs: Vec<Output>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    sc: Option<sc_mgr::ZwlrScreencopyManagerV1>,
    vp_mgr: Option<vp_mgr::ZwlrVirtualPointerManagerV1>,
    vk_mgr: Option<vk_mgr::ZwpVirtualKeyboardManagerV1>,
    cc: Option<cc_mgr::ExtImageCopyCaptureManagerV1>,
    tl_src: Option<tl_source_mgr::ExtForeignToplevelImageCaptureSourceManagerV1>,
    out_src: Option<out_source_mgr::ExtOutputImageCaptureSourceManagerV1>,
    toplevels: Vec<Toplevel>,
    ext_toplevels: Vec<ExtToplevel>,
    shot: Shot,
    session: Session,
}

pub struct Wl {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    pub st: State,
    pointer: Option<vp::ZwlrVirtualPointerV1>,
    keyboard: Option<(vk::ZwpVirtualKeyboardV1, Vec<char>)>,
    t0: Instant,
}

/// Where the session script puts the window server's privileged socket.
const PRIVILEGED_SOCKET: &str = "/run/bifrost/wm-privileged";

/// The window server's privileged socket (root only; this binary runs as root), where the window list, output
/// management and screen capture globals are listed. `BIFROST_WM_PRIVILEGED` names it, else the session default.
/// `None` when it cannot be opened (a development session): the caller falls back to the ordinary socket.
fn connect_privileged() -> Option<Connection> {
    let path = std::env::var("BIFROST_WM_PRIVILEGED").ok().filter(|p| p.starts_with('/')).unwrap_or_else(|| PRIVILEGED_SOCKET.into());
    let stream = std::os::unix::net::UnixStream::connect(path).ok()?;
    Connection::from_socket(stream).ok()
}

impl Wl {
    /// Connects to `$WAYLAND_DISPLAY` in `$XDG_RUNTIME_DIR` (set by the caller from the session file).
    pub fn connect() -> Result<Wl, String> {
        let conn = connect_privileged().map_or_else(|| Connection::connect_to_env().map_err(|e| format!("wayland: {e}")), Ok)?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let reg = conn.display().get_registry(&qh, ());
        let mut st = State::default();
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
        for (name, iface, ver) in st.globals.clone() {
            match iface.as_str() {
                "wl_output" => {
                    let idx = st.outputs.len();
                    st.outputs.push(Output::default());
                    st.outputs[idx].out = Some(reg.bind(name, ver.min(4), &qh, idx));
                }
                "wl_shm" => st.shm = Some(reg.bind(name, 1, &qh, ())),
                "wl_seat" if st.seat.is_none() => st.seat = Some(reg.bind(name, ver.min(7), &qh, ())),
                "zwlr_screencopy_manager_v1" => st.sc = Some(reg.bind(name, ver.min(3), &qh, ())),
                "zwlr_virtual_pointer_manager_v1" => st.vp_mgr = Some(reg.bind(name, ver.min(2), &qh, ())),
                "zwp_virtual_keyboard_manager_v1" => st.vk_mgr = Some(reg.bind(name, 1, &qh, ())),
                "zwlr_foreign_toplevel_manager_v1" => {
                    let _: wlr_mgr::ZwlrForeignToplevelManagerV1 = reg.bind(name, ver.min(3), &qh, ());
                }
                "ext_foreign_toplevel_list_v1" => {
                    let _: ext_list::ExtForeignToplevelListV1 = reg.bind(name, 1, &qh, ());
                }
                "ext_image_copy_capture_manager_v1" => st.cc = Some(reg.bind(name, 1, &qh, ())),
                "ext_foreign_toplevel_image_capture_source_manager_v1" => st.tl_src = Some(reg.bind(name, 1, &qh, ())),
                "ext_output_image_capture_source_manager_v1" => st.out_src = Some(reg.bind(name, 1, &qh, ())),
                _ => {}
            }
        }
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
        Ok(Wl { conn, queue, qh, st, pointer: None, keyboard: None, t0: Instant::now() })
    }

    fn ms(&self) -> u32 {
        self.t0.elapsed().as_millis() as u32
    }

    /// Reads whatever the compositor has sent since the last call (window list changes).
    pub fn refresh(&mut self) -> Result<(), String> {
        self.queue.roundtrip(&mut self.st).map(|_| ()).map_err(|e| e.to_string())
    }

    fn until(&mut self, timeout: Duration, mut done: impl FnMut(&State) -> bool) -> Result<(), String> {
        let end = Instant::now() + timeout;
        loop {
            self.queue.dispatch_pending(&mut self.st).map_err(|e| e.to_string())?;
            if done(&self.st) {
                return Ok(());
            }
            self.queue.flush().map_err(|e| e.to_string())?;
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("compositor did not answer in time".into());
            }
            if let Some(guard) = self.queue.prepare_read() {
                let mut pfd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
                let n = unsafe { libc::poll(&mut pfd, 1, left.as_millis().max(1) as i32) };
                if n > 0 {
                    guard.read().map_err(|e| e.to_string())?;
                }
            }
        }
    }

    /// The first output's size in compositor (buffer) pixels.
    pub fn screen_size(&self) -> (i32, i32) {
        self.st.outputs.first().map(|o| (o.w, o.h)).unwrap_or((0, 0))
    }

    pub fn windows(&mut self) -> Vec<Window> {
        let _ = self.refresh();
        self.st.toplevels.iter().filter(|t| !t.closed).map(|t| t.info.clone()).collect()
    }

    fn find(&mut self, query: &str) -> Result<usize, String> {
        let _ = self.refresh();
        let q = query.to_lowercase();
        let live: Vec<usize> = (0..self.st.toplevels.len()).filter(|&i| !self.st.toplevels[i].closed).collect();
        let pick = |f: &dyn Fn(&Window) -> bool| live.iter().copied().find(|&i| f(&self.st.toplevels[i].info));
        pick(&|w| w.app_id.to_lowercase() == q)
            .or_else(|| pick(&|w| w.title.to_lowercase() == q))
            .or_else(|| pick(&|w| w.app_id.to_lowercase().contains(&q) || w.title.to_lowercase().contains(&q)))
            .ok_or_else(|| format!("no window matches {query:?}"))
    }

    /// activate | close | maximize | unmaximize | minimize | unminimize | fullscreen | unfullscreen
    pub fn window_action(&mut self, query: &str, action: &str) -> Result<Window, String> {
        let i = self.find(query)?;
        let h = self.st.toplevels[i].wlr.clone();
        match action {
            "activate" | "focus" => {
                let seat = self.st.seat.clone().ok_or("no seat")?;
                h.activate(&seat)
            }
            "close" => h.close(),
            "maximize" => h.set_maximized(),
            "unmaximize" => h.unset_maximized(),
            "minimize" => h.set_minimized(),
            "unminimize" => h.unset_minimized(),
            "fullscreen" => h.set_fullscreen(None),
            "unfullscreen" => h.unset_fullscreen(),
            a => return Err(format!("unknown window action {a:?}")),
        }
        self.refresh()?;
        Ok(self.st.toplevels[i].info.clone())
    }

    fn shm_file(&self, size: usize) -> Result<File, String> {
        let fd = unsafe { libc::memfd_create(c"bifrost-mcp".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err("memfd_create failed".into());
        }
        let f = unsafe { File::from_raw_fd(fd) };
        f.set_len(size as u64).map_err(|e| e.to_string())?;
        Ok(f)
    }

    /// The whole screen, or a region of it in screen pixels, through wlr-screencopy.
    pub fn capture_screen(&mut self, region: Option<(i32, i32, i32, i32)>, cursor: bool) -> Result<Image, String> {
        let sc = self.st.sc.clone().ok_or("compositor has no wlr-screencopy")?;
        let out = self.st.outputs.first().and_then(|o| o.out.clone()).ok_or("no output")?;
        self.st.shot = Shot::default();
        let frame = match region {
            Some((x, y, w, h)) => sc.capture_output_region(cursor as i32, &out, x, y, w, h, &self.qh, ()),
            None => sc.capture_output(cursor as i32, &out, &self.qh, ()),
        };
        self.until(Duration::from_secs(3), |s| s.shot.buffer_done || s.shot.failed || (s.shot.format.is_some() && frame.version() < 3))?;
        let (format, w, h, stride) = self.st.shot.format.ok_or("screencopy offered no shm buffer")?;
        let size = (stride * h) as usize;
        let file = self.shm_file(size)?;
        let shm = self.st.shm.clone().ok_or("no wl_shm")?;
        let pool = shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
        let buf = pool.create_buffer(0, w as i32, h as i32, stride as i32, wl_shm::Format::try_from(format).map_err(|_| "bad format")?, &self.qh, ());
        frame.copy(&buf);
        let r = self.until(Duration::from_secs(3), |s| s.shot.ready || s.shot.failed);
        frame.destroy();
        buf.destroy();
        pool.destroy();
        r?;
        if self.st.shot.failed {
            return Err("screencopy failed".into());
        }
        let mut raw = vec![0u8; size];
        file.read_exact_at(&mut raw, 0).map_err(|e| e.to_string())?;
        Ok(to_rgba(&raw, format, w, h, stride, self.st.shot.y_invert))
    }

    /// One window's own pixels (even when covered), through ext-image-copy-capture.
    pub fn capture_window(&mut self, query: &str) -> Result<Image, String> {
        let _ = self.refresh();
        let q = query.to_lowercase();
        let live: Vec<usize> = (0..self.st.ext_toplevels.len()).filter(|&i| !self.st.ext_toplevels[i].closed).collect();
        let t = &self.st.ext_toplevels;
        let i = live.iter().copied().find(|&i| t[i].app_id.to_lowercase() == q || t[i].title.to_lowercase() == q)
            .or_else(|| live.iter().copied().find(|&i| t[i].app_id.to_lowercase().contains(&q) || t[i].title.to_lowercase().contains(&q)))
            .ok_or_else(|| format!("no window matches {query:?}"))?;
        let handle = self.st.ext_toplevels[i].handle.clone();
        let mgr = self.st.tl_src.clone().ok_or("compositor has no toplevel capture")?;
        let src = mgr.create_source(&handle, &self.qh, ());
        let r = self.capture_source(&src);
        src.destroy();
        r
    }

    fn capture_source(&mut self, src: &source::ExtImageCaptureSourceV1) -> Result<Image, String> {
        let cc = self.st.cc.clone().ok_or("compositor has no ext-image-copy-capture")?;
        self.st.session = Session::default();
        let session = cc.create_session(src, cc_mgr::Options::empty(), &self.qh, ());
        let r = (|| {
            self.until(Duration::from_secs(3), |s| s.session.done || s.session.stopped)?;
            if self.st.session.stopped {
                return Err("capture session stopped (window gone?)".to_string());
            }
            let (w, h) = self.st.session.size.ok_or("no buffer size")?;
            let pick = [wl_shm::Format::Xrgb8888 as u32, wl_shm::Format::Argb8888 as u32, wl_shm::Format::Xbgr8888 as u32, wl_shm::Format::Abgr8888 as u32];
            let format = pick.into_iter().find(|f| self.st.session.formats.contains(f)).ok_or("no 32-bit shm format")?;
            let stride = w * 4;
            let size = (stride * h) as usize;
            let file = self.shm_file(size)?;
            let shm = self.st.shm.clone().ok_or("no wl_shm")?;
            let pool = shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
            let buf = pool.create_buffer(0, w as i32, h as i32, stride as i32, wl_shm::Format::try_from(format).unwrap(), &self.qh, ());
            let frame = session.create_frame(&self.qh, ());
            frame.attach_buffer(&buf);
            frame.damage_buffer(0, 0, w as i32, h as i32);
            frame.capture();
            let r = self.until(Duration::from_secs(3), |s| s.session.ready || s.session.failed.is_some());
            frame.destroy();
            buf.destroy();
            pool.destroy();
            r?;
            if let Some(why) = self.st.session.failed {
                return Err(format!("capture failed (reason {why})"));
            }
            let mut raw = vec![0u8; size];
            file.read_exact_at(&mut raw, 0).map_err(|e| e.to_string())?;
            Ok(to_rgba(&raw, format, w, h, stride, false))
        })();
        session.destroy();
        r
    }

    fn pointer(&mut self) -> Result<vp::ZwlrVirtualPointerV1, String> {
        if self.pointer.is_none() {
            let m = self.st.vp_mgr.clone().ok_or("compositor has no virtual pointer")?;
            self.pointer = Some(m.create_virtual_pointer(self.st.seat.as_ref(), &self.qh, ()));
        }
        Ok(self.pointer.clone().unwrap())
    }

    /// Moves the pointer to screen pixel (x, y).
    pub fn motion(&mut self, x: f64, y: f64) -> Result<(), String> {
        let (w, h) = self.screen_size();
        if !(0.0..w as f64).contains(&x) || !(0.0..h as f64).contains(&y) {
            return Err(format!("({x}, {y}) is outside the {w}x{h} screen"));
        }
        let p = self.pointer()?;
        let t = self.ms();
        p.motion_absolute(t, x.round() as u32, y.round() as u32, w as u32, h as u32);
        p.frame();
        self.flush()
    }

    /// Linux button code: 0x110 left, 0x111 right, 0x112 middle.
    pub fn button(&mut self, code: u32, down: bool) -> Result<(), String> {
        let p = self.pointer()?;
        let t = self.ms();
        p.button(t, code, if down { wayland_client::protocol::wl_pointer::ButtonState::Pressed } else { wayland_client::protocol::wl_pointer::ButtonState::Released });
        p.frame();
        self.flush()
    }

    /// Scrolls by (dx, dy) wheel clicks (positive dy scrolls down).
    pub fn scroll(&mut self, dx: f64, dy: f64) -> Result<(), String> {
        use wayland_client::protocol::wl_pointer::Axis;
        let p = self.pointer()?;
        let t = self.ms();
        p.axis_source(wayland_client::protocol::wl_pointer::AxisSource::Wheel);
        for (axis, v) in [(Axis::VerticalScroll, dy), (Axis::HorizontalScroll, dx)] {
            if v != 0.0 {
                p.axis_discrete(t, axis, v * 15.0, v.round() as i32);
            }
        }
        p.frame();
        self.flush()
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.conn.flush().map_err(|e| e.to_string())
    }

    /// Waits until the compositor has processed everything sent so far.
    pub fn sync(&mut self) -> Result<(), String> {
        self.refresh()
    }

    /// Types `text` through a virtual keyboard whose keymap holds exactly its characters
    /// (any Unicode, independent of the system layout).
    pub fn type_text(&mut self, text: &str, delay_ms: u64) -> Result<(), String> {
        let mut chars: Vec<char> = text.chars().collect();
        chars.sort();
        chars.dedup();
        if chars.is_empty() {
            return Ok(());
        }
        let need_new = match &self.keyboard {
            Some((_, have)) => chars.iter().any(|c| !have.contains(c)),
            None => true,
        };
        if need_new {
            if let Some((k, _)) = self.keyboard.take() {
                k.destroy();
            }
            let m = self.st.vk_mgr.clone().ok_or("compositor has no virtual keyboard")?;
            let seat = self.st.seat.clone().ok_or("no seat")?;
            let k = m.create_virtual_keyboard(&seat, &self.qh, ());
            let map = keymap(&chars);
            let mut f = self.shm_file(map.len() + 1)?;
            f.write_all(map.as_bytes()).map_err(|e| e.to_string())?;
            f.write_all(&[0]).map_err(|e| e.to_string())?;
            k.keymap(1, f.as_fd(), (map.len() + 1) as u32);
            self.keyboard = Some((k, chars));
            self.sync()?;
        }
        let (k, have) = self.keyboard.clone().unwrap();
        for c in text.chars() {
            let code = have.iter().position(|&h| h == c).unwrap() as u32 + 1; // evdev code; xkb adds 8
            let t = self.ms();
            k.key(t, code, 1);
            k.key(t, code, 0);
            if delay_ms > 0 {
                self.flush()?;
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
        }
        self.sync()
    }

    pub fn st_counts(&self) -> (usize, usize) {
        (self.st.toplevels.len(), self.st.ext_toplevels.len())
    }

}

/// An xkb keymap with one key per character: key <K1> (evdev 1) types chars[0], and so on.
fn keymap(chars: &[char]) -> String {
    let mut s = String::from("xkb_keymap {\nxkb_keycodes \"bifrost\" {\nminimum = 8;\nmaximum = ");
    s += &format!("{};\n", chars.len() + 9);
    for i in 0..chars.len() {
        s += &format!("<K{}> = {};\n", i + 1, i + 9);
    }
    s += "};\nxkb_types \"bifrost\" { include \"complete\" };\nxkb_compatibility \"bifrost\" { include \"complete\" };\nxkb_symbols \"bifrost\" {\n";
    for (i, c) in chars.iter().enumerate() {
        let sym = match c {
            '\n' => "Return".to_string(),
            '\t' => "Tab".to_string(),
            ' ' => "space".to_string(),
            '\u{8}' => "BackSpace".to_string(),
            c => format!("U{:04X}", *c as u32),
        };
        s += &format!("key <K{}> {{[ {} ]}};\n", i + 1, sym);
    }
    s += "};\n};\n";
    s
}

fn to_rgba(raw: &[u8], format: u32, w: u32, h: u32, stride: u32, y_invert: bool) -> Image {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    // little-endian: [A|X]RGB8888 is B,G,R,A in memory; [A|X]BGR8888 is R,G,B,A
    let bgr = format == wl_shm::Format::Argb8888 as u32 || format == wl_shm::Format::Xrgb8888 as u32;
    for row in 0..h {
        let y = if y_invert { h - 1 - row } else { row };
        let line = &raw[(y * stride) as usize..(y * stride + w * 4) as usize];
        for p in line.as_chunks::<4>().0 {
            if bgr {
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            } else {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
    }
    Image { w, h, rgba }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(st: &mut Self, _: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            st.globals.push((name, interface, version));
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(st: &mut Self, _: &wl_output::WlOutput, ev: wl_output::Event, idx: &usize, _: &Connection, _: &QueueHandle<Self>) {
        let o = &mut st.outputs[*idx];
        match ev {
            wl_output::Event::Mode { flags: WEnum::Value(f), width, height, .. } if f.contains(wl_output::Mode::Current) => {
                o.w = width;
                o.h = height;
            }
            wl_output::Event::Name { name } => o.name = name,
            _ => {}
        }
    }
}

impl Dispatch<sc_frame::ZwlrScreencopyFrameV1, ()> for State {
    fn event(st: &mut Self, _: &sc_frame::ZwlrScreencopyFrameV1, ev: sc_frame::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            sc_frame::Event::Buffer { format: WEnum::Value(f), width, height, stride } => {
                st.shot.format.get_or_insert((f as u32, width, height, stride));
            }
            sc_frame::Event::Flags { flags: WEnum::Value(f) } => st.shot.y_invert = f.contains(sc_frame::Flags::YInvert),
            sc_frame::Event::BufferDone => st.shot.buffer_done = true,
            sc_frame::Event::Ready { .. } => st.shot.ready = true,
            sc_frame::Event::Failed => st.shot.failed = true,
            _ => {}
        }
    }
}

impl Dispatch<cc_session::ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(st: &mut Self, _: &cc_session::ExtImageCopyCaptureSessionV1, ev: cc_session::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            cc_session::Event::BufferSize { width, height } => st.session.size = Some((width, height)),
            cc_session::Event::ShmFormat { format: WEnum::Value(f) } => st.session.formats.push(f as u32),
            cc_session::Event::Done => st.session.done = true,
            cc_session::Event::Stopped => st.session.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<cc_frame::ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(st: &mut Self, _: &cc_frame::ExtImageCopyCaptureFrameV1, ev: cc_frame::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            cc_frame::Event::Ready => st.session.ready = true,
            cc_frame::Event::Failed { reason } => {
                st.session.failed = Some(match reason {
                    WEnum::Value(r) => r as u32,
                    WEnum::Unknown(r) => r,
                })
            }
            _ => {}
        }
    }
}

impl Dispatch<wlr_mgr::ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(st: &mut Self, _: &wlr_mgr::ZwlrForeignToplevelManagerV1, ev: wlr_mgr::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wlr_mgr::Event::Toplevel { toplevel } = ev {
            st.toplevels.push(Toplevel { wlr: toplevel, info: Window::default(), closed: false });
        }
    }
    event_created_child!(State, wlr_mgr::ZwlrForeignToplevelManagerV1, [
        wlr_mgr::EVT_TOPLEVEL_OPCODE => (wlr_handle::ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<wlr_handle::ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(st: &mut Self, h: &wlr_handle::ZwlrForeignToplevelHandleV1, ev: wlr_handle::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let Some(t) = st.toplevels.iter_mut().find(|t| &t.wlr == h) else { return };
        match ev {
            wlr_handle::Event::Title { title } => t.info.title = title,
            wlr_handle::Event::AppId { app_id } => t.info.app_id = app_id,
            wlr_handle::Event::State { state } => {
                let s: Vec<u32> = state.as_chunks::<4>().0.iter().map(|b| u32::from_ne_bytes(*b)).collect();
                t.info.maximized = s.contains(&0);
                t.info.minimized = s.contains(&1);
                t.info.activated = s.contains(&2);
                t.info.fullscreen = s.contains(&3);
            }
            wlr_handle::Event::Closed => t.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<ext_list::ExtForeignToplevelListV1, ()> for State {
    fn event(st: &mut Self, _: &ext_list::ExtForeignToplevelListV1, ev: ext_list::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let ext_list::Event::Toplevel { toplevel } = ev {
            st.ext_toplevels.push(ExtToplevel { handle: toplevel, title: String::new(), app_id: String::new(), closed: false });
        }
    }
    event_created_child!(State, ext_list::ExtForeignToplevelListV1, [
        ext_list::EVT_TOPLEVEL_OPCODE => (ext_handle::ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ext_handle::ExtForeignToplevelHandleV1, ()> for State {
    fn event(st: &mut Self, h: &ext_handle::ExtForeignToplevelHandleV1, ev: ext_handle::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let Some(t) = st.ext_toplevels.iter_mut().find(|t| &t.handle == h) else { return };
        match ev {
            ext_handle::Event::Title { title } => t.title = title,
            ext_handle::Event::AppId { app_id } => t.app_id = app_id,
            ext_handle::Event::Closed => t.closed = true,
            _ => {}
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(
    wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, wl_seat::WlSeat,
    sc_mgr::ZwlrScreencopyManagerV1, vp_mgr::ZwlrVirtualPointerManagerV1, vp::ZwlrVirtualPointerV1,
    vk_mgr::ZwpVirtualKeyboardManagerV1, vk::ZwpVirtualKeyboardV1, cc_mgr::ExtImageCopyCaptureManagerV1,
    tl_source_mgr::ExtForeignToplevelImageCaptureSourceManagerV1, out_source_mgr::ExtOutputImageCaptureSourceManagerV1,
    source::ExtImageCaptureSourceV1
);
