//! Fonts as an installed BifrOSt app in the full-size window: a sidebar of lists (All, Off, collections, New
//! Collection), the family list with search, and a preview with sample text, the repertoire, Info, the on/off switch,
//! Remove Font for the user's own installs, and collection membership. Installing a font opens it here (Open With on a
//! .ttf or .otf file) and a file that fails validation is refused. The model is in `model.rs`; the font checks are in
//! `validate.rs`.

mod model;
mod validate;

use bifrost_app::kit::ui::{self, Ui, Weight};
use bifrost_app::kit::{Damage, Font, Surface};
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use bifrost_render::fontprefs;
use model::{Library, Source};
use std::path::{Path, PathBuf};

/// The shell's seeded asset folder: its font library is `<dir>/fonts`, read by Settings > Fonts.
const SHELL_ASSETS: &str = "/var/lib/springboard/assets";
/// Sidebar width, list column width and the header height, in points.
const SIDEBAR_PT: f32 = 200.0;
const LIST_PT: f32 = 260.0;
const ROW_PT: f32 = 28.0;
const HEADER_PT: f32 = 56.0;
const PAD_PT: f32 = 12.0;
/// Width of the preview's buttons (On/Off, Remove Font), and the gap from the switch to the remove button.
const BUTTON_PT: f32 = 120.0;
const BUTTON_GAP_PT: f32 = 130.0;
/// The Info rows start here in the preview column, and the collection membership block below them.
const INFO_TOP_PT: f32 = HEADER_PT + 260.0;
const MEMBERSHIP_TOP_PT: f32 = HEADER_PT + 450.0;
const MEMBERSHIP_ROW_PT: f32 = 22.0;
/// Sizes shown in the preview, in points.
const PREVIEW_SIZES: [f32; 4] = [14.0, 22.0, 32.0, 48.0];

struct FontBookApp {
    library: Option<Library>,
    user_dir: PathBuf,
    source: Source,
    query: String,
    selected: Option<String>,
    /// the sample font for `selected`, drawn at the current scale
    preview: Option<(String, Ui)>,
    /// the validation and repertoire of the selected font's file, with the family it belongs to
    report: Option<(String, validate::Report)>,
    status: String,
}

impl FontBookApp {
    /// Reads the families from disk and the user's settings from the data dir.
    fn reload(&mut self, data_dir: &Path) {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        self.user_dir = home.join(model::USER_FONTS_DIR);
        let families = fontprefs::installed(&model::scan_dirs(&home, &fontprefs::library(SHELL_ASSETS)));
        let disabled = model::parse_disabled(&model::read_text(data_dir, model::DISABLED_FILE));
        let collections = model::parse_collections(&model::read_text(data_dir, model::COLLECTIONS_FILE));
        self.library = Some(Library::new(families, disabled, collections));
    }

    /// Saves the disabled list and the collections; a failed write is shown in the status line.
    fn save(&mut self, data_dir: &Path) {
        let Some(lib) = &self.library else { return };
        let a = model::write_text(data_dir, model::DISABLED_FILE, &lib.disabled_file());
        let b = model::write_text(data_dir, model::COLLECTIONS_FILE, &lib.collections_file());
        if let Err(e) = a.and(b) {
            self.status = format!("Could not save: {e}");
        }
    }

    /// The families the list shows now.
    fn shown(&self) -> Vec<String> {
        self.library.as_ref().map(|l| l.visible(&self.source, &self.query).iter().map(|f| f.family.clone()).collect()).unwrap_or_default()
    }

    /// Loads the preview font and its validation for the selected family, once per selection.
    fn ensure_preview(&mut self, scale: f32) {
        let Some(name) = self.selected.clone() else {
            self.preview = None;
            self.report = None;
            return;
        };
        if self.preview.as_ref().is_some_and(|(n, _)| *n == name) {
            return;
        }
        let path = self.library.as_ref().and_then(|l| l.find(&name)).map(|f| f.path.clone());
        let bytes = path.and_then(|p| std::fs::read(&p).ok());
        self.report = bytes.as_deref().map(|b| (name.clone(), validate::check(b)));
        self.preview = bytes.and_then(|bytes| {
            // the bytes live for the whole run: one preview font per selection, so the leak is bounded by what was viewed
            let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            Font::parse(data).map(|font| (name.clone(), Ui::new(font, scale)))
        });
    }

