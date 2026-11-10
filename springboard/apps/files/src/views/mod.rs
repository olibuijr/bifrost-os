//! The content views (icon, list, column, gallery) and the drawing helpers they share.
use crate::*;

pub(crate) mod column;
pub(crate) mod gallery;
pub(crate) mod icon;
pub(crate) mod list;
pub(crate) mod options;
pub(crate) mod scrolling;

/// What a row says its kind is, in every view: the description of the type its name maps to ("TIFF image",
/// "HTML Document", "Disk Image"), "Folder", or the coarse kind when the name has no known type. The coarse kind alone
/// says what the row previews as, so it calls a picture the viewer does not decode a document.
pub(crate) fn kind_text(e: &Entry) -> &'static str {
    fs::kind_description(&e.name, e.is_dir, e.kind)
}

pub(crate) fn text(s: &mut Surface, ui: &Ui, label: &str, pt: f32, color: u32, x: f32, y: f32, max: f32) {
    let label = ui.fit(label, pt, ui.pt(max.max(0.)));
    let im = ui.text(&label, pt, Weight::Regular, color);
    s.blit(&im, ui.pt(x) as i32, ui.pt(y) as i32, 255, 255);
}

/// `text` through a shared rendered-text cache: a string drawn again (a row repainted on scroll) is not rasterised again.
pub(crate) fn cached_text(s: &mut Surface, ui: &Ui, cache: &std::cell::RefCell<bifrost_app::kit::TextCache>, label: &str, pt: f32, color: u32, x: f32, y: f32, max: f32) {
    let label = ui.fit(label, pt, ui.pt(max.max(0.)));
    let mut cache = cache.borrow_mut();
    let im = cache.get(ui, &label, pt, Weight::Regular, color);
    s.blit(im, ui.pt(x) as i32, ui.pt(y) as i32, 255, 255);
}

pub(crate) fn rect(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32, color: u32) {
    s.fill_rrect(
        ui.pt(x),
        ui.pt(y),
        ui.pt(w.max(0.)),
        ui.pt(h.max(0.)),
        0.,
        color,
    );
}

/// Text on a selected row is white on the blue selection in both schemes; otherwise the scheme's label colour.
pub(crate) fn ink(selected: bool) -> u32 {
    if selected { WHITE } else { LABEL.now() }
}

impl Files {
    pub(crate) fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.vo.gallery {
            return self.gallery_hit(x, y);
        }
        if self.columns {
            // only the open folder's column takes item clicks; the folders above are handled by ancestor_hit
            if x < (self.side() + 16.) || y < self.rows_top() || y > self.bottom() {
                return None;
            }
            let col = ((x - (self.side() + 16.)) / self.vo.col_w).floor() as usize;
            if col != self.ancestor_shown() {
                return None;
            }
            let row = ((y - self.rows_top() + self.scroll) / ROW_H).floor() as usize;
            return self.visible().get(row).copied();
        }
        if x < self.side() || y < self.rows_top() || y > self.bottom() {
            return None;
        }
        let visible = self.visible();
        let n = if self.grid {
            let cols = ((self.w - (self.side() + 16.)) / self.grid_w()).floor().max(1.) as usize;
            let col = ((x - (self.side() + 16.)) / self.grid_w()).floor() as usize;
            if col >= cols {
                return None;
            }
            (((y - self.rows_top() - 16. + self.scroll) / self.grid_h()).floor().max(0.) as usize) * cols + col
        } else {
            ((y - self.rows_top() + self.scroll) / 32.).floor().max(0.) as usize
        };
        visible.get(n).copied()
    }
}

#[cfg(test)]
mod kind_tests {
    use super::*;

    /// A row as the listing makes it: the coarse kind is the one the name gives.
    fn row(name: &str) -> Entry {
        Entry { name: name.into(), path: Path::new("/fixture").join(name), is_dir: false, size: 1, mtime: 0, kind: fs::kind_of(name, false), items: None }
    }

    #[test]
    fn the_kind_text_names_the_type_and_not_only_what_the_row_previews_as() {
        for (name, kind) in [
            ("a.png", "PNG image"),
            ("a.jpg", "JPEG image"),
            ("a.bmp", "BMP image"),
            ("a.gif", "GIF image"),
            ("a.ico", "Icon image"),
            ("a.tiff", "TIFF image"),
            ("a.svg", "SVG image"),
            ("a.html", "HTML Document"),
            ("a.img", "Disk Image"),
            ("a.iso", "Disk Image"),
            ("a.txt", "Plain Text"),
        ] {
            assert_eq!(kind_text(&row(name)), kind, "{name}");
        }
    }

    #[test]
    fn a_name_of_no_known_type_keeps_the_coarse_kind_and_a_folder_is_a_folder() {
        assert_eq!(kind_text(&row("blob")), "Document");
        let mut sniffed = row("Makefile");
        sniffed.kind = Kind::Text;
        assert_eq!(kind_text(&sniffed), "Plain Text");
        let mut folder = row("pics.png");
        folder.is_dir = true;
        folder.kind = Kind::Folder;
        assert_eq!(kind_text(&folder), "Folder");
    }
}
