//! One layout model for every app, derived from the window size (the native toolkit's split view and layout
//! guides, the native toolkit's size classes and safe area). Nothing here assumes a particular screen: all values
//! are points computed from `Configure`, so the same code lays out a phone-sized window, a 4K panel
//! and an ultrawide.
//!
//! ```ignore
//! let l = Layout::new(&ctx.cfg);
//! let x = ui.pt(l.margin);              // leading edge of content
//! let y = ui.pt(l.content_top);         // below the toolbar, never `STATUS_PT + literal`
//! ```
use crate::proto::Configure;
use std::sync::atomic::{AtomicU32, Ordering};

/// The safe inset at the top, in points, as of the last `Configure` (the native toolkit `safeAreaInsets.top`).
/// 0 on the desktop, the status bar on a handheld. The runtime sets it before `App::configure`.
static TOP_INSET_BITS: AtomicU32 = AtomicU32::new(0);

/// The current top safe inset in points; apps add their own offsets to this, never to a fixed status bar.
pub fn top_inset() -> f32 {
    f32::from_bits(TOP_INSET_BITS.load(Ordering::Relaxed))
}

/// Records the inset from `cfg`. Called by the runtime; apps do not call it.
pub fn set_from(cfg: &Configure) {
    TOP_INSET_BITS.store(Layout::new(cfg).top.to_bits(), Ordering::Relaxed);
}

/// Leading and trailing content margin (HIG Golden Gate: 20 pt).
pub const MARGIN_PT: f32 = 20.0;
/// Default table and sidebar row height; also the default control hit size (HIG Golden Gate: 28 pt).
pub const ROW_PT: f32 = 28.0;
/// A window toolbar (HIG Golden Gate: 38 pt unified title and toolbar).
pub const TOOLBAR_PT: f32 = 38.0;
/// The unified title and toolbar row of a full-size-content window (manifest `window = "full-size"`, Golden Gate):
/// the traffic lights and the toolbar share it, and the sidebar starts below it.
pub const UNIFIED_BAR_PT: f32 = 52.0;
/// Space between the large title and the content below it.
pub const TITLE_GAP_PT: f32 = 12.0;
/// Below this width the sidebar collapses and navigation becomes a push stack (the native toolkit compact class).
pub const COMPACT_BELOW_PT: f32 = 640.0;
/// Sidebar width limits (the native toolkit `NativeSplitViewItem` min and max thickness) and the share of the window it takes.
pub const SIDEBAR_MIN_PT: f32 = 180.0;
pub const SIDEBAR_MAX_PT: f32 = 280.0;
pub const SIDEBAR_FRACTION: f32 = 0.26;
/// Reading width: lines of text and form columns stop growing here on very wide windows.
pub const READABLE_MAX_PT: f32 = 720.0;

/// In a full-size-content window the shell draws the traffic lights over the app at the top left (the same place
/// as in Settings). Returns the x, in points, where the app's own controls may start to the right of them.
pub fn traffic_lights_trailing() -> f32 {
    use bifrost_ui::prefs_layout::m;
    m::TRAFFIC_X0 + 2.0 * m::TRAFFIC_PITCH + TRAFFIC_LIGHT_RADIUS_PT + 8.0
}

/// Whether the shell draws traffic lights over this app's content at the top left: a full-size-content window on the
/// desktop form factor. A classic title-bar window has its lights above the content, and a tablet has none, so a
/// toolbar there needs no inset (`widgets::clear_of_traffic_lights`). The shell tells the app its window style in
/// `BIFROST_WINDOW` (the manifest's `window`); an app started without it (a test, a tool) keeps the full-size layout.
pub fn traffic_lights_over_content() -> bool {
    #[cfg(test)]
    if let Some(v) = TEST_LIGHTS.with(|t| t.get()) {
        return v;
    }
    static SHOWN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SHOWN.get_or_init(|| lights_over_content(std::env::var("BIFROST_WINDOW").ok().as_deref(), bifrost_ui::form_factor::FormFactor::current()))
}

