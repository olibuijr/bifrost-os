//! Decisions of the layer compositor that need no GPU: which path a frame takes, how the video
//! picture gets onto the GPU, and what part of the panel a frame redraws.

use super::damage;
use super::quad;
use crate::gfx::rotate::Rotation;
use crate::layout::Rect;

/// How a frame reaches the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// the CPU frame is the whole picture: rotated on the CPU (or not at all) and presented
    Cpu,
    /// the CPU frame is uploaded where it changed and drawn turned by the GPU
    Rotate,
    /// the video picture is drawn by the GPU, the CPU frame over it as an overlay
    Video,
}

/// What the GPU can do right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    /// a context and render target exist and nothing has failed
    pub gpu: bool,
    /// the layer programs built (else neither GPU path is possible)
    pub layers: bool,
    /// GPU rotation is allowed (`AKURAI_GPU_ROTATE=0` turns it off)
    pub rotate: bool,
}

/// The path for a frame. `video` is whether an app handed over a video layer; `fits` whether
/// the turned frame is exactly the render target.
pub fn choose(caps: Caps, video: bool, rot: Rotation, fits: bool) -> Path {
    if !caps.gpu || !caps.layers || !fits {
        return Path::Cpu;
    }
    if video {
        Path::Video
    } else if rot != Rotation::None && caps.rotate {
        Path::Rotate
    } else {
        Path::Cpu
    }
}

/// Whether the rotation turn animation is drawn by the GPU: the compositor works, GPU rotation
/// is allowed, the new frame turned by `rot` is exactly the render target (`fits`) and the
/// night tint (applied on the CPU frame) is off.
pub fn turn_on_gpu(caps: Caps, fits: bool, tinted: bool) -> bool {
    caps.gpu && caps.layers && caps.rotate && fits && !tinted
}

/// How the video picture is sampled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// the decoder's NV12 buffer imported as an external EGL image (zero copy)
    External,
    /// converted to RGBA on the CPU and uploaded (`glTexSubImage2D`)
    Upload,
}

/// Import state for NV12 pictures: untried, working, or refused by the driver for good.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Import {
    #[default]
    Untried,
    Works,
    Refused,
}

/// The source for the next picture: the import until the driver refused it once.
pub fn source(import: Import, external_program: bool) -> Source {
    if external_program && import != Import::Refused {
        Source::External
    } else {
        Source::Upload
    }
}

/// The logical rectangles a frame redraws: the overlay's changed bands, plus the video
/// rectangle when the picture (or where it goes) changed; the whole frame when `full`.
pub fn logical_damage(full: bool, overlay: &[Rect], video: Option<Rect>, video_changed: bool, sw: usize, sh: usize) -> Vec<Rect> {
    if full {
        return vec![Rect { x: 0, y: 0, w: sw as i32, h: sh as i32 }];
    }
    let mut out = overlay.to_vec();
    if let (Some(v), true) = (video, video_changed) {
        out.push(v);
    }
    out
}

/// Logical rectangles turned onto the panel, clipped and merged into the areas to redraw.
pub fn panel_rects(rot: Rotation, logical: &[Rect], sw: usize, sh: usize) -> Vec<Rect> {
    let turned: Vec<Rect> = logical.iter().filter_map(|r| quad::to_panel(rot, *r, sw, sh)).collect();
    let (pw, ph) = rot.size(sw, sh);
    damage::merge(&turned, pw, ph)
        .into_iter()
        .map(|a| Rect { x: a.x0 as i32, y: a.y0 as i32, w: (a.x1 - a.x0) as i32, h: (a.y1 - a.y0) as i32 })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Caps = Caps { gpu: true, layers: true, rotate: true };

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn the_turn_needs_every_gpu_capability_a_fitting_frame_and_no_tint() {
        assert!(turn_on_gpu(ON, true, false));
        assert!(!turn_on_gpu(ON, false, false));
        assert!(!turn_on_gpu(ON, true, true));
        assert!(!turn_on_gpu(Caps { rotate: false, ..ON }, true, false));
        assert!(!turn_on_gpu(Caps { layers: false, ..ON }, true, false));
        assert!(!turn_on_gpu(Caps::default(), true, false));
    }

    #[test]
    fn video_takes_the_gpu_in_every_orientation() {
        for rot in [Rotation::None, Rotation::Cw, Rotation::Ccw, Rotation::Half] {
            assert_eq!(choose(ON, true, rot, true), Path::Video, "{rot:?}");
        }
    }

    #[test]
    fn rotation_alone_takes_the_gpu_unless_turned_off() {
        assert_eq!(choose(ON, false, Rotation::Cw, true), Path::Rotate);
        assert_eq!(choose(ON, false, Rotation::None, true), Path::Cpu, "portrait needs no turn");
        assert_eq!(choose(Caps { rotate: false, ..ON }, false, Rotation::Ccw, true), Path::Cpu);
        assert_eq!(choose(Caps { rotate: false, ..ON }, true, Rotation::Ccw, true), Path::Video);
    }

    #[test]
    fn anything_missing_falls_back_to_the_cpu() {
        assert_eq!(choose(Caps { gpu: false, ..ON }, true, Rotation::Cw, true), Path::Cpu);
        assert_eq!(choose(Caps { layers: false, ..ON }, true, Rotation::Cw, true), Path::Cpu);
        assert_eq!(choose(ON, true, Rotation::Cw, false), Path::Cpu, "frame size does not match the target");
        assert_eq!(choose(Caps::default(), true, Rotation::Cw, true), Path::Cpu);
    }

    #[test]
    fn nv12_import_until_refused() {
        assert_eq!(source(Import::Untried, true), Source::External);
        assert_eq!(source(Import::Works, true), Source::External);
        assert_eq!(source(Import::Refused, true), Source::Upload);
        assert_eq!(source(Import::Works, false), Source::Upload, "no external sampler program");
    }

    #[test]
    fn damage_is_overlay_bands_plus_a_changed_picture() {
        let bands = [r(0, 0, 64, 16)];
        let v = r(0, 60, 1920, 1080);
        assert_eq!(logical_damage(false, &bands, Some(v), false, 1920, 1200), vec![bands[0]]);
        assert_eq!(logical_damage(false, &bands, Some(v), true, 1920, 1200), vec![bands[0], v]);
        assert_eq!(logical_damage(false, &[], None, true, 1920, 1200), vec![]);
        assert_eq!(logical_damage(true, &[], None, false, 1920, 1200), vec![r(0, 0, 1920, 1200)]);
    }

    #[test]
    fn panel_rects_turn_clip_and_merge() {
        // the whole landscape frame is the whole panel
        assert_eq!(panel_rects(Rotation::Cw, &[r(0, 0, 1920, 1200)], 1920, 1200), vec![r(0, 0, 1200, 1920)]);
        // a band at the top of the landscape frame is a column at the panel's right edge
        assert_eq!(panel_rects(Rotation::Cw, &[r(0, 0, 1920, 16)], 1920, 1200), vec![r(1184, 0, 16, 1920)]);
        assert_eq!(panel_rects(Rotation::Ccw, &[r(0, 0, 1920, 16)], 1920, 1200), vec![r(0, 0, 16, 1920)]);
        // off screen parts go, touching parts merge
        assert_eq!(panel_rects(Rotation::None, &[r(-50, 0, 100, 10), r(50, 0, 50, 10)], 1200, 1920), vec![r(0, 0, 100, 10)]);
        assert!(panel_rects(Rotation::Half, &[r(5000, 0, 10, 10)], 1200, 1920).is_empty());
    }
}
