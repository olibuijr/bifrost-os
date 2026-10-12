//! Protocol sync of the window server (docs/graphics-architecture.md, "Window server"): linux-dmabuf v4 feedback
//! from the render device and the scanout formats, `wp_linux_drm_syncobj_v1` explicit sync (acquire points gate a
//! commit, release points are held until the GPU is done with the buffer), and `wp_presentation` feedback sent
//! with the real vblank time from the frame clock.
//!
//! Owner: wave G2 agent `wm-sync-feedback`. The pure logic (tranche planning, the flip counter, deferred release
//! and presentation queues, timestamps) has no Smithay types and is host-tested. The Smithay glue at the bottom
//! only forwards to Smithay's protocol states. The DRM-only glue sits behind the `drm` feature.

use std::collections::VecDeque;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

use smithay::backend::allocator::{Format, Fourcc, Modifier};
use smithay::desktop::layer_map_for_output;
use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags;
use smithay::utils::Monotonic;
use smithay::reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle};
use smithay::wayland::compositor;
use smithay::wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal};
use smithay::wayland::presentation::Refresh;

use crate::state::WmState;

/// One dma-buf format as a DRM fourcc and modifier pair (the order a device lists them in).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fmt {
    pub fourcc: u32,
    pub modifier: u64,
}

/// One tranche of a feedback: the device that reads these formats, whether scanout reads them, and the formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tranche {
    pub device: u64,
    pub scanout: bool,
    pub formats: Vec<Fmt>,
}

/// The feedback a surface gets. Preference tranches come first (the scanout tranche when there is one) and the
/// main tranche, the formats the renderer imports on the main device, comes last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackPlan {
    pub main_device: u64,
    pub tranches: Vec<Tranche>,
}

/// Keeps the first occurrence of each format, in order.
fn dedup(formats: &[Fmt]) -> Vec<Fmt> {
    let mut out: Vec<Fmt> = Vec::with_capacity(formats.len());
    for f in formats {
        if !out.contains(f) {
            out.push(*f);
        }
    }
    out
}

/// Plans the feedback from the render device's formats and, when the scanout engine is a different or known
/// device, the formats scanout can read. A scanout format the renderer cannot import is dropped: a client must
/// never be told to allocate a buffer the compositor cannot sample. With no scanout formats there is no scanout
/// tranche, so the feedback is the main tranche only.
pub fn plan_feedback(main_device: u64, render: &[Fmt], scanout_device: Option<u64>, scanout: &[Fmt]) -> FeedbackPlan {
    let render = dedup(render);
    let mut tranches = Vec::new();
    if let Some(device) = scanout_device {
        let scanout: Vec<Fmt> = dedup(scanout).into_iter().filter(|f| render.contains(f)).collect();
        if !scanout.is_empty() {
            tranches.push(Tranche { device, scanout: true, formats: scanout });
        }
    }
    tranches.push(Tranche { device: main_device, scanout: false, formats: render });
    FeedbackPlan { main_device, tranches }
}

/// The feedback for one surface. A surface that may be scanned out directly (a full-screen window on the scanout
/// device) keeps the scanout tranche; any other surface gets the render-only tranches, so its client does not
/// allocate buffers that only the scanout engine accepts.
pub fn surface_plan(base: &FeedbackPlan, scanout_candidate: bool) -> FeedbackPlan {
    let tranches = base
        .tranches
        .iter()
        .filter(|t| scanout_candidate || !t.scanout)
        .cloned()
        .collect();
    FeedbackPlan { main_device: base.main_device, tranches }
}

/// The device number (`st_rdev`) of a device node, for the feedback's `main_device` and tranche devices.
pub fn device_number(path: &Path) -> io::Result<u64> {
    Ok(std::fs::metadata(path)?.rdev())
}

/// Counts vblanks and frame targets. A frame queued now is shown at the next vblank that is not already promised
/// to an earlier frame, so the queue targets are consecutive and strictly increasing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Flips {
    vblanks: u64,
    last_target: u64,
}

