//! Hand-off of a scene to a renderer that the app owns. The app calls `update` once per frame with the time, runs the
//! steps that the scene returns, and then asks for the render pass. The pass names the viewport in device pixels and
//! the scene time it shows, so the app can encode its own draw commands into its own pipeline. The model is pure: it
//! reads no clock and draws nothing, and it refuses a render pass that has no update behind it.

use crate::scene_loop::{Frame, SceneLoop};

/// A rectangle of the render target in device pixels, with its origin at the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// What the app's renderer draws for one frame of the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderPass {
    /// The area of the render target that the scene covers.
    pub viewport: Viewport,
    /// Seconds the scene has run, as of the last update.
    pub scene_time: f64,
}

/// A scene handed to an app-owned renderer. `scale` is the number of device pixels per point.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneHandoff {
    scene: SceneLoop,
    scale: f32,
    updated: bool,
}

impl SceneHandoff {
    /// A hand-off for a scene of the given size in points, rendered at `scale` device pixels per point. None unless the
    /// scale is finite and above zero.
    pub fn new(width: f32, height: f32, scale: f32) -> Option<Self> {
        if !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        Some(SceneHandoff { scene: SceneLoop::new(width, height), scale, updated: false })
    }

    /// The device pixels per point.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Changes the device pixels per point. Returns false and changes nothing unless the scale is finite and above zero.
    pub fn set_scale(&mut self, scale: f32) -> bool {
        if scale.is_finite() && scale > 0.0 {
            self.scale = scale;
            true
        } else {
            false
        }
    }

    /// The scene underneath, for reading its state.
    pub fn scene(&self) -> &SceneLoop {
        &self.scene
    }

    /// The scene is presented to the renderer. Returns false if it already was.
    pub fn present(&mut self) -> bool {
        self.updated = false;
        self.scene.present()
    }

    /// The scene is removed from the renderer. Returns false if it was not presented.
    pub fn dismiss(&mut self) -> bool {
        self.updated = false;
        self.scene.dismiss()
    }

    /// The scene changed size in points. Returns true when the size actually changed.
    pub fn resize(&mut self, width: f32, height: f32) -> bool {
        self.scene.resize(width, height)
    }

    /// Advances the scene to time `now` in seconds and returns the steps to run. None while the scene is not presented;
    /// the render pass for this frame is then refused.
    pub fn update(&mut self, now: f64) -> Option<Frame> {
        let frame = self.scene.frame(now);
        self.updated = frame.is_some();
        frame
    }

    /// The render pass for the last update, or None when the scene has not been updated since it was presented, or when
    /// its size in device pixels is zero in either direction.
    pub fn render_pass(&self) -> Option<RenderPass> {
        if !self.updated {
            return None;
        }
        let (w, h) = self.scene.size();
        let width = device_pixels(w, self.scale);
        let height = device_pixels(h, self.scale);
        if width == 0 || height == 0 {
            return None;
        }
        Some(RenderPass {
            viewport: Viewport { x: 0, y: 0, width, height },
            scene_time: self.scene.current_time(),
        })
    }
}

/// A length in points converted to whole device pixels, rounded to the nearest pixel.
fn device_pixels(points: f32, scale: f32) -> u32 {
    (points * scale).round().max(0.0) as u32
}
