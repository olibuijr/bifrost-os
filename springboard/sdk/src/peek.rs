//! Preview (`QLPreviewPanel`, `QLPreviewItem`, `QLThumbnailGenerator`).
//!
//! [`PreviewPanel`] is the data source: an ordered list of [`PreviewItem`]s and a current index that is always clamped.
//! [`PreviewPanel::make_key_and_order_front`] sends `ToShell::Preview` (the shell draws the floating panel) and
//! [`PreviewPanel::toggle`] is the Space-bar behaviour. Thumbnails: [`thumbnail_path`] names the cached PNG for a file and
//! size, and [`ThumbnailCache`] remembers which were generated (an `NativeCache`, see `crate::cache`).

pub use crate::proto::peek::{PreviewItem, MAX_ITEMS};
/// Data-based previews and thumbnail requests (`QLPreviewProvider`, `QLPreviewReply`, `QLThumbnailGenerator.Request`):
/// the pure models in `bifrost_ui::preview`, re-exported for apps. See [`Registry`] and [`ThumbnailRequest`].
pub use bifrost_ui::preview::{Attachment, Content, ModelError, Registry, Reply, Representation, ThumbnailRequest};
use crate::proto::ToShell;
use crate::Ctx;

#[derive(Clone, Debug, Default)]
pub struct PreviewPanel {
    items: Vec<PreviewItem>,
    index: usize,
    visible: bool,
}

impl PreviewPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// `dataSource` items, in order; the current index is clamped into the new list.
    pub fn set_items(&mut self, items: Vec<PreviewItem>) {
        self.items = items;
        self.index = self.index.min(self.items.len().saturating_sub(1));
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
}