impl Flips {
    /// Records a frame queued for scanout and returns the vblank index it is shown at.
    pub fn queue(&mut self) -> u64 {
        self.last_target = self.last_target.max(self.vblanks) + 1;
        self.last_target
    }

    /// Records a vblank and returns the new vblank count.
    pub fn vblank(&mut self) -> u64 {
        self.vblanks += 1;
        self.vblanks
    }

    pub fn vblanks(&self) -> u64 {
        self.vblanks
    }
}

/// Items held until a sequence number is reached, released in the order they were held. Used for presentation
/// feedback (released at the vblank that shows the frame) and for syncobj release points (released when the GPU
/// has finished the submission that read the buffer). Sequence numbers must not decrease.
#[derive(Debug)]
pub struct Deferred<T> {
    items: VecDeque<(u64, T)>,
}

impl<T> Default for Deferred<T> {
    fn default() -> Self {
        Deferred { items: VecDeque::new() }
    }
}

impl<T> Deferred<T> {
    pub fn hold(&mut self, seq: u64, item: T) {
        self.items.push_back((seq, item));
    }

    /// Removes and returns every held item whose sequence number is at most `seq`, oldest first.
    pub fn release_through(&mut self, seq: u64) -> Vec<T> {
        let mut out = Vec::new();
        while self.items.front().is_some_and(|(s, _)| *s <= seq) {
            out.push(self.items.pop_front().expect("front").1);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// `wp_presentation.presented` splits the time into seconds and nanoseconds (`tv_sec_hi:tv_sec_lo`, `tv_nsec`).
pub fn timestamp(t: Duration) -> (u64, u32) {
    (t.as_secs(), t.subsec_nanos())
}

/// The presentation flags of a page flip: `Vsync` when the flip waited for vblank, `HwClock` when the time is the
/// kernel's vblank timestamp and not a CPU reading.
pub fn presentation_kind(vsync: bool, hw_clock: bool) -> Kind {
    let mut kind = Kind::empty();
    if vsync {
        kind |= Kind::Vsync;
    }
    if hw_clock {
        kind |= Kind::HwClock;
    }
    kind
}

/// Converts a planned feedback into Smithay's dmabuf feedback (the format table is built from the same plan).
/// Fails when a fourcc is not a DRM fourcc or the format table cannot be created.
pub fn build_feedback(plan: &FeedbackPlan) -> io::Result<DmabufFeedback> {
    let to_format = |f: &Fmt| -> io::Result<Format> {
        let code = Fourcc::try_from(f.fourcc)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("fourcc {:#x}", f.fourcc)))?;
        Ok(Format { code, modifier: Modifier::from(f.modifier) })
    };
    let (main, prefs) = plan.tranches.split_last().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no tranches"))?;
    let main_formats = main.formats.iter().map(to_format).collect::<io::Result<Vec<_>>>()?;
    let mut builder = DmabufFeedbackBuilder::new(plan.main_device as DevT, main_formats);
    for t in prefs {
        let formats = t.formats.iter().map(to_format).collect::<io::Result<Vec<_>>>()?;
        let flags = t.scanout.then_some(TrancheFlags::Scanout);
        builder = builder.add_preference_tranche(t.device as DevT, flags, formats);
    }
    builder.build()
}

/// `dev_t` on Linux (64-bit). Written out so the crate needs no `libc` dependency.
type DevT = u64;

/// Creates the linux-dmabuf global with version 4 default feedback. The global itself is created by the backend
/// once the render device is known (`drm.rs`).
pub fn dmabuf_global(state: &mut WmState, plan: &FeedbackPlan) -> io::Result<DmabufGlobal> {
    let feedback = build_feedback(plan)?;
    Ok(state.dmabuf_state.create_global_with_default_feedback::<WmState>(&state.dh, &feedback))
}

/// The feedback of frames in flight. Each queued frame carries the presentation feedback of every window and
/// layer drawn in it, and is presented at the vblank that flips it to the screen.
#[derive(Default)]
pub struct FrameFeedback {
    flips: Flips,
    pending: Deferred<OutputPresentationFeedback>,
}

impl FrameFeedback {
    /// The number of frames waiting for their vblank.
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }
}

