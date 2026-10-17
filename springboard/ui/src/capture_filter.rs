//! Content filters: which windows a capture takes from the shareable content. A filter is either one window,
//! wherever it is, or one display with a rule for the windows on it. `captured` resolves the filter against
//! the content the shell reported, so the result always names windows that exist. Pure: no capture happens here.

use crate::capture_content::{CaptureWindow, ShareableContent};
use crate::Rect;

/// The rule for the windows of a display. A listed window id always counts as outside any listed app, so
/// it is the exception to that app's rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Captures only these windows.
    IncludeWindows(Vec<u32>),
    /// Captures every window except these.
    ExcludeWindows(Vec<u32>),
    /// Captures the windows of these apps, except the listed windows.
    IncludeApps { pids: Vec<u32>, except: Vec<u32> },
    /// Captures every window except those of these apps. The listed windows are captured even when their
    /// app is listed.
    ExcludeApps { pids: Vec<u32>, except: Vec<u32> },
}

impl Rule {
    /// Whether this rule keeps `window`.
    pub fn keeps(&self, window: &CaptureWindow) -> bool {
        match self {
            Rule::IncludeWindows(ids) => ids.contains(&window.id),
            Rule::ExcludeWindows(ids) => !ids.contains(&window.id),
            Rule::IncludeApps { pids, except } => in_app(pids, except, window),
            Rule::ExcludeApps { pids, except } => !in_app(pids, except, window),
        }
    }
}

/// True when the window belongs to one of the listed apps and is not an exception.
fn in_app(pids: &[u32], except: &[u32], window: &CaptureWindow) -> bool {
    pids.contains(&window.app_pid) && !except.contains(&window.id)
}

/// What a filter covers: one window on its own, or a display with a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Window(u32),
    Display { display: u32, rule: Rule },
}

/// A capture filter. `include_menu_bar` and `point_pixel_scale` are carried for the stream to read.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentFilter {
    pub scope: Scope,
    pub include_menu_bar: bool,
    /// Device pixels per point. Zero or negative values are read as 1.
    pub point_pixel_scale: f32,
}

impl ContentFilter {
    /// A filter that captures one window only.
    pub fn window(window: u32) -> Self {
        ContentFilter { scope: Scope::Window(window), include_menu_bar: false, point_pixel_scale: 1.0 }
    }

    /// A filter over one display, with `rule` deciding which of its windows are captured.
    pub fn display(display: u32, rule: Rule) -> Self {
        ContentFilter {
            scope: Scope::Display { display, rule },
            include_menu_bar: false,
            point_pixel_scale: 1.0,
        }
    }

    /// The same filter with the menu bar included or left out.
    pub fn with_menu_bar(mut self, include: bool) -> Self {
        self.include_menu_bar = include;
        self
    }

    /// The same filter rendered at `scale` device pixels per point.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.point_pixel_scale = scale;
        self
    }

    /// The device pixels per point the stream renders at. A zero, negative or non-finite scale is read as 1.
    pub fn scale(&self) -> f32 {
        if self.point_pixel_scale.is_finite() && self.point_pixel_scale > 0.0 {
            self.point_pixel_scale
        } else {
            1.0
        }
    }

    /// The windows this filter captures, in the content's front-to-back order. A window scope yields that
    /// window if it exists. A display scope yields the windows whose largest overlap is that display and that
    /// the rule keeps.
    pub fn captured<'a>(&self, content: &'a ShareableContent) -> Vec<&'a CaptureWindow> {
        match &self.scope {
            Scope::Window(id) => content.window(*id).into_iter().collect(),
            Scope::Display { display, rule } => content
                .windows
                .iter()
                .filter(|w| content.display_of_window(w.id).map(|d| d.id) == Some(*display))
                .filter(|w| rule.keeps(w))
                .collect(),
        }
    }

    /// Whether the window with this id is captured.
    pub fn captures_window(&self, content: &ShareableContent, id: u32) -> bool {
        self.captured(content).iter().any(|w| w.id == id)
    }

    /// The rectangle of the display or the windows the stream shows, in global points. A window scope gives that
    /// window's frame. A display scope with an exclusion rule gives the whole display. A display scope with an
    /// inclusion rule gives the bounding box of the windows it captures. None when nothing is captured.
    pub fn source_region(&self, content: &ShareableContent) -> Option<Rect> {
        match &self.scope {
            Scope::Window(id) => content.window(*id).map(|w| w.frame),
            Scope::Display { display, rule } => match rule {
                Rule::ExcludeWindows(_) | Rule::ExcludeApps { .. } => content.display(*display).map(|d| d.frame),
                Rule::IncludeWindows(_) | Rule::IncludeApps { .. } => {
                    let frames: Vec<Rect> = self.captured(content).iter().map(|w| w.frame).collect();
                    bounding_box(&frames)
                }
            },
        }
    }

    /// The frames of the on-screen windows this filter does not capture that overlap the source region. They are
    /// blanked in the picture, so a window that is left out never shows through a captured one.
    pub fn masked_frames(&self, content: &ShareableContent) -> Vec<Rect> {
        let Some(region) = self.source_region(content) else { return Vec::new() };
        content.windows.iter().filter(|w| w.on_screen && !self.captures_window(content, w.id)).filter(|w| overlaps(&w.frame, &region)).map(|w| w.frame).collect()
    }
}

