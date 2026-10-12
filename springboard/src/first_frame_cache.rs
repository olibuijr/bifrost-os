//! Caches for the first desktop frame, built at install and update (docs/boot-architecture.md, "Prepared, not probed";
//! task `boot-08`). One file, `first-frame.ffc`, in `/var/cache/bifrost/first-frame/`, holds three things decoded or
//! rasterised for exactly one panel size and one layout scale:
//!
//! - the wallpaper, cover-fitted to the panel (the cover scaling is what a first frame on a non-default panel costs);
//! - the icons of `assets/icons`, resized to the Dock's icon size;
//! - the menu bar font's glyph atlas: printable ASCII rendered once at the menu bar text size.
//!
//! The file is written atomically (temporary file, then rename), and it is only used when its key matches the panel,
//! the scale and the wallpaper it was built from. Anything else (missing, truncated, another key, another version) is
//! ignored and the shell decodes as it did before. Pure encoding, decoding and key logic are host-tested; the file I/O
//! is thin.
//!
//! The scale is an input, never read from global state here: the shell derives it with its own layout (after
//! `layout::set_adaptive` and `set_user_scale`), and the installer and updater derive the automatic scale from the
//! panel's EDID size. When the two differ the shell misses and rebuilds for its own scale, so the cache heals itself.

use crate::font::{Font, Style};
use crate::gfx::Image;
use crate::layout::Layout;
use crate::shell::wallpaper_page::cover_fit;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The cache directory on an installed system. The installer writes the same path under the target root.
pub const CACHE_DIR: &str = "/var/cache/bifrost/first-frame";
/// The shell's assets on an installed system (`wallpaper.raw`, `wallpapers/`, `icons/`, `SFUI.ttf`).
pub const ASSETS: &str = "/usr/share/springboard/assets";
/// The file in `CACHE_DIR`.
pub const FILE: &str = "first-frame.ffc";
/// File magic and format version. A different version is never read.
pub const MAGIC: &[u8; 4] = b"BFFC";
pub const VERSION: u32 = 1;

/// Menu bar text size in points (the reference platform's headline size, `render/src/ui.rs`), scaled by the panel's scale for the atlas.
pub const MENU_GLYPH_PT: f32 = 13.0;
/// The menu bar font inside the assets directory.
pub const MENU_FONT: &str = "SFUI.ttf";
/// The printable ASCII range rasterised into the glyph atlas.
pub const GLYPH_FIRST: u32 = 0x20;
pub const GLYPH_LAST: u32 = 0x7e;

/// Limits that keep a hostile or corrupt file from allocating without bound.
const MAX_DIM: u32 = 8192;
const MAX_ICONS: u32 = 256;
const MAX_GLYPHS: u32 = 128;
const MAX_NAME: usize = 64;

/// What a cache was built for: the panel size, the layout scale in thousandths and the wallpaper's content hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub w: u32,
    pub h: u32,
    /// The layout scale in thousandths (1.5625 is 1563).
    pub scale_milli: u32,
    pub wallpaper: u64,
}

/// One rasterised icon at the Dock's size.
#[derive(Debug, Clone, PartialEq)]
pub struct Icon {
    pub name: String,
    pub size: u32,
    pub image: Image,
}

/// The decoded contents of one cache file.
#[derive(Debug, Clone, PartialEq)]
pub struct Cache {
    pub key: Key,
    pub wallpaper: Image,
    pub icons: Vec<Icon>,
    /// `(character, glyph bitmap)` pairs for `GLYPH_FIRST..=GLYPH_LAST`.
    pub glyphs: Vec<(char, Image)>,
}

/// 64-bit FNV-1a of `bytes`: the wallpaper's content hash in the key (cheap, deterministic, no dependency).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The first `WxH` line of a DRM `modes` file (the connector's preferred mode is listed first). `None` when the text
/// has no mode or a zero or oversized one.
pub fn parse_drm_mode(text: &str) -> Option<(u32, u32)> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let (w, h) = line.split_once('x')?;
    let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0 && w <= MAX_DIM && h <= MAX_DIM).then_some((w, h))
}