/// Takes the presentation feedback of every window and layer drawn on `output` into one frame's feedback.
fn collect_frame_feedback(state: &WmState, output: &Output) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);
    for window in state.space.elements() {
        window.take_presentation_feedback(&mut feedback, |_, _| Some(output.clone()), |_, _| Kind::empty());
    }
    for layer in layer_map_for_output(output).layers() {
        layer.take_presentation_feedback(&mut feedback, |_, _| Some(output.clone()), |_, _| Kind::empty());
    }
    feedback
}

/// Called after a frame has been queued for scanout on `output`: its surfaces' feedback waits for that frame's vblank.
pub fn queue_frame(state: &mut WmState, output: &Output) {
    let feedback = collect_frame_feedback(state, output);
    let target = state.frame_feedback.flips.queue();
    state.frame_feedback.pending.hold(target, feedback);
}

/// Called at each page-flip vblank, with the flip's monotonic time, the refresh interval and whether the flip was
/// synchronised to vblank. Sends `wp_presentation` feedback for the frames this vblank showed. A zero refresh is
/// reported as unknown. The window server drives one output for now, so the feedback queue is shared.
pub fn on_vblank(state: &mut WmState, time: Duration, refresh: Duration, vsync: bool) {
    let seq = state.frame_feedback.flips.vblank();
    let kind = presentation_kind(vsync, true);
    for mut feedback in state.frame_feedback.pending.release_through(seq) {
        let refresh = if refresh.is_zero() { Refresh::Unknown } else { Refresh::fixed(refresh) };
        feedback.presented::<Duration, Monotonic>(time, refresh, seq, kind);
    }
}

#[cfg(feature = "drm")]
mod explicit {
    use std::fs::OpenOptions;
    use std::io;
    use std::os::unix::io::OwnedFd;
    use std::path::Path;

    use smithay::backend::drm::DrmDeviceFd;
    use smithay::backend::renderer::sync::Fence;
    use smithay::reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle, Resource};
    use smithay::utils::DeviceFd;
    use smithay::wayland::compositor::{self, with_states};
    use smithay::wayland::drm_syncobj::{supports_syncobj_eventfd, DrmSyncPoint, DrmSyncobjCachedState, DrmSyncobjState};

    use crate::state::{ClientState, WmState};

    /// The syncobj global for a DRM render node, or `None` when the kernel cannot signal syncobj eventfds (then the
    /// protocol is not offered and clients fall back to implicit sync).
    pub fn install(dh: &DisplayHandle, render_node: &Path) -> io::Result<Option<DrmSyncobjState>> {
        let file = OpenOptions::new().read(true).write(true).open(render_node)?;
        let device = DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)));
        if !supports_syncobj_eventfd(&device) {
            return Ok(None);
        }
        Ok(Some(DrmSyncobjState::new::<WmState>(dh, device)))
    }

    /// Pre-commit hook of every surface: a pending acquire point that has not signalled blocks the commit until it
    /// does, so the window server never samples a buffer the client is still writing.
    pub fn gate_acquire(state: &mut WmState, _dh: &DisplayHandle, surface: &WlSurface) {
        let Some(point) = with_acquire_point(surface) else { return };
        if point.is_signaled() {
            return;
        }
        let Ok((blocker, source)) = point.generate_blocker() else {
            eprintln!("bifrost-wm: acquire point cannot be waited on; committing without the gate");
            return;
        };
        compositor::add_blocker(surface, blocker);
        let surface = surface.clone();
        let _ = state.loop_handle.insert_source(source, move |_, _, st: &mut WmState| {
            if let Some(client) = surface.client() {
                if let Some(data) = client.get_data::<ClientState>() {
                    let dh = st.dh.clone();
                    data.compositor_state.blocker_cleared(st, &dh);
                }
            }
            Ok(())
        });
    }

    fn with_acquire_point(surface: &WlSurface) -> Option<DrmSyncPoint> {
        with_states(surface, |s| s.cached_state.get::<DrmSyncobjCachedState>().pending().acquire_point.clone())
    }

    /// Signals release points (the buffer is no longer read). Errors are reported, not fatal: a failed signal
    /// leaves the client waiting, which is the protocol's own failure mode.
    pub fn signal_released(points: Vec<DrmSyncPoint>) {
        for point in points {
            if let Err(e) = point.signal() {
                eprintln!("bifrost-wm: release point not signalled: {e}");
            }
        }
    }
}

