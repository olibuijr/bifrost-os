//! Settings › Wallpaper: a grid of the bifrost 15 stills (packed by
//! tools/pack.py) and photos; tapping one opens a preview with Set Lock
//! Screen / Set Home Screen / Set Both and the "Dark Appearance Dims
//! Wallpaper" switch. The navigation bar above is drawn by Settings.
//!
//! On the desktop (a pointer form factor) the page follows the Golden Gate Wallpaper pane instead: a click on a
//! thumbnail applies it at once, to the desktop and the lock screen together or to the lock screen only (the Show On
//! row), and "Add Photo…" / "Add Folder…" bring up the shell's open panel (`docpanel`) to add the user's own pictures.

use super::list::{nav_bottom, GROUP_GAP_PT, MARGIN_PT};
use super::controls::{draw_switch, SWITCH_H_PT, SWITCH_W_PT};
use super::live_wallpaper::Look;
use super::ui::{centred, home_zone, Ui, Weight, CARD_RADIUS_PT, CARD_BG, CELL_RADIUS_PT, FIELD_BG, LABEL, ROW_PRESSED, SECONDARY, WHITE};
use super::wifi_page::BLUE;
use crate::gfx::{rrect_mask, Image, Surface, View};

/// The id the gradient poster `name` is packed under: `gradient_ocean` for "Ocean".
pub fn gradient_id(name: &str) -> String {
    format!("gradient_{}", name.to_ascii_lowercase())
}

/// The gradient poster `name` (`bifrost_ui::procedural_texture::GRADIENT_PRESETS`) as an opaque `w` x `h` still.
/// None for an unknown name or an empty size.
pub fn gradient_image(name: &str, w: usize, h: usize) -> Option<Image> {
    let texture = bifrost_ui::procedural_texture::gradient_poster(name, w, h)?;
    Some(Image { w, h, px: texture.texels.iter().map(|t| crate::gfx::pack(t[0], t[1], t[2], t[3])).collect() })
}

/// The gradient posters as wallpapers, packed next to the stills: `wallpapers/gradient_<name>.raw` at `w` x `h` with a
/// 320 x 180 thumbnail beside it. A poster already packed is read back, not drawn again. A poster whose files cannot
/// be written is left out, so the picker never lists a wallpaper it cannot set.
pub fn gradient_wallpapers(dir: &str, w: usize, h: usize) -> Vec<Wallpaper> {
    use super::photos::thumbs::to_raw;
    let base = std::path::Path::new(dir).join("wallpapers");
    let mut out = Vec::new();
    for preset in bifrost_ui::procedural_texture::GRADIENT_PRESETS {
        let id = gradient_id(preset.name);
        let (raw, thumb_path) = (base.join(format!("{id}.raw")), base.join(format!("{id}.thumb.raw")));
        let packed = || -> Option<Image> {
            if !raw.exists() || !thumb_path.exists() {
                let full = gradient_image(preset.name, w, h)?;
                std::fs::create_dir_all(&base).ok()?;
                std::fs::write(&raw, to_raw(&full)).ok()?;
                std::fs::write(&thumb_path, to_raw(&cover_fit(&full, 320, 180))).ok()?;
            }
            Image::from_raw(&std::fs::read(&thumb_path).ok()?)
        };
        if let Some(thumb) = packed() {
            out.push(Wallpaper { id, name: preset.name.to_string(), thumb });
        }
    }
    out
}

/// The id a picture the user added (Add Photo…, Add Folder…) is cached under: `added_<stem>`, every character that is
/// not ASCII alphanumeric replaced by `_` and the stem cut to 48 characters, so it is a valid wallpaper id. None for a
/// path without a file name. (The Pictures folder's own pictures are `photo_<stem>`.)
pub fn added_id(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    (!stem.is_empty()).then(|| format!("added_{}", stem.chars().take(48).map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>()))
}

/// The pictures the user added, `id=name` (the stills themselves are `wallpapers/<id>.raw` beside it).
pub const ADDED_INDEX: &str = "wallpapers/added.ini";

/// The most pictures one Add Folder… takes, and the largest picture file read (bytes).
pub const MAX_FOLDER_PICTURES: usize = 64;
pub const MAX_PICTURE_BYTES: u64 = 64 << 20;

/// The pictures added earlier (`ADDED_INDEX`) whose thumbnails are still cached, for the catalogue at start-up.
pub fn added_wallpapers(dir: &std::path::Path) -> Vec<Wallpaper> {
    let index = super::prefs::Prefs::load(dir.join(ADDED_INDEX));
    index
        .entries()
        .filter_map(|(id, name)| {
            let thumb = Image::from_raw(&std::fs::read(dir.join(format!("wallpapers/{id}.thumb.raw"))).ok()?)?;
            Some(Wallpaper { id: id.to_string(), name: name.to_string(), thumb })
        })
        .collect()
}

/// The wallpaper size: the default still's (`wallpaper.raw`, a RIMG header `RIMG` + width + height, little-endian u32).
fn raw_size(path: &std::path::Path) -> Option<(usize, usize)> {
    use std::io::Read;
    let mut head = [0u8; 12];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    let le = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]) as usize;
    (&head[..4] == b"RIMG" && le(4) > 0 && le(8) > 0).then(|| (le(4), le(8)))
}

impl Import {
    /// The open panel's request number for this kind (`Panel::request` of a shell-owned panel).
    pub fn request(self) -> u32 {
        match self {
            Import::Photo => 1,
            Import::Folder => 2,
        }
    }

    pub fn from_request(r: u32) -> Option<Self> {
        Import::ALL.into_iter().find(|k| k.request() == r)
    }

    /// What the panel may choose: several pictures, or one folder.
    #[cfg(feature = "desktop-wayland")]
    pub fn options(self) -> super::docpanel::Options {
        match self {
            Import::Photo => super::docpanel::Options { multiple: true, files: true, dirs: false, create_dirs: false },
            Import::Folder => super::docpanel::Options { multiple: false, files: false, dirs: true, create_dirs: false },
        }
    }
}

#[cfg(feature = "desktop-wayland")]
impl super::Shell {
    /// Settings > Wallpaper > Add Photo… or Add Folder…: the open panel as a sheet on the Settings window, starting in
    /// Pictures. Its answer comes back through `doc_finish` to `wallpaper_panel_done`.
    pub(super) fn wallpaper_open_panel(&mut self, kind: Import) {
        use super::docpanel::{Kind, Panel, SHELL_SLOT};
        if self.doc_panel.is_some() {
            return;
        }
        let Some(b) = self.doc_broker() else { return };
        let title = kind.label().trim_end_matches('…');
        let mut panel = Panel::new(SHELL_SLOT, kind.request(), Kind::Open(bifrost_proto::DocMode::Read), title, vec!["public.image".into()], "", &b.opener).with_options(kind.options(), &b.opener);
        panel.go_to(std::path::Path::new("Pictures"), &b.opener);
        self.doc_panel = Some(panel);
        self.doc_fit_width();
        self.desktop_menu = Some(super::desktop_bar::Menu::Document);
        self.dirty = true;
    }

