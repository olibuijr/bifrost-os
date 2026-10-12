//! Preview (`QLPreviewPanel`, `QLPreviewItem`, `QLThumbnailGenerator`).
//!
//! [`PreviewPanel`] holds an ordered list of [`PreviewItem`]s and a current index that is always clamped. An app either
//! hands it a list or pulls one from a [`PreviewDataSource`] (at most [`MAX_ITEMS`]). Each item carries an [`ItemState`].
//! [`PreviewPanel::make_key_and_order_front`] sends `ToShell::Preview` (the shell draws the floating panel) and
//! [`PreviewPanel::toggle`] is the Space-bar behaviour. Thumbnails: [`thumbnail_path`] names the cached PNG for a file and
//! size, and [`ThumbnailCache`] remembers which were generated (an `NativeCache`, see `crate::cache`).

pub use crate::proto::peek::{PreviewItem, MAX_ITEMS};
/// Data-based previews and thumbnail requests (`QLPreviewProvider`, `QLPreviewReply`, `QLThumbnailGenerator.Request`):
/// the pure models in `bifrost_ui::preview`, re-exported for apps. See [`Registry`] and [`ThumbnailRequest`].
pub use bifrost_ui::preview::{Attachment, Content, ModelError, PreviewProvider, Registry, Reply, Representation, ThumbnailRequest};
/// Custom file type thumbnails (`QLThumbnailProvider`, `QLThumbnailReply`), generator errors and progressive
/// updates: the pure models in `bifrost_ui::thumbnail`. See [`FileThumbnailRequest`] and [`Progressive`].
pub use bifrost_ui::thumbnail::{FileThumbnailRequest, Progressive, ThumbnailContent, ThumbnailError, ThumbnailReply};
use crate::proto::ToShell;
use crate::Ctx;

/// `previewItemDisplayState`: how far an item's preview has got. The app sets it. The wire message carries only the URL
/// and title, so the shell does not draw this state yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemState {
    /// The preview content is still being produced.
    Loading,
    #[default]
    Ready,
    /// The item cannot be previewed.
    Failed,
}

/// `QLPreviewPanelDataSource`: the panel pulls its items from here instead of being handed a list.
pub trait PreviewDataSource {
    /// `numberOfPreviewItems(in:)`
    fn number_of_items(&self) -> usize;
    /// `previewPanel(_:previewItemAt:)`: None past the end.
    fn item_at(&self, index: usize) -> Option<PreviewItem>;
}

impl PreviewDataSource for Vec<PreviewItem> {
    fn number_of_items(&self) -> usize {
        self.len()
    }

    fn item_at(&self, index: usize) -> Option<PreviewItem> {
        self.get(index).cloned()
    }
}

#[derive(Clone, Debug, Default)]
pub struct PreviewPanel {
    items: Vec<PreviewItem>,
    /// One display state per item, kept in step with `items`.
    states: Vec<ItemState>,
    index: usize,
    visible: bool,
}

impl PreviewPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// `dataSource` items, in order; the current index is clamped into the new list. Every item starts `Ready`.
    pub fn set_items(&mut self, items: Vec<PreviewItem>) {
        self.states = vec![ItemState::Ready; items.len()];
        self.items = items;
        self.index = self.index.min(self.items.len().saturating_sub(1));
    }

    /// Pulls the items from `source`, at most [`MAX_ITEMS`], and clamps the index as [`PreviewPanel::set_items`] does.
    /// Does not send anything.
    pub fn load_from(&mut self, source: &dyn PreviewDataSource) {
        let n = source.number_of_items().min(MAX_ITEMS);
        let items = (0..n).filter_map(|i| source.item_at(i)).collect();
        self.set_items(items);
    }

    /// `reloadData` with a data source: pulls the items, then re-sends the list while visible.
    pub fn reload_data_from(&mut self, source: &dyn PreviewDataSource, ctx: &Ctx) {
        self.load_from(source);
        self.reload_data(ctx);
    }

    /// `previewItemDisplayState` of item `i`; None past the end.
    pub fn display_state(&self, i: usize) -> Option<ItemState> {
        self.states.get(i).copied()
    }

    /// Sets the display state of item `i`. Returns false, and changes nothing, past the end.
    pub fn set_display_state(&mut self, i: usize, state: ItemState) -> bool {
        match self.states.get_mut(i) {
            Some(s) => {
                *s = state;
                true
            }
            None => false,
        }
    }

    /// `refreshCurrentPreviewItem`: the wire has no per-item message, so this re-sends the whole list while visible.
    pub fn refresh_current_item(&self, ctx: &Ctx) {
        self.reload_data(ctx);
    }

    pub fn items(&self) -> &[PreviewItem] {
        &self.items
    }

    /// `currentPreviewItemIndex`
    pub fn current_preview_item_index(&self) -> usize {
        self.index
    }

    /// Sets the index, clamped to the list.
    pub fn set_current_preview_item_index(&mut self, i: usize) {
        self.index = i.min(self.items.len().saturating_sub(1));
    }

    pub fn current_item(&self) -> Option<&PreviewItem> {
        self.items.get(self.index)
    }

    /// Arrow keys: step by `delta` without wrapping.
    pub fn step(&mut self, delta: i32) {
        let i = (self.index as i64 + delta as i64).max(0) as usize;
        self.set_current_preview_item_index(i);
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Space: shows the panel when hidden (and there is something to show), hides it otherwise. Returns the new visibility.
    pub fn toggle(&mut self, ctx: &Ctx) -> bool {
        if self.visible {
            self.close(ctx);
        } else {
            self.make_key_and_order_front(ctx);
        }
        self.visible
    }

    pub fn make_key_and_order_front(&mut self, ctx: &Ctx) {
        if self.items.is_empty() {
            return;
        }
        self.visible = true;
        self.reload_data(ctx);
    }

    /// `reloadData`: re-sends the list while visible.
    pub fn reload_data(&self, ctx: &Ctx) {
        if self.visible {
            ctx.send_to_shell(self.message());
        }
    }

    pub fn close(&mut self, ctx: &Ctx) {
        if std::mem::replace(&mut self.visible, false) {
            ctx.send_to_shell(ToShell::PreviewClose);
        }
    }

    pub(crate) fn message(&self) -> ToShell {
        ToShell::Preview { items: self.items.clone(), index: self.index as u32 }
    }
}

