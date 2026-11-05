//! Get Info in a window of its own, as in the reference (one small window per item, beside the browser window). The
//! window is an extra SDK window (`Ctx::open_window`) holding a `Files` in Get Info mode (`info_window`): it draws only
//! the info rows and the permissions grid, filling the window, and a click on the grid changes the mode as the panel
//! did. When the shell refuses the window the in-window panel stays up instead.
use crate::*;
use bifrost_app::app_windows::WindowSpec;

/// The Get Info window's width, points.
pub(crate) const INFO_WINDOW_W: f32 = 340.;
/// The Get Info window's height bounds, points.
const INFO_WINDOW_MIN_H: f32 = 240.;
const INFO_WINDOW_MAX_H: f32 = 760.;

/// The window title: the item's name and "Info", as the reference titles it.
pub(crate) fn info_title(name: &str) -> String {
    format!("{name} Info")
}

/// The window height that fits the rows, the permissions grid when the item has a mode, and the hint line.
pub(crate) fn info_window_h(rows: usize, has_mode: bool) -> f32 {
    let grid = if has_mode { perms::GRID_H + 14. } else { 0. };
    (8. + 56. + rows as f32 * 26. + 12. + grid + 40.).clamp(INFO_WINDOW_MIN_H, INFO_WINDOW_MAX_H)
}

impl Files {
    /// Top-left of the Get Info content: the window's own corner in a Get Info window, else the panel's place.
    pub(crate) fn info_origin(&self) -> (f32, f32) {
        if self.info_window {
            (0., 8.)
        } else {
            (self.side() + 16., 72.)
        }
    }

    /// Get Info asked for a window: open it with the panel's content and close the panel. Without a window (the shell
    /// refused it) the panel stays up.
    pub(crate) fn open_info_window(&mut self, ctx: &Ctx) {
        if !std::mem::take(&mut self.info_wanted) {
            return;
        }
        let Some(info) = self.info.as_ref() else { return };
        let (w, h) = (INFO_WINDOW_W, info_window_h(info.rows.len(), info.mode.is_some()));
        let Ok(id) = ctx.open_window(WindowSpec::new(&info_title(&info.name), w, h)) else { return };
        let Some(info) = self.info.take() else { return };
        self.windows.push(windows::ExtraWindow::new(id, Box::new(Files::info_view(info, w, h, &ctx.data_dir))));
        self.revision += 1;
    }

    /// A `Files` that shows one item's Get Info, filling a window `w` x `h` points.
    pub(crate) fn info_view(info: Info, w: f32, h: f32, data_dir: &Path) -> Files {
        let mut files = Files::new();
        files.data_dir = Some(data_dir.to_path_buf());
        files.info_window = true;
        files.sidebar_hidden = true;
        files.info = Some(info);
        files.w = w;
        files.h = h;
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(rows: usize) -> Info {
        Info { name: "Report.pdf".into(), rows: (0..rows).map(|i| ("Row", i.to_string())).collect(), path: PathBuf::from("/tmp/Report.pdf"), mode: Some(0o644), editable: true }
    }

    #[test]
    fn the_window_is_titled_after_the_item_and_fits_its_rows() {
        assert_eq!(info_title("Report.pdf"), "Report.pdf Info");
        let small = info_window_h(2, false);
        assert_eq!(small, INFO_WINDOW_MIN_H, "a short list still gets a usable window");
        assert!(info_window_h(12, true) > info_window_h(12, false), "the permissions grid adds height");
        assert_eq!(info_window_h(200, true), INFO_WINDOW_MAX_H, "a long list scrolls no further than the cap");
    }

    #[test]
    fn an_info_view_shows_only_its_item_from_the_window_corner() {
        let v = Files::info_view(info(3), INFO_WINDOW_W, 400., Path::new("/tmp"));
        assert!(v.info_window && v.sidebar_hidden);
        assert_eq!(v.info_origin(), (0., 8.));
        assert_eq!(v.info.as_ref().map(|i| i.name.as_str()), Some("Report.pdf"));
        let main = Files::new();
        assert_eq!(main.info_origin(), (main.side() + 16., 72.), "the panel keeps its place in the browser window");
    }

    fn test_ctx() -> Ctx {
        use bifrost_app::kit::{Font, Ui};
        let font = Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let cfg = bifrost_app::proto::Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(font, 1.0), cfg)
    }

    /// Get Info from the menu bar opens the window, and the window's draw paints the whole body, also into the fresh
    /// empty buffers the SDK makes when the desk resizes the window after it opened (a transparent body before).
    #[test]
    fn the_info_window_paints_its_body_and_again_on_a_fresh_buffer_of_another_size() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join(format!("files-infowin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        let mut files = crate::tests::fixture(fs::list(&dir).unwrap());
        files.select_only(0);
        files.menu_action(&ctx, menus::MENU_INFO);
        assert!(files.info.is_none(), "the panel gives way to the window");
        assert_eq!(files.windows.len(), 1, "Get Info opened its window");
        let id = files.windows[0].id;
        let mut px = vec![0u32; 340 * 300];
        let d = files.draw_window(&ctx, id, &mut Surface { w: 340, h: 300, stride: 340, px: &mut px, y0: 0 }, 0.0);
        assert!(!d.is_empty());
        assert!(px.iter().all(|&p| p != 0), "the first draw fills the window with its background and content");
        // nothing changed and the buffer still holds the frame: nothing to repaint
        let d = files.draw_window(&ctx, id, &mut Surface { w: 340, h: 300, stride: 340, px: &mut px, y0: 0 }, 0.0);
        assert!(d.is_empty());
        // the desk resized the window: the SDK draws into new, empty buffers, and Files paints them in full
        let mut fresh = vec![0u32; 300 * 280];
        let d = files.draw_window(&ctx, id, &mut Surface { w: 300, h: 280, stride: 300, px: &mut fresh, y0: 0 }, 0.0);
        assert!(!d.is_empty());
        assert!(fresh.iter().all(|&p| p != 0), "a resized window is painted again, not left transparent");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