    /// The Settings open panel was answered: read the chosen pictures (every picture in a chosen folder) as the account,
    /// through the broker's opener, never as the shell, then add them.
    pub(super) fn wallpaper_panel_done(&mut self, request: u32, out: super::docpanel::Outcome) {
        use super::docpanel::Outcome;
        use std::io::Read;
        let Some(kind) = Import::from_request(request) else { return };
        let chosen = match out {
            Outcome::Open(p) => vec![p],
            Outcome::OpenMany(v) => v,
            _ => return,
        };
        let Some(b) = self.doc_broker() else { return };
        let mut files = Vec::new();
        for rel in chosen {
            let paths: Vec<std::path::PathBuf> = match kind {
                Import::Photo => vec![rel],
                Import::Folder => {
                    let mut v: Vec<_> = b.opener.list(&rel).unwrap_or_default().into_iter().filter(|(n, dir)| !dir && !n.starts_with('.')).map(|(n, _)| rel.join(n)).collect();
                    v.sort();
                    v
                }
            };
            for p in paths.into_iter().filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(is_picture)).take(MAX_FOLDER_PICTURES) {
                let Ok(f) = b.opener.open(&p, bifrost_proto::DocMode::Read) else { continue };
                let mut bytes = Vec::new();
                if f.take(MAX_PICTURE_BYTES).read_to_end(&mut bytes).is_ok() {
                    files.push((p, bytes));
                }
            }
        }
        self.wallpapers_import(files);
    }
}

impl super::Shell {
    /// Caches each picture (`path`, bytes) as a wallpaper still, lists it in the catalogue (and `ADDED_INDEX`, so it is
    /// there after a restart), and lets Settings apply the first one as a click on it would.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) fn wallpapers_import(&mut self, files: Vec<(std::path::PathBuf, Vec<u8>)>) {
        let dir = self.data_dir.clone();
        let (tw, th) = raw_size(&dir.join("wallpaper.raw")).unwrap_or((self.w.max(1) as usize, self.h.max(1) as usize));
        let mut index = super::prefs::Prefs::load(dir.join(ADDED_INDEX));
        let mut list = (*self.wallpapers).clone();
        let mut added = Vec::new();
        for (path, bytes) in files {
            let Some(id) = added_id(&path) else { continue };
            let Some(thumb) = cache_picture(&dir, &id, &bytes, tw, th) else { continue };
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| id.clone());
            if let Err(e) = index.set(&id, &name) {
                eprintln!("springboard: {ADDED_INDEX}: {e}");
            }
            // adding the same picture again replaces it
            list.retain(|w| w.id != id);
            list.push(Wallpaper { id: id.clone(), name, thumb });
            added.push(id);
        }
        if added.is_empty() {
            return;
        }
        self.set_wallpapers(list);
        let catalogue = self.wallpapers.clone();
        if let Some(st) = self.settings.as_mut() {
            let fx = st.wallpapers_added(&self.ui, &mut self.prefs, catalogue, &added);
            self.wallpaper_fx.extend(fx);
        }
        self.dirty = true;
    }
}

/// Whether `name` is a picture the wallpaper importer takes: its extension is a format the image reader decodes.
pub fn is_picture(name: &str) -> bool {
    crate::cg::imageio::is_image_name(name)
}

/// Decodes the picture `bytes` once, cover-fits it to the `tw` x `th` wallpaper size and caches it as
/// `dir/wallpapers/<id>.raw` with its 320 x 180 thumbnail as `<id>.thumb.raw`, so `Effect::Wallpaper(id)` can map it.
/// Returns the thumbnail; None when the bytes are not a picture or the files cannot be written.
pub fn cache_picture(dir: &std::path::Path, id: &str, bytes: &[u8], tw: usize, th: usize) -> Option<Image> {
    use super::photos::thumbs::to_raw;
    if tw == 0 || th == 0 {
        return None;
    }
    let img = crate::cg::imageio::decode(bytes)?;
    let full = cover_fit(&img, tw, th);
    let base = dir.join("wallpapers");
    std::fs::create_dir_all(&base).ok()?;
    let thumb = cover_fit(&full, THUMB.0, THUMB.1);
    std::fs::write(base.join(format!("{id}.raw")), to_raw(&full)).ok()?;
    std::fs::write(base.join(format!("{id}.thumb.raw")), to_raw(&thumb)).ok()?;
    Some(thumb)
}

/// Where a click on a desktop thumbnail puts the wallpaper (the Show On row).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShowOn {
    /// the desktop and the lock screen together
    #[default]
    All,
    /// the lock screen only; the desktop keeps its wallpaper
    LockScreen,
}

/// The pref the Show On choice is kept under.
pub const SHOW_ON_PREF: &str = "wallpaper_show_on";

impl ShowOn {
    pub const ALL: [ShowOn; 2] = [ShowOn::All, ShowOn::LockScreen];

    pub fn label(self) -> &'static str {
        match self {
            ShowOn::All => "Desktop and Lock Screen",
            ShowOn::LockScreen => "Lock Screen Only",
        }
    }

    pub fn pref(self) -> &'static str {
        match self {
            ShowOn::All => "all",
            ShowOn::LockScreen => "lock",
        }
    }

    pub fn parse(v: &str) -> Self {
        if v == "lock" { ShowOn::LockScreen } else { ShowOn::All }
    }

    /// What a click on a thumbnail sets.
    pub fn target(self) -> Target {
        match self {
            ShowOn::All => Target::Both,
            ShowOn::LockScreen => Target::Lock,
        }
    }
}

/// What the desktop's add buttons choose in the open panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Import {
    /// one or more pictures
    Photo,
    /// a folder: every picture in it
    Folder,
}

impl Import {
    pub const ALL: [Import; 2] = [Import::Photo, Import::Folder];

    pub fn label(self) -> &'static str {
        match self {
            Import::Photo => "Add Photo…",
            Import::Folder => "Add Folder…",
        }
    }
}

/// One packed wallpaper: `id` names `wallpapers/<id>.raw`.
#[derive(Clone, Debug, PartialEq)]
pub struct Wallpaper {
    pub id: String,
    pub name: String,
    pub thumb: Image,
}

