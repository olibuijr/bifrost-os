//! The worker that captures the composed screen for a recording (`screenrec_composed.rs` has the pacing, cropping and encoder).
//!
//! It runs on a thread of its own with its own connection to the window server, so the shell's frame loop never waits for a
//! capture. It binds `wl_output`, `wl_shm` and `zwlr_screencopy_manager_v1` (the protocol the window server implements on its
//! GLES backends), takes one probe frame of the whole output to learn the output's size and to prove that the composed screen
//! can be read, then captures the chosen region every frame interval into a small pool of reused shm buffers. A filled buffer
//! goes to the encoder without a copy and comes back to the pool when the encoder has written it. When the pool is empty, the
//! encoder's queue is full or a deadline was missed, the frame is dropped and counted; the worker never waits for the encoder.
//!
//! The window server offers shm buffers only (no dma-buf), and answers `copy_with_damage` like `copy` without a damage event, so
//! the worker asks for plain copies.

use super::screenrec::Area;
use super::screenrec_composed::{self as comp, Captured, FrameData, Job, Status, Unavailable};
use crate::bare::sys::{now_ms, wait};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1 as sc_frame, zwlr_screencopy_manager_v1 as sc_mgr};

const XRGB8888: u32 = wl_shm::Format::Xrgb8888 as u32;
const ARGB8888: u32 = wl_shm::Format::Argb8888 as u32;

/// Starts the worker thread of `job`; it reports to the shell through `status`.
pub fn spawn(job: Job, status: Sender<Status>) -> JoinHandle<Result<usize, String>> {
    crate::bare::sys::spawn_named("screen-record", move || run(job, status))
}

/// What the window server announced for a frame: pixel format, size and row stride.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Info {
    format: u32,
    w: u32,
    h: u32,
    stride: u32,
}

/// Why one capture did not give a frame.
#[derive(Debug)]
enum GrabError {
    /// the window server answered `failed`
    Refused,
    /// no answer in time
    Stalled,
    /// the connection broke
    Io(String),
    /// the frame is not the size the buffers were made for (the screen changed)
    Geometry(Info),
    /// a pixel format the recorder does not read
    Format(u32),
}

impl GrabError {
    /// The reason the screen cannot be recorded at all, when this happens on the probe.
    fn unavailable(&self) -> Unavailable {
        match self {
            GrabError::Refused => Unavailable::Refused,
            GrabError::Stalled => Unavailable::Stalled,
            GrabError::Io(e) => Unavailable::NoConnection(e.clone()),
            GrabError::Geometry(_) => Unavailable::Refused,
            GrabError::Format(f) => Unavailable::Format(*f),
        }
    }

    /// The sentence for a recording that stops because of this.
    fn stopped(&self) -> String {
        match self {
            GrabError::Geometry(i) => format!("The screen changed to {} by {} pixels, so the recording was stopped.", i.w, i.h),
            GrabError::Format(_) => "The window server changed its picture format, so the recording was stopped.".into(),
            _ => "The window server stopped giving pictures of the screen, so the recording was stopped.".into(),
        }
    }
}

#[derive(Default)]
struct FrameState {
    format: Option<(u32, u32, u32, u32)>,
    y_invert: bool,
    buffer_done: bool,
    ready: bool,
    failed: bool,
}

#[derive(Default)]
struct State {
    globals: Vec<(u32, String, u32)>,
    frame: FrameState,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(st: &mut Self, _: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            st.globals.push((name, interface, version));
        }
    }
}

