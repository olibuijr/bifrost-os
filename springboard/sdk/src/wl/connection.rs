//! The direct path's Wayland connection (docs/graphics-architecture.md, "SDK"): connect to `WAYLAND_DISPLAY`, bind the
//! globals the app needs, make one xdg toplevel with its app id and title, and turn configure and close events into a
//! small queue the frame module drains.
//!
//! Also here: the linux-dmabuf v4 default feedback (main device and format table, parsed by the pure functions below),
//! and `create_dmabuf_buffer`, which wraps a dma-buf as a `wl_buffer` for `backing`.
//!
//! Owner: `sdk-wl-connection`. Input objects (wl_seat events) belong to `input`; this module binds the seat and
//! ignores its events until `input` takes them over.

use bifrost_gpu::{DmaBuf, DmaBufFormat};
use std::fmt;
use std::io::Read;
use std::os::fd::AsFd;
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection as WlConnection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::wp::presentation_time::client::wp_presentation;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// The private layer protocol's global (`bifrost_layer_v1`, owner `layers-protocol`). Recorded here so the layer
/// owner can bind it with `Globals::layer_manager_name`.
pub const LAYER_MANAGER_INTERFACE: &str = "bifrost_layer_manager_v1";

/// Something that went wrong on the direct path. The caller falls back to the shared-memory path on any of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No `WAYLAND_DISPLAY` or the socket refused the connection.
    Display(String),
    /// A global the direct path needs is not offered by the compositor.
    MissingGlobal(&'static str),
    /// Binding a global, or a protocol request, failed.
    Protocol(String),
    /// A GPU call (dma-buf export) failed while making a buffer.
    Gpu(bifrost_gpu::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Display(s) => write!(f, "Wayland display: {s}"),
            Error::MissingGlobal(g) => write!(f, "Wayland global {g} is not offered"),
            Error::Protocol(s) => write!(f, "Wayland protocol: {s}"),
            Error::Gpu(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

/// What the frame and input modules read from the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The compositor asks for the toplevel's size (`0` means the client chooses). Acknowledged already.
    Configure { width: u32, height: u32 },
    /// The compositor asks the app to close its window.
    Close,
    /// The compositor is done with a `wl_buffer` made by `create_dmabuf_buffer` with this tag.
    BufferReleased { tag: u64 },
}

/// Globals bound at connect time. Optional ones are `None` when the compositor does not offer them.
pub struct Globals {
    pub compositor: wl_compositor::WlCompositor,
    pub xdg_wm_base: xdg_wm_base::XdgWmBase,
    /// linux-dmabuf (version 4 or later gives the default feedback this module reads).
    pub dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    pub presentation: Option<wp_presentation::WpPresentation>,
    pub seat: Option<wl_seat::WlSeat>,
    /// The registry name and version of `bifrost_layer_manager_v1`, when offered. The layer owner binds it.
    pub layer_manager: Option<(u32, u32)>,
}

/// The default feedback accumulated from linux-dmabuf v4 events.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Feedback {
    /// The format table: (DRM fourcc, modifier) entries, indexed by the tranche indices.
    table: Vec<(u32, u64)>,
    /// The main device's dev_t, when the compositor sent one.
    main_device: Option<u64>,
    /// The formats of the tranche being received.
    pending: Vec<DmaBufFormat>,
    /// Completed tranches, in preference order.
    tranches: Vec<Vec<DmaBufFormat>>,
    /// Set by the `done` event.
    done: bool,
}

impl Feedback {
    /// Stores the format table (the bytes of the fd the compositor sent).
    pub fn set_table_bytes(&mut self, bytes: &[u8]) {
        self.table = parse_format_table(bytes);
    }

    /// Stores the main device from its native-endian dev_t bytes.
    pub fn set_main_device(&mut self, bytes: &[u8]) {
        self.main_device = parse_dev_t(bytes);
    }

    /// Adds the formats a tranche lists by index into the table.
    pub fn add_tranche_indices(&mut self, indices: &[u8]) {
        self.pending.extend(tranche_formats(&self.table, indices));
    }

    /// Closes the tranche being received.
    pub fn finish_tranche(&mut self) {
        let formats = std::mem::take(&mut self.pending);
        self.tranches.push(formats);
    }

    /// The `done` event.
    pub fn finish(&mut self) {
        self.done = true;
    }

    /// Whether the compositor has sent `done`.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The main device's dev_t, if known.
    pub fn main_device(&self) -> Option<u64> {
        self.main_device
    }

    /// Every (fourcc, modifier) pair the tranches offer, in preference order, without duplicates.
    pub fn formats(&self) -> Vec<DmaBufFormat> {
        let mut out: Vec<DmaBufFormat> = Vec::new();
        for f in self.tranches.iter().flatten() {
            if !out.contains(f) {
                out.push(*f);
            }
        }
        out
    }
}

/// Parses a linux-dmabuf format table: 16-byte entries of `u32` format, 4 bytes of padding, `u64` modifier, native
/// endian. A trailing partial entry is ignored.
pub fn parse_format_table(bytes: &[u8]) -> Vec<(u32, u64)> {
    bytes
        .chunks_exact(16)
        .map(|c| {
            let fourcc = u32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
            let mut m = [0u8; 8];
            m.copy_from_slice(&c[8..16]);
            (fourcc, u64::from_ne_bytes(m))
        })
        .collect()
}

/// Parses a dev_t (8 native-endian bytes); anything else is `None`.
pub fn parse_dev_t(bytes: &[u8]) -> Option<u64> {
    let arr: [u8; 8] = bytes.try_into().ok()?;
    Some(u64::from_ne_bytes(arr))
}

/// Resolves a tranche's index array (native-endian `u16` values) against the format table. Indices past the table
/// end are skipped.
pub fn tranche_formats(table: &[(u32, u64)], indices: &[u8]) -> Vec<DmaBufFormat> {
    indices
        .chunks_exact(2)
        .filter_map(|c| {
            let i = u16::from_ne_bytes([c[0], c[1]]) as usize;
            table.get(i).map(|&(fourcc, modifier)| DmaBufFormat { fourcc, modifier })
        })
        .collect()
}

/// State the Wayland event handlers write into.
pub(crate) struct State {
    events: Vec<Event>,
    feedback: Feedback,
}

/// One Wayland connection for the app: the globals, the toplevel and the event queue.
pub struct Connection {
    conn: WlConnection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    globals: Globals,
    _feedback: Option<zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1>,
    surface: wl_surface::WlSurface,
    _xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
}

/// The highest version of each interface this module was written against.
const COMPOSITOR_VERSIONS: std::ops::RangeInclusive<u32> = 4..=6;
const XDG_WM_BASE_VERSIONS: std::ops::RangeInclusive<u32> = 1..=6;
const DMABUF_VERSIONS: std::ops::RangeInclusive<u32> = 4..=5;
const PRESENTATION_VERSIONS: std::ops::RangeInclusive<u32> = 1..=2;
const SEAT_VERSIONS: std::ops::RangeInclusive<u32> = 5..=9;

impl Connection {
    /// Connects to `WAYLAND_DISPLAY`, binds the globals, makes the toplevel with `app_id` and `title`, and waits for
    /// the first configure so the caller starts with a size.
    pub fn connect(app_id: &str, title: &str) -> Result<Connection, Error> {
        let conn = WlConnection::connect_to_env().map_err(|e| Error::Display(format!("{e:?}")))?;
        let (list, queue): (GlobalList, EventQueue<State>) =
            registry_queue_init(&conn).map_err(|e| Error::Protocol(format!("{e:?}")))?;
        let qh = queue.handle();
        let state = State { events: Vec::new(), feedback: Feedback::default() };

        let layer_manager = list.contents().with_list(|globals| {
            globals
                .iter()
                .find(|g| g.interface == LAYER_MANAGER_INTERFACE)
                .map(|g| (g.name, g.version))
        });
        let compositor: wl_compositor::WlCompositor =
            bind(&list, &qh, COMPOSITOR_VERSIONS, "wl_compositor")?;
        let xdg_wm_base: xdg_wm_base::XdgWmBase = bind(&list, &qh, XDG_WM_BASE_VERSIONS, "xdg_wm_base")?;
        let dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1> = list.bind(&qh, DMABUF_VERSIONS, ()).ok();
        let presentation: Option<wp_presentation::WpPresentation> = list.bind(&qh, PRESENTATION_VERSIONS, ()).ok();
        let seat: Option<wl_seat::WlSeat> = list.bind(&qh, SEAT_VERSIONS, ()).ok();

        let feedback = match &dmabuf {
            Some(d) if d.version() >= 4 => Some(d.get_default_feedback(&qh, ())),
            _ => None,
        };

        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        toplevel.set_app_id(app_id.to_string());
        toplevel.set_title(title.to_string());
        surface.commit();

        let globals = Globals {
            compositor,
            xdg_wm_base,
            dmabuf,
            presentation,
            seat,
            layer_manager,
        };
        let mut connection = Connection {
            conn,
            queue,
            qh,
            state,
            globals,
            _feedback: feedback,
            surface,
            _xdg_surface: xdg_surface,
            toplevel,
        };
        // The feedback and the first configure arrive in reply to these requests.
        connection.roundtrip()?;
        connection.roundtrip()?;
        Ok(connection)
    }

    /// The bound globals.
    pub fn globals(&self) -> &Globals {
        &self.globals
    }

    /// The toplevel's wl_surface. The layer owner makes its context on it.
    pub fn surface(&self) -> &wl_surface::WlSurface {
        &self.surface
    }

    /// The xdg toplevel (for title, minimise, close requests).
    pub fn toplevel(&self) -> &xdg_toplevel::XdgToplevel {
        &self.toplevel
    }

    /// The DRM formats the compositor's default feedback offers, in preference order. Empty without linux-dmabuf.
    pub fn dmabuf_formats(&self) -> Vec<DmaBufFormat> {
        self.state.feedback.formats()
    }

    /// The main device's dev_t from the feedback, if the compositor sent one.
    pub fn dmabuf_main_device(&self) -> Option<u64> {
        self.state.feedback.main_device()
    }

    /// Takes the events received since the last call, in arrival order.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.state.events)
    }

    /// Dispatches the events already read, without blocking.
    pub fn dispatch_pending(&mut self) -> Result<(), Error> {
        self.queue
            .dispatch_pending(&mut self.state)
            .map(|_| ())
            .map_err(|e| Error::Protocol(format!("{e:?}")))
    }

    /// Blocks until events arrive and dispatches them.
    pub fn dispatch_blocking(&mut self) -> Result<(), Error> {
        self.queue
            .blocking_dispatch(&mut self.state)
            .map(|_| ())
            .map_err(|e| Error::Protocol(format!("{e:?}")))
    }

    /// Sends the requests queued so far.
    pub fn flush(&self) -> Result<(), Error> {
        self.conn.flush().map_err(|e| Error::Protocol(format!("{e:?}")))
    }

    /// Sends the queued requests and dispatches every reply.
    pub fn roundtrip(&mut self) -> Result<(), Error> {
        self.queue
            .roundtrip(&mut self.state)
            .map(|_| ())
            .map_err(|e| Error::Protocol(format!("{e:?}")))
    }

    /// Wraps a dma-buf as a `wl_buffer` with linux-dmabuf `create_immed`. Releases come back as
    /// `Event::BufferReleased { tag }`. The compositor takes its own references to the fds, so `dmabuf` can be
    /// dropped after this returns.
    pub fn create_dmabuf_buffer(&self, dmabuf: &DmaBuf, tag: u64) -> Result<wl_buffer::WlBuffer, Error> {
        let Some(zdmabuf) = &self.globals.dmabuf else {
            return Err(Error::MissingGlobal("zwp_linux_dmabuf_v1"));
        };
        let width = i32::try_from(dmabuf.width).map_err(|_| Error::Protocol("width".into()))?;
        let height = i32::try_from(dmabuf.height).map_err(|_| Error::Protocol("height".into()))?;
        let params = zdmabuf.create_params(&self.qh, ());
        for (idx, plane) in dmabuf.planes.iter().enumerate() {
            let hi = (dmabuf.modifier >> 32) as u32;
            let lo = dmabuf.modifier as u32;
            params.add(plane.fd.as_fd(), idx as u32, plane.offset, plane.stride, hi, lo);
        }
        let buffer = params.create_immed(
            width,
            height,
            dmabuf.fourcc,
            zwp_linux_buffer_params_v1::Flags::empty(),
            &self.qh,
            tag,
        );
        params.destroy();
        Ok(buffer)
    }
}