#[cfg(feature = "drm")]
pub use explicit::{gate_acquire, install as install_syncobj, signal_released};

/// Installs a surface's acquire gate. Called from the compositor's `new_surface` for every surface; a surface
/// without an acquire point passes straight through.
pub fn watch_surface(surface: &WlSurface) {
    compositor::add_pre_commit_hook::<WmState, _>(surface, acquire_gate);
}

#[cfg(feature = "drm")]
fn acquire_gate(state: &mut WmState, dh: &DisplayHandle, surface: &WlSurface) {
    explicit::gate_acquire(state, dh, surface);
}

#[cfg(not(feature = "drm"))]
fn acquire_gate(_state: &mut WmState, _dh: &DisplayHandle, _surface: &WlSurface) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(fourcc: u32, modifier: u64) -> Fmt {
        Fmt { fourcc, modifier }
    }

    const XR24: u32 = 0x3432_5258; // XRGB8888
    const AR24: u32 = 0x3432_5241; // ARGB8888
    const NV12: u32 = 0x3231_564e;
    const LINEAR: u64 = 0;
    const TILED: u64 = 0x0100_0000_0000_0001;

    #[test]
    fn plan_keeps_the_render_formats_in_order_without_duplicates() {
        let render = [f(XR24, LINEAR), f(AR24, LINEAR), f(XR24, LINEAR)];
        let plan = plan_feedback(10, &render, None, &[]);
        assert_eq!(plan.tranches.len(), 1);
        assert_eq!(plan.tranches[0].formats, vec![f(XR24, LINEAR), f(AR24, LINEAR)]);
        assert!(!plan.tranches[0].scanout);
        assert_eq!(plan.tranches[0].device, 10);
    }

    #[test]
    fn scanout_tranche_comes_first_and_only_holds_importable_formats() {
        let render = [f(XR24, LINEAR), f(AR24, LINEAR), f(XR24, TILED)];
        // NV12 is scanout-only (the renderer cannot import it), and XR24 with the tiled modifier is scanout-ok.
        let scanout = [f(XR24, TILED), f(NV12, LINEAR), f(XR24, TILED)];
        let plan = plan_feedback(10, &render, Some(20), &scanout);
        assert_eq!(plan.tranches.len(), 2);
        assert_eq!(plan.tranches[0], Tranche { device: 20, scanout: true, formats: vec![f(XR24, TILED)] });
        assert_eq!(plan.tranches[1].device, 10);
        assert!(!plan.tranches[1].scanout);
        assert_eq!(plan.tranches[1].formats.len(), 3);
    }

    #[test]
    fn no_scanout_tranche_when_no_scanout_format_is_importable() {
        let render = [f(XR24, LINEAR)];
        let plan = plan_feedback(10, &render, Some(20), &[f(NV12, LINEAR)]);
        assert_eq!(plan.tranches.len(), 1);
        assert!(!plan.tranches[0].scanout);
    }

    #[test]
    fn no_scanout_device_means_no_scanout_tranche_even_with_formats() {
        let render = [f(XR24, LINEAR)];
        let plan = plan_feedback(10, &render, None, &[f(XR24, LINEAR)]);
        assert_eq!(plan.tranches.len(), 1);
    }

    #[test]
    fn surface_plan_drops_scanout_for_a_surface_that_is_not_a_candidate() {
        let render = [f(XR24, LINEAR), f(XR24, TILED)];
        let base = plan_feedback(10, &render, Some(20), &[f(XR24, TILED)]);
        let windowed = surface_plan(&base, false);
        assert_eq!(windowed.tranches.len(), 1);
        assert!(!windowed.tranches[0].scanout);
        let fullscreen = surface_plan(&base, true);
        assert_eq!(fullscreen, base);
    }

    #[test]
    fn plan_builds_a_dmabuf_feedback_only_with_valid_fourccs() {
        let good = plan_feedback(10, &[f(XR24, LINEAR)], None, &[]);
        assert!(build_feedback(&good).is_ok());
        let bad = plan_feedback(10, &[f(0, LINEAR)], None, &[]);
        assert!(build_feedback(&bad).is_err());
    }

    #[test]
    fn flips_target_consecutive_vblanks_and_count_them() {
        let mut flips = Flips::default();
        // Two frames queued before any vblank: shown at vblank 1 and 2.
        assert_eq!(flips.queue(), 1);
        assert_eq!(flips.queue(), 2);
        assert_eq!(flips.vblank(), 1);
        // A frame queued after vblank 1 is still behind frame 2.
        assert_eq!(flips.queue(), 3);
        assert_eq!(flips.vblank(), 2);
        assert_eq!(flips.vblank(), 3);
        // Nothing queued for a while: the next frame is shown at the next vblank, not an old one.
        assert_eq!(flips.queue(), 4);
        assert_eq!(flips.vblanks(), 3);
    }

    #[test]
    fn deferred_releases_in_hold_order_up_to_the_sequence() {
        let mut d: Deferred<&str> = Deferred::default();
        d.hold(1, "a");
        d.hold(2, "b");
        d.hold(2, "c");
        d.hold(5, "d");
        assert_eq!(d.len(), 4);
        assert_eq!(d.release_through(0), Vec::<&str>::new());
        assert_eq!(d.release_through(2), vec!["a", "b", "c"]);
        assert_eq!(d.len(), 1);
        assert_eq!(d.release_through(9), vec!["d"]);
        assert!(d.is_empty());
    }

    #[test]
    fn deferred_keeps_later_items_until_their_sequence_is_reached() {
        let mut d: Deferred<u32> = Deferred::default();
        d.hold(3, 30);
        assert_eq!(d.release_through(2), Vec::<u32>::new());
        assert_eq!(d.release_through(3), vec![30]);
    }

    #[test]
    fn timestamps_split_seconds_and_nanoseconds() {
        assert_eq!(timestamp(Duration::from_millis(1500)), (1, 500_000_000));
        assert_eq!(timestamp(Duration::new(7, 9)), (7, 9));
        assert_eq!(timestamp(Duration::ZERO), (0, 0));
    }

    #[test]
    fn page_flip_flags_follow_vsync_and_hw_clock() {
        assert_eq!(presentation_kind(true, true), Kind::Vsync | Kind::HwClock);
        assert_eq!(presentation_kind(true, false), Kind::Vsync);
        assert_eq!(presentation_kind(false, true), Kind::HwClock);
        assert_eq!(presentation_kind(false, false), Kind::empty());
    }

    #[test]
    fn device_number_of_a_regular_file_is_zero_and_of_dev_null_is_1_3() {
        let dir = std::env::temp_dir().join(format!("bifrost-sync-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("plain");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(device_number(&file).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
        // /dev/null is character device 1:3; glibc's st_rdev encoding gives (1 << 8) | 3.
        if Path::new("/dev/null").exists() {
            assert_eq!(device_number(Path::new("/dev/null")).unwrap(), 259);
        }
    }
}
