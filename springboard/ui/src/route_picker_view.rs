//! The route picker button as pure state: how the button is highlighted, and the delegate events an app receives while the
//! routing menu is open. The menu itself (the rows) comes from `route_picker`; this file only tracks the button. Nothing is
//! drawn here, and no route is chosen here.

/// The look of the button. `System` lets the button follow whether a route is in use. `Inactive` and `Active` pin the look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonStyle {
    #[default]
    System,
    Inactive,
    Active,
}

/// Delegate events. `WillBegin` is sent before the menu opens and `DidEnd` after it has closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    WillBeginPresentingRoutes,
    DidEndPresentingRoutes,
}

/// The state of one route picker button.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoutePickerView {
    style: ButtonStyle,
    route_active: bool,
    presenting: bool,
    /// When set, the button is drawn with a border. Off by default, so the button is a plain glyph.
    pub bordered: bool,
}

impl RoutePickerView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn style(&self) -> ButtonStyle {
        self.style
    }

    pub fn set_style(&mut self, style: ButtonStyle) {
        self.style = style;
    }

    /// Records whether a route is in use. Only the `System` style reads this value.
    pub fn set_route_active(&mut self, active: bool) {
        self.route_active = active;
    }

    /// True when the button should be drawn highlighted.
    pub fn is_highlighted(&self) -> bool {
        match self.style {
            ButtonStyle::System => self.route_active,
            ButtonStyle::Inactive => false,
            ButtonStyle::Active => true,
        }
    }

    pub fn is_presenting(&self) -> bool {
        self.presenting
    }

    /// Opens the routing menu. Returns the event to deliver, or `None` when the menu is already open.
    pub fn begin_presenting(&mut self) -> Option<Event> {
        if self.presenting {
            return None;
        }
        self.presenting = true;
        Some(Event::WillBeginPresentingRoutes)
    }

    /// Closes the routing menu. Returns the event to deliver, or `None` when no menu is open.
    pub fn end_presenting(&mut self) -> Option<Event> {
        if !self.presenting {
            return None;
        }
        self.presenting = false;
        Some(Event::DidEndPresentingRoutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_button_uses_the_system_style_and_is_not_bordered() {
        let view = RoutePickerView::new();
        assert_eq!(view.style(), ButtonStyle::System);
        assert!(!view.bordered);
        assert!(!view.is_presenting());
    }

    #[test]
    fn the_system_style_highlights_only_while_a_route_is_active() {
        let mut view = RoutePickerView::new();
        assert!(!view.is_highlighted());
        view.set_route_active(true);
        assert!(view.is_highlighted());
        view.set_route_active(false);
        assert!(!view.is_highlighted());
    }

    #[test]
    fn a_pinned_style_ignores_whether_a_route_is_active() {
        let mut view = RoutePickerView::new();
        view.set_route_active(true);
        view.set_style(ButtonStyle::Inactive);
        assert!(!view.is_highlighted());
        view.set_route_active(false);
        view.set_style(ButtonStyle::Active);
        assert!(view.is_highlighted());
    }

    #[test]
    fn opening_the_menu_sends_will_begin_once() {
        let mut view = RoutePickerView::new();
        assert_eq!(view.begin_presenting(), Some(Event::WillBeginPresentingRoutes));
        assert!(view.is_presenting());
        assert_eq!(view.begin_presenting(), None);
    }

    #[test]
    fn closing_the_menu_sends_did_end_only_when_it_was_open() {
        let mut view = RoutePickerView::new();
        assert_eq!(view.end_presenting(), None);
        view.begin_presenting();
        assert_eq!(view.end_presenting(), Some(Event::DidEndPresentingRoutes));
        assert!(!view.is_presenting());
        assert_eq!(view.end_presenting(), None);
    }
}
