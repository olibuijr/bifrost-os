//! The pure parts of Folder Actions Setup: reading the folder the person typed, naming a row, and the geometry that
//! the window draws and hit-tests with (points, no pixels).

use bifrost_app::folder_actions::{Action, Offer, Setup};

/// A rectangle in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Reads the folder path the person typed: leading and trailing spaces and trailing slashes go, a leading `~` is the
/// home folder, and the result must be absolute. The shell then checks that it is a folder.
pub fn clean_folder(input: &str, home: &str) -> Result<String, String> {
    let t = input.trim();
    if t.is_empty() {
        return Err("Type the path of a folder.".to_string());
    }
    if t.contains(['\t', '\n', '\r']) {
        return Err("The path has a tab or a line break in it.".to_string());
    }
    let full = if t == "~" || t.starts_with("~/") {
        if home.is_empty() {
            return Err("The home folder is not known here; type the full path.".to_string());
        }
        format!("{}{}", home.trim_end_matches('/'), &t[1..])
    } else {
        t.to_string()
    };
    if !full.starts_with('/') {
        return Err("The path must start with / or ~.".to_string());
    }
    let trimmed = full.trim_end_matches('/');
    Ok(if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() })
}

/// The offer an attached action runs, when its app is installed and still offers it.
pub fn offer_for<'a>(setup: &'a Setup, a: &Action) -> Option<&'a Offer> {
    setup.offers.iter().find(|o| o.app == a.app && o.intent == a.intent)
}

/// The two lines a row shows for an attached action: the folder, then `App: Action` (the ids when the app no longer
/// offers it).
pub fn action_lines(setup: &Setup, a: &Action) -> (String, String) {
    let second = match offer_for(setup, a) {
        Some(o) => format!("{}: {}", o.name, o.title),
        None => format!("{}: {} (not available)", a.app, a.intent),
    };
    (a.folder.clone(), second)
}

/// The two lines a row shows for an action that can be attached.
pub fn offer_lines(o: &Offer) -> (String, String) {
    (o.title.clone(), o.name.clone())
}

/// How many list rows fit in `height_pt`, never fewer than `min` nor more than `max`.
pub fn rows_that_fit(height_pt: f32, row_pt: f32, min: usize, max: usize) -> usize {
    ((height_pt / row_pt).floor().max(0.0) as usize).clamp(min, max)
}

/// The window's controls, in points, for a column starting at `col` that is `w` wide in a window `win_h` tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geo {
    /// Top of the first attached-action row, and the rows shown.
    pub attached_y: f32,
    pub attached_rows: usize,
    pub remove: Rect,
    pub field: Rect,
    /// Top of the first offered-action row, and the rows shown.
    pub offers_y: f32,
    pub offer_rows: usize,
    pub attach: Rect,
    pub status_y: f32,
    pub col: f32,
    pub w: f32,
}

pub const ROW_PT: f32 = 40.0;
pub const BUTTON_W: f32 = 110.0;
pub const BUTTON_H: f32 = 30.0;
pub const FIELD_H: f32 = 34.0;
/// Space under a section label before its first row.
const LABEL_GAP: f32 = 22.0;
const TOP: f32 = 100.0;
const ATTACHED_ROWS: usize = 4;

impl Geo {
    pub fn new(col: f32, w: f32, win_h: f32) -> Geo {
        let attached_y = TOP + LABEL_GAP;
        let attached_end = attached_y + ATTACHED_ROWS as f32 * ROW_PT;
        let remove = Rect { x: col + w - BUTTON_W, y: attached_end + 8.0, w: BUTTON_W, h: BUTTON_H };
        let add_top = remove.y + BUTTON_H + 24.0;
        let field = Rect { x: col, y: add_top + LABEL_GAP, w, h: FIELD_H };
        let offers_label = field.y + FIELD_H + 14.0;
        let offers_y = offers_label + LABEL_GAP;
        let offer_rows = rows_that_fit(win_h - offers_y - BUTTON_H - 64.0, ROW_PT, 2, 6);
        let attach = Rect { x: col + w - BUTTON_W, y: offers_y + offer_rows as f32 * ROW_PT + 8.0, w: BUTTON_W, h: BUTTON_H };
        Geo { attached_y, attached_rows: ATTACHED_ROWS, remove, field, offers_y, offer_rows, attach, status_y: attach.y + BUTTON_H + 12.0, col, w }
    }