    /// True when the family's file is one this user installed (the only fonts Remove Font may delete).
    fn is_user_font(&self, name: &str) -> bool {
        self.library.as_ref().and_then(|l| l.find(name)).is_some_and(|f| Path::new(&f.path).starts_with(&self.user_dir))
    }

    fn install(&mut self, ctx: &Ctx, path: &str) {
        match model::install(Path::new(path), &self.user_dir) {
            Ok((_, family)) => {
                self.status = format!("Installed {family}.");
                self.reload(&ctx.data_dir);
                self.selected = Some(family);
            }
            Err(e) => self.status = e,
        }
    }

    /// Remove Font: deletes the selected family's file if the user installed it, then rereads the list.
    fn remove(&mut self, data_dir: &Path, name: &str) {
        let Some(path) = self.library.as_ref().and_then(|l| l.find(name)).map(|f| PathBuf::from(&f.path)) else { return };
        match model::uninstall(&path, &self.user_dir) {
            Ok(()) => {
                self.status = format!("Removed {name}.");
                self.selected = None;
                self.reload(data_dir);
            }
            Err(e) => self.status = e,
        }
    }

    /// Adds the family to the collection, or takes it out when it is already a member, then saves.
    fn toggle_membership(&mut self, data_dir: &Path, collection: &str, family: &str) {
        let Some(lib) = &mut self.library else { return };
        let member = lib.members(collection).is_some_and(|m| m.iter().any(|x| x.eq_ignore_ascii_case(family)));
        if member {
            lib.remove_from_collection(collection, family);
        } else {
            lib.add_to_collection(collection, family);
        }
        self.save(data_dir);
    }

    /// New Collection: adds an empty collection with a free name and shows it in the list.
    fn new_collection(&mut self, data_dir: &Path) {
        let Some(lib) = &mut self.library else { return };
        let taken = lib.collection_names();
        let name = (1..)
            .map(|n: u32| if n == 1 { "New Collection".to_string() } else { format!("New Collection {n}") })
            .find(|c| !taken.iter().any(|k| k.eq_ignore_ascii_case(c)))
            .unwrap_or_default();
        if lib.create_collection(&name) {
            self.source = Source::Collection(name);
            self.save(data_dir);
        }
    }

    /// A press in the sidebar, the list or the preview column (window points, scaled by the caller).
    fn touch_down(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        let Some(lib) = &self.library else { return false };
        if x < SIDEBAR_PT {
            if let Some(source) = sidebar_row(lib, y) {
                self.source = source;
                self.query.clear();
                return true;
            }
            if new_collection_row(lib, y) {
                self.new_collection(&ctx.data_dir);
                return true;
            }
            return false;
        }
        if x < SIDEBAR_PT + LIST_PT {
            let index = ((y - HEADER_PT) / ROW_PT).floor();
            if y >= HEADER_PT && index >= 0.0 {
                if let Some(name) = self.shown().get(index as usize).cloned() {
                    self.selected = Some(name);
                    return true;
                }
            }
            return false;
        }
        let Some(name) = self.selected.clone() else { return false };
        let switch_x = SIDEBAR_PT + LIST_PT + PAD_PT;
        let remove_x = switch_x + BUTTON_GAP_PT;
        // the on/off switch and Remove Font sit at the top of the preview column
        if (PAD_PT..PAD_PT + 28.0).contains(&y) {
            if (switch_x..switch_x + BUTTON_PT).contains(&x) {
                if let Some(lib) = &mut self.library {
                    let on = lib.is_enabled(&name);
                    lib.set_enabled(&name, !on);
                }
                self.save(&ctx.data_dir);
                return true;
            }
            if (remove_x..remove_x + BUTTON_PT).contains(&x) && self.is_user_font(&name) {
                self.remove(&ctx.data_dir, &name);
                return true;
            }
        }
        // one row per collection: a press adds the family to it or takes it out
        let rows_top = MEMBERSHIP_TOP_PT + MEMBERSHIP_ROW_PT;
        if (switch_x..switch_x + 260.0).contains(&x) && y >= rows_top {
            let index = ((y - rows_top) / MEMBERSHIP_ROW_PT).floor() as usize;
            let collection = self.library.as_ref().and_then(|l| l.collection_names().get(index).cloned());
            if let Some(collection) = collection {
                self.toggle_membership(&ctx.data_dir, &collection, &name);
                return true;
            }
        }
        false
    }
}