/// The physical size in millimetres from an EDID block: bytes 21 and 22 hold the width and height in centimetres.
/// `None` when the block is short or either size is zero.
pub fn edid_size_mm(edid: &[u8]) -> Option<(u32, u32)> {
    let (w, h) = (*edid.get(21)? as u32, *edid.get(22)? as u32);
    (w > 0 && h > 0).then_some((w * 10, h * 10))
}

/// The first connected DRM connector: its mode size and its EDID size, read from sysfs.
fn connected_panel() -> Option<((u32, u32), Option<(u32, u32)>)> {
    let mut connectors: Vec<PathBuf> = std::fs::read_dir("/sys/class/drm").ok()?.flatten().map(|e| e.path()).collect();
    connectors.sort();
    connectors.into_iter().find_map(|dir| {
        if std::fs::read_to_string(dir.join("status")).ok()?.trim() != "connected" {
            return None;
        }
        let size = parse_drm_mode(&std::fs::read_to_string(dir.join("modes")).ok()?)?;
        let mm = std::fs::read(dir.join("edid")).ok().and_then(|e| edid_size_mm(&e));
        Some((size, mm))
    })
}

/// The installed panel's size and the automatic layout scale for it. Used at install and update, never at boot. The
/// shell uses a scale chosen in Settings when there is one; then its key differs and it rebuilds on its first start.
pub fn installed_panel() -> Option<((u32, u32), f32)> {
    let (size, mm) = connected_panel()?;
    Some((size, crate::layout::auto_scale(size.0 as i32, size.1 as i32, mm)))
}

/// The wallpaper file for `id`, as the shell maps it: `wallpaper.raw` for the packaged default (empty id), else
/// `wallpapers/<id>.raw`. `None` for an id that is not a plain name.
pub fn wallpaper_path(assets: &str, id: &str) -> Option<String> {
    if id.is_empty() {
        return Some(format!("{assets}/wallpaper.raw"));
    }
    crate::installer::plan::valid_wallpaper_id(id).then(|| format!("{assets}/wallpapers/{id}.raw"))
}

/// The key for a panel, a layout scale and a wallpaper's bytes.
pub fn key_for(panel: (u32, u32), scale: f32, wallpaper_raw: &[u8]) -> Key {
    Key { w: panel.0, h: panel.1, scale_milli: (scale * 1000.0).round() as u32, wallpaper: fnv1a(wallpaper_raw) }
}

/// The Dock icon size in pixels for a panel and scale: the same figure the shell's layout gives its Dock icons.
pub fn icon_px(panel: (u32, u32), scale: f32) -> u32 {
    Layout::build(panel.0 as i32, panel.1 as i32, 0, 1.0, scale, false).icon.max(1) as u32
}

/// Encodes a cache. Layout: magic, version, key (w, h, scale, wallpaper hash), then counts and blocks. Pixels are
/// little-endian `u32`, the same as `Image`.
pub fn encode(c: &Cache) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, c.key.w);
    put_u32(&mut out, c.key.h);
    put_u32(&mut out, c.key.scale_milli);
    out.extend_from_slice(&c.key.wallpaper.to_le_bytes());
    put_u32(&mut out, c.icons.len() as u32);
    put_u32(&mut out, c.glyphs.len() as u32);
    put_image(&mut out, &c.wallpaper);
    for icon in &c.icons {
        put_u32(&mut out, icon.name.len() as u32);
        out.extend_from_slice(icon.name.as_bytes());
        put_u32(&mut out, icon.size);
        put_image(&mut out, &icon.image);
    }
    for (ch, image) in &c.glyphs {
        put_u32(&mut out, *ch as u32);
        put_image(&mut out, image);
    }
    out
}