impl Dispatch<sc_frame::ZwlrScreencopyFrameV1, ()> for State {
    fn event(st: &mut Self, _: &sc_frame::ZwlrScreencopyFrameV1, ev: sc_frame::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            sc_frame::Event::Buffer { format: WEnum::Value(f), width, height, stride } => {
                st.frame.format.get_or_insert((f as u32, width, height, stride));
            }
            sc_frame::Event::Flags { flags: WEnum::Value(f) } => st.frame.y_invert = f.contains(sc_frame::Flags::YInvert),
            sc_frame::Event::BufferDone => st.frame.buffer_done = true,
            sc_frame::Event::Ready { .. } => st.frame.ready = true,
            sc_frame::Event::Failed => st.frame.failed = true,
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
ignore!(wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, wl_output::WlOutput, sc_mgr::ZwlrScreencopyManagerV1);

/// One shm buffer of the pool: the window server writes into it, the encoder reads it through `map`.
struct Buf {
    buffer: wl_buffer::WlBuffer,
    map: Arc<memmap2::Mmap>,
}

/// A filled buffer on its way to the encoder; dropping it (when the encoder has written it, or when the queue was full) gives
/// the buffer back to the pool.
struct Slot {
    index: usize,
    map: Arc<memmap2::Mmap>,
    back: Sender<usize>,
}

impl FrameData for Slot {
    fn bytes(&self) -> &[u8] {
        &self.map[..]
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let _ = self.back.send(self.index);
    }
}

/// The reused shm buffers.
struct Pool {
    bufs: Vec<Buf>,
    free: Receiver<usize>,
    back: Sender<usize>,
}

impl Pool {
    fn new(cap: &Capturer, info: Info, count: usize) -> Result<Pool, Unavailable> {
        let (back, free) = channel();
        let mut bufs = Vec::with_capacity(count);
        for i in 0..count {
            bufs.push(cap.make_buf(info)?);
            let _ = back.send(i);
        }
        Ok(Pool { bufs, free, back })
    }

    /// A buffer nobody is using, or None: the frame is dropped then, not waited for.
    fn take(&self) -> Option<usize> {
        self.free.try_recv().ok()
    }
}

/// The worker's connection to the window server.
struct Capturer {
    _conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    st: State,
    shm: wl_shm::WlShm,
    sc: sc_mgr::ZwlrScreencopyManagerV1,
    out: wl_output::WlOutput,
}

impl Capturer {
    fn connect() -> Result<Capturer, Unavailable> {
        let conn = Connection::connect_to_env().map_err(|e| Unavailable::NoConnection(e.to_string()))?;
        let mut queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let reg = conn.display().get_registry(&qh, ());
        let mut st = State::default();
        queue.roundtrip(&mut st).map_err(|e| Unavailable::NoConnection(e.to_string()))?;
        let (mut shm, mut sc, mut out) = (None, None, None);
        for (name, iface, ver) in st.globals.clone() {
            match iface.as_str() {
                "wl_shm" if shm.is_none() => shm = Some(reg.bind::<wl_shm::WlShm, _, _>(name, 1, &qh, ())),
                "zwlr_screencopy_manager_v1" if sc.is_none() => sc = Some(reg.bind::<sc_mgr::ZwlrScreencopyManagerV1, _, _>(name, ver.min(3), &qh, ())),
                "wl_output" if out.is_none() => out = Some(reg.bind::<wl_output::WlOutput, _, _>(name, ver.min(4), &qh, ())),
                _ => {}
            }
        }
        queue.roundtrip(&mut st).map_err(|e| Unavailable::NoConnection(e.to_string()))?;
        Ok(Capturer {
            _conn: conn,
            queue,
            qh,
            st,
            shm: shm.ok_or(Unavailable::NoOutput)?,
            sc: sc.ok_or(Unavailable::NoScreencopy)?,
            out: out.ok_or(Unavailable::NoOutput)?,
        })
    }

    /// Dispatches until `done` holds or `timeout_ms` has passed.
    fn until(&mut self, timeout_ms: f64, mut done: impl FnMut(&State) -> bool) -> Result<(), GrabError> {
        let end = now_ms() + timeout_ms;
        loop {
            self.queue.dispatch_pending(&mut self.st).map_err(|e| GrabError::Io(e.to_string()))?;
            if done(&self.st) {
                return Ok(());
            }
            self.queue.flush().map_err(|e| GrabError::Io(e.to_string()))?;
            let left = end - now_ms();
            if left <= 0.0 {
                return Err(GrabError::Stalled);
            }
            if let Some(guard) = self.queue.prepare_read() {
                let fd = guard.connection_fd().as_raw_fd();
                if wait(&[fd], left.ceil().max(1.0) as i32).first().copied().unwrap_or(false) {
                    guard.read().map_err(|e| GrabError::Io(e.to_string()))?;
                }
            }
        }
    }

    /// One shm buffer for frames described by `info`.
    fn make_buf(&self, info: Info) -> Result<Buf, Unavailable> {
        let setup = |e: String| Unavailable::Setup(format!("no shared memory for the frames ({e}). Nothing was recorded."));
        let format = wl_shm::Format::try_from(info.format).map_err(|_| Unavailable::Format(info.format))?;
        let size = info.stride as usize * info.h as usize;
        let file = tempfile::tempfile().map_err(|e| setup(e.to_string()))?;
        file.set_len(size as u64).map_err(|e| setup(e.to_string()))?;
        // SAFETY: the file is private to this process and is only truncated by dropping it; the window server writes into the
        // same pages through its own mapping, and a buffer is read here only after it has answered `ready`.
        let map = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| setup(e.to_string()))?;
        let pool = self.shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
        let buffer = pool.create_buffer(0, info.w as i32, info.h as i32, info.stride as i32, format, &self.qh, ());
        pool.destroy();
        Ok(Buf { buffer, map: Arc::new(map) })
    }

    /// Asks for a frame of `region` (None: the whole output) and waits for the window server to say what buffer it needs.
    fn begin(&mut self, region: Option<Area>, cursor: bool) -> Result<(sc_frame::ZwlrScreencopyFrameV1, Info), GrabError> {
        self.st.frame = FrameState::default();
        let frame = match region {
            Some(a) => self.sc.capture_output_region(cursor as i32, &self.out, a.x as i32, a.y as i32, a.w as i32, a.h as i32, &self.qh, ()),
            None => self.sc.capture_output(cursor as i32, &self.out, &self.qh, ()),
        };
        let v3 = frame.version() >= 3;
        if let Err(e) = self.until(comp::GRAB_TIMEOUT_MS, |s| s.frame.failed || if v3 { s.frame.buffer_done } else { s.frame.format.is_some() }) {
            frame.destroy();
            return Err(e);
        }
        match self.st.frame.format {
            Some((format, w, h, stride)) if !self.st.frame.failed => Ok((frame, Info { format, w, h, stride })),
            _ => {
                frame.destroy();
                Err(GrabError::Refused)
            }
        }
    }

    /// Has the window server copy the frame into `buffer` and waits for `ready`. Returns whether the rows are upside down.
    fn copy(&mut self, frame: sc_frame::ZwlrScreencopyFrameV1, buffer: &wl_buffer::WlBuffer) -> Result<bool, GrabError> {
        frame.copy(buffer);
        let waited = self.until(comp::GRAB_TIMEOUT_MS, |s| s.frame.ready || s.frame.failed);
        frame.destroy();
        waited?;
        if self.st.frame.failed {
            return Err(GrabError::Refused);
        }
        Ok(self.st.frame.y_invert)
    }

    /// One frame of `region` into `buf`, which was made for frames described by `expect`.
    fn grab(&mut self, region: Option<Area>, cursor: bool, expect: Info, buf: &Buf) -> Result<bool, GrabError> {
        let (frame, info) = self.begin(region, cursor)?;
        if info != expect {
            frame.destroy();
            // a frame of another size is the screen changing; the same size in another format is the window server changing it
            return Err(if (info.w, info.h) != (expect.w, expect.h) { GrabError::Geometry(info) } else { GrabError::Format(info.format) });
        }
        self.copy(frame, &buf.buffer)
    }
}

/// Waits `ms` unless the shell stops the recording first. Returns whether it was stopped.
fn settle(ms: f64, job: &Job) -> bool {
    let end = now_ms() + ms;
    while now_ms() < end {
        if job.stop.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    job.stop.load(Ordering::Relaxed)
}

fn run(job: Job, status: Sender<Status>) -> Result<usize, String> {
    let refuse = |u: Unavailable| -> Result<usize, String> {
        let message = u.message();
        let _ = status.send(Status::Unavailable(u));
        Err(message)
    };

    let mut cap = match Capturer::connect() {
        Ok(c) => c,
        Err(u) => return refuse(u),
    };

    // The probe: one real frame of the whole output. It tells the size of the output and proves that the composed screen can
    // be read; a window server that cannot (no readback on the Vulkan renderer) fails here, before any file exists.
    let (frame, info) = match cap.begin(None, false) {
        Ok(v) => v,
        Err(e) => return refuse(e.unavailable()),
    };
    if !matches!(info.format, XRGB8888 | ARGB8888) {
        frame.destroy();
        return refuse(Unavailable::Format(info.format));
    }
    let probe = match cap.make_buf(info) {
        Ok(b) => b,
        Err(u) => {
            frame.destroy();
            return refuse(u);
        }
    };
    let probed = cap.copy(frame, &probe.buffer);
    probe.buffer.destroy();
    if let Err(e) = probed {
        return refuse(e.unavailable());
    }

    // the chosen area in the output's pixels, once, from the probe's size
    let area = match comp::scale_area(job.area, job.screen, (info.w, info.h)) {
        Ok(a) => a,
        Err(m) => return refuse(Unavailable::Setup(format!("{m}. Nothing was recorded."))),
    };
    let whole = area.x == 0 && area.y == 0 && area.w == info.w && area.h == info.h;
    let region = (!whole).then_some(area);
    let expect = Info { format: info.format, w: area.w, h: area.h, stride: area.w * 4 };
    let pool = match Pool::new(&cap, expect, comp::POOL) {
        Ok(p) => p,
        Err(u) => return refuse(u),
    };

    // the shell's own overlays are taken off the screen first (the toolbar that started the recording)
    if settle(job.settle_ms, &job) {
        return Ok(0);
    }
    let (tx, encoder) = match comp::start_encoder(&job, area) {
        Ok(v) => v,
        Err(m) => return refuse(Unavailable::Setup(format!("{m}. Nothing was recorded."))),
    };
    let _ = status.send(Status::Running { w: area.w, h: area.h });

    let stats = job.stats.clone();
    let interval = 1000.0 / job.fps.max(1) as f64;
    let t0 = now_ms();
    let mut due = t0;
    let mut fails = 0u32;
    let mut ended: Option<String> = None;
    while !job.stop.load(Ordering::Relaxed) {
        let now = now_ms();
        if now < due {
            std::thread::sleep(Duration::from_secs_f64((due - now).clamp(0.5, 20.0) / 1000.0));
            continue;
        }
        if job.paused.load(Ordering::Relaxed) {
            // the screen is off: no frame, and the deadlines go by without counting as dropped
            due = comp::next_tick(due, now, interval).0;
            continue;
        }
        let Some(i) = pool.take() else {
            stats.dropped.fetch_add(1, Ordering::Relaxed);
            let (next, skipped) = comp::next_tick(due, now_ms(), interval);
            stats.dropped.fetch_add(skipped as u64, Ordering::Relaxed);
            due = next;
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        match cap.grab(region, job.cursor, expect, &pool.bufs[i]) {
            Ok(y_invert) => {
                fails = 0;
                stats.captured.fetch_add(1, Ordering::Relaxed);
                let slot = Slot { index: i, map: pool.bufs[i].map.clone(), back: pool.back.clone() };
                let at_ms = now_ms() - t0;
                match tx.try_send(Captured { data: Box::new(slot), stride: expect.stride, y_invert, at_ms }) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        // the encoder is behind: the frame (and its buffer) goes back, and is counted
                        stats.captured.fetch_sub(1, Ordering::Relaxed);
                        stats.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        ended = Some("The movie could not be written any more, so the recording was stopped.".into());
                        break;
                    }
                }
            }
            Err(e) => {
                let _ = pool.back.send(i);
                stats.failed.fetch_add(1, Ordering::Relaxed);
                match &e {
                    GrabError::Geometry(_) | GrabError::Format(_) => {
                        ended = Some(e.stopped());
                        break;
                    }
                    _ => {
                        fails += 1;
                        if fails >= comp::MAX_FAILS {
                            ended = Some(e.stopped());
                            break;
                        }
                    }
                }
            }
        }
        let (next, skipped) = comp::next_tick(due, now_ms(), interval);
        stats.dropped.fetch_add(skipped as u64, Ordering::Relaxed);
        due = next;
    }

    drop(tx);
    let result = encoder.join().map_err(|_| "the encoder stopped unexpectedly".to_string()).and_then(|r| r);
    if let Some(why) = ended {
        let _ = status.send(Status::Ended(why));
    }
    result
}
