//! Two-dimensional scenes as a pure model: the scene's size, how it is scaled and placed in the view that shows it, the
//! conversion between scene and view points, the frame cycle that orders one frame of work, and the positional mix of
//! an audio node heard from a listener node. It mirrors the reference SDK's 2D scene object, its four scale modes, its
//! frame-cycle order and its positional audio rule. The model draws nothing, opens no audio output and reads no clock:
//! the caller passes the time in seconds and runs the phases in the order `FRAME_PHASES` gives.
//!
//! Coordinates follow the reference SDK: the view and the scene both have their origin at the bottom-left and y points
//! up. The anchor is a fraction from 0 to 1 of the free space left after scaling. Anchor (0, 0) puts the scene's
//! bottom-left corner at the view's bottom-left, and (0.5, 0.5) centres the scene. Where the scene is larger than the
//! view, the anchor decides which part is cut away.

/// How the scene's area maps to the view that presents it. The four modes match the reference SDK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    /// Each axis is scaled on its own, so the scene fills the view exactly.
    Fill,
    /// One factor for both axes, the larger of the two. The view is filled and the scene may be cropped.
    AspectFill,
    /// One factor for both axes, the smaller of the two. The whole scene shows and the view may be letterboxed.
    AspectFit,
    /// No scaling: the scene takes the size of the view.
    ResizeFill,
}

/// The phases of one frame, in the order the reference SDK runs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePhase {
    /// The app's own per-frame logic.
    Update,
    /// Running the actions attached to nodes.
    EvaluateActions,
    /// Stepping the physics world.
    SimulatePhysics,
    /// Applying the node constraints.
    ApplyConstraints,
    /// The last step, after the animation work of the frame is done.
    FinishUpdate,
}

/// Every phase of one frame, in run order.
pub const FRAME_PHASES: [FramePhase; 5] = [
    FramePhase::Update,
    FramePhase::EvaluateActions,
    FramePhase::SimulatePhysics,
    FramePhase::ApplyConstraints,
    FramePhase::FinishUpdate,
];

/// A scene of a fixed size, shown in a view of another size.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene2d {
    width: f32,
    height: f32,
    view_width: f32,
    view_height: f32,
    scale_mode: ScaleMode,
    anchor: (f32, f32),
}

/// Keeps an anchor fraction in 0 to 1. A NaN reads as 0.
fn unit(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

impl Scene2d {
    /// A scene of the given size, with no view yet (size 0 by 0). Negative sizes count as 0. The default mode is
    /// `Fill` and the anchor is (0, 0).
    pub fn new(width: f32, height: f32) -> Self {
        Scene2d {
            width: width.max(0.0),
            height: height.max(0.0),
            view_width: 0.0,
            view_height: 0.0,
            scale_mode: ScaleMode::Fill,
            anchor: (0.0, 0.0),
        }
    }

    /// Sets the size of the view that shows the scene. Negative sizes count as 0.
    pub fn set_view_size(&mut self, width: f32, height: f32) {
        self.view_width = width.max(0.0);
        self.view_height = height.max(0.0);
    }

    pub fn set_scale_mode(&mut self, mode: ScaleMode) {
        self.scale_mode = mode;
    }

    pub fn scale_mode(&self) -> ScaleMode {
        self.scale_mode
    }

    /// Sets the anchor fraction for each axis. Values are kept in 0 to 1.
    pub fn set_anchor(&mut self, x: f32, y: f32) {
        self.anchor = (unit(x), unit(y));
    }

    /// The size of the scene in scene points. `ResizeFill` takes the view's size; every other mode keeps the size given.
    pub fn size(&self) -> (f32, f32) {
        match self.scale_mode {
            ScaleMode::ResizeFill => (self.view_width, self.view_height),
            _ => (self.width, self.height),
        }
    }

    /// The factor from scene points to view points on each axis. An empty scene axis keeps a factor of 1, so a zero
    /// size never divides by zero. The aspect modes use one factor on both axes.
    pub fn scale(&self) -> (f32, f32) {
        let fx = if self.width > 0.0 { self.view_width / self.width } else { 1.0 };
        let fy = if self.height > 0.0 { self.view_height / self.height } else { 1.0 };
        match self.scale_mode {
            ScaleMode::Fill => (fx, fy),
            ScaleMode::ResizeFill => (1.0, 1.0),
            ScaleMode::AspectFill | ScaleMode::AspectFit if self.width <= 0.0 || self.height <= 0.0 => (1.0, 1.0),
            ScaleMode::AspectFill => {
                let f = fx.max(fy);
                (f, f)
            }
            ScaleMode::AspectFit => {
                let f = fx.min(fy);
                (f, f)
            }
        }
    }

    /// The scene's origin in view points. The free space left after scaling is split by the anchor. Where the scaled
    /// scene is larger than the view, the free space is negative and the anchor picks the part that is cut away.
    pub fn origin(&self) -> (f32, f32) {
        let (sw, sh) = self.size();
        let (fx, fy) = self.scale();
        let free_x = self.view_width - sw * fx;
        let free_y = self.view_height - sh * fy;
        (self.anchor.0 * free_x, self.anchor.1 * free_y)
    }

    /// Maps a scene point to a view point.
    pub fn scene_to_view(&self, x: f32, y: f32) -> (f32, f32) {
        let (ox, oy) = self.origin();
        let (fx, fy) = self.scale();
        (ox + x * fx, oy + y * fy)
    }

    /// Maps a view point to a scene point. An axis with a zero factor maps to 0, so the result is always finite.
    pub fn view_to_scene(&self, x: f32, y: f32) -> (f32, f32) {
        let (ox, oy) = self.origin();
        let (fx, fy) = self.scale();
        let sx = if fx != 0.0 { (x - ox) / fx } else { 0.0 };
        let sy = if fy != 0.0 { (y - oy) / fy } else { 0.0 };
        (sx, sy)
    }
}

/// The frame clock: the step each frame passes to the update phase. It reads no clock; the caller passes the time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameClock {
    last: Option<f64>,
    paused: bool,
}

