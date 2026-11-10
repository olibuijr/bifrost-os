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
}
