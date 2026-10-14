//! Live output rotation: an output's transform changes while the desktop runs, so a tablet can turn its desktop when
//! the sensor reports a new orientation, and a PC keeps the transform it booted with.
//!
//! The request arrives through the output-management protocol (`zwlr_output_configuration_head_v1.set_transform`,
//! handled in `session_protocols`), the same path a shell uses for a mode change. [`WmState::rotate_output`] then:
//!
//! 1. asks the backend's `set_transform` hook whether it can take the change (the DRM backend refuses while its
//!    Vulkan scanout, which draws unrotated, is running),
//! 2. changes the output's transform, which the window server, the DRM compositor's render transform, the touch
//!    mapping and the captures all read from the output,
//! 3. re-arranges the layer surfaces and recomputes the usable area, so the menu bar and the windows follow,
//! 4. sends a configure to every toplevel whose frame size changed, so clients draw at the rotated size,
//! 5. announces the new head state to the output-management clients and marks the frame dirty.
//!
//! [`change`] and [`needs_configure`] are pure and host-tested. A request for the transform an output already has is a
//! no-op, so a configuration that repeats the current transform changes nothing.

use smithay::{
    desktop::layer_map_for_output,
    output::Output,
    utils::Transform,
};

use crate::state::WmState;

/// The transform an output must change to, or `None` when it already has `want` (nothing to do).
pub fn change(current: Transform, want: Transform) -> Option<Transform> {
    (current != want).then_some(want)
}

/// Whether a toplevel must be sent a configure for its frame: its last configured size (`None` before the first
/// configure) differs from the frame size.
pub fn needs_configure(configured: Option<(i32, i32)>, frame: (i32, i32)) -> bool {
    configured != Some(frame)
}

impl WmState {
    /// Turns `output` to `want` at runtime. Returns whether the change took: a refused hook, or a backend without
    /// the hook, leaves the output as it was. Turning to the transform the output already has succeeds at once.
    pub fn rotate_output(&mut self, output: &Output, want: Transform) -> bool {
        let Some(t) = change(output.current_transform(), want) else { return true };
        let name = output.name();
        let Some(hook) = self.session.hooks.set_transform.as_mut() else { return false };
        if !hook(&name, t) {
            return false;
        }
        output.change_current_state(None, Some(t), None, None);
        layer_map_for_output(output).arrange();
        self.refresh_area();
        self.configure_resized();
        self.publish_outputs();
        self.dirty = true;
        eprintln!("bifrost-wm: {name} turned to {t:?}");
        true
    }

    /// Sends a configure to every toplevel whose frame size no longer matches the size it was last configured with.
    /// The frame comes from `wm`, which `refresh_area` has just re-fitted to the usable area.
    pub fn configure_resized(&mut self) {
        for id in self.wm.stack() {
            let (Some(window), Some(rect)) = (self.window(id), self.wm.rect(id)) else { continue };
            let Some(toplevel) = window.toplevel() else { continue };
            let configured = toplevel.current_state().size.map(|s| (s.w, s.h));
            if needs_configure(configured, (rect.w, rect.h)) {
                toplevel.with_pending_state(|s| s.size = Some((rect.w, rect.h).into()));
                toplevel.send_pending_configure();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transform_the_output_already_has_is_no_change() {
        assert_eq!(change(Transform::Normal, Transform::Normal), None);
        assert_eq!(change(Transform::_90, Transform::_90), None);
        assert_eq!(change(Transform::Normal, Transform::_90), Some(Transform::_90));
        assert_eq!(change(Transform::_270, Transform::Normal), Some(Transform::Normal));
        assert_eq!(change(Transform::Flipped, Transform::Flipped180), Some(Transform::Flipped180));
    }

    #[test]
    fn a_toplevel_is_configured_until_it_has_the_frame_size() {
        assert!(needs_configure(None, (1920, 1200)), "never configured");
        assert!(needs_configure(Some((1200, 1920)), (1920, 1200)), "the rotated frame is wider");
        assert!(!needs_configure(Some((1920, 1200)), (1920, 1200)), "already at the frame size");
        assert!(needs_configure(Some((1920, 1200)), (1920, 1199)));
    }
}
