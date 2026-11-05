//! The DRM backend's Vulkan scanout path (wave G2, `wm-drm-vulkan`). When `render_server::selected()`, `drm.rs` starts
//! this path after the GLES compositor is set up on the same GPU; GLES stays the fallback when any step fails.
//!
//! Per output: the scanout buffers are GBM buffer objects in a format and modifier that both the primary plane and
//! Vulkan accept (`pick_scanout_format`), imported as render targets (`OutputTargets::from_gbm`, with `SAMPLED` usage
//! so live glass can read them) and given KMS framebuffers. Each frame acquires a free buffer, asks the render server
//! for the scene (`scene_build::build`) over the damage of the buffer's age (`Pacing`), and presents it with an atomic commit
//! whose IN_FENCE_FD is the submission's sync_file (`PlaneConfig::fence`). The page flip's vblank time feeds
//! `FrameClock`, and the flip frees the previously shown buffer (`OutputTargets::flipped`).
//!
//! The scene is `scene_build::build` (Wayland and X11 windows, layers, glass); a frame is drawn only when
//! `diff_damage` against the last frame is not empty. What is not wired yet (see docs/graphics/g2-wm-drm-vulkan.md):
//! the cursor, the boot fade, and the scheduling of composition at `FrameClock::start_at` (frames are paced by vblank).

use std::{
    collections::VecDeque,
    os::fd::{AsFd, OwnedFd},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use bifrost_gpu::{DmaBufFormat, LoadAction, PixelFormat};
use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice},
            Allocator, Fourcc, Modifier,
        },
        drm::{gbm::{framebuffer_from_bo, GbmFramebuffer}, DrmDeviceFd, DrmSurface, PlaneConfig, PlaneState},
    },
    output::Output,
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform},
};

use crate::{
    drm::CLEAR,
    state::WmState,
    render_server::{
        compose::{DamageRects, SceneNode},
        output::{OutputTargets, RING},
        scene_build,
        server::Server,
    },
};

const XRGB8888: u32 = 0x3432_5258;
const ARGB8888: u32 = 0x3432_5241;

/// The render node name (`renderD128`) among the entries of a card's `device/drm` sysfs directory, or `None`.
/// The card's directory lists `card0` and `renderD128` next to each other; only `renderD*` names count.
pub fn render_node_name(entries: &[String]) -> Option<String> {
    let mut nodes: Vec<&String> = entries.iter().filter(|e| is_render_node(e)).collect();
    nodes.sort();
    nodes.first().map(|s| (*s).clone())
}

