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

/// Written by `bifrost-bootweave shutdown` (`docs/boot-architecture.md`, "Symmetric shutdown") before systemd stops the
/// session. Its presence starts the shutdown fade; it lives on /run, so a reboot clears it.
pub const SHUTDOWN_REQUEST: &str = "/run/bifrost/bootweave/shutdown";
/// Written here once the boot frame covers the desktop, so the stopping supervisor can go on. It lives in the account's
/// runtime directory: the window server runs as that account, while `/run/bifrost/bootweave` belongs to root.
pub const SHUTDOWN_FADED: &str = "/run/bifrost/user/shutdown-faded";
/// How long the desktop takes to fade back to the boot frame at shutdown (the boot's own fade length).
pub const SHUTDOWN_FADE_MS: f64 = 450.0;

/// Opacity of the boot frame over the desktop during a shutdown. `since_request` is the time since the request was
/// first seen (None when there is no request). It eases from 0 to 1 over `SHUTDOWN_FADE_MS` and then holds at 1, so
/// the boot frame stays on screen until power-off. Separate from `splash_alpha`, which is the boot's fade-in.
pub fn shutdown_alpha(since_request: Option<f64>) -> Option<f32> {
    let t = since_request?;
    let e = (t / SHUTDOWN_FADE_MS).clamp(0.0, 1.0);
    Some((e * e * (3.0 - 2.0 * e)) as f32)
}

/// True once the shutdown fade has reached the boot frame (the moment to write `SHUTDOWN_FADED`).
pub fn shutdown_faded(since_request: Option<f64>) -> bool {
    since_request.is_some_and(|t| t >= SHUTDOWN_FADE_MS)
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

    #[test]
    fn no_shutdown_request_means_no_shutdown_overlay() {
        assert_eq!(shutdown_alpha(None), None);
        assert!(!shutdown_faded(None));
    }

    #[test]
    fn the_shutdown_fade_starts_at_zero_and_eases_to_the_boot_frame() {
        assert_eq!(shutdown_alpha(Some(0.0)), Some(0.0));
        let mid = shutdown_alpha(Some(SHUTDOWN_FADE_MS / 2.0)).unwrap();
        assert!((mid - 0.5).abs() < 1e-3, "smoothstep is symmetric: {mid}");
        let early = shutdown_alpha(Some(SHUTDOWN_FADE_MS / 4.0)).unwrap();
        assert!(early < 0.25, "eased: slow at the start ({early})");
    }

    #[test]
    fn the_boot_frame_holds_at_full_opacity_until_power_off() {
        assert_eq!(shutdown_alpha(Some(SHUTDOWN_FADE_MS)), Some(1.0));
        assert_eq!(shutdown_alpha(Some(SHUTDOWN_FADE_MS * 10.0)), Some(1.0));
    }

    #[test]
    fn the_fade_is_reported_done_exactly_at_its_length() {
        assert!(!shutdown_faded(Some(SHUTDOWN_FADE_MS - 1.0)));
        assert!(shutdown_faded(Some(SHUTDOWN_FADE_MS)));
        assert!(shutdown_faded(Some(SHUTDOWN_FADE_MS + 100.0)));
    }

    #[test]
    fn the_request_and_the_faded_marker_live_where_their_writers_can_write() {
        // the request is written by root (bootweave), the marker by the window server's account
        assert!(SHUTDOWN_REQUEST.starts_with("/run/bifrost/bootweave/"));
        assert!(SHUTDOWN_FADED.starts_with("/run/bifrost/user/"));
        assert_ne!(SHUTDOWN_REQUEST, SHUTDOWN_FADED);
    }
}
