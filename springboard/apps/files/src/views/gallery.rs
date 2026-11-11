//! Gallery view (Files's fourth view): a large preview of the focused item over a filmstrip of the rest.
//! The mode itself is `ViewOpts::gallery` (set by `set_gallery` in `options.rs`), and it is painted by
//! `paint_gallery` and hit-tested by `gallery_hit`, both in `options.rs`. This module holds no code yet.

#[cfg(test)]
mod tests {
    use crate::*;

    /// Show in Files in the gallery: the view has no scroll of its own, so the item is "scrolled to" by the strip,
    /// which is laid out around the focused item.
    #[test]
    fn the_strip_shows_the_item_a_request_selected() {
        let mut files = crate::tests::fixture((0..100).map(|i| crate::tests::folder(&format!("f{i:03}"))).collect());
        files.set_gallery();
        let path = files.rows[77].path.clone();
        files.fx.pending = Some(Pending { path, rename: false });
        files.apply_pending();
        assert_eq!(files.selection, vec![77]);
        let visible = files.visible();
        let (_, cols, first) = files.strip_geom(&visible);
        assert!(cols < 100, "the strip does not hold the whole folder");
        assert!((first..first + cols).contains(&77), "the selected item is one of the thumbnails shown: {first}..{}", first + cols);
        // and a click on its thumbnail picks it
        let x = files.side() + 16. + ((77 - first) as f32 + 0.5) * STRIP_CELL;
        assert_eq!(files.gallery_hit(x, files.bottom() - 10.), Some(77));
    }

    /// E2E8 R08c: Select All and then the gallery showed the last item with the strip scrolled to its end. With
    /// everything selected every view starts at the first item; with one focused item the view shows that item.
    #[test]
    fn after_select_all_the_gallery_starts_at_the_first_item() {
        let mut files = crate::tests::fixture((0..100).map(|i| crate::tests::folder(&format!("f{i:03}"))).collect());
        files.select_all();
        files.set_gallery();
        assert_eq!(files.selection.len(), 100, "the selection is kept");
        assert_eq!(files.focus(), Some(99), "the focus is the last item picked");
        let visible = files.visible();
        assert_eq!(files.gallery_current(&visible), Some(0), "the first item is shown large");
        let (_, cols, first) = files.strip_geom(&visible);
        assert!(cols < 100, "the strip does not hold the whole folder");
        assert_eq!(first, 0, "and the strip starts at the first item");
        // a click on the large preview keeps the item that is shown
        assert_eq!(files.gallery_hit(files.side() + 200., files.body_top() + 50.), Some(0));
        // one focused item: the gallery shows it, and the strip holds it
        files.select_only(60);
        assert_eq!(files.gallery_current(&visible), Some(60));
        let (_, cols, first) = files.strip_geom(&visible);
        assert!((first..first + cols).contains(&60), "{first}..{}", first + cols);
        // several of many: the focus, the last one picked
        files.selection = vec![40, 3];
        assert_eq!(files.gallery_current(&visible), Some(3));
        // nothing selected: the first item
        files.selection.clear();
        assert_eq!(files.gallery_current(&visible), Some(0));
        assert_eq!(files.gallery_current(&[]), None, "an empty folder shows nothing");
    }
}
