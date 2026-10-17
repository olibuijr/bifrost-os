//! Accessibility audit for apps (the Accessibility Inspector checks): an app passes the elements of its screen and the
//! window bounds, and gets back the findings for unnamed controls, empty or clipped elements, small targets, duplicate
//! names and low text contrast. The model is `bifrost_ui::ax_audit`; this module exposes it to apps and tests it.
//!
//! ```ignore
//! let audit = Audit::new(window);
//! for issue in audit.run(&elements) {
//!     println!("{} {}: {}", issue.address, issue.kind.code(), issue.detail);
//! }
//! ```

pub use bifrost_ui::ax_audit::*;

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::ax::Element;
    use bifrost_ui::Rect;

    fn element(address: &str, name: &str, role: &str, bounds: Option<Rect>) -> Element {
        Element {
            address: address.to_string(),
            name: name.to_string(),
            role: role.to_string(),
            value: String::new(),
            enabled: true,
            bounds,
            handle: String::new(),
        }
    }

    fn window() -> Rect {
        Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 }
    }

    fn big(x: f32, y: f32) -> Rect {
        Rect { x, y, w: 80.0, h: 60.0 }
    }

    #[test]
    fn a_named_button_on_screen_with_a_usable_size_has_no_issue() {
        let audit = Audit::new(window());
        let elements = [element("1", "Save", "button", Some(big(10.0, 10.0)))];
        assert!(audit.run(&elements).is_empty());
    }

    #[test]
    fn an_unnamed_interactive_element_is_reported_as_unlabeled() {
        let audit = Audit::new(window());
        let issues = audit.run(&[element("1", "  ", "button", Some(big(10.0, 10.0)))]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, Kind::Unlabeled);
        assert_eq!(issues[0].kind.code(), "unlabeled");
        assert_eq!(issues[0].address, "1");
    }

    #[test]
    fn a_small_interactive_target_is_reported_against_the_minimum() {
        let audit = Audit::new(window());
        let small = Rect { x: 10.0, y: 10.0, w: 20.0, h: 20.0 };
        let issues = audit.run(&[element("1", "Go", "button", Some(small))]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, Kind::SmallTarget);
    }

    #[test]
    fn a_caller_can_lower_the_minimum_target() {
        let audit = Audit { min_target: 16.0, ..Audit::new(window()) };
        let small = Rect { x: 10.0, y: 10.0, w: 20.0, h: 20.0 };
        assert!(audit.run(&[element("1", "Go", "button", Some(small))]).is_empty());
    }

    #[test]
    fn an_element_outside_the_window_is_off_window_and_one_past_the_edge_is_clipped() {
        let audit = Audit::new(window());
        let off = audit.run(&[element("1", "Far", "row", Some(Rect { x: 900.0, y: 0.0, w: 10.0, h: 10.0 }))]);
        assert_eq!(off[0].kind, Kind::OffWindow);
        let clipped = audit.run(&[element("2", "Edge", "row", Some(Rect { x: 350.0, y: 0.0, w: 100.0, h: 10.0 }))]);
        assert_eq!(clipped[0].kind, Kind::Clipped);
    }

    #[test]
    fn a_repeated_name_on_the_same_role_is_a_duplicate() {
        let audit = Audit::new(window());
        let elements = [
            element("1", "OK", "button", Some(big(10.0, 10.0))),
            element("2", "OK", "button", Some(big(100.0, 10.0))),
        ];
        let issues = audit.run(&elements);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, Kind::DuplicateName);
        assert_eq!(issues[0].address, "2");
    }

    #[test]
    fn an_element_with_no_size_is_empty_and_an_unplaced_element_is_checked_for_a_name_only() {
        let audit = Audit::new(window());
        let empty = audit.run(&[element("1", "Bar", "row", Some(Rect { x: 0.0, y: 0.0, w: 0.0, h: 5.0 }))]);
        assert_eq!(empty[0].kind, Kind::Empty);
        assert!(audit.run(&[element("2", "Hidden", "row", None)]).is_empty());
    }
}