/// `QLThumbnailGenerator` target: the cached PNG path for `url` at `size` points and `scale`.
pub fn thumbnail_path(url: &str, size: u32, scale: u32) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (url, size, scale).hash(&mut h);
    let base = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("bifrost-peek").join(format!("{:016x}.png", h.finish()))
}

/// Remembers generated thumbnails (`generateBestRepresentation` results). Purgeable under memory pressure.
pub struct ThumbnailCache(crate::cache::Cache<String, std::path::PathBuf>);

impl ThumbnailCache {
    pub fn new() -> Self {
        Self(crate::cache::Cache::new(256, 256))
    }

    /// The cached PNG path for `url` if generated and still on disk; otherwise `generate(path)` renders the PNG at the
    /// returned path and, when it returns true, the path is cached.
    pub fn best_representation(&self, url: &str, size: u32, scale: u32, generate: impl FnOnce(&std::path::Path) -> bool) -> Option<std::path::PathBuf> {
        let key = format!("{url}@{size}x{scale}");
        if let Some(p) = self.0.get(&key) {
            if p.exists() {
                return Some(p);
            }
        }
        let p = thumbnail_path(url, size, scale);
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if generate(&p) {
            self.0.set(key, p.clone(), 1);
            return Some(p);
        }
        None
    }
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self::new()
    }
}

/// `QLPreviewViewStyle`: the layout of an embedded preview. `normal` is for larger, full-sized previews and `compact`
/// for smaller previews or inspectors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewStyle {
    #[default]
    Normal,
    Compact,
}

/// `QLPreviewView`: a preview an app places in its own view hierarchy, where [`PreviewPanel`] is a floating panel.
/// It holds one item, the item's display state, and two switches: whether a new item starts loading on its own
/// (`autostarts`) and whether the preview closes with its window. Pure model: the app's view layer draws it.
#[derive(Clone, Debug)]
pub struct PreviewView {
    style: ViewStyle,
    item: Option<PreviewItem>,
    state: ItemState,
    autostarts: bool,
    closes_with_window: bool,
    refreshes: u64,
}

impl PreviewView {
    pub fn new(style: ViewStyle) -> Self {
        PreviewView {
            style,
            item: None,
            state: ItemState::Ready,
            autostarts: true,
            closes_with_window: true,
            refreshes: 0,
        }
    }

    pub fn style(&self) -> ViewStyle {
        self.style
    }

    /// `previewItem`: the item on show, if any.
    pub fn preview_item(&self) -> Option<&PreviewItem> {
        self.item.as_ref()
    }

    /// `displayState`: how far the current item's preview has got.
    pub fn display_state(&self) -> ItemState {
        self.state
    }

    /// The app reports how far the preview got (for example after a load finishes).
    pub fn set_display_state(&mut self, state: ItemState) {
        self.state = state;
    }

    pub fn autostarts(&self) -> bool {
        self.autostarts
    }

    pub fn set_autostarts(&mut self, on: bool) {
        self.autostarts = on;
    }

    pub fn closes_with_window(&self) -> bool {
        self.closes_with_window
    }

