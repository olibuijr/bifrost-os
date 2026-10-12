//! The boot's one fade (`docs/bifrost-boot-design.md`, "One modeset, one fade"). The compositor owns the screen, so it
//! draws the loader's frame from its first scanout and cross-fades into the desktop once the shell has drawn it.

// the boot marks are the shell's file (the boot log is shared with the shell); the frame is bootweave-frame
#[allow(dead_code)]
#[path = "../../src/bootsplash.rs"]
mod bootsplash;

pub use bootsplash::mark;

/// How long the boot frame takes to fade into the desktop (ms).
pub const FADE_MS: f64 = bootweave_frame::FADE_MS as f64;

/// Opacity of the loader's frame over the desktop. `since_ready` is the time since the shell's first desktop frame
/// (None until then: the frame stays fully opaque). None once the fade is over.
pub fn splash_alpha(since_ready: Option<f64>) -> Option<f32> {
    let Some(t) = since_ready else { return Some(1.0) };
    if t >= FADE_MS {
        return None;
    }
    // the same curve as the hand-off in bootweave-frame (smoothstep)
    Some(bootweave_frame::fade_weight((t / FADE_MS) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loader_frame_holds_until_the_shell_is_ready() {
        assert_eq!(splash_alpha(None), Some(1.0));
    }

    #[test]
    fn it_fades_with_the_loaders_easing_and_then_goes_away() {
        assert_eq!(splash_alpha(Some(0.0)), Some(1.0));
        let mid = splash_alpha(Some(FADE_MS / 2.0)).unwrap();
        assert!((mid - 0.5).abs() < 1e-3, "smoothstep is symmetric: {mid}");
        let early = splash_alpha(Some(FADE_MS / 4.0)).unwrap();
        assert!(early > 0.75, "eased: slow at the start ({early})");
        assert_eq!(splash_alpha(Some(FADE_MS)), None);
        assert_eq!(splash_alpha(Some(FADE_MS * 3.0)), None);
    }
}