/// Decodes a cache. `None` for a wrong magic or version, any truncated block, a dimension or count out of range, a
/// non-UTF-8 name, or trailing bytes. Never panics on hostile input.
pub fn decode(bytes: &[u8]) -> Option<Cache> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(4)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    let key = Key { w: r.u32()?, h: r.u32()?, scale_milli: r.u32()?, wallpaper: r.u64()? };
    let (n_icons, n_glyphs) = (r.u32()?, r.u32()?);
    if n_icons > MAX_ICONS || n_glyphs > MAX_GLYPHS {
        return None;
    }
    let wallpaper = r.image()?;
    let mut icons = Vec::with_capacity(n_icons as usize);
    for _ in 0..n_icons {
        let len = r.u32()? as usize;
        if len == 0 || len > MAX_NAME {
            return None;
        }
        let name = String::from_utf8(r.take(len)?.to_vec()).ok()?;
        let size = r.u32()?;
        icons.push(Icon { name, size, image: r.image()? });
    }
    let mut glyphs = Vec::with_capacity(n_glyphs as usize);
    for _ in 0..n_glyphs {
        let ch = char::from_u32(r.u32()?)?;
        glyphs.push((ch, r.image()?));
    }
    (r.at == bytes.len()).then_some(Cache { key, wallpaper, icons, glyphs })
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_image(out: &mut Vec<u8>, img: &Image) {
    put_u32(out, img.w as u32);
    put_u32(out, img.h as u32);
    for p in &img.px {
        out.extend_from_slice(&p.to_le_bytes());
    }
}

/// A bounds-checked cursor over the file bytes.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn image(&mut self) -> Option<Image> {
        let (w, h) = (self.u32()?, self.u32()?);
        if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
            return None;
        }
        let n = (w as usize).checked_mul(h as usize)?;
        let raw = self.take(n.checked_mul(4)?)?;
        let px = raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Some(Image { w: w as usize, h: h as usize, px })
    }
}

/// True when a decoded cache was built for this key.
pub fn matches(c: &Cache, key: &Key) -> bool {
    c.key == *key
}

/// The icon named `name` at `size`, if the cache holds it.
pub fn icon_in<'c>(c: &'c Cache, name: &str, size: u32) -> Option<&'c Image> {
    c.icons.iter().find(|i| i.name == name && i.size == size).map(|i| &i.image)
}

/// Builds the cache for `panel` and `scale` from the wallpaper file `wallpaper` and the shell's assets, into `dir`.
/// Every stage is best effort on its own input: a missing icon or font is skipped (the shell falls back to decoding
/// that one), but a wallpaper that does not decode is an error, since the cache is pointless without it.
pub fn build_into(dir: &Path, panel: (u32, u32), scale: f32, assets: &str, wallpaper: &str) -> io::Result<()> {
    let raw = std::fs::read(wallpaper)?;
    let src = Image::from_raw(&raw).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "wallpaper is not an image"))?;
    let key = key_for(panel, scale, &raw);
    let wallpaper_img = cover_fit(&src, panel.0 as usize, panel.1 as usize);

    let size = icon_px(panel, scale);
    let mut names: Vec<String> = std::fs::read_dir(format!("{assets}/icons"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".raw")).map(str::to_string))
        .collect();
    names.sort();
    let icons = names
        .into_iter()
        .take(MAX_ICONS as usize)
        .filter_map(|name| {
            let img = Image::from_raw(&std::fs::read(format!("{assets}/icons/{name}.raw")).ok()?)?;
            Some(Icon { image: img.resized(size as usize, size as usize), name, size })
        })
        .collect();

    let glyphs = std::fs::read(format!("{assets}/{MENU_FONT}"))
        .ok()
        .and_then(|data| {
            let font = Font::parse(&data)?;
            let st = Style { px: MENU_GLYPH_PT * scale, embolden: 0.0, color: 0xffff_ffff, shadow: false };
            Some(
                (GLYPH_FIRST..=GLYPH_LAST)
                    .filter_map(char::from_u32)
                    .map(|ch| (ch, font.render_glyph(font.glyph(ch), &st)))
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default();

    let cache = Cache { key, wallpaper: wallpaper_img, icons, glyphs };
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, encode(&cache))?;
    std::fs::rename(&tmp, dir.join(FILE))
}

/// Loads the cache in `dir` when it was built for this panel, scale and wallpaper file. `None` otherwise.
pub fn load_from(dir: &Path, panel: (u32, u32), scale: f32, wallpaper: &str) -> Option<Cache> {
    let raw = std::fs::read(wallpaper).ok()?;
    let key = key_for(panel, scale, &raw);
    let cache = decode(&std::fs::read(dir.join(FILE)).ok()?)?;
    matches(&cache, &key).then_some(cache)
}

/// Removes the cache so nothing stale is read (the updater calls this before it rebuilds).
pub fn invalidate(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(FILE));
}