/// Which sidebar row sits at point `y` (rows start under the header), if any.
fn sidebar_row(lib: &Library, y: f32) -> Option<Source> {
    let index = ((y - HEADER_PT) / ROW_PT).floor();
    if y < HEADER_PT || index < 0.0 {
        return None;
    }
    let mut rows = vec![Source::All, Source::Disabled];
    rows.extend(lib.collection_names().into_iter().map(Source::Collection));
    rows.get(index as usize).cloned()
}

/// True when `y` is on the New Collection row, which sits under the last list row.
fn new_collection_row(lib: &Library, y: f32) -> bool {
    let index = ((y - HEADER_PT) / ROW_PT).floor();
    y >= HEADER_PT && index >= 0.0 && index as usize == 2 + lib.collection_names().len()
}

impl App for FontBookApp {
    fn id(&self) -> &str {
        "is.olibuijr.fontbook.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // all changes are saved as they are made
        if self.library.is_none() {
            self.reload(&ctx.data_dir);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                let (x, y) = (x / ctx.ui.s, y / ctx.ui.s);
                self.touch_down(ctx, x, y)
            }
            Input::Key(KeyInput::Text(text)) => {
                self.query.push_str(&text);
                self.selected = None;
                true
            }
            Input::Key(KeyInput::Code(8)) => {
                self.query.pop();
                true
            }
            Input::Key(KeyInput::Code(27)) => {
                self.query.clear();
                true
            }
            Input::Open(path) => {
                self.install(ctx, &path);
                true
            }
            _ => false,
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui = &ctx.ui;
        let scale = ui.s;
        self.ensure_preview(scale);
        let p = |v: f32| (v * scale) as usize;
        // positions for blit_colored, which takes signed pixels
        let pi = |v: f32| (v * scale) as i32;
        let (w, h) = (s.w, s.h);
        s.fill_rect(0, 0, w, h, ui::DETAIL_BG.now());
        // sidebar
        s.fill_rect(0, 0, p(SIDEBAR_PT), h, ui::SIDEBAR_BG.now());
        let mut rows = vec![("All Fonts".to_string(), Source::All), ("Off".to_string(), Source::Disabled)];
        if let Some(lib) = &self.library {
            rows.extend(lib.collection_names().into_iter().map(|n| (n.clone(), Source::Collection(n))));
        }
        for (i, (label, source)) in rows.iter().enumerate() {
            let y = HEADER_PT + i as f32 * ROW_PT;
            if *source == self.source {
                s.fill_rect(p(6.0), p(y), p(SIDEBAR_PT - 12.0), p(ROW_PT), ui::SIDEBAR_SELECTED);
            }
            let img = ui.text(&ui.fit(label, ui::MAC_BODY_PT, ui.pt(SIDEBAR_PT - 2.0 * PAD_PT)), ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
            s.blit_colored(&img, pi(PAD_PT), pi(y + 6.0), ui::LABEL.now(), 255);
        }
        let new_y = HEADER_PT + rows.len() as f32 * ROW_PT;
        let img = ui.text("New Collection", ui::MAC_BODY_PT, Weight::Regular, ui::LINK);
        s.blit_colored(&img, pi(PAD_PT), pi(new_y + 6.0), ui::LINK, 255);
        // list column
        let x0 = SIDEBAR_PT;
        s.fill_rect(p(x0 + LIST_PT - 1.0), 0, 1, h, ui::SEPARATOR);
        let shown = self.shown();
        let total = shown.len();
        let header = ui.text(&format!("{total} families"), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
        s.blit_colored(&header, pi(x0 + PAD_PT), pi(HEADER_PT - 20.0), ui::LABEL_2.now(), 255);
        for (i, name) in shown.iter().enumerate() {
            let y = HEADER_PT + i as f32 * ROW_PT;
            if y > h as f32 / scale {
                break;
            }
            let on = self.library.as_ref().is_none_or(|l| l.is_enabled(name));
            if self.selected.as_deref() == Some(name.as_str()) {
                s.fill_rect(p(x0 + 6.0), p(y), p(LIST_PT - 12.0), p(ROW_PT), ui::SIDEBAR_SELECTED);
            }
            let colour = if on { ui::LABEL.now() } else { ui::LABEL_2.now() };
            let img = ui.text(&ui.fit(name, ui::MAC_BODY_PT, ui.pt(LIST_PT - 2.0 * PAD_PT)), ui::MAC_BODY_PT, Weight::Regular, colour);
            s.blit_colored(&img, pi(x0 + PAD_PT), pi(y + 6.0), colour, 255);
        }
        // preview column
        let px = SIDEBAR_PT + LIST_PT + PAD_PT;
        match (&self.selected, &self.preview) {
            (Some(name), preview) => {
                let on = self.library.as_ref().is_none_or(|l| l.is_enabled(name));
                s.fill_rect(p(px), p(PAD_PT), p(BUTTON_PT), p(28.0), if on { ui::LINK } else { ui::SEPARATOR });
                let label = if on { "Turn Off" } else { "Turn On" };
                let img = ui.text(label, ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&img, pi(px + 12.0), pi(PAD_PT + 6.0), ui::LABEL.now(), 255);
                if self.is_user_font(name) {
                    let rx = px + BUTTON_GAP_PT;
                    s.fill_rect(p(rx), p(PAD_PT), p(BUTTON_PT), p(28.0), ui::DESTRUCTIVE);
                    let img = ui.text("Remove Font", ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
                    s.blit_colored(&img, pi(rx + 12.0), pi(PAD_PT + 6.0), ui::LABEL.now(), 255);
                }
                let title = ui.text(&ui.fit(name, ui::MAC_TITLE2_PT, ui.pt(w as f32 / scale - px - PAD_PT)), ui::MAC_TITLE2_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&title, pi(px), pi(HEADER_PT + PAD_PT), ui::LABEL.now(), 255);
                let mut y = HEADER_PT + 56.0;
                if let Some((_, font_ui)) = preview {
                    for size in PREVIEW_SIZES {
                        let img = font_ui.text(model::PREVIEW_TEXT, size, Weight::Regular, ui::LABEL.now());
                        s.blit_colored(&img, pi(px), pi(y), ui::LABEL.now(), 255);
                        y += size * 1.6;
                    }
                }
                if let Some(lib) = &self.library {
                    y = y.max(INFO_TOP_PT);
                    let mut info = lib.info(name);
                    if let Some((n, r)) = &self.report {
                        if n == name {
                            info.extend(model::repertoire_rows(r));
                        }
                    }
                    for (k, v) in info {
                        let line = format!("{k}: {v}");
                        let img = ui.text(&ui.fit(&line, ui::MAC_FOOTNOTE_PT, ui.pt(w as f32 / scale - px - PAD_PT)), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
                        s.blit_colored(&img, pi(px), pi(y), ui::LABEL_2.now(), 255);
                        y += 18.0;
                    }
                    // collection membership: one row per collection, the action named on the row
                    let head = ui.text("Collections", ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
                    s.blit_colored(&head, pi(px), pi(MEMBERSHIP_TOP_PT), ui::LABEL_2.now(), 255);
                    let names = lib.collection_names();
                    if names.is_empty() {
                        let hint = ui.text("No collections yet. Choose New Collection in the sidebar.", ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
                        s.blit_colored(&hint, pi(px), pi(MEMBERSHIP_TOP_PT + MEMBERSHIP_ROW_PT), ui::LABEL_2.now(), 255);
                    }
                    for (i, collection) in names.iter().enumerate() {
                        let member = lib.members(collection).is_some_and(|m| m.iter().any(|x| x.eq_ignore_ascii_case(name)));
                        let label = if member { format!("Remove from {collection}") } else { format!("Add to {collection}") };
                        let y = MEMBERSHIP_TOP_PT + MEMBERSHIP_ROW_PT * (i + 1) as f32;
                        let img = ui.text(&ui.fit(&label, ui::MAC_BODY_PT, ui.pt(260.0)), ui::MAC_BODY_PT, Weight::Regular, ui::LINK);
                        s.blit_colored(&img, pi(px), pi(y), ui::LINK, 255);
                    }
                }
            }
            (None, _) => {
                let hint = ui.text("Select a font to preview it", ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
                s.blit_colored(&hint, pi(px), pi(HEADER_PT + PAD_PT), ui::LABEL_2.now(), 255);
            }
        }
        if !self.status.is_empty() {
            let img = ui.text(&ui.fit(&self.status, ui::MAC_FOOTNOTE_PT, ui.pt(w as f32 / scale - px - PAD_PT)), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, pi(px), pi(h as f32 / scale - 24.0), ui::LABEL_2.now(), 255);
        }
        let mut d = Damage::new(w, h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(FontBookApp {
        library: None,
        user_dir: PathBuf::new(),
        source: Source::All,
        query: String::new(),
        selected: None,
        preview: None,
        report: None,
        status: String::new(),
    });
}
