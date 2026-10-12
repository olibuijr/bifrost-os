//! Shareable content: the displays, apps and windows an app may capture, and the queries that pick windows
//! from them. The shell reports the windows front to back; this module holds that list and answers questions
//! about it. It captures nothing and talks to no compositor.

use crate::Rect;

/// One display that can be captured. `frame` is in global points.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureDisplay {
    pub id: u32,
    pub frame: Rect,
}

/// One running app that can be captured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureApp {
    pub pid: u32,
    pub bundle_id: String,
    pub name: String,
}

/// One window that can be captured. `frame` is in global points. `desktop` marks the desktop background
/// windows that most queries leave out.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureWindow {
    pub id: u32,
    pub title: String,
    pub app_pid: u32,
    pub frame: Rect,
    pub on_screen: bool,
    pub desktop: bool,
}

/// Where a query looks in the stacking order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stacking {
    /// Every window.
    Any,
    /// Only the windows in front of this window id. The window itself is not included.
    Above(u32),
    /// Only the windows behind this window id. The window itself is not included.
    Below(u32),
}

/// Filters for `ShareableContent::query`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowQuery {
    pub on_screen_only: bool,
    pub exclude_desktop: bool,
    pub stacking: Stacking,
}

impl WindowQuery {
    /// A query that keeps every window.
    pub fn all() -> Self {
        WindowQuery { on_screen_only: false, exclude_desktop: false, stacking: Stacking::Any }
    }
}

/// The displays, apps and windows an app can capture. `windows` is ordered front to back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShareableContent {
    pub displays: Vec<CaptureDisplay>,
    pub apps: Vec<CaptureApp>,
    pub windows: Vec<CaptureWindow>,
}

impl ShareableContent {
    pub fn new(displays: Vec<CaptureDisplay>, apps: Vec<CaptureApp>, windows: Vec<CaptureWindow>) -> Self {
        ShareableContent { displays, apps, windows }
    }

    pub fn display(&self, id: u32) -> Option<&CaptureDisplay> {
        self.displays.iter().find(|d| d.id == id)
    }

    pub fn app(&self, pid: u32) -> Option<&CaptureApp> {
        self.apps.iter().find(|a| a.pid == pid)
    }

    pub fn window(&self, id: u32) -> Option<&CaptureWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// The windows of one app, front to back.
    pub fn windows_of_app(&self, pid: u32) -> Vec<&CaptureWindow> {
        self.windows.iter().filter(|w| w.app_pid == pid).collect()
    }

    /// The display that shows the largest part of a window. `None` when the window is unknown or lies
    /// outside every display. On a tie the display listed first wins.
    pub fn display_of_window(&self, id: u32) -> Option<&CaptureDisplay> {
        let win = self.window(id)?;
        let mut best: Option<(&CaptureDisplay, f32)> = None;
        for d in &self.displays {
            let area = overlap_area(win.frame, d.frame);
            if area > 0.0 && best.map_or(true, |(_, a)| area > a) {
                best = Some((d, area));
            }
        }
        best.map(|(d, _)| d)
    }

    /// The windows that pass `query`, front to back. An unknown stacking anchor gives an empty list.
    pub fn query(&self, query: WindowQuery) -> Vec<&CaptureWindow> {
        let (lo, hi) = match query.stacking {
            Stacking::Any => (0, self.windows.len()),
            Stacking::Above(id) => match self.position(id) {
                Some(i) => (0, i),
                None => return Vec::new(),
            },
            Stacking::Below(id) => match self.position(id) {
                Some(i) => (i + 1, self.windows.len()),
                None => return Vec::new(),
            },
        };
        self.windows[lo..hi]
            .iter()
            .filter(|w| !query.on_screen_only || w.on_screen)
            .filter(|w| !query.exclude_desktop || !w.desktop)
            .collect()
    }

    fn position(&self, id: u32) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }
}