/// Rebuilds the cache for this machine's panel with the packaged default wallpaper. The updater runs this after an OS
/// update (`springboard --first-frame-cache`); a shell whose wallpaper or scale differs misses and rebuilds for its own.
pub fn rebuild_default() -> io::Result<()> {
    let (panel, scale) = installed_panel().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no connected panel"))?;
    let wallpaper = wallpaper_path(ASSETS, "").ok_or_else(|| io::Error::other("no default wallpaper"))?;
    invalidate(Path::new(CACHE_DIR));
    build_into(Path::new(CACHE_DIR), panel, scale, ASSETS, &wallpaper)
}

/// Builds the cache for this shell's panel and layout scale in a background thread, into `CACHE_DIR`. The shell calls
/// this on a miss and when its wallpaper changes. A failed build leaves the shell on its decoding path.
pub fn spawn_build_here(panel: (u32, u32), assets: &str, wallpaper: &str) {
    let scale = crate::layout::base_scale(panel.0 as i32, panel.1 as i32);
    let (assets, wallpaper) = (assets.to_string(), wallpaper.to_string());
    let _ = std::thread::Builder::new().name("first-frame-cache".into()).spawn(move || {
        if let Err(e) = build_into(Path::new(CACHE_DIR), panel, scale, &assets, &wallpaper) {
            eprintln!("first-frame cache: {e}");
        }
    });
}

/// The cache the shell loaded for this boot, if any. Set once by `activate` before the shell is built.
static ACTIVE: OnceLock<Cache> = OnceLock::new();

/// Loads the cache for the shell's panel, layout scale and wallpaper and keeps it for the first frame. The layout scale
/// must already be set (`layout::set_adaptive`, `set_user_scale`). Returns true when the cache was used. On a miss the
/// shell builds it in the background so the next start hits.
pub fn activate(panel: (u32, u32), assets: &str, wallpaper: &str) -> bool {
    let scale = crate::layout::base_scale(panel.0 as i32, panel.1 as i32);
    match load_from(Path::new(CACHE_DIR), panel, scale, wallpaper) {
        Some(cache) => ACTIVE.set(cache).is_ok(),
        None => {
            spawn_build_here(panel, assets, wallpaper);
            false
        }
    }
}

/// The active cache's wallpaper, when it is the panel's size. The shell uses it instead of cover-scaling the still.
pub fn wallpaper() -> Option<&'static Image> {
    ACTIVE.get().map(|c| &c.wallpaper)
}

