//! An embedded preview view: one item drawn inside the app's own view hierarchy, rather than in the floating preview
//! panel (the reference SDK's preview view, see [`crate::preview`] for the panel-free data side).
//!
//! Pure model: which item is shown, how far its preview has got ([`DisplayState`]), whether the view starts loading by
//! itself when an item is set, and whether it closes with its window. The shell draws the view and reports when a
//! preview finishes with [`PreviewView::finish_loading`].
//!
//! Limit: an embedded view is not a separate process, so the item is a name or a path that the app loads itself. The
//! reference SDK's view also renders the item; that renderer is the file viewer in this desktop, not this model.

/// The two layouts of a preview: the full-size one and the compact one for inspectors and small areas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Normal,
    Compact,
}

/// How far the preview of the current item has got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayState {
    /// No item, or an item waiting for `refresh_preview_item` because autostart is off.
    #[default]
    Idle,
    /// The preview is being produced.
    Loading,
    /// The preview is on screen.
    Ready,
    /// The item cannot be previewed.
    Failed,
}

/// The embedded preview: the item it shows and the settings that decide when it loads and closes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewView {
    style: Style,
    item: Option<String>,
    state: DisplayState,
    autostarts: bool,
    should_close_with_window: bool,
}

impl PreviewView {
    /// A view with no item. It starts loading by itself and closes with its window.
    pub fn new(style: Style) -> Self {
        PreviewView { style, item: None, state: DisplayState::Idle, autostarts: true, should_close_with_window: true }
    }

    pub fn style(&self) -> Style {
        self.style
    }

    /// `previewItem`: the name or path shown, `None` when the view is empty.
    pub fn preview_item(&self) -> Option<&str> {
        self.item.as_deref()
    }

    /// `displayState`.
    pub fn display_state(&self) -> DisplayState {
        self.state
    }

    /// `autostarts`: whether setting an item starts its preview at once.
    pub fn autostarts(&self) -> bool {
        self.autostarts
    }

    pub fn set_autostarts(&mut self, autostarts: bool) {
        self.autostarts = autostarts;
    }

    /// `shouldCloseWithWindow`.
    pub fn should_close_with_window(&self) -> bool {
        self.should_close_with_window
    }

    pub fn set_should_close_with_window(&mut self, close: bool) {
        self.should_close_with_window = close;
    }

    /// Sets `previewItem`. A blank name empties the view. With autostart on, a new item goes straight to `Loading`;
    /// otherwise it waits in `Idle` for [`PreviewView::refresh_preview_item`].
    pub fn set_preview_item(&mut self, item: Option<&str>) {
        self.item = item.filter(|s| !s.trim().is_empty()).map(str::to_string);
        self.state = match (&self.item, self.autostarts) {
            (Some(_), true) => DisplayState::Loading,
            _ => DisplayState::Idle,
        };
    }

    /// `refreshPreviewItem()`: starts loading the current item again. Without an item the view stays `Idle`.
    pub fn refresh_preview_item(&mut self) {
        self.state = if self.item.is_some() { DisplayState::Loading } else { DisplayState::Idle };
    }

    /// The loader reports how the preview ended. Only a `Loading` view changes; the result says whether it did.
    pub fn finish_loading(&mut self, ok: bool) -> bool {
        if self.state != DisplayState::Loading {
            return false;
        }
        self.state = if ok { DisplayState::Ready } else { DisplayState::Failed };
        true
    }

    /// `close()`: releases the item and returns to `Idle`.
    pub fn close(&mut self) {
        self.item = None;
        self.state = DisplayState::Idle;
    }

    /// The window holding the view closes. Returns true when the view closed with it.
    pub fn window_closed(&mut self) -> bool {
        if self.should_close_with_window {
            self.close();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_view_is_empty_and_loads_and_closes_by_itself() {
        let v = PreviewView::new(Style::Compact);
        assert_eq!(v.style(), Style::Compact);
        assert_eq!(v.preview_item(), None);
        assert_eq!(v.display_state(), DisplayState::Idle);
        assert!(v.autostarts());
        assert!(v.should_close_with_window());
    }

    #[test]
    fn an_item_with_autostart_goes_to_loading_then_ready() {
        let mut v = PreviewView::new(Style::Normal);
        v.set_preview_item(Some("/home/u/report.pdf"));
        assert_eq!(v.preview_item(), Some("/home/u/report.pdf"));
        assert_eq!(v.display_state(), DisplayState::Loading);
        assert!(v.finish_loading(true));
        assert_eq!(v.display_state(), DisplayState::Ready);
    }

    #[test]
    fn without_autostart_an_item_waits_for_a_refresh() {
        let mut v = PreviewView::new(Style::Normal);
        v.set_autostarts(false);
        v.set_preview_item(Some("notes.md"));
        assert_eq!(v.display_state(), DisplayState::Idle);
        assert!(!v.finish_loading(true), "nothing is loading yet");
        v.refresh_preview_item();
        assert_eq!(v.display_state(), DisplayState::Loading);
        assert!(v.finish_loading(false));
        assert_eq!(v.display_state(), DisplayState::Failed);
    }

    #[test]
    fn a_blank_item_empties_the_view() {
        let mut v = PreviewView::new(Style::Normal);
        v.set_preview_item(Some("   "));
        assert_eq!(v.preview_item(), None);
        assert_eq!(v.display_state(), DisplayState::Idle);
        v.refresh_preview_item();
        assert_eq!(v.display_state(), DisplayState::Idle);
    }

    #[test]
    fn close_releases_the_item() {
        let mut v = PreviewView::new(Style::Normal);
        v.set_preview_item(Some("a.txt"));
        v.finish_loading(true);
        v.close();
        assert_eq!(v.preview_item(), None);
        assert_eq!(v.display_state(), DisplayState::Idle);
    }

    #[test]
    fn the_window_closes_the_view_only_when_asked_to() {
        let mut keep = PreviewView::new(Style::Normal);
        keep.set_should_close_with_window(false);
        keep.set_preview_item(Some("a.txt"));
        assert!(!keep.window_closed());
        assert_eq!(keep.preview_item(), Some("a.txt"));

        let mut shut = PreviewView::new(Style::Normal);
        shut.set_preview_item(Some("a.txt"));
        assert!(shut.window_closed());
        assert_eq!(shut.preview_item(), None);
    }
}
