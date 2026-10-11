//! Accessibility audit (the checks of the Accessibility Inspector developer tool): walks the elements of a screen and
//! reports what a screen reader or a pointer user would trip on. Checks: an interactive element with no name, an element
//! with no size, an element clipped by the window or entirely outside it, an interactive target smaller than the minimum,
//! two interactive elements with the same role and name, and text contrast. Pure: the caller supplies the elements
//! (`ax::Element`, from `ax::Map`) and the window bounds. Nothing is drawn or read from the screen.

use crate::ax::Element;
use crate::Rect;

/// Default smallest side of an interactive element, in points. This is a BifrOSt default for pointer and touch targets; it
/// is not a value taken from the platform's documentation. Callers can pass their own through [`Audit::min_target`].
pub const MIN_TARGET_PT: f32 = 44.0;

/// Roles that take input, so they need a name and a usable size.
const INTERACTIVE: &[&str] = &["button", "field", "toggle", "checkbox", "slider", "segmented-control", "menu-item"];

/// What an audit found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An interactive element with no name, so a screen reader has nothing to announce.
    Unlabeled,
    /// An element with a zero or negative width or height.
    Empty,
    /// An element partly outside the window, so its text or controls are cut off.
    Clipped,
    /// An element entirely outside the window.
    OffWindow,
    /// An interactive element smaller than the minimum target in one dimension.
    SmallTarget,
    /// An interactive element with the same role and name as an earlier one, so a user cannot tell them apart by name.
    DuplicateName,
    /// Text whose colour contrast is below the minimum ratio.
    LowContrast,
}

impl Kind {
    /// A stable short code for reports and logs.
    pub fn code(self) -> &'static str {
        match self {
            Kind::Unlabeled => "unlabeled",
            Kind::Empty => "empty",
            Kind::Clipped => "clipped",
            Kind::OffWindow => "off-window",
            Kind::SmallTarget => "small-target",
            Kind::DuplicateName => "duplicate-name",
            Kind::LowContrast => "low-contrast",
        }
    }
}

/// One finding: the element's address, what is wrong, and a line for a report.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub address: String,
    pub kind: Kind,
    pub detail: String,
}

/// Audit settings: the window the elements are drawn in, and the smallest interactive target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Audit {
    pub window: Rect,
    pub min_target: f32,
}

impl Audit {
    /// An audit of a window with the default minimum target, [`MIN_TARGET_PT`].
    pub fn new(window: Rect) -> Audit {
        Audit { window, min_target: MIN_TARGET_PT }
    }

    /// Every issue, in element order. An element that is not on screen (`bounds: None`) is checked for a name only.
    pub fn run(&self, elements: &[Element]) -> Vec<Issue> {
        let mut out = Vec::new();
        for (i, e) in elements.iter().enumerate() {
            let interactive = INTERACTIVE.contains(&e.role.as_str());
            let named = !e.name.trim().is_empty();
            if interactive && !named {
                push(&mut out, e, Kind::Unlabeled, format!("{} has no name", e.role));
            }
            if interactive && named && elements[..i].iter().any(|p| p.role == e.role && p.name.trim() == e.name.trim()) {
                push(&mut out, e, Kind::DuplicateName, format!("{} '{}' repeats an earlier name", e.role, e.name.trim()));
            }
            let Some(b) = e.bounds else { continue };
            if b.w <= 0.0 || b.h <= 0.0 {
                push(&mut out, e, Kind::Empty, format!("{} has size {:.0} x {:.0} pt", e.role, b.w, b.h));
                continue;
            }
            if !intersects(b, self.window) {
                push(&mut out, e, Kind::OffWindow, format!("{} is outside the window", e.role));
            } else if !contains(self.window, b) {
                push(&mut out, e, Kind::Clipped, format!("{} is cut off by the window edge", e.role));
            }
            if interactive && (b.w < self.min_target || b.h < self.min_target) {
                push(
                    &mut out,
                    e,
                    Kind::SmallTarget,
                    format!("{} '{}' is {:.0} x {:.0} pt, below {:.0} pt", e.role, e.name.trim(), b.w, b.h, self.min_target),
                );
            }
        }
        out
    }
}

