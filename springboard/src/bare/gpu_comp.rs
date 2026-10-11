//! GPU compositing for the app open/close zoom. The home screen and the app
//! frame are uploaded once when the animation starts; every frame then draws
//! the shadow and the rounded, scaled card on the GPU and copies only the
//! changed area out. Any GPU error switches the compositor off for good and
//! the shell keeps its CPU path. The geometry (rect, radius) comes from the
//! shell; this file only maps it to draw calls and damage.

use crate::layout::Rect;

/// `AKURAI_GPU=0` forces the CPU path; anything else (or unset) allows the GPU.
/// The same reading serves `AKURAI_GPU_ROTATE` (GPU rotation of landscape frames) and
/// `AKURAI_GPU_VIDEO` (video pictures drawn by the GPU).
pub fn enabled(var: Option<&str>) -> bool {
    var.map(str::trim) != Some("0")
}

/// Blur radius and opacity of the card's shadow.
pub const SHADOW_BLUR: f32 = 28.0;
pub const SHADOW_ALPHA: f32 = 0.35;

/// Pixels around the card the shadow can touch.
pub fn shadow_margin() -> i32 {
    (SHADOW_BLUR * 1.5).ceil() as i32
}

/// What the compositor does for one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// not animating, nothing to do
    Nothing,
    /// upload the snapshots (then draw)
    Snapshot,
    /// draw with the snapshots already uploaded
    Draw,
    /// the animation ended: free the textures, CPU repaints everything
    Free,
    /// the kind changed mid-flight (open to close): free, then snapshot again
    Restart,
}

/// Snapshot/free state machine. `opening` is Some while a zoom plays.
#[derive(Default, Debug)]
pub struct Phase {
    active: Option<bool>,
}

impl Phase {
    pub fn next(&mut self, opening: Option<bool>) -> Action {
        let act = match (self.active, opening) {
            (None, None) => Action::Nothing,
            (None, Some(_)) => Action::Snapshot,
            (Some(a), Some(b)) if a == b => Action::Draw,
            (Some(_), Some(_)) => Action::Restart,
            (Some(_), None) => Action::Free,
        };
        self.active = opening;
        act
    }

    pub fn active(&self) -> bool {
        self.active.is_some()
    }
}

/// Where the GPU draws the card for a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub dst: Rect,
    pub radius: f32,
    /// whether the card is smaller than the screen and so casts a shadow
    pub shadow: bool,
}

pub fn plan(rect: Rect, radius: f32, screen: (usize, usize)) -> Draw {
    Draw { dst: rect, radius, shadow: rect.w < screen.0 as i32 || rect.h < screen.1 as i32 }
}