/// Whether two rectangles share some area (touching edges do not count).
fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// The smallest rectangle that holds every rectangle in `rects`, or None when there are none.
pub fn bounding_box(rects: &[Rect]) -> Option<Rect> {
    let first = rects.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x + first.w, first.y + first.h);
    for r in &rects[1..] {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.w);
        y1 = y1.max(r.y + r.h);
    }
    Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_content::{CaptureDisplay, CaptureWindow};
    use crate::Rect;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn win(id: u32, pid: u32, frame: Rect) -> CaptureWindow {
        CaptureWindow { id, title: format!("w{id}"), app_pid: pid, frame, on_screen: true, desktop: false }
    }

    /// Display 1 holds windows 1 and 3 (app 10). Display 2 holds window 2 (app 20) and window 4 (app 30).
    /// Front to back the order is 1, 2, 3, 4.
    fn sample() -> ShareableContent {
        let displays = vec![
            CaptureDisplay { id: 1, frame: rect(0.0, 0.0, 1000.0, 800.0) },
            CaptureDisplay { id: 2, frame: rect(1000.0, 0.0, 1000.0, 800.0) },
        ];
        let windows = vec![
            win(1, 10, rect(100.0, 100.0, 200.0, 200.0)),
            win(2, 20, rect(1100.0, 100.0, 200.0, 200.0)),
            win(3, 10, rect(300.0, 300.0, 200.0, 200.0)),
            win(4, 30, rect(1200.0, 300.0, 100.0, 100.0)),
        ];
        ShareableContent::new(displays, Vec::new(), windows)
    }

    fn ids(list: Vec<&CaptureWindow>) -> Vec<u32> {
        list.iter().map(|w| w.id).collect()
    }

    #[test]
    fn a_window_region_is_that_window_and_masks_the_others() {
        let c = sample();
        assert_eq!(ContentFilter::window(2).source_region(&c), Some(rect(1100.0, 100.0, 200.0, 200.0)));
        assert_eq!(ContentFilter::window(99).source_region(&c), None);
        // no other window overlaps window 2's frame (window 4 only touches its bottom edge)
        assert!(ContentFilter::window(2).masked_frames(&c).is_empty());
        // a window that overlaps the captured one is blanked where it covers it
        let mut overlapped = sample();
        overlapped.windows.push(win(5, 40, rect(1150.0, 150.0, 100.0, 100.0)));
        assert_eq!(ContentFilter::window(2).masked_frames(&overlapped), vec![rect(1150.0, 150.0, 100.0, 100.0)]);
    }

    #[test]
    fn a_display_minus_excluded_apps_is_the_whole_display() {
        let c = sample();
        let f = ContentFilter::display(1, Rule::ExcludeApps { pids: vec![10], except: vec![] });
        assert_eq!(f.source_region(&c), Some(rect(0.0, 0.0, 1000.0, 800.0)));
        // the excluded app's two windows are blanked, the others are not
        let masks = f.masked_frames(&c);
        assert_eq!(masks, vec![rect(100.0, 100.0, 200.0, 200.0), rect(300.0, 300.0, 200.0, 200.0)]);
    }

    #[test]
    fn one_app_is_the_bounding_box_of_its_windows() {
        let c = sample();
        let f = ContentFilter::display(1, Rule::IncludeApps { pids: vec![10], except: vec![] });
        assert_eq!(f.source_region(&c), Some(rect(100.0, 100.0, 400.0, 400.0)));
        assert_eq!(bounding_box(&[]), None);
    }

    #[test]
    fn window_filter_yields_that_window_or_nothing() {
        let c = sample();
        assert_eq!(ids(ContentFilter::window(2).captured(&c)), vec![2]);
        assert!(ContentFilter::window(99).captured(&c).is_empty());
        assert!(ContentFilter::window(2).captures_window(&c, 2));
        assert!(!ContentFilter::window(2).captures_window(&c, 1));
    }

    #[test]
    fn include_windows_keeps_only_listed_windows_of_that_display() {
        let c = sample();
        let f = ContentFilter::display(1, Rule::IncludeWindows(vec![3, 1, 2]));
        // Window 2 is listed but lives on display 2, so it is not captured here.
        assert_eq!(ids(f.captured(&c)), vec![1, 3]);
    }

    #[test]
    fn exclude_windows_drops_only_the_listed_windows() {
        let c = sample();
        let f = ContentFilter::display(1, Rule::ExcludeWindows(vec![1]));
        assert_eq!(ids(f.captured(&c)), vec![3]);
    }

    #[test]
    fn include_apps_with_an_exception_leaves_the_exception_out() {
        let c = sample();
        let f = ContentFilter::display(2, Rule::IncludeApps { pids: vec![30, 20], except: vec![4] });
        // Window 2 belongs to an included app and is not excepted. Window 4 is excepted, so it is out.
        assert_eq!(ids(f.captured(&c)), vec![2]);
    }

    #[test]
    fn exclude_apps_with_an_exception_keeps_the_exception() {
        let c = sample();
        let f = ContentFilter::display(1, Rule::ExcludeApps { pids: vec![10], except: vec![3] });
        // Window 1 belongs to the excluded app, so it is dropped. Window 3 is excepted, so it stays.
        assert_eq!(ids(f.captured(&c)), vec![3]);
    }

    #[test]
    fn display_scope_ignores_windows_on_other_displays() {
        let c = sample();
        let f = ContentFilter::display(2, Rule::IncludeWindows(vec![1]));
        assert!(f.captured(&c).is_empty());
        assert!(!f.captures_window(&c, 1));
    }
}