/// The active cache's icon, or `None` so the caller decodes it.
pub fn cached_icon(name: &str, size: usize) -> Option<Image> {
    ACTIVE.get().and_then(|c| icon_in(c, name, size as u32)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, v: u32) -> Image {
        Image { w, h, px: vec![v; w * h] }
    }

    fn sample() -> Cache {
        Cache {
            key: Key { w: 4, h: 2, scale_milli: 1500, wallpaper: 0xdead_beef },
            wallpaper: img(4, 2, 0xff00_00ff),
            icons: vec![Icon { name: "dock".into(), size: 3, image: img(3, 3, 7) }],
            glyphs: vec![('A', img(2, 1, 9))],
        }
    }

    #[test]
    fn fnv1a_matches_the_published_test_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv1a(b"wallpaper-a"), fnv1a(b"wallpaper-b"));
    }

    #[test]
    fn drm_modes_take_the_first_line_and_reject_bad_sizes() {
        assert_eq!(parse_drm_mode("1920x1080\n1280x720\n"), Some((1920, 1080)));
        assert_eq!(parse_drm_mode("\n  2560x1600 \n"), Some((2560, 1600)));
        assert_eq!(parse_drm_mode(""), None);
        assert_eq!(parse_drm_mode("0x720"), None);
        assert_eq!(parse_drm_mode("1920x"), None);
        assert_eq!(parse_drm_mode("99999x1080"), None);
    }

    #[test]
    fn edid_size_is_centimetres_at_bytes_21_and_22() {
        let mut e = vec![0u8; 128];
        e[21] = 60;
        e[22] = 34;
        assert_eq!(edid_size_mm(&e), Some((600, 340)));
        e[22] = 0;
        assert_eq!(edid_size_mm(&e), None);
        assert_eq!(edid_size_mm(&e[..22]), None);
    }

    #[test]
    fn wallpaper_paths_follow_the_shell_and_refuse_bad_ids() {
        assert_eq!(wallpaper_path("/a", ""), Some("/a/wallpaper.raw".into()));
        assert_eq!(wallpaper_path("/a", "iceland_3"), Some("/a/wallpapers/iceland_3.raw".into()));
        assert_eq!(wallpaper_path("/a", "../etc"), None);
        assert_eq!(wallpaper_path("/a", "x/y"), None);
    }

    #[test]
    fn the_key_carries_the_scale_in_thousandths() {
        let k = key_for((1920, 1080), 1.5625, &[]);
        assert_eq!(k.scale_milli, 1563);
        assert_eq!((k.w, k.h), (1920, 1080));
        assert_eq!(k.wallpaper, fnv1a(&[]));
        assert_eq!(key_for((1920, 1080), 1.0, &[]).scale_milli, 1000);
    }

    #[test]
    fn the_dock_icon_follows_the_scale() {
        // 64 pt at scale 1.0 is 64 px; at 1.5625 it is 100 px
        assert_eq!(icon_px((1920, 1080), 1.0), 64);
        assert_eq!(icon_px((1920, 1080), 1.5625), 100);
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let c = sample();
        let bytes = encode(&c);
        assert_eq!(&bytes[..4], MAGIC);
        assert_eq!(decode(&bytes), Some(c));
    }

    #[test]
    fn decode_rejects_wrong_magic_version_truncation_and_trailing_bytes() {
        let bytes = encode(&sample());
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(decode(&magic), None);
        let mut version = bytes.clone();
        version[4] = 2;
        assert_eq!(decode(&version), None);
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None);
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(decode(&trailing), None);
        assert_eq!(decode(&[]), None);
    }

    #[test]
    fn decode_rejects_oversized_counts_and_dimensions_without_allocating() {
        let mut bytes = encode(&sample());
        // icon count at offset 4 + 4 + 4 + 4 + 4 + 8 = 28
        bytes[28..32].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(decode(&bytes), None);
        // wallpaper width at offset 36 (after the glyph count at 32)
        let mut huge = encode(&sample());
        huge[36..40].copy_from_slice(&(MAX_DIM + 1).to_le_bytes());
        assert_eq!(decode(&huge), None);
    }

    #[test]
    fn a_key_mismatch_is_a_miss() {
        let c = sample();
        let same = Key { w: 4, h: 2, scale_milli: 1500, wallpaper: 0xdead_beef };
        assert!(matches(&c, &same));
        assert!(!matches(&c, &Key { w: 1920, ..same }));
        assert!(!matches(&c, &Key { scale_milli: 1000, ..same }));
        assert!(!matches(&c, &Key { wallpaper: 1, ..same }));
    }

    #[test]
    fn icons_are_found_by_name_and_size_only() {
        let c = sample();
        assert!(icon_in(&c, "dock", 3).is_some());
        assert!(icon_in(&c, "dock", 4).is_none());
        assert!(icon_in(&c, "other", 3).is_none());
    }

    #[test]
    fn a_cache_file_loads_only_for_its_own_panel_scale_and_wallpaper() {
        let dir = std::env::temp_dir().join(format!("ffc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wp_bytes = {
            let mut v = b"RIMG".to_vec();
            v.extend(2u32.to_le_bytes());
            v.extend(2u32.to_le_bytes());
            v.extend(std::iter::repeat(0x11u8).take(16));
            v
        };
        let wp = dir.join("wp.raw");
        std::fs::write(&wp, &wp_bytes).unwrap();
        let key = key_for((1200, 1920), 1.0, &wp_bytes);
        let c = Cache { key, wallpaper: img(1200, 1920, 5), icons: vec![], glyphs: vec![] };
        std::fs::write(dir.join(FILE), encode(&c)).unwrap();
        let wp_str = wp.to_str().unwrap();
        assert!(load_from(&dir, (1200, 1920), 1.0, wp_str).is_some());
        assert!(load_from(&dir, (1920, 1080), 1.0, wp_str).is_none());
        assert!(load_from(&dir, (1200, 1920), 1.25, wp_str).is_none());
        invalidate(&dir);
        assert!(load_from(&dir, (1200, 1920), 1.0, wp_str).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