/// The area that changed since the previous frame: both card rects plus the shadow margin,
/// clipped to the screen. The first frame (no previous) is the whole screen.
pub fn damage(prev: Option<Rect>, cur: Rect, screen: (usize, usize)) -> Rect {
    let full = Rect { x: 0, y: 0, w: screen.0 as i32, h: screen.1 as i32 };
    let Some(prev) = prev else { return full };
    let m = shadow_margin();
    let grow = |r: Rect| Rect { x: r.x - m, y: r.y - m + m / 2, w: r.w + 2 * m, h: r.h + 2 * m + m };
    let (a, b) = (grow(prev), grow(cur));
    let (x0, y0) = (a.x.min(b.x).max(0), a.y.min(b.y).max(0));
    let (x1, y1) = ((a.x + a.w).max(b.x + b.w).min(full.w), (a.y + a.h).max(b.y + b.h).min(full.h));
    if x1 <= x0 || y1 <= y0 {
        return Rect { x: 0, y: 0, w: 0, h: 0 };
    }
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// The parts of `area` outside `inner` (up to four strips): the card is opaque inside
/// `inner`, so the home layer only has to be drawn in these.
pub fn uncovered(area: Rect, inner: Rect) -> Vec<Rect> {
    let x0 = inner.x.max(area.x);
    let y0 = inner.y.max(area.y);
    let x1 = (inner.x + inner.w).min(area.x + area.w);
    let y1 = (inner.y + inner.h).min(area.y + area.h);
    if x1 <= x0 || y1 <= y0 {
        return vec![area];
    }
    let mut out = Vec::with_capacity(4);
    if y0 > area.y {
        out.push(Rect { x: area.x, y: area.y, w: area.w, h: y0 - area.y });
    }
    if y1 < area.y + area.h {
        out.push(Rect { x: area.x, y: y1, w: area.w, h: area.y + area.h - y1 });
    }
    if x0 > area.x {
        out.push(Rect { x: area.x, y: y0, w: x0 - area.x, h: y1 - y0 });
    }
    if x1 < area.x + area.w {
        out.push(Rect { x: x1, y: y0, w: area.x + area.w - x1, h: y1 - y0 });
    }
    out
}

/// The card's opaque interior: its rect pulled in by the corner radius.
pub fn opaque_inner(card: Rect, radius: f32) -> Rect {
    let r = radius.ceil() as i32 + 1;
    Rect { x: card.x + r, y: card.y + r, w: (card.w - 2 * r).max(0), h: (card.h - 2 * r).max(0) }
}

/// Holds the GPU's devfreq minimum at its maximum while animations play, so the GPU does
/// not render at its idle clock, and puts it back after.
pub struct GpuClock {
    dir: std::path::PathBuf,
    low: Option<String>,
    raised: bool,
}

impl GpuClock {
    pub const DEVFREQ: &'static str = "/sys/class/devfreq/60000000.gpu";

    pub fn at(dir: &str) -> Self {
        let dir = std::path::PathBuf::from(dir);
        let low = std::fs::read_to_string(dir.join("min_freq")).ok().map(|s| s.trim().to_string());
        GpuClock { dir, low, raised: false }
    }

    pub fn boost(&mut self) {
        if self.raised || self.low.is_none() {
            return;
        }
        if let Ok(max) = std::fs::read_to_string(self.dir.join("max_freq")) {
            self.raised = std::fs::write(self.dir.join("min_freq"), max.trim()).is_ok();
        }
    }

    pub fn release(&mut self) {
        if let (true, Some(low)) = (self.raised, &self.low) {
            let _ = std::fs::write(self.dir.join("min_freq"), low);
            self.raised = false;
        }
    }
}

impl Drop for GpuClock {
    fn drop(&mut self) {
        self.release();
    }
}

/// Which textures to upload and which to free so that exactly `want` are live: images are
/// keyed by generation, so an unchanged key is never uploaded again.
pub fn plan_uploads<K: Copy + PartialEq>(have: &[K], want: &[K]) -> (Vec<K>, Vec<K>) {
    let up = want.iter().copied().filter(|k| !have.contains(k)).collect();
    let free = have.iter().copied().filter(|k| !want.contains(k)).collect();
    (up, free)
}

/// What `GpuComp::step` tells the shell loop.
pub enum Step {
    /// draw on the CPU as usual
    Cpu,
    /// the frame buffer holds the new frame; present this damage (empty = nothing changed)
    Presented { damage: Rect, gpu_ms: f64 },
    /// GPU frames just ended (or failed): the shell must repaint fully with the CPU
    Ended,
}

#[cfg(target_os = "linux")]
pub use imp::GpuComp;
#[cfg(target_os = "linux")]
pub use crate::gpu::layer::{Composed, VideoInput};
pub use crate::gpu::plan::{Caps, Path};

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::bare::sys;
    use crate::gfx::rotate::Rotation;
    use crate::gfx::{Area, Image};
    use crate::gpu::layer::{Composed, Layers, VideoInput};
    use crate::gpu::{plan, Gpu, TextureId};
    use crate::shell::switcher::{SwitcherView, TexKey};
    use std::collections::HashMap;

    struct Live {
        home: TextureId,
        card: TextureId,
    }

    /// Lazily created GPU compositor; `Off` after the first failure.
    pub struct GpuComp {
        allowed: bool,
        failed: bool,
        gpu: Option<Gpu>,
        phase: Phase,
        live: Option<Live>,
        prev: Option<Rect>,
        screen: (usize, usize),
        clock: GpuClock,
        /// the video / rotation compositor, built on first use
        layers: Option<Layers>,
        /// building or running it failed: those frames stay on the CPU (the zoom keeps the GPU)
        layers_failed: bool,
        rotate_allowed: bool,
        video_allowed: bool,
        /// the switcher's textures by key (uploaded once per generation)
        sw: HashMap<TexKey, TextureId>,
        /// the switcher path failed: its frames stay on the CPU (the zoom keeps the GPU)
        sw_failed: bool,
        /// textures uploaded by the last switcher frame (frame trace)
        sw_uploads: usize,
    }

    impl GpuComp {
        /// Zero-copy output: probes the display's DRM device, allocates 3 buffers and renders
        /// into them. Returns the log line; any error leaves the copy path in force.
        pub fn enable_scanout(&mut self, disp: &mut crate::bare::kms::Display) -> Result<String, String> {
            use crate::bare::scanout::{fourcc_name, Method};
            let (w, h) = self.screen;
            let gpu = self.gpu.as_mut().ok_or("no gpu")?;
            let (d, _log) = crate::gpu::scanout::probe(gpu, disp.fd(), w, h);
            if d.method == Method::Copy {
                return Err("probe chose the copy path".into());
            }
            let bufs = disp.scanout_alloc(d.method, d.fourcc, 3).map_err(|e| format!("alloc: {e}"))?;
            match gpu.enable_scanout(bufs) {
                Ok(()) => Ok(format!("{} {}", d.method.name(), fourcc_name(d.fourcc))),
                Err((e, bufs)) => {
                    disp.scanout_free(bufs);
                    Err(format!("enable: {e}"))
                }
            }
        }

        pub fn scanout_active(&self) -> bool {
            self.gpu.as_ref().is_some_and(|g| g.scanout_active())
        }

        /// The framebuffer holding the frame just drawn, to page-flip (None: nothing changed).
        pub fn take_flip(&mut self) -> Option<u32> {
            self.gpu.as_mut()?.take_flip()
        }

        /// Back to the copy path; the buffers go to `Display::scanout_free` once the panel
        /// shows a CPU frame.
        pub fn disable_scanout(&mut self) -> Vec<crate::bare::scanout::ScanoutBuf> {
            self.gpu.as_mut().map(|g| g.disable_scanout()).unwrap_or_default()
        }

        pub fn new(w: usize, h: usize) -> Self {
            let allowed = enabled(std::env::var("AKURAI_GPU").ok().as_deref());
            let rotate_allowed = enabled(std::env::var("AKURAI_GPU_ROTATE").ok().as_deref());
            let video_allowed = enabled(std::env::var("AKURAI_GPU_VIDEO").ok().as_deref());
            GpuComp {
                allowed,
                failed: false,
                gpu: None,
                phase: Phase::default(),
                live: None,
                prev: None,
                screen: (w, h),
                clock: GpuClock::at(GpuClock::DEVFREQ),
                layers: None,
                layers_failed: false,
                rotate_allowed,
                video_allowed,
                sw: HashMap::new(),
                sw_failed: false,
                sw_uploads: 0,
            }
        }

        /// The Wayland desktop renders through `gpu::wl::Presenter`: this compositor stays off there,
        /// so two EGL contexts never compete on the render thread.
        pub fn disable(&mut self) {
            self.allowed = false;
            self.gpu = None;
        }

        /// Creates the GPU context ahead of the first animation (it takes ~60 ms), when nothing
        /// is on screen to stutter. A failure turns the compositor off, like any later one.
        pub fn warm(&mut self) {
            if !self.allowed || self.failed || self.gpu.is_some() {
                return;
            }
            match Gpu::new(self.screen.0, self.screen.1) {
                Ok(g) => self.gpu = Some(g),
                Err(e) => {
                    eprintln!("springboard: gpu compositor off: {e}");
                    self.failed = true;
                }
            }
        }

        /// For the frame trace: why the CPU path is being used.
        pub fn state(&self) -> String {
            let src = self.layers.as_ref().map_or("-", |l| match l.source() {
                plan::Source::External => "nv12",
                plan::Source::Upload => "upload",
            });
            format!("allowed={} failed={} gpu={} layers_failed={} video_source={src}", self.allowed, self.failed, self.gpu.is_some(), self.layers_failed)
        }

        /// What the layer compositor can do now (see `plan::choose`).
        pub fn caps(&self) -> Caps {
            let gpu = self.allowed && !self.failed && self.gpu.is_some();
            Caps { gpu, layers: gpu && !self.layers_failed, rotate: self.rotate_allowed }
        }

        /// Whether apps should hand their video to the GPU (and stop converting pictures).
        pub fn video_capable(&self) -> bool {
            self.video_allowed && self.caps().layers
        }

        /// The next composed frame redraws in full (a CPU or zoom frame was presented meanwhile).
        pub fn invalidate_layers(&mut self) {
            if let Some(l) = self.layers.as_mut() {
                l.invalidate();
            }
        }

        /// One frame through the layer compositor (`plan::Path::Rotate` or `Video`): the
        /// changed part lands in `out` (the panel-sized presenter buffer) and its panel
        /// rectangles come back. None means draw this frame on the CPU: the frame does not fit
        /// the target, or the compositor failed (then it stays off and says why once).
        #[allow(clippy::too_many_arguments)]
        pub fn compose(&mut self, video: Option<VideoInput>, frame: &[u32], fw: usize, fh: usize, areas: &[Area], rot: Rotation, full: bool, out: &mut [u32]) -> Option<Composed> {
            if !self.caps().layers {
                return None;
            }
            let gpu = self.gpu.as_mut()?;
            if self.layers.is_none() {
                match Layers::new(gpu) {
                    Ok(l) => self.layers = Some(l),
                    Err(e) => {
                        eprintln!("springboard: gpu layers off: {e}");
                        self.layers_failed = true;
                        return None;
                    }
                }
            }
            let layers = self.layers.as_mut()?;
            match layers.compose(gpu, video, frame, fw, fh, areas, rot, full, out) {
                Ok(c) => Some(c),
                Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => None,
                Err(e) => {
                    eprintln!("springboard: gpu layers off: {e}");
                    self.layers_failed = true;
                    if let Some(l) = self.layers.take() {
                        l.destroy(gpu);
                    }
                    None
                }
            }
        }

        /// One frame of the rotation turn through the layer compositor: the whole panel lands
        /// in `out`. None means draw it on the CPU (the frames do not fit, or it failed and the
        /// compositor stays off for layers).
        #[allow(clippy::too_many_arguments)]
        pub fn turn(&mut self, id: u64, old: &Image, new: &Image, from: Rotation, k: f32, rot: Rotation, out: &mut [u32]) -> Option<Composed> {
            if !self.caps().layers {
                return None;
            }
            let gpu = self.gpu.as_mut()?;
            if self.layers.is_none() {
                match Layers::new(gpu) {
                    Ok(l) => self.layers = Some(l),
                    Err(e) => {
                        eprintln!("springboard: gpu layers off: {e}");
                        self.layers_failed = true;
                        return None;
                    }
                }
            }
            self.clock.boost();
            let layers = self.layers.as_mut()?;
            match layers.turn(gpu, id, old, new, from, k, rot, out) {
                Ok(c) => Some(c),
                Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => None,
                Err(e) => {
                    eprintln!("springboard: gpu turn off: {e}");
                    self.layers_failed = true;
                    self.clock.release();
                    if let Some(l) = self.layers.take() {
                        l.destroy(gpu);
                    }
                    None
                }
            }
        }

        /// No turn plays: frees its textures and lets the GPU clock down (cheap when idle).
        pub fn turn_over(&mut self) {
            if let (Some(l), Some(g)) = (self.layers.as_mut(), self.gpu.as_ref()) {
                if l.turn_live() {
                    l.end_turn(g);
                    self.clock.release();
                }
            }
        }

        /// One animation frame. `view` is the shell's current zoom (None when no
        /// plain zoom plays). `home` renders the home screen into the given
        /// pixel buffer (w*h) when closing; `frame` is the CPU frame the presenter reads.
        pub fn step(&mut self, view: Option<(&Image, Rect, f32, bool)>, frame: &mut [u32], home: &mut dyn FnMut(&mut [u32])) -> Step {
            if !self.allowed || self.failed {
                return Step::Cpu;
            }
            let action = self.phase.next(view.map(|v| v.3));
            match action {
                Action::Nothing => Step::Cpu,
                Action::Free => {
                    self.release();
                    self.clock.release();
                    Step::Ended
                }
                Action::Snapshot | Action::Restart | Action::Draw => {
                    let Some((card, rect, radius, opening)) = view else { return Step::Cpu };
                    if matches!(action, Action::Restart) {
                        self.release();
                    }
                    self.clock.boost();
                    let t0 = sys::now_ms();
                    match self.draw(action != Action::Draw, card, rect, radius, opening, frame, home) {
                        Ok(damage) => Step::Presented { damage, gpu_ms: sys::now_ms() - t0 },
                        Err(e) => {
                            eprintln!("springboard: gpu compositor off: {e}");
                            self.failed = true;
                            self.clock.release();
                            self.release();
                            if let (Some(l), Some(g)) = (self.layers.take(), self.gpu.as_ref()) {
                                l.destroy(g);
                            }
                            self.sw.clear();
                            self.gpu = None;
                            Step::Ended
                        }
                    }
                }
            }
        }

        /// Textures the last switcher frame uploaded (the frame trace).
        pub fn switcher_uploads(&self) -> usize {
            self.sw_uploads
        }

        /// The switcher is not on screen: frees its textures and lets the GPU clock down.
        /// True when it was live (the shell must then repaint with the CPU).
        pub fn switcher_over(&mut self) -> bool {
            if self.sw.is_empty() {
                return false;
            }
            if let Some(g) = self.gpu.as_mut() {
                for (_, t) in self.sw.drain() {
                    g.free(t);
                }
            }
            self.sw.clear();
            if !self.phase.active() {
                self.clock.release();
            }
            true
        }

        /// One frame of the app switcher: backdrop, bar, dock and cards as quads over textures
        /// uploaded once per generation. The whole frame lands in `frame`. Cpu means draw it on
        /// the CPU (GPU off, or the switcher path failed, which stays off for good).
        pub fn switcher(&mut self, view: &SwitcherView, frame: &mut [u32]) -> Step {
            if !self.allowed || self.failed || self.sw_failed {
                return Step::Cpu;
            }
            let t0 = sys::now_ms();
            match self.draw_switcher(view, frame) {
                Ok(()) => {
                    let (w, h) = self.screen;
                    Step::Presented { damage: Rect { x: 0, y: 0, w: w as i32, h: h as i32 }, gpu_ms: sys::now_ms() - t0 }
                }
                Err(e) => {
                    eprintln!("springboard: gpu switcher off: {e}");
                    self.sw_failed = true;
                    self.switcher_over();
                    Step::Cpu
                }
            }
        }

        fn draw_switcher(&mut self, view: &SwitcherView, frame: &mut [u32]) -> std::io::Result<()> {
            let (w, h) = self.screen;
            if self.gpu.is_none() {
                self.gpu = Some(Gpu::new(w, h)?);
            }
            let gpu = self.gpu.as_mut().ok_or_else(|| std::io::Error::other("no gpu"))?;
            self.clock.boost();
            let have: Vec<TexKey> = self.sw.keys().copied().collect();
            let (up, free) = plan_uploads(&have, &view.keys());
            for k in free {
                if let Some(t) = self.sw.remove(&k) {
                    gpu.free(t);
                }
            }
            self.sw_uploads = up.len();
            for k in up {
                let img = view.image(k).ok_or_else(|| std::io::Error::other("no image"))?;
                let t = gpu.upload(img)?;
                self.sw.insert(k, t);
            }
            gpu.begin_frame();
            gpu.clip(None);
            for q in &view.layers {
                if let Some(&t) = self.sw.get(&q.key) {
                    gpu.draw_texture(t, Rect { x: 0, y: 0, w: q.image.w as i32, h: q.image.h as i32 }, q.rect, q.radius, q.alpha);
                }
            }
            let fence = gpu.end_frame();
            gpu.present(fence, frame, w, &[Rect { x: 0, y: 0, w: w as i32, h: h as i32 }]);
            if let Some(l) = self.layers.as_mut() {
                l.invalidate();
            }
            self.prev = None;
            Ok(())
        }

        fn release(&mut self) {
            if let (Some(gpu), Some(l)) = (self.gpu.as_mut(), self.live.take()) {
                gpu.free(l.home);
                gpu.free(l.card);
            }
            self.live = None;
            self.prev = None;
        }

        #[allow(clippy::too_many_arguments)]
        fn draw(&mut self, snapshot: bool, card: &Image, rect: Rect, radius: f32, opening: bool, frame: &mut [u32], home: &mut dyn FnMut(&mut [u32])) -> std::io::Result<Rect> {
            let (w, h) = self.screen;
            if self.gpu.is_none() {
                self.gpu = Some(Gpu::new(w, h)?);
            }
            let gpu = self.gpu.as_mut().ok_or_else(|| std::io::Error::other("no gpu"))?;
            if snapshot {
                // opening: the CPU frame still shows home; closing: it shows the app, so render home
                let mut px = if opening { frame.to_vec() } else { vec![0u32; w * h] };
                if !opening {
                    home(&mut px);
                }
                let home_tex = gpu.upload(&Image { w, h, px })?;
                match gpu.upload(card) {
                    Ok(card_tex) => self.live = Some(Live { home: home_tex, card: card_tex }),
                    Err(e) => {
                        gpu.free(home_tex);
                        return Err(e);
                    }
                }
                self.prev = None;
            }
            let live = self.live.as_ref().ok_or_else(|| std::io::Error::other("no snapshot"))?;
            let d = plan(rect, radius, self.screen);
            let area = damage(self.prev, rect, self.screen);
            let full = Rect { x: 0, y: 0, w: w as i32, h: h as i32 };
            gpu.begin_frame();
            gpu.clip(Some(area));
            for strip in uncovered(area, opaque_inner(d.dst, d.radius)) {
                gpu.clip(Some(strip));
                gpu.draw_texture(live.home, full, full, 0.0, 1.0);
            }
            gpu.clip(Some(area));
            if d.shadow {
                gpu.shadow(d.dst, d.radius, SHADOW_BLUR, [0.0, 0.0, 0.0, SHADOW_ALPHA]);
            }
            gpu.draw_texture(live.card, Rect { x: 0, y: 0, w: card.w as i32, h: card.h as i32 }, d.dst, d.radius, 1.0);
            let fence = gpu.end_frame();
            gpu.present(fence, frame, w, &[area]);
            self.prev = Some(rect);
            if let Some(l) = self.layers.as_mut() {
                l.invalidate();
            }
            Ok(area)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: (usize, usize) = (1200, 1920);

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn uploads_only_what_is_new_and_frees_what_left() {
        assert_eq!(plan_uploads(&[1, 2], &[1, 2]), (vec![], vec![]), "same generations: nothing to upload");
        assert_eq!(plan_uploads::<u32>(&[], &[1, 2]), (vec![1, 2], vec![]));
        assert_eq!(plan_uploads(&[1, 2, 3], &[2, 4]), (vec![4], vec![1, 3]));
        assert_eq!(plan_uploads(&[5], &[]), (vec![], vec![5]));
    }

    #[test]
    fn env_zero_forces_cpu() {
        assert!(!enabled(Some("0")));
        assert!(!enabled(Some(" 0\n")));
        assert!(enabled(Some("1")));
        assert!(enabled(None));
    }

    #[test]
    fn phase_snapshots_once_draws_then_frees() {
        let mut p = Phase::default();
        assert_eq!(p.next(None), Action::Nothing);
        assert_eq!(p.next(Some(true)), Action::Snapshot);
        assert!(p.active());
        assert_eq!(p.next(Some(true)), Action::Draw);
        assert_eq!(p.next(Some(true)), Action::Draw);
        assert_eq!(p.next(None), Action::Free);
        assert!(!p.active());
        assert_eq!(p.next(None), Action::Nothing);
    }

    #[test]
    fn phase_restarts_when_open_turns_into_close() {
        let mut p = Phase::default();
        p.next(Some(true));
        assert_eq!(p.next(Some(false)), Action::Restart);
        assert_eq!(p.next(Some(false)), Action::Draw);
    }

    #[test]
    fn shadow_only_when_card_is_smaller_than_screen() {
        assert!(plan(r(100, 100, 600, 800), 20.0, S).shadow);
        assert!(!plan(r(0, 0, 1200, 1920), 0.0, S).shadow);
        assert_eq!(plan(r(1, 2, 3, 4), 5.0, S).dst, r(1, 2, 3, 4));
        assert_eq!(plan(r(1, 2, 3, 4), 5.0, S).radius, 5.0);
    }

    #[test]
    fn first_frame_damages_the_whole_screen() {
        assert_eq!(damage(None, r(10, 10, 50, 50), S), r(0, 0, 1200, 1920));
    }

    #[test]
    fn damage_covers_both_rects_and_the_shadow_inside_the_screen() {
        let (a, b) = (r(500, 800, 100, 100), r(400, 700, 300, 300));
        let d = damage(Some(a), b, S);
        let m = shadow_margin();
        assert!(d.x <= 400 - m && d.x + d.w >= 700 + m);
        // the shadow is drawn m/2 lower, so only half the margin is needed above the card
        assert!(d.y <= 700 - m / 2 && d.y + d.h >= 1000 + m + m / 2);
        let full = r(0, 0, 1200, 1920);
        assert_eq!(damage(Some(full), full, S), full, "clipped to the screen");
    }

    #[test]
    fn uncovered_is_the_ring_around_the_inner_rect() {
        let area = r(0, 0, 100, 100);
        assert_eq!(uncovered(area, r(10, 20, 50, 40)), vec![r(0, 0, 100, 20), r(0, 60, 100, 40), r(0, 20, 10, 40), r(60, 20, 40, 40)]);
        assert_eq!(uncovered(area, r(0, 0, 100, 100)), vec![]);
        assert_eq!(uncovered(area, r(200, 200, 5, 5)), vec![area], "no overlap: all of it");
        assert_eq!(uncovered(area, r(0, 0, 0, 0)), vec![area], "empty inner");
        let pixels: i32 = uncovered(area, r(10, 20, 50, 40)).iter().map(|s| s.w * s.h).sum();
        assert_eq!(pixels, 100 * 100 - 50 * 40);
    }

    #[test]
    fn opaque_inner_is_pulled_in_by_the_radius() {
        assert_eq!(opaque_inner(r(0, 0, 200, 300), 20.0), r(21, 21, 158, 258));
        assert_eq!(opaque_inner(r(0, 0, 10, 10), 20.0).w, 0);
    }

    #[test]
    fn gpu_clock_raises_min_to_max_and_restores() {
        let dir = std::env::temp_dir().join(format!("gpuclk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("min_freq"), "384000000\n").unwrap();
        std::fs::write(dir.join("max_freq"), "850000000\n").unwrap();
        let mut c = GpuClock::at(dir.to_str().unwrap());
        c.boost();
        assert_eq!(std::fs::read_to_string(dir.join("min_freq")).unwrap(), "850000000");
        c.boost();
        c.release();
        assert_eq!(std::fs::read_to_string(dir.join("min_freq")).unwrap(), "384000000");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gpu_clock_without_devfreq_is_inert() {
        let mut c = GpuClock::at("/nonexistent/devfreq");
        c.boost();
        c.release();
    }

    #[test]
    fn damage_off_screen_is_empty() {
        let d = damage(Some(r(-900, -900, 10, 10)), r(-800, -800, 10, 10), S);
        assert!(d.w <= 0 || d.h <= 0 || d.x >= 0);
        assert!(d.w >= 0 && d.h >= 0);
    }
}
