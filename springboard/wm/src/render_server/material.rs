//! Materials drawn by the server: live glass (dual-Kawase blur in compute shaders `kawase_down.comp` and
//! `kawase_up.comp` over the already-composited content under a region, at half resolution, with `passes` from the
//! blur radius; then the blurred region is drawn back clipped to the rounded rect with the saturation boost and the
//! tint), and window drop shadows (SDF Gaussian approximation in `shadow.frag`, the reference platform's large soft
//! active-window shadow and smaller inactive one).
//!
//! Owner: wave G1 agent `wm-material`.

/// A blurred region behind an item, in output pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialRegion {
    pub rect: [f32; 4],
    pub radius: f32,
    /// Blur radius in output pixels.
    pub blur: f32,
    /// Premultiplied tint drawn over the blur.
    pub tint: [f32; 4],
    pub saturation: f32,
}

/// A drop shadow behind an item, in output pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSpec {
    pub rect: [f32; 4],
    pub radius: f32,
    pub sigma: f32,
    pub offset: [f32; 2],
    pub color: [f32; 4],
}

/// The number of dual-Kawase down/up passes for a blur radius in pixels (pure): 1 pass ~ 2 px, each further pass
/// doubles the reach; clamped to 1..6.
pub fn kawase_passes(radius_px: f32) -> u32 {
    let _ = radius_px;
    todo!("wm-material")
}

/// The window shadow for a window rect: active windows get the large shadow, inactive the small one (values in the
/// module tests, measured from the reference look: active sigma 24 px offset 0/16 alpha 0.35; inactive sigma 12 px
/// offset 0/6 alpha 0.20, scaled by the output scale).
pub fn window_shadow(rect: [f32; 4], radius: f32, active: bool, scale: f32) -> ShadowSpec {
    let _ = (rect, radius, active, scale);
    todo!("wm-material")
}
