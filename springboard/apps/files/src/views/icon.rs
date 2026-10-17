//! Icon view: a grid of icons with labels and tag dots.
use crate::*;

/// Icon view: the default icon's side (the cell pitch comes from `grid_w` and `grid_h`, which follow the icon size).
pub(crate) const GRID_ICON: f32 = 64.;

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
        let label_y = y + self.grid_icon() + 6.;
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