fn is_render_node(name: &str) -> bool {
    name.strip_prefix("renderD").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The render node path for a primary node such as `/dev/dri/card0`: the `renderD*` entry under
/// `/sys/class/drm/card0/device/drm`, placed in the same `/dev/dri` directory. `None` when sysfs has no such entry.
pub fn render_node_for_card(card: &Path) -> Option<PathBuf> {
    let card_name = card.file_name()?.to_str()?;
    let sysfs = Path::new("/sys/class/drm").join(card_name).join("device/drm");
    let entries: Vec<String> = std::fs::read_dir(sysfs)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let node = render_node_name(&entries)?;
    Some(card.parent()?.join(node))
}

/// Opens the render server on the scanout GPU (`card`, the seat's primary GPU). Any failure is returned as text for
/// the log; the caller keeps the GLES path.
pub fn probe(card: &Path) -> Result<Server, String> {
    let node = render_node_for_card(card).ok_or_else(|| format!("no render node for {}", card.display()))?;
    Server::new(Some(&node), PixelFormat::Bgra8Unorm).map_err(|e| format!("{}: {e}", node.display()))
}

/// The scanout format both sides accept: the first of XRGB8888 then ARGB8888 for which some modifier is listed by the
/// plane (in the plane's order) and by Vulkan (`device`). Returns `(fourcc, modifier)`.
pub fn pick_scanout_format(plane: &[(u32, u64)], device: &[DmaBufFormat]) -> Option<(u32, u64)> {
    [XRGB8888, ARGB8888].into_iter().find_map(|fourcc| {
        plane
            .iter()
            .filter(|(f, _)| *f == fourcc)
            .map(|(_, m)| *m)
            .find(|m| device.iter().any(|d| d.fourcc == fourcc && d.modifier == *m))
            .map(|m| (fourcc, m))
    })
}

/// The frame period for a refresh in millihertz (`Output` modes carry mHz). Zero means unknown: `FrameClock` falls
/// back to 60 Hz.
pub fn refresh_period(refresh_mhz: i32) -> Duration {
    if refresh_mhz <= 0 {
        return Duration::ZERO;
    }
    Duration::from_nanos(1_000_000_000_000 / refresh_mhz as u64)
}

/// Which pixels a frame must redraw, from the age of the buffer it renders into (pure, host-tested).
///
/// `rendered[slot]` is the frame that last drew into that buffer. A buffer of age `a` (frames since) still holds the
/// output of that frame, so it only needs the damage of the last `a` frames, the current one included. A steady
/// rotation through the ring gives age `RING`, so `a` may be up to the ring size. When the age is unknown (never
/// drawn, or forgotten after a failure) or larger, the whole output is redrawn with a clear. A redrawn frame is
/// recorded as full damage, since every buffer's old contents are then stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pacing {
    seq: u64,
    rendered: Vec<Option<u64>>,
    history: VecDeque<Vec<[i32; 4]>>,
}

impl Pacing {
    pub fn new(slots: usize) -> Pacing {
        Pacing { seq: 0, rendered: vec![None; slots], history: VecDeque::new() }
    }

    /// Plans the frame that renders into `slot`, whose scene changed in `frame_damage` (output pixels `x, y, w, h`).
    /// Returns the damage to draw and what the target holds on entry.
    pub fn plan(&mut self, slot: usize, frame_damage: &[[i32; 4]], full: [i32; 4]) -> (DamageRects, LoadAction) {
        self.seq += 1;
        let age = self.rendered.get(slot).copied().flatten().map(|r| (self.seq - r) as usize);
        let keep = matches!(age, Some(a) if a <= self.rendered.len());
        let recorded = if keep { frame_damage.to_vec() } else { vec![full] };
        self.history.push_back(recorded);
        while self.history.len() > self.rendered.len() {
            self.history.pop_front();
        }
        if let Some(r) = self.rendered.get_mut(slot) {
            *r = Some(self.seq);
        }
        match age {
            Some(a) if keep => {
                let damage: DamageRects = self.history.iter().rev().take(a).flatten().copied().collect();
                (damage, LoadAction::Load)
            }
            _ => (vec![full], LoadAction::Clear(CLEAR)),
        }
    }

    /// The frame in `slot` was not presented: its contents are unknown, so the next frame in it clears.
    pub fn forget(&mut self, slot: usize) {
        if let Some(r) = self.rendered.get_mut(slot) {
            *r = None;
        }
    }

    /// Every buffer's contents are unknown (after a VT switch or a reset of the ring).
    pub fn forget_all(&mut self) {
        self.rendered.iter_mut().for_each(|r| *r = None);
        self.history.clear();
    }
}

/// The Vulkan scanout of one output.
pub struct Vulkan {
    server: Server,
    /// Keeps the GBM device alive for the buffer objects and framebuffers.
    _gbm: GbmDevice<DrmDeviceFd>,
    /// The scanout buffer objects; the framebuffers and the render targets refer to them.
    _bos: Vec<GbmBuffer>,
    /// The KMS framebuffer of each ring slot, parallel to `targets`.
    fbs: Vec<GbmFramebuffer>,
    targets: OutputTargets,
    pacing: Pacing,
    clock: crate::render_server::clock::FrameClock,
    /// The slot whose page flip is queued, until its vblank arrives.
    in_flight: Option<usize>,
    width: u32,
    height: u32,
    shown_first: bool,
    /// The scene of the last frame that went out; `None` until one did, and after a reset (then the next frame is full).
    prev: Option<Vec<SceneNode>>,
}