/// The bundled stills photographed in Iceland (CREDITS.md); a fresh install starts on one of them.
pub const ICELAND: [&str; 6] = ["waterfall_idyllic", "svartifoss_basalt", "kirkjufell_dusk", "gullfoss_canyon", "arnarstapi_coast", "iceland_falls"];

/// The wallpaper a fresh install starts on: one of the `ICELAND` stills packed in `dir/wallpapers`,
/// `seed` picking which. None when none is packed (the shell then keeps `wallpaper.raw`).
pub fn first_wallpaper(dir: &str, seed: u64) -> Option<&'static str> {
    let packed: Vec<&'static str> = ICELAND.into_iter().filter(|id| std::path::Path::new(&format!("{dir}/wallpapers/{id}.raw")).exists()).collect();
    (!packed.is_empty()).then(|| packed[(seed % packed.len() as u64) as usize])
}

/// The shell's lock screen wallpaper and the dimmed copies (Dark Appearance
/// Dims Wallpaper) of the home and lock stills.
#[derive(Default)]
pub struct Extra {
    pub dim: bool,
    pub lock: Option<View<'static>>,
    pub home_dim: Option<Image>,
    pub lock_dim: Option<Image>,
}

/// `v` at 70 % brightness (opaque pixels).
pub fn dimmed(v: View) -> Image {
    let mut img = Image::new(v.w, v.h);
    for (o, &p) in img.px.iter_mut().zip(v.px) {
        *o = (((p & 0x00ff_00ff) * 179 >> 8) & 0x00ff_00ff) | (((p & 0x0000_ff00) * 179 >> 8) & 0x0000_ff00) | (p & 0xff00_0000);
    }
    img
}

pub const COLS: usize = 6;
/// Corner radius in px of the preview buttons (rule: at most 6, no pills).
const BUTTON_RADIUS: f32 = 6.0;
const GAP_PT: f32 = 12.0;
const LABEL_PT: f32 = 11.0;
/// Space under a thumbnail for its label.
const LABEL_ROW_PT: f32 = 22.0;
const THUMB_RADIUS_PT: f32 = 6.0;
const RING_PT: f32 = 3.0;
/// The stored thumbnails: 16:9 centre crops of the stills (`pack.py` THUMB). The grid and the preview take the
/// screen's aspect (`aspect` = screen height / width) and cover-fit these, so a landscape desktop shows landscape art.
pub const THUMB: (usize, usize) = (320, 180);

/// `img` centre-cropped to `w` x `h` (cover, as the desktop backdrop), then resized.
pub fn cover_fit(img: &Image, w: usize, h: usize) -> Image {
    if img.w == 0 || img.h == 0 || w == 0 || h == 0 {
        return Image::new(w, h);
    }
    let scale = (w as f32 / img.w as f32).max(h as f32 / img.h as f32);
    let (cw, ch) = (((w as f32 / scale).round() as usize).clamp(1, img.w), ((h as f32 / scale).round() as usize).clamp(1, img.h));
    img.crop((img.w - cw) / 2, (img.h - ch) / 2, cw, ch).resized(w, h)
}

/// Thumbnail cell size in px: 6 columns between the page margins, `aspect` = screen height / width.
pub fn cell_size(ui: &Ui, w: i32, aspect: f32) -> (f32, f32) {
    let cw = ((w as f32 - 2.0 * ui.pt(MARGIN_PT) - (COLS - 1) as f32 * ui.pt(GAP_PT)) / COLS as f32).floor();
    (cw, (cw * aspect).floor())
}

/// Top of the Dynamic Wallpaper switch row, under the nav bar.
fn dynamic_row_top(ui: &Ui) -> f32 {
    nav_bottom(ui) + ui.pt(GROUP_GAP_PT)
}

/// Top of the desktop's Show On row, where the tablet's grid starts.
fn show_on_row_top(ui: &Ui) -> f32 {
    grid_top(ui)
}

/// The desktop's Show On row (x, y, w, h): one segment per `ShowOn`.
pub fn show_on_row(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
    (ui.pt(MARGIN_PT), show_on_row_top(ui), w as f32 - 2.0 * ui.pt(MARGIN_PT), ui.pt(ROW_PT))
}

/// Segment `i` of the Show On row (x, y, w, h).
pub fn show_on_segment(ui: &Ui, w: i32, i: usize) -> (f32, f32, f32, f32) {
    let (x, y, rw, rh) = show_on_row(ui, w);
    let inset = ui.pt(SEGMENT_INSET_PT);
    let sw = (rw - 2.0 * inset) / ShowOn::ALL.len() as f32;
    (x + inset + i as f32 * sw, y + inset, sw, rh - 2.0 * inset)
}

/// The desktop's add button `i` (`Import::ALL` order), right-aligned under the Show On row like the reference's pane
/// buttons (x, y, w, h).
pub fn add_button(ui: &Ui, w: i32, i: usize) -> (f32, f32, f32, f32) {
    let (_, y, _, rh) = show_on_row(ui, w);
    let (bw, bh, gap) = (ui.pt(ADD_BUTTON_W_PT), ui.pt(ADD_BUTTON_H_PT), ui.pt(BUTTON_GAP_PT));
    let right = w as f32 - ui.pt(MARGIN_PT);
    let n = Import::ALL.len();
    (right - (n - i) as f32 * bw - (n - 1 - i) as f32 * gap, y + rh + ui.pt(GROUP_GAP_PT), bw, bh)
}

/// The desktop grid starts under the add buttons.
pub fn desktop_grid_top(ui: &Ui) -> f32 {
    show_on_row_top(ui) + ui.pt(ROW_PT) + ui.pt(GROUP_GAP_PT) + ui.pt(ADD_BUTTON_H_PT) + ui.pt(GROUP_GAP_PT)
}

/// Top of the Live Look row, under the Dynamic Wallpaper switch.
fn look_row_top(ui: &Ui) -> f32 {
    dynamic_row_top(ui) + ui.pt(ROW_PT) + ui.pt(GROUP_GAP_PT)
}

/// The grid starts under the Live Look row.
pub fn grid_top(ui: &Ui) -> f32 {
    look_row_top(ui) + ui.pt(ROW_PT) + ui.pt(GROUP_GAP_PT)
}

/// The Live Look row (x, y, w, h): one segment per `Look`, still photo first.
pub fn look_row(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
    (ui.pt(MARGIN_PT), look_row_top(ui), w as f32 - 2.0 * ui.pt(MARGIN_PT), ui.pt(ROW_PT))
}