impl FrameClock {
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Starts a frame at time `now` in seconds. It returns the seconds since the previous frame, which is 0 for the
    /// first frame and never negative when the clock goes back. A paused frame returns `None`, and it still moves the
    /// reference time, so the time spent paused is not simulated when the clock resumes.
    pub fn begin_frame(&mut self, now: f64) -> Option<f32> {
        let previous = self.last.replace(now);
        if self.paused {
            return None;
        }
        match previous {
            None => Some(0.0),
            Some(p) => Some((now - p).max(0.0) as f32),
        }
    }
}

/// The stereo balance and volume of one audio node, as the positional rule sets them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    /// From 0 (silent) to 1 (full).
    pub volume: f32,
    /// From -1 (left edge) to 1 (right edge).
    pub pan: f32,
}

/// The mix for an audio node at `node`, heard by a listener at `listener`. Inside `reference` the volume is 1. It falls
/// linearly to 0 at `max` and stays 0 beyond it. The pan is the node's horizontal offset from the listener over `max`,
/// clamped to -1 to 1. A node that is not positional plays at full volume and centred.
pub fn positional_mix(
    node: (f32, f32),
    listener: (f32, f32),
    is_positional: bool,
    reference: f32,
    max: f32,
) -> Mix {
    if !is_positional {
        return Mix { volume: 1.0, pan: 0.0 };
    }
    let dx = node.0 - listener.0;
    let dy = node.1 - listener.1;
    let d = (dx * dx + dy * dy).sqrt();
    let volume = if d <= reference {
        1.0
    } else if max <= reference || d >= max {
        0.0
    } else {
        1.0 - (d - reference) / (max - reference)
    };
    let pan = if max > 0.0 { (dx / max).clamp(-1.0, 1.0) } else { 0.0 };
    Mix { volume, pan }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_modes_map_a_400_by_200_scene_into_an_800_square_view() {
        let mut s = Scene2d::new(400.0, 200.0);
        s.set_view_size(800.0, 800.0);
        s.set_scale_mode(ScaleMode::Fill);
        assert_eq!(s.scale(), (2.0, 4.0), "each axis on its own: 800/400 and 800/200");
        s.set_scale_mode(ScaleMode::AspectFill);
        assert_eq!(s.scale(), (4.0, 4.0), "one factor, the larger of 2 and 4");
        s.set_scale_mode(ScaleMode::AspectFit);
        assert_eq!(s.scale(), (2.0, 2.0), "one factor, the smaller of 2 and 4");
        s.set_scale_mode(ScaleMode::ResizeFill);
        assert_eq!(s.scale(), (1.0, 1.0));
        assert_eq!(s.size(), (800.0, 800.0), "resize takes the view's size");
    }

    #[test]
    fn the_anchor_places_a_letterboxed_scene_and_clamps_its_input() {
        let mut s = Scene2d::new(400.0, 200.0);
        s.set_view_size(800.0, 800.0);
        s.set_scale_mode(ScaleMode::AspectFit);
        // the scaled scene is 800 by 400, so 400 points of vertical space are free
        assert_eq!(s.origin(), (0.0, 0.0), "anchor (0, 0) pins the bottom-left corner");
        s.set_anchor(0.5, 0.5);
        assert_eq!(s.origin(), (0.0, 200.0), "centred: half of 400 free points below");
        s.set_anchor(3.0, f32::NAN);
        assert_eq!(s.origin(), (0.0, 0.0), "anchor 3 clamps to 1, NaN reads as 0; no horizontal space to share");
    }

    #[test]
    fn scene_and_view_points_convert_both_ways() {
        let mut s = Scene2d::new(400.0, 200.0);
        s.set_view_size(800.0, 800.0);
        s.set_scale_mode(ScaleMode::AspectFit);
        s.set_anchor(0.5, 0.5);
        // origin (0, 200), factor 2: scene (100, 50) sits at view (0 + 200, 200 + 100)
        assert_eq!(s.scene_to_view(100.0, 50.0), (200.0, 300.0));
        assert_eq!(s.view_to_scene(200.0, 300.0), (100.0, 50.0));

        // AspectFill crops: factor 4 and free space -800 on x, centred to -400, so the scene centre is the view centre
        s.set_scale_mode(ScaleMode::AspectFill);
        assert_eq!(s.origin(), (-400.0, 0.0));
        assert_eq!(s.scene_to_view(200.0, 100.0), (400.0, 400.0));
    }

    #[test]
    fn an_empty_scene_keeps_unit_scale_and_never_divides_by_zero() {
        let mut s = Scene2d::new(-5.0, 10.0);
        assert_eq!(s.size(), (0.0, 10.0), "a negative width counts as 0");
        s.set_view_size(640.0, 480.0);
        s.set_scale_mode(ScaleMode::AspectFit);
        assert_eq!(s.scale(), (1.0, 1.0), "an empty axis keeps a factor of 1 in an aspect mode");
        assert_eq!(s.scene_to_view(3.0, 4.0), (3.0, 4.0), "origin is 0 at anchor (0, 0), factor 1");

        // a view of size 0 gives a zero factor: view points then map to 0, never to infinity
        let mut z = Scene2d::new(100.0, 100.0);
        z.set_scale_mode(ScaleMode::Fill);
        assert_eq!(z.scale(), (0.0, 0.0));
        assert_eq!(z.view_to_scene(5.0, 5.0), (0.0, 0.0));
    }

    #[test]
    fn frame_clock_gives_the_step_and_skips_paused_frames() {
        let mut c = FrameClock::default();
        assert_eq!(c.begin_frame(10.0), Some(0.0), "the first frame has no step");
        assert_eq!(c.begin_frame(10.25), Some(0.25));
        c.set_paused(true);
        assert!(c.is_paused());
        assert_eq!(c.begin_frame(11.0), None, "a paused frame is not simulated");
        c.set_paused(false);
        assert_eq!(c.begin_frame(11.5), Some(0.5), "the paused time is not replayed on resume");
        assert_eq!(c.begin_frame(9.0), Some(0.0), "a clock that goes back gives no negative step");
    }

    #[test]
    fn frame_phases_run_in_the_reference_order() {
        assert_eq!(FRAME_PHASES.len(), 5);
        assert_eq!(FRAME_PHASES[0], FramePhase::Update);
        assert_eq!(FRAME_PHASES[1], FramePhase::EvaluateActions);
        assert_eq!(FRAME_PHASES[2], FramePhase::SimulatePhysics);
        assert_eq!(FRAME_PHASES[3], FramePhase::ApplyConstraints);
        assert_eq!(FRAME_PHASES[4], FramePhase::FinishUpdate);
    }

    #[test]
    fn positional_mix_fades_with_distance_and_pans_by_offset() {
        // 30 right and 40 up is 50 away: reference 10, max 110 gives 1 - (50 - 10) / 100 = 0.6
        let m = positional_mix((30.0, 40.0), (0.0, 0.0), true, 10.0, 110.0);
        assert!((m.volume - 0.6).abs() < 1e-5);
        assert!((m.pan - 30.0 / 110.0).abs() < 1e-5);

        let near = positional_mix((5.0, 0.0), (0.0, 0.0), true, 10.0, 110.0);
        assert_eq!(near.volume, 1.0, "inside the reference distance");

        let far = positional_mix((-200.0, 0.0), (0.0, 0.0), true, 10.0, 110.0);
        assert_eq!((far.volume, far.pan), (0.0, -1.0), "beyond max is silent and the pan clamps to the left edge");

        let flat = positional_mix((-200.0, 0.0), (0.0, 0.0), false, 10.0, 110.0);
        assert_eq!(flat, Mix { volume: 1.0, pan: 0.0 }, "a node that is not positional is full and centred");
    }
}