/// Starts the Vulkan scanout on `card` for an output of `size` at `refresh_mhz`, on the CRTC `surface` drives. Opens
/// the render server, allocates the ring of scanout buffers, imports them as render targets and makes framebuffers.
pub fn start(card: &Path, surface: &DrmSurface, size: (i32, i32), refresh_mhz: i32) -> Result<Vulkan, String> {
    let (width, height) = (size.0.max(1) as u32, size.1.max(1) as u32);
    let server = probe(card)?;
    let fd = surface.device_fd().clone();
    let plane: Vec<(u32, u64)> = surface
        .plane_info()
        .formats
        .iter()
        .map(|f| (f.code as u32, u64::from(f.modifier)))
        .collect();
    let (fourcc, modifier) = pick_scanout_format(&plane, &server.device.supported_dmabuf_formats())
        .ok_or("no scanout format is accepted by both the primary plane and Vulkan")?;
    let code = if fourcc == XRGB8888 { Fourcc::Xrgb8888 } else { Fourcc::Argb8888 };
    let gbm = GbmDevice::new(fd.clone()).map_err(|e| format!("gbm device: {e}"))?;
    let mut allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let mut bos = Vec::with_capacity(RING);
    for _ in 0..RING {
        let bo = allocator
            .create_buffer(width, height, code, &[Modifier::from(modifier)])
            .map_err(|e| format!("scanout buffer: {e}"))?;
        bos.push(bo);
    }
    let mut fbs = Vec::with_capacity(RING);
    for bo in &bos {
        fbs.push(framebuffer_from_bo(&fd, bo, fourcc == XRGB8888).map_err(|e| format!("framebuffer: {e}"))?);
    }
    let targets = OutputTargets::from_gbm(&server.device, width, height, &bos).map_err(|e| e.to_string())?;
    eprintln!("bifrost-wm: vulkan scanout {width}x{height} {code:?} modifier {modifier:#x}, {RING} buffers");
    Ok(Vulkan {
        server,
        _gbm: gbm,
        _bos: bos,
        fbs,
        targets,
        pacing: Pacing::new(RING),
        clock: crate::render_server::clock::FrameClock::new(refresh_period(refresh_mhz)),
        in_flight: None,
        width,
        height,
        shown_first: false,
        prev: None,
    })
}

impl Vulkan {
    /// Renders and queues one frame when a buffer is free and no flip is pending. Returns whether a frame was queued.
    /// On failure the buffer is released and the caller falls back to retrying on the next timer tick.
    pub fn frame(&mut self, surface: &DrmSurface, state: &WmState, output: &Output) -> Result<bool, String> {
        if self.in_flight.is_some() {
            return Ok(false);
        }
        let started = Instant::now();
        let full = [0, 0, self.width as i32, self.height as i32];
        // the real scene, and what changed since the last frame that went out (WM-3, WM-4)
        let nodes = scene_build::build(state, output, state.start.elapsed().as_secs_f64(), &mut self.server);
        let changed = match &self.prev {
            Some(prev) => scene_build::diff_damage(prev, &nodes),
            None => vec![full],
        };
        if changed.is_empty() {
            return Ok(false);
        }
        let Some((slot, target)) = self.targets.acquire() else { return Ok(false) };
        let (damage, load) = self.pacing.plan(slot, &changed, full);
        let submission = match self.server.frame(&nodes, &target, &damage, load) {
            Ok(s) => s,
            Err(e) => return Err(self.fail(slot, format!("render: {e}"))),
        };
        // `Unsupported` means the GPU already finished the frame: no fence is needed, the flip can go at once.
        let sync: Option<OwnedFd> = match submission.sync_file() {
            Ok(fd) => Some(fd),
            Err(bifrost_gpu::Error::Unsupported(_)) => None,
            Err(e) => return Err(self.fail(slot, format!("sync_file: {e}"))),
        };
        let config = PlaneConfig {
            src: Rectangle::from_size(Size::<f64, BufferCoords>::from((self.width as f64, self.height as f64))),
            dst: Rectangle::from_size(Size::<i32, Physical>::from((self.width as i32, self.height as i32))),
            transform: Transform::Normal,
            alpha: 1.0,
            damage_clips: None,
            fb: *self.fbs[slot].as_ref(),
            fence: sync.as_ref().map(|fd| fd.as_fd()),
        };
        let plane = PlaneState { handle: surface.plane(), config: Some(config) };
        // The first commit after start may modeset (the mode is the one the CRTC shows); later frames only flip.
        let result = if surface.commit_pending() { surface.commit([plane], true) } else { surface.page_flip([plane], true) };
        if let Err(e) = result {
            return Err(self.fail(slot, format!("kms: {e}")));
        }
        self.in_flight = Some(slot);
        self.prev = Some(nodes);
        self.clock.frame_took(started.elapsed());
        Ok(true)
    }