/// Segment `i` of the Live Look row (x, y, w, h), inset so the selected plate sits inside the card.
pub fn look_segment(ui: &Ui, w: i32, i: usize) -> (f32, f32, f32, f32) {
    let (x, y, rw, rh) = look_row(ui, w);
    let inset = ui.pt(SEGMENT_INSET_PT);
    let sw = (rw - 2.0 * inset) / Look::ALL.len() as f32;
    (x + inset + i as f32 * sw, y + inset, sw, rh - 2.0 * inset)
}

/// The Dynamic Wallpaper switch row (x, y, w, h): a day of stills follows the time of day.
pub fn dynamic_row(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
    (ui.pt(MARGIN_PT), dynamic_row_top(ui), w as f32 - 2.0 * ui.pt(MARGIN_PT), ui.pt(ROW_PT))
}

/// Thumbnail rect (x, y, w, h) in px of item `i`.
pub fn cell(ui: &Ui, w: i32, aspect: f32, i: usize) -> (f32, f32, f32, f32) {
    cell_from(ui, w, aspect, i, grid_top(ui))
}

/// `cell` for a grid whose first row is at `top`.
fn cell_from(ui: &Ui, w: i32, aspect: f32, i: usize, top: f32) -> (f32, f32, f32, f32) {
    let (cw, ch) = cell_size(ui, w, aspect);
    let (c, r) = ((i % COLS) as f32, (i / COLS) as f32);
    let x = ui.pt(MARGIN_PT) + c * (cw + ui.pt(GAP_PT));
    let y = top + r * (ch + ui.pt(LABEL_ROW_PT) + ui.pt(GAP_PT));
    (x, y, cw, ch)
}

/// Where a previewed wallpaper goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Lock,
    Home,
    Both,
}

/// What a tap on the page meant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tap {
    /// a grid thumbnail
    Thumb(usize),
    /// a Set button in the preview
    Set(usize, Target),
    /// the Cancel button
    Cancel,
    /// the dim switch
    Dim,
    /// the Dynamic Wallpaper switch above the grid
    Dynamic,
    /// a segment of the Live Look row: still photo, or an animated or time-of-day wallpaper
    Look(Look),
    /// a segment of the desktop's Show On row
    ShowOn(ShowOn),
    /// a desktop add button: bring up the open panel
    Add(Import),
}

const ROW_PT: f32 = 44.0;
/// Space around the selected plate of the Live Look row.
const SEGMENT_INSET_PT: f32 = 4.0;
const BUTTON_PT: f32 = 50.0;
const BUTTON_GAP_PT: f32 = 12.0;
/// The desktop's add buttons (the reference's pane buttons, a 28 pt plate).
const ADD_BUTTON_W_PT: f32 = 120.0;
const ADD_BUTTON_H_PT: f32 = 28.0;
const BUTTONS: [(&str, Option<Target>); 4] = [("Set Lock Screen", Some(Target::Lock)), ("Set Home Screen", Some(Target::Home)), ("Set Both", Some(Target::Both)), ("Cancel", None)];

/// The open preview: the picked item's enlarged thumbnail and button labels.
struct Preview {
    item: usize,
    img: Image,
    dim_label: Image,
    labels: Vec<Image>,
}

pub struct WallpaperPage {
    items: Vec<(String, Image, Image)>,
    current: Option<usize>,
    empty: Image,
    dynamic_label: Image,
    preview: Option<Preview>,
    dim: bool,
    /// Dynamic Wallpaper: the shell changes the still with the time of day
    dynamic: bool,
    /// the desktop wallpaper look (`wallpaper_look`)
    look: Look,
    /// each look's name, in the label colour and in white for the selected plate
    look_labels: Vec<(Image, Image)>,
    /// screen height / width: the cells and the preview are this shape
    aspect: f32,
    /// unmasked thumbnails, enlarged for the preview
    sources: Vec<Image>,
    /// the desktop layout: a click applies a thumbnail at once, with the Show On row and the add buttons
    desktop: bool,
    show_on: ShowOn,
    /// each Show On choice's name, in the label colour and in white for the selected plate
    show_on_labels: Vec<(Image, Image)>,
    add_labels: Vec<Image>,
}

impl WallpaperPage {
    pub fn new(ui: &Ui, w: i32, aspect: f32, wallpapers: &[Wallpaper], current: &str, dim: bool) -> Self {
        let (cw, ch) = cell_size(ui, w, aspect);
        let mask = rrect_mask(cw as usize, ch as usize, ui.pt(THUMB_RADIUS_PT));
        let items = wallpapers
            .iter()
            .map(|wp| {
                let mut thumb = cover_fit(&wp.thumb, cw as usize, ch as usize);
                thumb.apply_mask(&mask);
                let label = ui.text(&ui.fit(&wp.name, LABEL_PT, cw), LABEL_PT, Weight::Regular, LABEL.now());
                (wp.id.clone(), thumb, label)
            })
            .collect();
        WallpaperPage { current: wallpapers.iter().position(|wp| wp.id == current), items, empty: ui.text("No wallpapers packed", 17.0, Weight::Regular, SECONDARY), dynamic_label: ui.text("Dynamic Wallpaper", 17.0, Weight::Regular, LABEL.now()), preview: None, dim, dynamic: false, look: Look::Still, look_labels: Look::ALL.iter().map(|l| (ui.text(l.label(), LABEL_PT, Weight::Regular, LABEL.now()), ui.text(l.label(), LABEL_PT, Weight::Semibold, WHITE))).collect(), aspect, sources: wallpapers.iter().map(|wp| wp.thumb.clone()).collect(), desktop: false, show_on: ShowOn::All, show_on_labels: Vec::new(), add_labels: Vec::new() }
    }

    /// Switches to the desktop layout (click to apply, Show On, Add Photo… and Add Folder…).
    pub fn set_desktop(&mut self, ui: &Ui, on: bool) {
        self.desktop = on;
        if on && self.add_labels.is_empty() {
            self.show_on_labels = ShowOn::ALL.iter().map(|o| (ui.text(o.label(), LABEL_PT, Weight::Regular, LABEL.now()), ui.text(o.label(), LABEL_PT, Weight::Semibold, WHITE))).collect();
            self.add_labels = Import::ALL.iter().map(|k| ui.text(k.label(), LABEL_PT, Weight::Regular, LABEL.now())).collect();
        }
    }

    pub fn desktop(&self) -> bool {
        self.desktop
    }

    pub fn show_on(&self) -> ShowOn {
        self.show_on
    }

    pub fn set_show_on(&mut self, show_on: ShowOn) {
        self.show_on = show_on;
    }

    /// Where the grid starts in this layout.
    fn grid_y(&self, ui: &Ui) -> f32 {
        if self.desktop { desktop_grid_top(ui) } else { grid_top(ui) }
    }