    pub fn set_closes_with_window(&mut self, on: bool) {
        self.closes_with_window = on;
    }

    /// `previewItem` setter. With `autostarts` a new item starts as `Loading`; without it the item waits at `Ready`
    /// until the app sets the state. Clearing the item sets `Ready`.
    pub fn set_preview_item(&mut self, item: Option<PreviewItem>) {
        self.state = match (&item, self.autostarts) {
            (Some(_), true) => ItemState::Loading,
            _ => ItemState::Ready,
        };
        self.item = item;
    }

    /// `refreshPreviewItem()`: re-reads the current item. Returns false when there is no item to refresh.
    pub fn refresh_preview_item(&mut self) -> bool {
        if self.item.is_none() {
            return false;
        }
        self.refreshes += 1;
        true
    }

    /// How many refreshes have been accepted since the view was made.
    pub fn refresh_count(&self) -> u64 {
        self.refreshes
    }

    /// `close()`: drops the current item and returns it. The state goes back to `Ready`.
    pub fn close(&mut self) -> Option<PreviewItem> {
        self.state = ItemState::Ready;
        self.item.take()
    }

    /// The owning window closed. Closes the view only when `closes_with_window` is set; returns whether it did.
    pub fn window_closed(&mut self) -> bool {
        if self.closes_with_window {
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

    fn item(n: &str) -> PreviewItem {
        PreviewItem { url: format!("file:///{n}"), title: n.into() }
    }

    #[test]
    fn index_clamps_and_order_is_kept() {
        let mut p = PreviewPanel::new();
        p.set_current_preview_item_index(5);
        assert_eq!(p.current_preview_item_index(), 0);
        p.set_items(vec![item("a"), item("b"), item("c")]);
        p.set_current_preview_item_index(9);
        assert_eq!(p.current_item().unwrap().title, "c");
        p.step(1);
        assert_eq!(p.current_preview_item_index(), 2);
        p.step(-5);
        assert_eq!(p.current_preview_item_index(), 0);
        p.set_current_preview_item_index(2);
        p.set_items(vec![item("x")]);
        assert_eq!(p.current_preview_item_index(), 0);
        assert!(matches!(p.message(), ToShell::Preview { index: 0, .. }));
    }

    #[test]
    fn thumbnails_are_cached() {
        let c = ThumbnailCache::new();
        let u = "file:///peek-test-unique.png";
        let a = c.best_representation(u, 64, 1, |p| std::fs::write(p, b"png").is_ok()).unwrap();
        let b = c.best_representation(u, 64, 1, |_| panic!("cached")).unwrap();
        assert_eq!(a, b);
        let _ = std::fs::remove_file(a);
    }

    #[test]
    fn provider_routing_is_reachable_from_the_sdk() {
        let mut reg = Registry::new();
        reg.register("text", &["public.plain-text"]).unwrap();
        reg.register("md", &["net.daringfireball.markdown"]).unwrap();
        assert_eq!(reg.provider_for("README.md"), Some("md"));
        assert_eq!(reg.provider_for("todo.txt"), Some("text"));
        assert_eq!(reg.provider_for("image.png"), None);
    }

    #[test]
    fn html_reply_resolves_its_inline_attachments() {
        let body = Content::Html {
            body: "<img src=\"cid:chart\">".into(),
            attachments: vec![Attachment { id: "chart".into(), data: vec![0x89, b'P'], content_type: "image/png".into() }],
        };
        let reply = Reply::new(body).unwrap().with_title("Q3 report");
        assert_eq!(reply.display_title("q3.html"), "Q3 report");
        assert_eq!(reply.resolve_cid("cid:chart").unwrap().data.len(), 2);
        assert!(Reply::new(Content::Bitmap { width: 0, height: 1 }).is_err());
    }

    #[test]
    fn thumbnail_request_picks_the_best_produced_size() {
        let req = ThumbnailRequest::new("file:///a.png", (128.0, 128.0), 2.0, &[Representation::Thumbnail, Representation::Icon]).unwrap();
        assert_eq!(req.pixel_size(), (256, 256));
        assert_eq!(req.best_of(&[Representation::Icon]), Some(Representation::Icon));
        assert_eq!(req.best_of(&[Representation::Thumbnail, Representation::Icon]), Some(Representation::Thumbnail));
        assert_eq!(req.best_of(&[Representation::LowQuality]), None);
    }

    /// A data source that makes its items on demand, like an app with a large folder.
    struct Generated(usize);

    impl PreviewDataSource for Generated {
        fn number_of_items(&self) -> usize {
            self.0
        }

        fn item_at(&self, index: usize) -> Option<PreviewItem> {
            (index < self.0).then(|| item(&format!("g{index}")))
        }
    }

    #[test]
    fn data_source_pull_keeps_order_and_caps_at_max_items() {
        let mut p = PreviewPanel::new();
        p.load_from(&Generated(3));
        let titles: Vec<&str> = p.items().iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["g0", "g1", "g2"]);
        p.load_from(&Generated(MAX_ITEMS + 9));
        assert_eq!(p.items().len(), MAX_ITEMS);
        assert_eq!(p.items().last().unwrap().title, format!("g{}", MAX_ITEMS - 1));
    }

    #[test]
    fn data_source_pull_clamps_the_current_index() {
        let mut p = PreviewPanel::new();
        p.load_from(&Generated(5));
        p.set_current_preview_item_index(4);
        p.load_from(&Generated(2));
        assert_eq!(p.current_preview_item_index(), 1);
        assert_eq!(p.current_item().unwrap().title, "g1");
        p.load_from(&Generated(0));
        assert!(p.current_item().is_none());
    }

    #[test]
    fn vec_is_a_data_source() {
        let v = vec![item("a"), item("b")];
        assert_eq!(v.number_of_items(), 2);
        assert_eq!(v.item_at(1).unwrap().title, "b");
        assert!(v.item_at(2).is_none());
    }

    #[test]
    fn display_state_starts_ready_and_resets_with_the_list() {
        let mut p = PreviewPanel::new();
        p.set_items(vec![item("a"), item("b")]);
        assert_eq!(p.display_state(0), Some(ItemState::Ready));
        assert!(p.set_display_state(1, ItemState::Failed));
        assert_eq!(p.display_state(1), Some(ItemState::Failed));
        assert_eq!(p.display_state(0), Some(ItemState::Ready));
        p.set_items(vec![item("a"), item("b")]);
        assert_eq!(p.display_state(1), Some(ItemState::Ready));
    }

    #[test]
    fn display_state_past_the_end_is_refused() {
        let mut p = PreviewPanel::new();
        p.set_items(vec![item("a")]);
        assert!(!p.set_display_state(3, ItemState::Loading));
        assert_eq!(p.display_state(3), None);
        p.load_from(&Generated(0));
        assert_eq!(p.display_state(0), None);
    }
}

#[cfg(test)]
mod preview_view_tests {
    use super::*;