/// Area of the intersection of two rectangles, or zero when they do not overlap.
fn overlap_area(a: Rect, b: Rect) -> f32 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    if w > 0.0 && h > 0.0 {
        w * h
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn win(id: u32, pid: u32, frame: Rect) -> CaptureWindow {
        CaptureWindow { id, title: format!("w{id}"), app_pid: pid, frame, on_screen: true, desktop: false }
    }

    /// Two displays side by side, four windows front to back.
    fn sample() -> ShareableContent {
        let displays = vec![
            CaptureDisplay { id: 1, frame: rect(0.0, 0.0, 1000.0, 800.0) },
            CaptureDisplay { id: 2, frame: rect(1000.0, 0.0, 1000.0, 800.0) },
        ];
        let apps = vec![
            CaptureApp { pid: 10, bundle_id: "org.example.a".into(), name: "A".into() },
            CaptureApp { pid: 20, bundle_id: "org.example.b".into(), name: "B".into() },
        ];
        let mut hidden = win(4, 30, rect(1200.0, 300.0, 100.0, 100.0));
        hidden.on_screen = false;
        let mut desk = win(5, 10, rect(0.0, 0.0, 1000.0, 800.0));
        desk.desktop = true;
        let windows = vec![
            win(1, 10, rect(100.0, 100.0, 200.0, 200.0)),
            win(2, 20, rect(1100.0, 100.0, 200.0, 200.0)),
            win(3, 10, rect(300.0, 300.0, 200.0, 200.0)),
            hidden,
            desk,
        ];
        ShareableContent::new(displays, apps, windows)
    }

    fn ids(list: Vec<&CaptureWindow>) -> Vec<u32> {
        list.iter().map(|w| w.id).collect()
    }

    #[test]
    fn query_all_keeps_every_window_in_front_to_back_order() {
        let c = sample();
        assert_eq!(ids(c.query(WindowQuery::all())), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn on_screen_and_desktop_filters_drop_their_windows() {
        let c = sample();
        let q = WindowQuery { on_screen_only: true, exclude_desktop: false, stacking: Stacking::Any };
        assert_eq!(ids(c.query(q)), vec![1, 2, 3, 5]);
        let q = WindowQuery { on_screen_only: true, exclude_desktop: true, stacking: Stacking::Any };
        assert_eq!(ids(c.query(q)), vec![1, 2, 3]);
    }

    #[test]
    fn above_and_below_exclude_the_anchor_itself() {
        let c = sample();
        let above = WindowQuery { on_screen_only: false, exclude_desktop: false, stacking: Stacking::Above(3) };
        assert_eq!(ids(c.query(above)), vec![1, 2]);
        let below = WindowQuery { on_screen_only: false, exclude_desktop: false, stacking: Stacking::Below(3) };
        assert_eq!(ids(c.query(below)), vec![4, 5]);
        let top = WindowQuery { on_screen_only: false, exclude_desktop: false, stacking: Stacking::Above(1) };
        assert!(c.query(top).is_empty());
        let unknown = WindowQuery { on_screen_only: false, exclude_desktop: false, stacking: Stacking::Below(99) };
        assert!(c.query(unknown).is_empty());
    }

    #[test]
    fn window_spanning_two_displays_belongs_to_the_one_with_more_area() {
        let mut c = sample();
        // Window 1 at x 900..1200 has 100 columns on display 1 and 200 on display 2, so display 2 wins.
        c.windows[0].frame = rect(900.0, 100.0, 300.0, 200.0);
        assert_eq!(c.display_of_window(1).map(|d| d.id), Some(2));
        // A window entirely off every display belongs to none.
        c.windows[0].frame = rect(5000.0, 5000.0, 10.0, 10.0);
        assert_eq!(c.display_of_window(1), None);
        assert_eq!(c.display_of_window(999), None);
    }

    #[test]
    fn app_lookup_and_its_windows() {
        let c = sample();
        assert_eq!(c.app(20).map(|a| a.name.as_str()), Some("B"));
        assert_eq!(c.app(77), None);
        assert_eq!(ids(c.windows_of_app(10)), vec![1, 3, 5]);
        assert_eq!(c.display(2).map(|d| d.frame.x), Some(1000.0));
    }
}
