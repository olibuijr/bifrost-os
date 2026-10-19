//! Icon view: a grid of icons with labels and tag dots.
use crate::*;

/// Icon view: the default icon's side, points. Golden Gate's Files draws a folder 3.7 sidebar rows wide in its Home
/// window (a 32 pt row, so the 0.92 of the box the folder fills is about 118 pt); the cell pitch comes from `grid_w`
/// and `grid_h`, which follow the icon size. View Options > Icon size steps it between `GRID_ICON_MIN` and `GRID_ICON_MAX`.
pub(crate) const GRID_ICON: f32 = 128.;

pub(crate) const GRID_ICON_MIN: f32 = 32.;

pub(crate) const GRID_ICON_MAX: f32 = 160.;

/// A cell is wider than its icon by this much (the gutter between icons), and taller by `GRID_PAD_H` (the name's room and
/// the space to the next line): the reference's 166 x 192 pt cell around a 128 pt icon.
pub(crate) const GRID_PAD_W: f32 = 40.;

pub(crate) const GRID_PAD_H: f32 = 60.;

/// From the icon's bottom edge to the top of the name's plate.
pub(crate) const GRID_LABEL_GAP: f32 = 10.;

/// The Files emblem of a special folder in the home folder.
pub(crate) fn folder_emblem(home: &Path, path: &Path) -> Option<&'static str> {
    if path.parent() != Some(home) {
        return None;
    }
    Some(match path.file_name()?.to_str()? {
        "Desktop" => "menubar.dock.rectangle",
        "Documents" => "doc",
        "Downloads" => "arrow.down.circle",
        "Pictures" => "photo",
        "Music" => "music.note",
        "Videos" | "Movies" => "film",
        _ => return None,
    })
}

impl Files {
    /// One cell of the icon view: icon, tag dots and name (Files's icon view).
    pub(crate) fn paint_icon_cell(&self, ui: &Ui, s: &mut Surface, n: usize, i: usize, cols: usize, bottom: f32) {
        let e = &self.rows[i];
        let name_field = self.naming.as_ref().filter(|(p, _)| *p == e.path).map(|(_, f)| f);
        let selected = self.selection.contains(&i);
        let x = (self.side() + 16.) + (n % cols) as f32 * self.grid_w();
        let y = self.rows_top() + 16. + (n / cols) as f32 * self.grid_h() - self.scroll;
        let cell_h = self.grid_h() - 6.;
        if y < self.body_top() + 4. || y + cell_h > bottom || ui.pt(y+cell_h) < s.y0 as f32 || ui.pt(y-6.) >= s.h as f32 {
            return;
        }
        let icon_x = x + (self.grid_w() - self.grid_icon()) / 2.;
        let label_y = y + self.grid_icon() + GRID_LABEL_GAP;
        if selected {
            // Files: a grey plate behind the icon and an accent capsule behind the name
            s.fill_rrect(ui.pt(icon_x - 4.), ui.pt(y - 4.), ui.pt(self.grid_icon() + 8.), ui.pt(self.grid_icon() + 8.), ui.pt(SPACE_S), CARD_BG.now());
            let lw = (ui.measure(&e.name, self.vo.text, Weight::Regular) / ui.pt(1.) + 10.).min(self.grid_w() - 4.);
            s.fill_rrect(ui.pt(x + (self.grid_w() - lw) / 2.), ui.pt(label_y - 1.), ui.pt(lw), ui.pt(self.vo.text + 6.), ui.pt(SPACE_XS), SELECTION.now());
        }
        self.paint_entry_icon(ui, s, e, icon_x, y, self.grid_icon(), true);
        tag_dots(s, ui, &self.tag_index, &e.path, x + 4., y + 2.);
        self.paint_badges(ui, s, &e.path, icon_x + self.grid_icon() - 2., y + self.grid_icon() - 12.);
        if let Some(field) = name_field {
            bifrost_app::widgets::text_field(s, ui, field, Rect { x: x + 2., y: label_y - 2., w: self.grid_w() - 4., h: 22. }, "", true);
        } else {
            let (colour, max, pt) = (ink(selected), ui.pt(self.grid_w() - 8.), self.vo.text);
            let key = (e.path.clone(), e.mtime, colour, ui.pt(pt).to_bits(), max as u32);
            let im = self.labels.borrow_mut().get_or_insert(key, || {
                let label = ui.fit(&e.name, pt, max);
                std::rc::Rc::new(ui.text(&label, pt, Weight::Regular, colour))
            });
            s.blit(&im, ui.pt(x + self.grid_w() / 2.) as i32 - im.w as i32 / 2, ui.pt(label_y + 2.) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fixture;

    #[test]
    fn the_default_icon_is_as_wide_as_the_reference_next_to_a_sidebar_row() {
        let files = fixture(vec![]);
        // the folder fills 0.92 of its box; the reference draws it 3.7 sidebar rows wide
        let ratio = files.grid_icon() * 0.92 / files.sidebar().row_h;
        assert!((3.4..=4.0).contains(&ratio), "folder width over row pitch: {ratio}");
        assert_eq!(files.grid_w(), files.grid_icon() + GRID_PAD_W);
        assert_eq!(files.grid_h(), files.grid_icon() + GRID_PAD_H);
        // the name's plate fits in the cell under the icon
        assert!(GRID_LABEL_GAP + files.vo.text + 6. <= GRID_PAD_H);
    }
}