/// [`traffic_lights_over_content`] for a manifest `window` value (`None`: not told) and a form factor.
pub fn lights_over_content(window: Option<&str>, form: bifrost_ui::form_factor::FormFactor) -> bool {
    form.is_desktop() && window.is_none_or(|w| w == "full-size")
}

#[cfg(test)]
thread_local! {
    /// Lets a test pick the answer of [`traffic_lights_over_content`] (the real one reads the machine's form factor).
    pub(crate) static TEST_LIGHTS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(Some(true)) };
}

/// Radius of a traffic light the shell draws over a full-size-content window, in points (the lights are 14 pt).
pub const TRAFFIC_LIGHT_RADIUS_PT: f32 = 7.0;

/// The y, in points, of the lights' top and bottom edges in a full-size-content window. A bar whose rows lie wholly
/// above or below this band does not meet the lights.
pub fn traffic_lights_band() -> (f32, f32) {
    use bifrost_ui::prefs_layout::m;
    (m::TRAFFIC_Y - TRAFFIC_LIGHT_RADIUS_PT, m::TRAFFIC_Y + TRAFFIC_LIGHT_RADIUS_PT)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeClass {
    /// Narrow window: one column, push navigation, no sidebar.
    Compact,
    /// Sidebar beside content.
    Regular,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Window size in points.
    pub w: f32,
    pub h: f32,
    pub class: SizeClass,
    /// Leading and trailing content margin.
    pub margin: f32,
    /// Width of the sidebar column, 0 when collapsed.
    pub sidebar: f32,
    /// Safe inset at the top (points): 0 on the desktop, the status bar on a handheld.
    pub top: f32,
    /// Top of content: safe inset (0 on the desktop) plus the toolbar.
    pub content_top: f32,
    /// Inset the shell asks apps to keep clear at the bottom.
    pub bottom_inset: f32,
    pub row_h: f32,
}

impl Layout {
    /// Layout for a window of `w` x `h` points with the given safe insets (points).
    pub fn from_points(w: f32, h: f32, safe_top: f32, safe_bottom: f32) -> Self {
        let (w, h) = (w.max(1.0), h.max(1.0));
        let class = if w < COMPACT_BELOW_PT { SizeClass::Compact } else { SizeClass::Regular };
        let sidebar = match class {
            SizeClass::Compact => 0.0,
            SizeClass::Regular => (w * SIDEBAR_FRACTION).clamp(SIDEBAR_MIN_PT, SIDEBAR_MAX_PT),
        };
        // Keep margins proportionate in tiny windows instead of eating the content.
        let margin = MARGIN_PT.min(w * 0.06).max(8.0).min(w / 2.0);
        Layout { w, h, class, margin, sidebar, top: safe_top.max(0.0), content_top: safe_top.max(0.0) + TOOLBAR_PT, bottom_inset: safe_bottom.max(0.0), row_h: ROW_PT }
    }

    /// Layout from the shell's configuration (pixels and scale; safe insets arrive in pixels).
    pub fn new(cfg: &Configure) -> Self {
        let s = if cfg.scale > 0.0 { cfg.scale } else { 1.0 };
        Self::from_points(cfg.w as f32 / s, cfg.h as f32 / s, cfg.safe_top / s, cfg.safe_bottom / s)
    }

    /// Left edge and width of the content column (everything right of the sidebar).
    pub fn content_x(&self) -> f32 {
        self.sidebar
    }
    pub fn content_w(&self) -> f32 {
        (self.w - self.sidebar).max(0.0)
    }
    /// Left edge and width of a text or form column inside the content area: full width up to
    /// `READABLE_MAX_PT`, then centred.
    pub fn readable(&self) -> (f32, f32) {
        let avail = (self.content_w() - 2.0 * self.margin).max(0.0);
        let w = avail.min(READABLE_MAX_PT);
        (self.content_x() + self.margin + (avail - w) / 2.0, w)
    }
    /// Height of the area below the toolbar and above the bottom inset.
    pub fn content_h(&self) -> f32 {
        (self.h - self.content_top - self.bottom_inset).max(0.0)
    }
    /// Rows that fit in `content_h`.
    pub fn rows_visible(&self) -> usize {
        (self.content_h() / self.row_h) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Phone-sized window, laptop, 4K at 2x, ultrawide, and a degenerate one.
    const SIZES: [(f32, f32); 6] = [(360.0, 640.0), (800.0, 600.0), (1440.0, 900.0), (1920.0, 1080.0), (3440.0, 1440.0), (1.0, 1.0)];

    #[test]
    fn narrow_windows_collapse_the_sidebar_wide_ones_keep_it() {
        assert_eq!(Layout::from_points(639.0, 800.0, 0.0, 0.0).sidebar, 0.0);
        assert_eq!(Layout::from_points(639.0, 800.0, 0.0, 0.0).class, SizeClass::Compact);
        let l = Layout::from_points(1000.0, 800.0, 0.0, 0.0);
        assert_eq!(l.class, SizeClass::Regular);
        assert!((SIDEBAR_MIN_PT..=SIDEBAR_MAX_PT).contains(&l.sidebar));
    }

    #[test]
    fn the_sidebar_is_clamped_on_huge_screens() {
        assert_eq!(Layout::from_points(3440.0, 1440.0, 0.0, 0.0).sidebar, SIDEBAR_MAX_PT);
        assert_eq!(Layout::from_points(640.0, 480.0, 0.0, 0.0).sidebar, SIDEBAR_MIN_PT);
    }

    #[test]
    fn every_size_keeps_content_inside_the_window() {
        for (w, h) in SIZES {
            let l = Layout::from_points(w, h, 0.0, 0.0);
            assert!(l.sidebar + l.content_w() <= l.w + 0.001, "{w}x{h}");
            let (x, rw) = l.readable();
            assert!(x >= l.content_x() && x + rw <= l.w + 0.001 && rw >= 0.0, "{w}x{h}: {x} {rw}");
            assert!(l.content_h() >= 0.0 && l.margin <= MARGIN_PT && 2.0 * l.margin <= l.w, "{w}x{h}");
        }
    }

    #[test]
    fn text_columns_stop_growing_on_ultrawide_and_stay_centred() {
        let l = Layout::from_points(3440.0, 1440.0, 0.0, 0.0);
        let (x, w) = l.readable();
        assert_eq!(w, READABLE_MAX_PT);
        let right_gap = l.w - (x + w);
        let left_gap = x - l.content_x();
        assert!((left_gap - right_gap).abs() < 0.01, "{left_gap} {right_gap}");
    }

    #[test]
    fn configure_is_converted_from_pixels_to_points() {
        let cfg = Configure { w: 2880, h: 1800, scale: 2.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let l = Layout::new(&cfg);
        assert_eq!((l.w, l.h), (1440.0, 900.0));
        assert_eq!((l.top, l.content_top), (0.0, TOOLBAR_PT), "no safe inset on the desktop, so no status-bar gap");
        let inset = Configure { safe_top: 40.0, ..cfg };
        assert_eq!(Layout::new(&inset).content_top, 20.0 + TOOLBAR_PT, "a safe inset is honoured when the shell sends one");
    }

    #[test]
    fn the_runtime_inset_follows_configure_in_points() {
        let cfg = Configure { w: 2000, h: 1000, scale: 2.0, safe_top: 48.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        set_from(&cfg);
        assert_eq!(top_inset(), 24.0);
        set_from(&Configure { safe_top: 0.0, ..cfg });
        assert_eq!(top_inset(), 0.0);
    }

    #[test]
    fn rows_visible_follows_the_window_height() {
        let a = Layout::from_points(1000.0, 400.0, 0.0, 0.0).rows_visible();
        let b = Layout::from_points(1000.0, 1200.0, 0.0, 0.0).rows_visible();
        assert!(b > a && a > 0);
    }
}
