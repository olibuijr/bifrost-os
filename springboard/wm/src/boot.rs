//! The boot's one fade (`docs/bifrost-boot-design.md`, "One modeset, one fade"). The compositor owns the screen, so it
//! draws the loader's frame from its first scanout and cross-fades into the desktop once the shell has drawn it.

// the shell's file, so the loader frame has one source; the CPU `fade` is the shell's, the GPU blends here
#[allow(dead_code)]
#[path = "../../src/bootsplash.rs"]
mod bootsplash;

pub use bootsplash::{mark, paint, FADE_MS};

/// Opacity of the loader's frame over the desktop. `since_ready` is the time since the shell's first desktop frame
/// (None until then: the frame stays fully opaque). None once the fade is over.
pub fn splash_alpha(since_ready: Option<f64>) -> Option<f32> {
    let Some(t) = since_ready else { return Some(1.0) };
    if t >= FADE_MS {
        return None;
    }
    // the loader's own curve (`bootsplash::fade`): smoothstep
    let e = (t / FADE_MS).clamp(0.0, 1.0);
    Some((1.0 - e * e * (3.0 - 2.0 * e)) as f32)
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