    /// The row of an attached-action list at `(x, y)`, when it is one.
    pub fn attached_row_at(&self, x: f32, y: f32) -> Option<usize> {
        self.row_at(self.attached_y, self.attached_rows, x, y)
    }

    /// The row of the offered-action list at `(x, y)`, when it is one.
    pub fn offer_row_at(&self, x: f32, y: f32) -> Option<usize> {
        self.row_at(self.offers_y, self.offer_rows, x, y)
    }

    fn row_at(&self, top: f32, rows: usize, x: f32, y: f32) -> Option<usize> {
        if x < self.col || x >= self.col + self.w || y < top {
            return None;
        }
        let i = ((y - top) / ROW_PT) as usize;
        (i < rows).then_some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Setup {
        Setup {
            actions: vec![Action { folder: "/srv/in".into(), app: "a.notes".into(), intent: "import".into() }],
            offers: vec![Offer { app: "a.notes".into(), name: "Notes".into(), intent: "import".into(), title: "Import File".into() }],
        }
    }

    #[test]
    fn a_typed_folder_is_made_absolute_and_tidy() {
        assert_eq!(clean_folder("  /srv/in/  ", "/home/u"), Ok("/srv/in".to_string()));
        assert_eq!(clean_folder("~/Downloads", "/home/u/"), Ok("/home/u/Downloads".to_string()));
        assert_eq!(clean_folder("~", "/home/u"), Ok("/home/u".to_string()));
        assert_eq!(clean_folder("///", "/home/u"), Ok("/".to_string()));
    }

    #[test]
    fn bad_folders_are_explained() {
        assert!(clean_folder("   ", "/h").unwrap_err().contains("Type the path"));
        assert!(clean_folder("relative/dir", "/h").unwrap_err().contains("start with"));
        assert!(clean_folder("~/x", "").unwrap_err().contains("home folder"));
        assert!(clean_folder("/a\tb", "/h").unwrap_err().contains("tab"));
    }

    #[test]
    fn rows_name_the_app_and_the_action_or_the_ids_when_it_is_gone() {
        let s = setup();
        assert_eq!(action_lines(&s, &s.actions[0]), ("/srv/in".to_string(), "Notes: Import File".to_string()));
        let gone = Action { folder: "/x".into(), app: "a.old".into(), intent: "tag".into() };
        assert_eq!(action_lines(&s, &gone).1, "a.old: tag (not available)");
        assert_eq!(offer_lines(&s.offers[0]), ("Import File".to_string(), "Notes".to_string()));
    }

    #[test]
    fn the_row_count_follows_the_window_height_within_bounds() {
        assert_eq!(rows_that_fit(1000.0, 40.0, 2, 6), 6);
        assert_eq!(rows_that_fit(100.0, 40.0, 2, 6), 2);
        assert_eq!(rows_that_fit(-5.0, 40.0, 2, 6), 2);
        assert_eq!(rows_that_fit(130.0, 40.0, 2, 6), 3);
    }

    #[test]
    fn the_layout_stacks_the_sections_and_hit_tests_rows() {
        let g = Geo::new(32.0, 600.0, 780.0);
        assert!(g.remove.y > g.attached_y + 3.0 * ROW_PT, "Remove sits under the attached list");
        assert!(g.field.y > g.remove.y + BUTTON_H, "the path field is a section below");
        assert!(g.offers_y > g.field.y + FIELD_H);
        assert!(g.attach.y >= g.offers_y + g.offer_rows as f32 * ROW_PT);
        assert!(g.status_y > g.attach.y);
        assert_eq!(g.attached_row_at(40.0, g.attached_y + 1.0), Some(0));
        assert_eq!(g.attached_row_at(40.0, g.attached_y + ROW_PT * 2.5), Some(2));
        assert_eq!(g.attached_row_at(40.0, g.attached_y + ROW_PT * 4.5), None, "below the last row");
        assert_eq!(g.attached_row_at(10.0, g.attached_y + 1.0), None, "left of the column");
        assert_eq!(g.offer_row_at(40.0, g.offers_y + ROW_PT + 1.0), Some(1));
        assert!(g.attach.contains(g.attach.x + 1.0, g.attach.y + 1.0));
        assert!(!g.attach.contains(g.attach.x - 1.0, g.attach.y + 1.0));
    }

    #[test]
    fn a_small_window_keeps_two_offer_rows_and_a_tall_one_stops_at_six() {
        assert_eq!(Geo::new(16.0, 300.0, 300.0).offer_rows, 2);
        assert_eq!(Geo::new(16.0, 300.0, 4000.0).offer_rows, 6);
    }
}
