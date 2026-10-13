//! Fonts as an installed BifrOSt app in the full-size window: a sidebar of lists (All, Off, collections), the family
//! list with search, and a preview with sample text, Info and the on/off switch. Installing a font opens it here
//! (Open With on a .ttf or .otf file). The model is in `model.rs`.

mod model;

use bifrost_app::kit::ui::{self, Ui, Weight};
use bifrost_app::kit::{Damage, Event, Font, Surface};
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

    /// Loads the preview font for the selected family, once per selection.
    fn ensure_preview(&mut self, scale: f32) {
        let Some(name) = self.selected.clone() else {
            self.preview = None;
            return;
        };
        if self.preview.as_ref().is_some_and(|(n, _)| *n == name) {
            return;
        }
        let path = self.library.as_ref().and_then(|l| l.find(&name)).map(|f| f.path.clone());
        self.preview = path.and_then(|p| std::fs::read(&p).ok()).and_then(|bytes| {
            // the bytes live for the whole run: one preview font per selection, so the leak is bounded by what was viewed
            let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            Font::parse(data).map(|font| (name.clone(), Ui::new(font, scale)))
        });
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
                let Some(lib) = &self.library else { return false };
                if x < SIDEBAR_PT {
                    if let Some(source) = sidebar_row(lib, y) {
                        self.source = source;
                        self.query.clear();
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
                // the on/off switch sits at the top of the preview column
                let switch_x = SIDEBAR_PT + LIST_PT + PAD_PT;
                if (switch_x..switch_x + 120.0).contains(&x) && (PAD_PT..PAD_PT + 28.0).contains(&y) {
                    if let Some(name) = self.selected.clone() {
                        if let Some(lib) = &mut self.library {
                            let on = lib.is_enabled(&name);
                            lib.set_enabled(&name, !on);
                        }
                        self.save(&ctx.data_dir);
                        return true;
                    }
                }
                false
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
            s.blit_colored(&img, p(PAD_PT), p(y + 6.0), ui::LABEL.now(), 255);
        }
        // list column
        let x0 = SIDEBAR_PT;
        s.fill_rect(p(x0 + LIST_PT - 1.0), 0, 1, h, ui::SEPARATOR);
        let shown = self.shown();
        let total = shown.len();
        let header = ui.text(&format!("{total} families"), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
        s.blit_colored(&header, p(x0 + PAD_PT), p(HEADER_PT - 20.0), ui::LABEL_2.now(), 255);
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
            s.blit_colored(&img, p(x0 + PAD_PT), p(y + 6.0), colour, 255);
        }
        // preview column
        let px = SIDEBAR_PT + LIST_PT + PAD_PT;
        match (&self.selected, &self.preview) {
            (Some(name), preview) => {
                let on = self.library.as_ref().is_none_or(|l| l.is_enabled(name));
                s.fill_rect(p(px), p(PAD_PT), p(120.0), p(28.0), if on { ui::LINK } else { ui::SEPARATOR });
                let label = if on { "Turn Off" } else { "Turn On" };
                let img = ui.text(label, ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&img, p(px + 12.0), p(PAD_PT + 6.0), ui::LABEL.now(), 255);
                let title = ui.text(&ui.fit(name, ui::MAC_TITLE2_PT, ui.pt(w as f32 / scale - px - PAD_PT)), ui::MAC_TITLE2_PT, Weight::Regular, ui::LABEL.now());
                s.blit_colored(&title, p(px), p(HEADER_PT + PAD_PT), ui::LABEL.now(), 255);
                let mut y = HEADER_PT + 56.0;
                if let Some((_, font_ui)) = preview {
                    for size in PREVIEW_SIZES {
                        let img = font_ui.text(model::PREVIEW_TEXT, size, Weight::Regular, ui::LABEL.now());
                        s.blit_colored(&img, p(px), p(y), ui::LABEL.now(), 255);
                        y += size * 1.6;
                    }
                }
                if let Some(lib) = &self.library {
                    y = y.max(HEADER_PT + 260.0);
                    for (k, v) in lib.info(name) {
                        let line = format!("{k}: {v}");
                        let img = ui.text(&ui.fit(&line, ui::MAC_FOOTNOTE_PT, ui.pt(w as f32 / scale - px - PAD_PT)), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
                        s.blit_colored(&img, p(px), p(y), ui::LABEL_2.now(), 255);
                        y += 18.0;
                    }
                }
            }
            (None, _) => {
                let hint = ui.text("Select a font to preview it", ui::MAC_BODY_PT, Weight::Regular, ui::LABEL_2.now());
                s.blit_colored(&hint, p(px), p(HEADER_PT + PAD_PT), ui::LABEL_2.now(), 255);
            }
        }
        if !self.status.is_empty() {
            let img = ui.text(&ui.fit(&self.status, ui::MAC_FOOTNOTE_PT, ui.pt(w as f32 / scale - px - PAD_PT)), ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL_2.now());
            s.blit_colored(&img, p(px), p(h as f32 / scale - 24.0), ui::LABEL_2.now(), 255);
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
        status: String::new(),
    });
}