/// Binds a required global, mapping the failure to `Error::MissingGlobal`.
fn bind<I>(list: &GlobalList, qh: &QueueHandle<State>, versions: std::ops::RangeInclusive<u32>, name: &'static str) -> Result<I, Error>
where
    I: Proxy + 'static,
    State: Dispatch<I, ()>,
{
    list.bind(qh, versions, ()).map_err(|e| match e {
        wayland_client::globals::BindError::NotPresent => Error::MissingGlobal(name),
        other => Error::Protocol(format!("{name}: {other:?}")),
    })
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut State, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(_: &mut State, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(_: &mut State, _: &wl_surface::WlSurface, _: wayland_client::protocol::wl_surface::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut State, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<wp_presentation::WpPresentation, ()> for State {
    fn event(_: &mut State, _: &wp_presentation::WpPresentation, _: wp_presentation::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for State {
    fn event(_: &mut State, _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _: zwp_linux_dmabuf_v1::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for State {
    fn event(_: &mut State, _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, _: zwp_linux_buffer_params_v1::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(_: &mut State, wm: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {
        // The compositor pings to see whether the app is alive; an unanswered ping gets the app killed.
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(_: &mut State, surface: &xdg_surface::XdgSurface, event: xdg_surface::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(state: &mut State, _: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => state.events.push(Event::Configure {
                width: width.max(0) as u32,
                height: height.max(0) as u32,
            }),
            xdg_toplevel::Event::Close => state.events.push(Event::Close),
            _ => {}
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1, ()> for State {
    fn event(
        state: &mut State,
        _: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &WlConnection,
        _: &QueueHandle<State>,
    ) {
        use zwp_linux_dmabuf_feedback_v1::Event;
        match event {
            Event::FormatTable { fd, size } => {
                let mut file = std::fs::File::from(fd);
                let mut bytes = vec![0u8; size as usize];
                if file.read_exact(&mut bytes).is_ok() {
                    state.feedback.set_table_bytes(&bytes);
                }
            }
            Event::MainDevice { device } => state.feedback.set_main_device(&device),
            Event::TrancheFormats { indices } => state.feedback.add_tranche_indices(&indices),
            Event::TrancheDone => state.feedback.finish_tranche(),
            Event::Done => state.feedback.finish(),
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, u64> for State {
    fn event(state: &mut State, _: &wl_buffer::WlBuffer, event: wl_buffer::Event, tag: &u64, _: &WlConnection, _: &QueueHandle<State>) {
        if let wl_buffer::Event::Release = event {
            state.events.push(Event::BufferReleased { tag: *tag });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(fourcc: u32, modifier: u64) -> [u8; 16] {
        let mut e = [0u8; 16];
        e[0..4].copy_from_slice(&fourcc.to_ne_bytes());
        e[8..16].copy_from_slice(&modifier.to_ne_bytes());
        e
    }

    #[test]
    fn format_table_entries_are_sixteen_bytes() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&entry(0x3432_5241, 0));
        bytes.extend_from_slice(&entry(0x3432_5241, 1));
        bytes.extend_from_slice(&[0xff; 5]); // a trailing partial entry is ignored
        assert_eq!(parse_format_table(&bytes), vec![(0x3432_5241, 0), (0x3432_5241, 1)]);
    }

    #[test]
    fn empty_table_is_empty() {
        assert!(parse_format_table(&[]).is_empty());
    }

    #[test]
    fn dev_t_needs_eight_bytes() {
        assert_eq!(parse_dev_t(&0x0123u64.to_ne_bytes()), Some(0x0123));
        assert_eq!(parse_dev_t(&[1, 2, 3]), None);
    }

    #[test]
    fn tranche_indices_select_table_entries() {
        let table = vec![(1u32, 0u64), (2, 5), (3, 9)];
        let mut idx = Vec::new();
        idx.extend_from_slice(&2u16.to_ne_bytes());
        idx.extend_from_slice(&0u16.to_ne_bytes());
        idx.extend_from_slice(&7u16.to_ne_bytes()); // out of range, skipped
        assert_eq!(
            tranche_formats(&table, &idx),
            vec![DmaBufFormat { fourcc: 3, modifier: 9 }, DmaBufFormat { fourcc: 1, modifier: 0 }]
        );
    }

    #[test]
    fn feedback_collects_tranches_in_order_without_duplicates() {
        let mut f = Feedback::default();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&entry(10, 0));
        bytes.extend_from_slice(&entry(20, 0));
        f.set_table_bytes(&bytes);
        f.set_main_device(&7u64.to_ne_bytes());
        // Tranche one: indices 1 then 0 (20, 10). Tranche two: 0, 0, 1, 0 (10, 10, 20, 10).
        f.add_tranche_indices(&[1, 0, 0, 0]);
        f.finish_tranche();
        assert!(!f.is_done());
        f.add_tranche_indices(&[0, 0, 0, 0, 1, 0, 0, 0]);
        f.finish_tranche();
        f.finish();
        assert!(f.is_done());
        assert_eq!(f.main_device(), Some(7));
        assert_eq!(
            f.formats(),
            vec![DmaBufFormat { fourcc: 20, modifier: 0 }, DmaBufFormat { fourcc: 10, modifier: 0 }]
        );
    }

    #[test]
    fn feedback_without_tranches_offers_nothing() {
        let f = Feedback::default();
        assert!(f.formats().is_empty());
        assert_eq!(f.main_device(), None);
    }

    #[test]
    fn partial_index_pair_is_ignored() {
        let table = vec![(1u32, 1u64)];
        assert_eq!(tranche_formats(&table, &[0]), Vec::new());
    }
}