/// An sRGB colour with 0 to 255 per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Relative luminance of a colour, from 0 (black) to 1 (white), by the WCAG 2.x definition.
pub fn luminance(c: Rgb) -> f32 {
    fn linear(v: u8) -> f32 {
        let s = v as f32 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * linear(c.0) + 0.7152 * linear(c.1) + 0.0722 * linear(c.2)
}

/// The contrast ratio of two colours, from 1 (no contrast) to 21 (black on white).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Whether a text colour on a background meets the minimum ratio: 4.5 to 1 for body text, 3 to 1 for large text.
pub fn contrast_ok(fg: Rgb, bg: Rgb, large_text: bool) -> bool {
    contrast_ratio(fg, bg) >= if large_text { 3.0 } else { 4.5 }
}

/// An issue for a text element at `address` drawn in `fg` on `bg`, when its contrast is too low. None when it passes.
pub fn contrast_issue(address: &str, fg: Rgb, bg: Rgb, large_text: bool) -> Option<Issue> {
    if contrast_ok(fg, bg, large_text) {
        return None;
    }
    let min = if large_text { 3.0 } else { 4.5 };
    Some(Issue {
        address: address.to_string(),
        kind: Kind::LowContrast,
        detail: format!("text contrast {:.2} to 1 is below {min} to 1", contrast_ratio(fg, bg)),
    })
}

fn push(out: &mut Vec<Issue>, e: &Element, kind: Kind, detail: String) {
    out.push(Issue { address: e.address.clone(), kind, detail });
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn contains(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Rect = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };

    fn el(address: &str, role: &str, name: &str, bounds: Option<Rect>) -> Element {
        Element {
            address: address.into(),
            name: name.into(),
            role: role.into(),
            value: String::new(),
            enabled: true,
            bounds,
            handle: String::new(),
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Option<Rect> {
        Some(Rect { x, y, w, h })
    }

    #[test]
    fn interactive_elements_without_a_name_are_reported() {
        let issues = Audit::new(WINDOW).run(&[
            el("win/button/1", "button", "", rect(10.0, 10.0, 60.0, 60.0)),
            el("win/text/2", "text", "", rect(10.0, 80.0, 60.0, 60.0)),
        ]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, Kind::Unlabeled);
        assert_eq!(issues[0].address, "win/button/1");
    }

    #[test]
    fn elements_cut_off_by_or_outside_the_window_are_reported() {
        let issues = Audit::new(WINDOW).run(&[
            el("a", "text", "Edge", rect(380.0, 0.0, 40.0, 20.0)),
            el("b", "text", "Gone", rect(500.0, 0.0, 40.0, 20.0)),
            el("c", "text", "Inside", rect(10.0, 10.0, 40.0, 20.0)),
        ]);
        let kinds: Vec<(&str, Kind)> = issues.iter().map(|i| (i.address.as_str(), i.kind)).collect();
        assert_eq!(kinds, vec![("a", Kind::Clipped), ("b", Kind::OffWindow)]);
    }

    #[test]
    fn small_interactive_targets_use_the_configured_minimum() {
        let small = [el("btn", "button", "Close", rect(10.0, 10.0, 30.0, 30.0))];
        let default = Audit::new(WINDOW).run(&small);
        assert_eq!(default.len(), 1);
        assert_eq!(default[0].kind, Kind::SmallTarget);
        let lowered = Audit { window: WINDOW, min_target: 20.0 }.run(&small);
        assert!(lowered.is_empty());
    }

    #[test]
    fn a_repeated_name_is_reported_on_the_later_element() {
        let issues = Audit::new(WINDOW).run(&[
            el("list/save", "button", "Save", rect(10.0, 10.0, 60.0, 60.0)),
            el("list/save-2", "button", "Save", rect(10.0, 80.0, 60.0, 60.0)),
        ]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, Kind::DuplicateName);
        assert_eq!(issues[0].address, "list/save-2");
    }

    #[test]
    fn an_empty_element_is_reported_once_and_not_as_a_small_target() {
        let issues = Audit::new(WINDOW).run(&[el("btn", "button", "Hidden", rect(10.0, 10.0, 0.0, 0.0))]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, Kind::Empty);
    }

    #[test]
    fn an_element_with_no_bounds_is_checked_for_its_name_only() {
        let issues = Audit::new(WINDOW).run(&[el("menu/copy", "menu-item", "Copy", None)]);
        assert!(issues.is_empty());
    }

    #[test]
    fn black_on_white_is_full_contrast_and_grey_text_sits_on_the_threshold() {
        let white = Rgb(255, 255, 255);
        assert!(contrast_ratio(Rgb(0, 0, 0), white) > 20.9);
        // #767676 on white is 4.54 to 1, which passes for body text; #777777 is 4.48 to 1, which does not.
        assert!(contrast_ok(Rgb(0x76, 0x76, 0x76), white, false));
        assert!(!contrast_ok(Rgb(0x77, 0x77, 0x77), white, false));
        // The same grey passes for large text, where 3 to 1 is enough.
        assert!(contrast_ok(Rgb(0x77, 0x77, 0x77), white, true));
    }

    #[test]
    fn low_contrast_text_yields_an_issue_and_good_contrast_does_not() {
        let white = Rgb(255, 255, 255);
        let issue = contrast_issue("label/hint", Rgb(0xbb, 0xbb, 0xbb), white, false).expect("light grey fails");
        assert_eq!(issue.kind, Kind::LowContrast);
        assert_eq!(issue.address, "label/hint");
        assert!(contrast_issue("label/title", Rgb(0, 0, 0), white, false).is_none());
    }
}