    /// A page flip completed: the flipped buffer is on screen, and `time` (the vblank's monotonic time, when the
    /// driver reports one) feeds the frame clock.
    pub fn on_vblank(&mut self, time: Option<Duration>) {
        if let Some(slot) = self.in_flight.take() {
            self.targets.flipped(slot);
            if !self.shown_first {
                self.shown_first = true;
                eprintln!("bifrost-wm: vulkan scanout: first frame on screen");
            }
        }
        if let Some(t) = time {
            self.clock.vblank(t);
        }
    }

    /// A window capture (`SessionHooks::capture_window`): `window` alone at `size` as RGBA rows over a transparent
    /// ground, drawn by the render server from the window's current buffers. `None` when the window has no dma-buf
    /// to sample (the consumer keeps its fallback) or the render failed.
    pub fn capture_window(&mut self, state: &WmState, window: &smithay::desktop::Window, size: (i32, i32)) -> Option<Vec<u8>> {
        crate::render_server::window_shot::capture_window(&mut self.server, state, window, size)
    }

    /// Drops the importer's cache of destroyed client buffers (WM-7).
    pub fn forget_buffers(&mut self, keys: &[u64]) {
        for &k in keys {
            self.server.importer.forget(k);
        }
    }

    /// Session resume: nothing is on screen or queued, so every buffer is rewritten.
    pub fn reset(&mut self) {
        self.in_flight = None;
        self.prev = None;
        self.pacing.forget_all();
        self.targets = OutputTargets::from_textures(self.width, self.height, self.targets.targets.clone());
    }

    /// The frame-clock state (vblank phase, refresh, recent frame times).
    pub fn clock(&self) -> &crate::render_server::clock::FrameClock {
        &self.clock
    }

