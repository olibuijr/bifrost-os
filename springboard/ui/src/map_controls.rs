//! Map controls: which on-map controls a map view shows for its current state.
//!
//! The compass shows while the map is turned away from north. The zoom-in control is off at the largest zoom and
//! the zoom-out control is off at the smallest; both are off when zooming is switched off for the view. The scale bar
//! takes the width the caller gives it. The tracking and pitch buttons follow the view's own flags. [`MapControls`] only
//! reports what to show: the shell draws the controls and sends the gestures back to the [`MapView`].

use crate::mapview::{compass_visible, MapCamera, MapView, ScaleBar, MAX_ZOOM, MIN_ZOOM};

/// The controls a map view shows now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapControls {
    pub compass: bool,
    pub zoom_in: bool,
    pub zoom_out: bool,
    pub scale: Option<ScaleBar>,
    pub tracking_button: bool,
    pub pitch_button: bool,
}

impl MapControls {
    /// The controls for `view`, with a scale bar at most `scale_max_px` pixels wide.
    pub fn for_view(view: &MapView, scale_max_px: f64) -> Self {
        let camera: &MapCamera = view.camera();
        MapControls {
            compass: compass_visible(camera.heading_deg),
            zoom_in: view.interaction.zoom && camera.zoom < MAX_ZOOM,
            zoom_out: view.interaction.zoom && camera.zoom > MIN_ZOOM,
            scale: camera.scale_bar(scale_max_px),
            tracking_button: view.show_tracking_button,
            pitch_button: view.show_pitch_button,
        }
    }
}