    /// Thumbnail rect of item `i` in this layout.
    pub fn cell(&self, ui: &Ui, w: i32, i: usize) -> (f32, f32, f32, f32) {
        cell_from(ui, w, self.aspect, i, self.grid_y(ui))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The id of the highlighted wallpaper.
    pub fn current(&self) -> Option<&str> {
        self.current.map(|i| self.items[i].0.as_str())
    }

    /// Highlights item `i` and returns its id.
    pub fn select(&mut self, i: usize) -> Option<String> {
        let id = self.items.get(i)?.0.clone();
        self.current = Some(i);
        Some(id)
    }

    /// The id of item `i`.
    pub fn id_of(&self, i: usize) -> Option<&str> {
        self.items.get(i).map(|it| it.0.as_str())
    }

    pub fn dim(&self) -> bool {
        self.dim
    }

    pub fn set_dim(&mut self, on: bool) {
        self.dim = on;
    }

    pub fn dynamic(&self) -> bool {
        self.dynamic
    }

    pub fn set_dynamic(&mut self, on: bool) {
        self.dynamic = on;
    }

    pub fn look(&self) -> Look {
        self.look
    }

    pub fn set_look(&mut self, look: Look) {
        self.look = look;
    }

    pub fn previewing(&self) -> bool {
        self.preview.is_some()
    }

    pub fn close_preview(&mut self) {
        self.preview = None;
    }

    /// Opens the preview of item `i`.
    pub fn open_preview(&mut self, ui: &Ui, w: i32, h: i32, i: usize) {
        let Some(src) = self.sources.get(i) else { return };
        let (pw, ph, ..) = self.preview_rect(ui, w, h);
        let (pw, ph) = (pw as usize, ph as usize);
        let mut img = cover_fit(src, pw, ph);
        img.apply_mask(&rrect_mask(pw, ph, ui.pt(CARD_RADIUS_PT)));
        let labels = BUTTONS.iter().map(|(t, target)| ui.text(t, 17.0, if target.is_some() { Weight::Semibold } else { Weight::Regular }, BLUE)).collect();
        self.preview = Some(Preview { item: i, img, dim_label: ui.text("Dark Appearance Dims Wallpaper", 17.0, Weight::Regular, LABEL.now()), labels });
    }

    /// (w, h, x, y) of the preview image: half the free height, in the screen's aspect.
    fn preview_rect(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let free = h as f32 - home_zone(ui) - grid_top(ui);
        let ph = (free * 0.45).floor();
        let pw = (ph / self.aspect).floor();
        (pw, ph, ((w as f32 - pw) / 2.0).floor(), grid_top(ui))
    }

    /// The dim switch row (x, y, w, h).
    pub fn dim_row(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (_, ph, _, py) = self.preview_rect(ui, w, h);
        (ui.pt(MARGIN_PT), py + ph + ui.pt(GROUP_GAP_PT), w as f32 - 2.0 * ui.pt(MARGIN_PT), ui.pt(ROW_PT))
    }

    /// Button `i` (x, y, w, h).
    pub fn button(&self, ui: &Ui, w: i32, h: i32, i: usize) -> (f32, f32, f32, f32) {
        let (rx, ry, rw, rh) = self.dim_row(ui, w, h);
        (rx, ry + rh + ui.pt(GROUP_GAP_PT) + i as f32 * (ui.pt(BUTTON_PT) + ui.pt(BUTTON_GAP_PT)), rw, ui.pt(BUTTON_PT))
    }

    /// What a touch at (x, y) taps: grid thumbnails, or the preview's controls.
    pub fn tap(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Tap> {
        let inside = |(rx, ry, rw, rh): (f32, f32, f32, f32)| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        let Some(p) = &self.preview else {
            if inside(dynamic_row(ui, w)) {
                return Some(Tap::Dynamic);
            }
            if let Some(l) = Look::ALL.iter().enumerate().find(|&(i, _)| inside(look_segment(ui, w, i))).map(|(_, &l)| l) {
                return Some(Tap::Look(l));
            }
            if self.desktop {
                if let Some(o) = ShowOn::ALL.iter().enumerate().find(|&(i, _)| inside(show_on_segment(ui, w, i))).map(|(_, &o)| o) {
                    return Some(Tap::ShowOn(o));
                }
                if let Some(k) = Import::ALL.iter().enumerate().find(|&(i, _)| inside(add_button(ui, w, i))).map(|(_, &k)| k) {
                    return Some(Tap::Add(k));
                }
            }
            return self.hit(ui, w, h, x, y).map(Tap::Thumb);
        };
        if inside(self.dim_row(ui, w, h)) {
            return Some(Tap::Dim);
        }
        (0..BUTTONS.len()).find(|&i| inside(self.button(ui, w, h, i))).map(|i| match BUTTONS[i].1 {
            Some(t) => Tap::Set(p.item, t),
            None => Tap::Cancel,
        })
    }

    /// How many items fit above the home indicator.
    fn visible(&self, ui: &Ui, w: i32, h: i32) -> usize {
        let limit = h as f32 - home_zone(ui);
        (0..self.items.len()).take_while(|&i| self.cell(ui, w, i).1 + cell_size(ui, w, self.aspect).1 + ui.pt(LABEL_ROW_PT) <= limit).count()
    }

    /// The thumbnail under (x, y).
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<usize> {
        (0..self.visible(ui, w, h)).find(|&i| {
            let (cx, cy, cw, ch) = self.cell(ui, w, i);
            x >= cx && x < cx + cw && y >= cy && y < cy + ch
        })
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32, pressed: Option<usize>) {
        let m = ui.pt(MARGIN_PT);
        if let Some(p) = &self.preview {
            let (_, _, px, py) = self.preview_rect(ui, w, h);
            s.blit(&p.img, px as i32, py as i32, 255, 255);
            let (x, y, rw, rh) = self.dim_row(ui, w, h);
            s.fill_rrect(x, y, rw, rh, ui.pt(CELL_RADIUS_PT), CARD_BG.now());
            s.blit(&p.dim_label, (x + ui.pt(16.0)) as i32, (y + (rh - p.dim_label.h as f32) / 2.0) as i32, 255, 255);
            draw_switch(s, ui, x + rw - ui.pt(16.0) - ui.pt(SWITCH_W_PT), y + (rh - ui.pt(SWITCH_H_PT)) / 2.0, if self.dim { 1.0 } else { 0.0 });
            for (i, label) in p.labels.iter().enumerate() {
                let (bx, by, bw, bh) = self.button(ui, w, h, i);
                s.fill_rrect(bx, by, bw, bh, BUTTON_RADIUS.min(bh / 2.0), if pressed == Some(i) { ROW_PRESSED.now() } else { FIELD_BG.now() });
                centred(s, label, (bx + bw / 2.0) as i32, (by + (bh - label.h as f32) / 2.0) as i32);
            }
            return;
        }
        let (x, y, rw, rh) = dynamic_row(ui, w);
        s.fill_rrect(x, y, rw, rh, ui.pt(CELL_RADIUS_PT), CARD_BG.now());
        s.blit(&self.dynamic_label, (x + ui.pt(16.0)) as i32, (y + (rh - self.dynamic_label.h as f32) / 2.0) as i32, 255, 255);
        draw_switch(s, ui, x + rw - ui.pt(16.0) - ui.pt(SWITCH_W_PT), y + (rh - ui.pt(SWITCH_H_PT)) / 2.0, if self.dynamic { 1.0 } else { 0.0 });
        let (x, y, rw, rh) = look_row(ui, w);
        s.fill_rrect(x, y, rw, rh, ui.pt(CELL_RADIUS_PT), CARD_BG.now());
        for (i, (look, (plain, white))) in Look::ALL.iter().zip(&self.look_labels).enumerate() {
            let (sx, sy, sw, sh) = look_segment(ui, w, i);
            let on = *look == self.look;
            if on {
                s.fill_rrect(sx, sy, sw, sh, ui.pt(CELL_RADIUS_PT) - ui.pt(SEGMENT_INSET_PT) / 2.0, BLUE);
            }
            let label = if on { white } else { plain };
            centred(s, label, (sx + sw / 2.0) as i32, (sy + (sh - label.h as f32) / 2.0) as i32);
        }
        if self.desktop {
            let (x, y, rw, rh) = show_on_row(ui, w);
            s.fill_rrect(x, y, rw, rh, ui.pt(CELL_RADIUS_PT), CARD_BG.now());
            for (i, (o, (plain, white))) in ShowOn::ALL.iter().zip(&self.show_on_labels).enumerate() {
                let (sx, sy, sw, sh) = show_on_segment(ui, w, i);
                let on = *o == self.show_on;
                if on {
                    s.fill_rrect(sx, sy, sw, sh, ui.pt(CELL_RADIUS_PT) - ui.pt(SEGMENT_INSET_PT) / 2.0, BLUE);
                }
                let label = if on { white } else { plain };
                centred(s, label, (sx + sw / 2.0) as i32, (sy + (sh - label.h as f32) / 2.0) as i32);
            }
            for (i, label) in self.add_labels.iter().enumerate() {
                let (bx, by, bw, bh) = add_button(ui, w, i);
                s.fill_rrect(bx, by, bw, bh, BUTTON_RADIUS.min(bh / 2.0), FIELD_BG.now());
                centred(s, label, (bx + bw / 2.0) as i32, (by + (bh - label.h as f32) / 2.0) as i32);
            }
        }
        if self.items.is_empty() {
            s.blit(&self.empty, (m + ui.pt(16.0)) as i32, self.grid_y(ui) as i32, 255, 255);
            return;
        }
        for (i, (_, thumb, label)) in self.items.iter().take(self.visible(ui, w, h)).enumerate() {
            let (x, y, cw, ch) = self.cell(ui, w, i);
            let tint = if pressed == Some(i) { 150 } else { 255 };
            s.blit(thumb, x as i32, y as i32, tint, 255);
            if self.current == Some(i) {
                let r = ui.pt(RING_PT);
                s.stroke_rrect(x - r, y - r, cw + 2.0 * r, ch + 2.0 * r, ui.pt(THUMB_RADIUS_PT) + r, r, BLUE);
            }
            centred(s, label, (x + cw / 2.0) as i32, (y + ch + (ui.pt(LABEL_ROW_PT) - label.h as f32) / 2.0) as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn ui() -> Ui {
        super::super::tests::ui()
    }

    #[test]
    fn the_live_look_row_sits_between_the_dynamic_switch_and_the_grid_and_its_segments_are_tappable() {
        let ui = ui();
        let (_, dy, _, dh) = dynamic_row(&ui, 1200);
        let (_, ly, lw, lh) = look_row(&ui, 1200);
        assert!(ly >= dy + dh && grid_top(&ui) >= ly + lh, "dynamic row, then the look row, then the grid");
        let mut page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(3), "wp0", true);
        assert_eq!(page.look(), Look::Still);
        let mut seen = Vec::new();
        for i in 0..Look::ALL.len() {
            let (x, y, w, h) = look_segment(&ui, 1200, i);
            assert!(x >= 0.0 && x + w <= lw + look_row(&ui, 1200).0 && h <= lh);
            seen.push(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0));
        }
        assert_eq!(seen, Look::ALL.iter().map(|&l| Some(Tap::Look(l))).collect::<Vec<_>>());
        page.set_look(Look::Waves);
        assert_eq!(page.look(), Look::Waves);
        let mut buf = vec![0u32; 1200 * 1920];
        let mut s = Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 };
        page.draw(&mut s, &ui, 1200, 1920, None);
        let (x, y, w, _h) = look_segment(&ui, 1200, 2);
        assert_eq!(buf[(y + 2.0) as usize * 1200 + (x + w / 2.0) as usize], BLUE, "the selected plate is filled with the tint");
    }

    #[test]
    fn a_fresh_install_starts_on_a_packed_icelandic_still() {
        let dir = std::env::temp_dir().join(format!("wp-first-{}", std::process::id()));
        let d = dir.to_str().unwrap();
        assert_eq!(first_wallpaper(d, 0), None, "nothing packed: keep wallpaper.raw");
        std::fs::create_dir_all(dir.join("wallpapers")).unwrap();
        for id in ["kirkjufell_dusk", "gullfoss_canyon", "desert_day"] {
            std::fs::write(dir.join(format!("wallpapers/{id}.raw")), b"").unwrap();
        }
        let picks: std::collections::BTreeSet<_> = (0..8).filter_map(|seed| first_wallpaper(d, seed)).collect();
        assert_eq!(picks.into_iter().collect::<Vec<_>>(), ["gullfoss_canyon", "kirkjufell_dusk"], "only Icelandic stills, and the seed varies the pick");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gradient_posters_are_packed_once_and_listed_with_their_names() {
        let dir = std::env::temp_dir().join(format!("wp-gradient-{}", std::process::id()));
        let d = dir.to_str().unwrap();
        let listed = gradient_wallpapers(d, 64, 36);
        let names: Vec<&str> = listed.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Sunrise", "Ocean", "Meadow", "Dusk", "Graphite"]);
        assert_eq!(listed[1].id, "gradient_ocean");
        assert!(crate::installer::plan::valid_wallpaper_id(&listed[1].id), "Settings can set it by id");
        let raw = std::fs::read(dir.join("wallpapers/gradient_ocean.raw")).unwrap();
        let still = Image::from_raw(&raw).expect("a RIMG still");
        assert_eq!((still.w, still.h), (64, 36));
        assert_ne!(still.px[0], still.px[63], "the left and right edges differ");
        assert_eq!((listed[1].thumb.w, listed[1].thumb.h), (320, 180));
        // a second call reads the packed files back
        assert_eq!(gradient_wallpapers(d, 64, 36), listed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unknown_gradient_or_an_empty_size_gives_no_still() {
        assert!(gradient_image("Nope", 8, 4).is_none());
        assert!(gradient_image("Ocean", 0, 4).is_none());
        assert_eq!(gradient_id("Graphite"), "gradient_graphite");
    }

    fn wallpapers(n: usize) -> Vec<Wallpaper> {
        (0..n)
            .map(|i| Wallpaper { id: format!("wp{i}"), name: format!("Wallpaper {i}"), thumb: Image { w: 320, h: 180, px: vec![pack(10 * i as u8, 0, 0, 255); 320 * 180] } })
            .collect()
    }

    /// Developer preview: `PREVIEW_DIR=/tmp cargo test preview_wallpaper_grid -- --ignored` writes wallpaper-grid.png,
    /// the packed 16:9 thumbnails on a 16:9 screen (1280 x 720 px).
    #[test]
    #[ignore]
    fn preview_wallpaper_grid() {
        let Some(dir) = std::env::var_os("PREVIEW_DIR") else { return };
        let assets = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/wallpapers");
        let index = crate::shell::prefs::Prefs::load(format!("{assets}/index.ini"));
        let wps: Vec<Wallpaper> = index
            .entries()
            .filter_map(|(id, name)| {
                let thumb = Image::from_raw(&std::fs::read(format!("{assets}/{id}.thumb.raw")).ok()?)?;
                Some(Wallpaper { id: id.to_string(), name: name.to_string(), thumb })
            })
            .collect();
        let ui = ui();
        let (w, h) = (1280i32, 720i32);
        let page = WallpaperPage::new(&ui, w, h as f32 / w as f32, &wps, "svartifoss_basalt", true);
        let mut img = Image::new(w as usize, h as usize);
        img.px.fill(u32::MAX);
        page.draw(&mut Surface { w: w as usize, h: h as usize, stride: w as usize, px: &mut img.px, y0: 0 }, &ui, w, h, None);
        std::fs::write(std::path::Path::new(&dir).join("wallpaper-grid.png"), crate::shell::screenshot::encode(&img)).unwrap();
    }

    #[test]
    fn the_desktop_layout_has_show_on_and_add_buttons_above_a_lower_grid() {
        let ui = ui();
        let mut page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(3), "wp0", true);
        page.set_desktop(&ui, true);
        assert!(page.desktop());
        assert_eq!(page.show_on(), ShowOn::All);
        let (_, ly, _, lh) = look_row(&ui, 1200);
        let (_, sy, _, sh) = show_on_row(&ui, 1200);
        assert!(sy >= ly + lh, "the Show On row is under the Live Look row");
        for (i, o) in ShowOn::ALL.into_iter().enumerate() {
            let (x, y, w, h) = show_on_segment(&ui, 1200, i);
            assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(Tap::ShowOn(o)));
        }
        for (i, k) in Import::ALL.into_iter().enumerate() {
            let (x, y, w, h) = add_button(&ui, 1200, i);
            assert!(y >= sy + sh && x + w <= 1200.0 - ui.pt(MARGIN_PT) + 0.5, "the add buttons sit right-aligned under the row");
            assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(Tap::Add(k)));
        }
        let (_, by, _, bh) = add_button(&ui, 1200, 0);
        let (cx, cy, cw, ch) = page.cell(&ui, 1200, 1);
        assert!(cy >= by + bh, "the grid starts under the add buttons");
        assert_eq!(page.tap(&ui, 1200, 1920, cx + cw / 2.0, cy + ch / 2.0), Some(Tap::Thumb(1)));
        // the tablet layout has none of it
        let tablet = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(3), "wp0", true);
        let (x, y, w, h) = show_on_segment(&ui, 1200, 1);
        assert!(!matches!(tablet.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(Tap::ShowOn(_) | Tap::Add(_))));
        page.set_show_on(ShowOn::LockScreen);
        let mut buf = vec![0u32; 1200 * 1920];
        page.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, None);
        let (x, y, w, _) = show_on_segment(&ui, 1200, 1);
        assert_eq!(buf[(y + 2.0) as usize * 1200 + (x + w / 2.0) as usize], BLUE, "the chosen Show On plate is tinted");
    }