    fn fail(&mut self, slot: usize, message: String) -> String {
        self.targets.abandon(slot);
        self.pacing.forget(slot);
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn fmt(fourcc: u32, modifier: u64) -> DmaBufFormat {
        DmaBufFormat { fourcc, modifier }
    }

    const LINEAR: u64 = 0;
    const X_TILED: u64 = 0x0100_0000_0000_0001;
    const Y_TILED: u64 = 0x0100_0000_0000_0009;
    const FULL: [i32; 4] = [0, 0, 100, 100];

    #[test]
    fn the_render_node_is_the_renderd_entry_next_to_the_card() {
        assert_eq!(render_node_name(&names(&["card0", "renderD128"])), Some("renderD128".into()));
    }

    #[test]
    fn card_and_control_entries_are_not_render_nodes() {
        assert_eq!(render_node_name(&names(&["card1", "controlD64"])), None);
    }

    #[test]
    fn with_two_render_nodes_the_lowest_number_wins() {
        assert_eq!(render_node_name(&names(&["renderD129", "card0", "renderD128"])), Some("renderD128".into()));
    }

    #[test]
    fn a_bare_renderd_prefix_without_a_number_is_rejected() {
        assert!(!is_render_node("renderD"));
        assert!(!is_render_node("renderDx"));
    }

    #[test]
    fn a_prefix_match_with_trailing_text_is_rejected() {
        assert!(!is_render_node("renderD128-old"));
        assert!(!is_render_node("xrenderD128"));
    }

    #[test]
    fn an_empty_listing_has_no_render_node() {
        assert_eq!(render_node_name(&[]), None);
    }

    #[test]
    fn a_three_digit_node_number_is_accepted() {
        assert!(is_render_node("renderD129"));
        assert!(is_render_node("renderD1000"));
    }

    #[test]
    fn the_scanout_format_prefers_xrgb_when_both_sides_take_it() {
        let plane = [(ARGB8888, LINEAR), (XRGB8888, LINEAR)];
        let device = [fmt(ARGB8888, LINEAR), fmt(XRGB8888, LINEAR)];
        assert_eq!(pick_scanout_format(&plane, &device), Some((XRGB8888, LINEAR)));
    }

    #[test]
    fn argb_is_the_fallback_when_the_plane_has_no_xrgb() {
        let plane = [(ARGB8888, LINEAR)];
        let device = [fmt(XRGB8888, LINEAR), fmt(ARGB8888, LINEAR)];
        assert_eq!(pick_scanout_format(&plane, &device), Some((ARGB8888, LINEAR)));
    }

    #[test]
    fn the_plane_order_decides_between_modifiers() {
        // The plane lists Y-tiled first; both are usable, so the plane's first choice wins.
        let plane = [(XRGB8888, Y_TILED), (XRGB8888, LINEAR)];
        let device = [fmt(XRGB8888, LINEAR), fmt(XRGB8888, Y_TILED)];
        assert_eq!(pick_scanout_format(&plane, &device), Some((XRGB8888, Y_TILED)));
    }

    #[test]
    fn a_modifier_only_the_plane_takes_is_skipped_for_one_vulkan_takes() {
        // Vulkan cannot import X-tiled here: the plane's X-tiled entry is passed over for linear.
        let plane = [(XRGB8888, X_TILED), (XRGB8888, LINEAR)];
        let device = [fmt(XRGB8888, LINEAR)];
        assert_eq!(pick_scanout_format(&plane, &device), Some((XRGB8888, LINEAR)));
    }

    #[test]
    fn no_common_pair_means_no_vulkan_scanout() {
        let plane = [(XRGB8888, LINEAR)];
        let device = [fmt(ARGB8888, X_TILED)];
        assert_eq!(pick_scanout_format(&plane, &device), None);
        assert_eq!(pick_scanout_format(&[], &device), None);
        assert_eq!(pick_scanout_format(&plane, &[]), None);
    }

    #[test]
    fn a_sixty_hertz_mode_is_about_16_67_ms_per_frame() {
        let p = refresh_period(60_000);
        // 1e12 / 60000 = 16_666_666 ns (integer division of the nanosecond period).
        assert_eq!(p, Duration::from_nanos(16_666_666));
        let fast = refresh_period(165_000);
        assert_eq!(fast, Duration::from_nanos(6_060_606));
    }

    #[test]
    fn an_unknown_refresh_is_zero_so_the_clock_falls_back() {
        assert_eq!(refresh_period(0), Duration::ZERO);
        assert_eq!(refresh_period(-5), Duration::ZERO);
    }

    #[test]
    fn the_first_frame_into_a_buffer_clears_and_draws_everything() {
        let mut p = Pacing::new(3);
        let (damage, load) = p.plan(0, &[[10, 10, 5, 5]], FULL);
        assert_eq!(damage, vec![FULL]);
        assert_eq!(load, LoadAction::Clear(CLEAR));
    }

    #[test]
    fn a_buffer_older_than_the_ring_is_cleared() {
        let mut p = Pacing::new(3);
        p.plan(0, &[FULL], FULL); // frame 1: slot 0 cleared
        p.plan(1, &[[1, 1, 1, 1]], FULL); // frame 2: slot 1 never drawn, cleared
        p.plan(1, &[[2, 2, 2, 2]], FULL); // frame 3: slot 1 age 1
        p.plan(1, &[[3, 3, 3, 3]], FULL); // frame 4: slot 1 age 1
        // Frame 5: slot 0 was last drawn in frame 1, so its age is 4, more than the ring holds.
        let (damage, load) = p.plan(0, &[[9, 9, 1, 1]], FULL);
        assert_eq!(load, LoadAction::Clear(CLEAR));
        assert_eq!(damage, vec![FULL]);
    }

    #[test]
    fn a_buffer_of_age_two_gets_the_last_two_frames_damage() {
        let mut p = Pacing::new(3);
        p.plan(0, &[FULL], FULL); // frame 1: slot 0 cleared
        p.plan(1, &[[0, 0, 1, 1]], FULL); // frame 2: slot 1 has never been drawn: a clear, recorded as full
        let (damage, load) = p.plan(0, &[[50, 50, 10, 10]], FULL); // frame 3: slot 0 age 2
        assert_eq!(load, LoadAction::Load);
        // Frames 2 (recorded full, since it cleared) and 3 (this frame's change).
        assert_eq!(damage, vec![[50, 50, 10, 10], FULL]);
    }

    #[test]
    fn a_forgotten_buffer_is_cleared_on_its_next_use() {
        let mut p = Pacing::new(3);
        p.plan(0, &[FULL], FULL);
        p.plan(1, &[[1, 1, 1, 1]], FULL);
        p.forget(0);
        let (_, load) = p.plan(0, &[[2, 2, 2, 2]], FULL);
        assert_eq!(load, LoadAction::Clear(CLEAR));
    }

    #[test]
    fn steady_triple_buffering_clears_only_the_first_round() {
        // Frames rotate through three buffers: the first three frames clear (never drawn), then every buffer is age 3
        // and keeps. Frame 4 (slot 0) needs frames 2, 3 (both cleared, so full) and its own small change.
        let mut p = Pacing::new(3);
        let small = [0, 0, 1, 1];
        let mut clears = 0;
        let mut frame4 = None;
        for frame in 0..9 {
            let (damage, load) = p.plan(frame % 3, &[small], FULL);
            if matches!(load, LoadAction::Clear(_)) {
                clears += 1;
            }
            if frame == 3 {
                frame4 = Some((damage, load));
            }
        }
        assert_eq!(clears, 3);
        assert_eq!(frame4, Some((vec![small, FULL, FULL], LoadAction::Load)));
    }

    #[test]
    fn a_buffer_drawn_twice_in_a_row_is_age_one_and_keeps() {
        let mut p = Pacing::new(3);
        p.plan(0, &[FULL], FULL);
        let (damage, load) = p.plan(0, &[[5, 5, 2, 2]], FULL);
        assert_eq!(load, LoadAction::Load);
        assert_eq!(damage, vec![[5, 5, 2, 2]]);
    }

    #[test]
    fn forget_all_clears_every_buffer_and_the_history() {
        let mut p = Pacing::new(2);
        p.plan(0, &[FULL], FULL);
        p.plan(1, &[FULL], FULL);
        p.forget_all();
        let (_, load) = p.plan(1, &[[1, 1, 1, 1]], FULL);
        assert_eq!(load, LoadAction::Clear(CLEAR));
    }

    #[test]
    fn an_unchanged_scene_has_no_damage() {
        let a = vec![SceneNode::solid([0.0, 0.0, 10.0, 10.0], [1.0; 4])];
        assert!(scene_build::diff_damage(&a, &a.clone()).is_empty());
        assert!(!scene_build::diff_damage(&[], &a).is_empty());
    }
}