    fn item(title: &str) -> PreviewItem {
        PreviewItem { url: format!("file:///tmp/{title}.txt"), title: title.to_string() }
    }

    #[test]
    fn new_view_has_no_item_and_default_switches() {
        let v = PreviewView::new(ViewStyle::default());
        assert_eq!(v.style(), ViewStyle::Normal);
        assert!(v.preview_item().is_none());
        assert_eq!(v.display_state(), ItemState::Ready);
        assert!(v.autostarts());
        assert!(v.closes_with_window());
    }

    #[test]
    fn autostarting_item_loads_until_the_app_reports_ready() {
        let mut v = PreviewView::new(ViewStyle::Compact);
        v.set_preview_item(Some(item("notes")));
        assert_eq!(v.display_state(), ItemState::Loading);
        assert_eq!(v.preview_item().map(|i| i.title.as_str()), Some("notes"));
        v.set_display_state(ItemState::Ready);
        assert_eq!(v.display_state(), ItemState::Ready);
    }

    #[test]
    fn without_autostart_the_item_waits_at_ready() {
        let mut v = PreviewView::new(ViewStyle::Normal);
        v.set_autostarts(false);
        v.set_preview_item(Some(item("photo")));
        assert_eq!(v.display_state(), ItemState::Ready);
    }

    #[test]
    fn refresh_needs_an_item_and_is_counted() {
        let mut v = PreviewView::new(ViewStyle::Normal);
        assert!(!v.refresh_preview_item());
        v.set_preview_item(Some(item("a")));
        assert!(v.refresh_preview_item());
        assert!(v.refresh_preview_item());
        assert_eq!(v.refresh_count(), 2);
    }

    #[test]
    fn close_returns_the_item_and_resets_state() {
        let mut v = PreviewView::new(ViewStyle::Normal);
        v.set_preview_item(Some(item("doc")));
        let gone = v.close().unwrap();
        assert_eq!(gone.title, "doc");
        assert!(v.preview_item().is_none());
        assert_eq!(v.display_state(), ItemState::Ready);
        assert!(v.close().is_none());
    }

    #[test]
    fn window_close_only_closes_when_the_switch_is_on() {
        let mut v = PreviewView::new(ViewStyle::Normal);
        v.set_closes_with_window(false);
        v.set_preview_item(Some(item("kept")));
        assert!(!v.window_closed());
        assert!(v.preview_item().is_some());
        v.set_closes_with_window(true);
        assert!(v.window_closed());
        assert!(v.preview_item().is_none());
    }
}