    #[test]
    fn show_on_picks_the_click_target_and_round_trips_its_pref() {
        assert_eq!(ShowOn::All.target(), Target::Both);
        assert_eq!(ShowOn::LockScreen.target(), Target::Lock);
        for o in ShowOn::ALL {
            assert_eq!(ShowOn::parse(o.pref()), o);
        }
        assert_eq!(ShowOn::parse(""), ShowOn::All, "the default shows a wallpaper everywhere");
    }

    #[test]
    fn an_added_picture_is_cached_as_a_wallpaper_still_with_its_thumbnail() {
        use std::path::Path;
        assert_eq!(added_id(Path::new("/home/u/Pictures/Fjord view.jpg")).as_deref(), Some("added_Fjord_view"));
        assert_eq!(added_id(Path::new("")), None);
        assert!(is_picture("a.png") && is_picture("b.JPG") && !is_picture("notes.txt"));
        assert!(crate::installer::plan::valid_wallpaper_id("added_Fjord_view"), "Settings can set it by id");
        let long = added_id(Path::new(&format!("{}.png", "x".repeat(200)))).unwrap();
        assert!(crate::installer::plan::valid_wallpaper_id(&long), "a long name still gives a valid id");
        assert_eq!(Import::from_request(Import::Folder.request()), Some(Import::Folder));
        #[cfg(feature = "desktop-wayland")]
        assert!(Import::Folder.options().dirs && !Import::Folder.options().files && Import::Photo.options().multiple);
        let dir = std::env::temp_dir().join(format!("wp-import-{}", std::process::id()));
        // a 4 x 2 BMP: 24-bit, bottom-up rows padded to 4 bytes
        let mut bmp = Vec::new();
        let row = 12usize;
        let size = 54 + row * 2;
        bmp.extend(b"BM");
        bmp.extend((size as u32).to_le_bytes());
        bmp.extend([0u8; 4]);
        bmp.extend(54u32.to_le_bytes());
        bmp.extend(40u32.to_le_bytes());
        bmp.extend(4i32.to_le_bytes());
        bmp.extend(2i32.to_le_bytes());
        bmp.extend(1u16.to_le_bytes());
        bmp.extend(24u16.to_le_bytes());
        bmp.extend([0u8; 24]);
        bmp.extend(std::iter::repeat([0u8, 0, 255]).take(8).flatten());
        assert!(cache_picture(&dir, "photo_x", b"not a picture", 64, 36).is_none());
        let thumb = cache_picture(&dir, "photo_x", &bmp, 64, 36).expect("a decodable picture is cached");
        assert_eq!((thumb.w, thumb.h), THUMB);
        let still = Image::from_raw(&std::fs::read(dir.join("wallpapers/photo_x.raw")).unwrap()).unwrap();
        assert_eq!((still.w, still.h), (64, 36), "cover-fitted to the wallpaper size");
        assert!(dir.join("wallpapers/photo_x.thumb.raw").exists());
        assert_eq!(raw_size(&dir.join("wallpapers/photo_x.raw")), Some((64, 36)), "the size comes from the RIMG header");
        assert_eq!(raw_size(&dir.join("missing.raw")), None);
        // an index entry lists the picture again at start-up; one whose thumbnail is gone is left out
        let mut index = super::super::prefs::Prefs::load(dir.join(ADDED_INDEX));
        index.set("photo_x", "Fjord").unwrap();
        index.set("gone", "Gone").unwrap();
        let listed = added_wallpapers(&dir);
        assert_eq!(listed.iter().map(|w| (w.id.as_str(), w.name.as_str())).collect::<Vec<_>>(), [("photo_x", "Fjord")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cells_tile_six_columns_inside_the_margins() {
        let ui = ui();
        let (cw, ch) = cell_size(&ui, 1200, 9.0 / 16.0);
        assert!((ch / cw - 9.0 / 16.0).abs() < 0.02, "the grid follows the screen's 16:9 aspect");
        let (x0, y0, ..) = cell(&ui, 1200, 9.0 / 16.0, 0);
        let (x5, y5, ..) = cell(&ui, 1200, 9.0 / 16.0, 5);
        let (x6, y6, ..) = cell(&ui, 1200, 9.0 / 16.0, 6);
        assert_eq!(x0, ui.pt(MARGIN_PT));
        assert!(x5 + cw <= 1200.0 - ui.pt(MARGIN_PT) + 1.0);
        assert_eq!((x6, y0), (x0, y5));
        assert!(y6 > y0 + ch);
        assert_eq!(y0, grid_top(&ui));
        assert!(y0 > dynamic_row(&ui, 1200).1 + dynamic_row(&ui, 1200).3, "the grid sits under the Dynamic Wallpaper row");
    }

    #[test]
    fn the_dynamic_switch_row_taps_toggle_only_outside_the_preview() {
        let ui = ui();
        let mut page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(3), "wp0", true);
        assert!(!page.dynamic());
        let (x, y, w, h) = dynamic_row(&ui, 1200);
        assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(Tap::Dynamic));
        page.set_dynamic(true);
        assert!(page.dynamic());
        let (cx, cy, cw, ch) = cell(&ui, 1200, 9.0 / 16.0, 1);
        assert_eq!(page.tap(&ui, 1200, 1920, cx + cw / 2.0, cy + ch / 2.0), Some(Tap::Thumb(1)), "the grid is below the row");
        page.open_preview(&ui, 1200, 1920, 0);
        assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), None, "the preview hides the row");
    }

    #[test]
    fn taps_resolve_to_items() {
        let ui = ui();
        let mut page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(25), "wp3", true);
        assert_eq!(page.current(), Some("wp3"));
        let (x, y, cw, ch) = cell(&ui, 1200, 9.0 / 16.0, 7);
        assert_eq!(page.hit(&ui, 1200, 1920, x + cw / 2.0, y + ch / 2.0), Some(7));
        assert_eq!(page.hit(&ui, 1200, 1920, x - 2.0, y + ch / 2.0), None, "between columns");
        assert_eq!(page.hit(&ui, 1200, 1920, ui.pt(60.0), ui.pt(42.0)), None, "the bar is not an item");
        assert_eq!(page.select(7).as_deref(), Some("wp7"));
        assert_eq!(page.current(), Some("wp7"));
        assert_eq!(page.select(99), None);
        assert_eq!(page.len(), 25);
        assert!(page.visible(&ui, 1200, 1920) == 25, "all 25 stills fit above the home indicator");
        assert!(page.visible(&ui, 1200, 400) < 25, "a short screen hides the lower rows");
    }

    #[test]
    fn draws_thumbnails_and_highlights_the_current_one() {
        let ui = ui();
        let page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(2), "wp1", true);
        let mut buf = vec![0u32; 1200 * 1920];
        page.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, None);
        let (x, y, cw, ch) = cell(&ui, 1200, 9.0 / 16.0, 1);
        let centre = buf[(y + ch / 2.0) as usize * 1200 + (x + cw / 2.0) as usize];
        assert_eq!(centre, pack(10, 0, 0, 255));
        let ring = buf[(y + ch / 2.0) as usize * 1200 + (x - ui.pt(RING_PT) / 2.0) as usize];
        assert_eq!(ring, BLUE);
        let (x0, y0, ..) = cell(&ui, 1200, 9.0 / 16.0, 0);
        assert_ne!(buf[(y0 + 3.0) as usize * 1200 + (x0 - ui.pt(RING_PT) / 2.0) as usize], BLUE, "only the current one has a ring");
        let empty = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &[], "", true);
        assert!(empty.is_empty());
        empty.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, None);
    }

    #[test]
    fn preview_buttons_and_dim_switch_resolve_to_taps() {
        let ui = ui();
        let mut page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(3), "wp0", true);
        page.open_preview(&ui, 1200, 1920, 2);
        assert!(page.previewing());
        let (x, y, w, h) = page.dim_row(&ui, 1200, 1920);
        assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(Tap::Dim));
        for (i, want) in [Tap::Set(2, Target::Lock), Tap::Set(2, Target::Home), Tap::Set(2, Target::Both), Tap::Cancel].into_iter().enumerate() {
            let (x, y, w, h) = page.button(&ui, 1200, 1920, i);
            assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(want));
        }
        let mut buf = vec![0u32; 1200 * 1920];
        page.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, None);
        page.close_preview();
        assert!(!page.previewing());
    }
}
