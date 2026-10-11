//! Site icons (Browser): tabs, the idle address field and the suggestion list show each site's icon at 16 pt.
//!
//! After a page loads the app evals `FAV_JS`, which lists the page's `link rel=icon / apple-touch-icon`
//! hrefs (SVG skipped: the SDK has no rasterizer for it), best size first. A thread then fetches the
//! candidates with curl, then `/favicon.ico`, decodes PNG or ICO (PNG or BMP frames), reduces to 32x32 and
//! caches it as `<profile>/favicons/<host>.png` (+ `<host>.url`). Icons are keyed by host, so a host seen
//! earlier draws before its page has loaded. Private windows keep icons in memory only.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

use bifrost_app::kit::{rgba, Image};
use bifrost_app::webview::WebViewId;

pub const FAV_PREFIX: &str = "bifrost-fav:";
/// `bifrost-fav:<page url> <icon href>...`, best candidate first, http(s) and non-SVG only.
pub const FAV_JS: &str = "(function(){var a=[];document.querySelectorAll('link[rel]').forEach(function(l){var r=(l.rel||'').toLowerCase();if(!/(^|\\s)(icon|shortcut|apple-touch-icon|apple-touch-icon-precomposed)(\\s|$)/.test(r))return;var h=l.href;if(!h||!/^https?:/.test(h)||/\\.svg([?#]|$)/i.test(h)||(l.type||'').indexOf('svg')>=0)return;var s=parseInt(((l.getAttribute('sizes')||'').split('x')[0]))||0;a.push([s==0?40:(s>=32?s-32:100+32-s),h]);});a.sort(function(x,y){return x[0]-y[0]});return 'bifrost-fav:'+location.href+' '+a.map(function(e){return e[1]}).join(' ')})()";
/// Stored size in pixels; drawn scaled to `16 * ui scale`.
pub const STORE_PX: usize = 32;
/// Icon size and its gap to the text, points (Browser draws 16 pt).
pub const ICON_PT: f32 = 16.0;
pub const GAP_PT: f32 = 6.0;
const MAX_AGE_SECS: u64 = 7 * 86_400;
const MAX_CANDIDATES: usize = 4;

struct Icon {
    img: Image,
    url: String,
    /// The last size it was drawn at, kept so a frame does not rescale.
    scaled: Option<(usize, Image)>,
}

type Fetched = (String, Option<(Vec<u8>, String)>);

pub struct Favicons {
    dir: Option<PathBuf>,
    cache: HashMap<String, Option<Icon>>,
    views: HashMap<WebViewId, String>,
    pending: HashSet<String>,
    tx: Sender<Fetched>,
    rx: Receiver<Fetched>,
}

impl Default for Favicons {
    fn default() -> Self {
        Favicons::new(Some(crate::store::data_dir().join("favicons")))
    }
}

/// Cache key: lower-case authority without credentials or a leading `www.`; None for non-web URLs.
pub fn host_key(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let auth = rest.split(['/', '?', '#']).next()?;
    let auth = auth.rsplit('@').next()?.to_ascii_lowercase();
    let auth = auth.strip_prefix("www.").unwrap_or(&auth).to_string();
    (!auth.is_empty()).then_some(auth)
}

fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let auth = rest.split(['/', '?', '#']).next()?;
    Some(format!("{scheme}://{auth}"))
}

fn file_stem(host: &str) -> String {
    host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

impl Favicons {
    pub fn new(dir: Option<PathBuf>) -> Favicons {
        let (tx, rx) = channel();
        Favicons { dir, cache: HashMap::new(), views: HashMap::new(), pending: HashSet::new(), tx, rx }
    }

    /// A private window keeps its icons in memory only.
    pub fn set_private(&mut self, private: bool) {
        if private {
            self.dir = None;
        }
    }

    fn load_disk(&self, host: &str) -> Option<(Icon, bool)> {
        let dir = self.dir.as_ref()?;
        let stem = file_stem(host);
        let path = dir.join(format!("{stem}.png"));
        let bytes = std::fs::read(&path).ok()?;
        let (w, h, px) = decode_png(&bytes)?;
        let img = to_image(w, h, &px);
        let url = std::fs::read_to_string(dir.join(format!("{stem}.url"))).unwrap_or_default();
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(u64::MAX, |d| d.as_secs());
        Some((Icon { img, url: url.trim().to_string(), scaled: None }, age < MAX_AGE_SECS))
    }

    /// The icon of `host` (from memory, else the disk cache) as `px` x `px` premultiplied pixels.
    pub fn get(&mut self, host: &str, px: usize) -> Option<&Image> {
        if !self.cache.contains_key(host) {
            let loaded = self.load_disk(host).map(|(i, _)| i);
            self.cache.insert(host.to_string(), loaded);
        }
        let icon = self.cache.get_mut(host)?.as_mut()?;
        if px == icon.img.w && px == icon.img.h {
            return Some(&icon.img);
        }
        if icon.scaled.as_ref().is_none_or(|(p, _)| *p != px) {
            icon.scaled = Some((px, icon.img.resized(px, px)));
        }
        icon.scaled.as_ref().map(|(_, i)| i)
    }

    /// The icon URL last used for the tab's host (None until one was found), for `browser.favicon`.
    pub fn url_of(&mut self, view: WebViewId, page_url: &str) -> Option<String> {
        let host = self.views.get(&view).cloned().or_else(|| host_key(page_url))?;
        self.get(&host, STORE_PX)?;
        self.cache.get(&host)?.as_ref().map(|i| i.url.clone())
    }

    pub fn forget(&mut self, view: WebViewId) {
        self.views.remove(&view);
    }

    /// The answer to `FAV_JS`; true when it was one (the caller then hides it from the console).
    pub fn on_eval(&mut self, view: WebViewId, text: &str) -> bool {
        let Some(body) = text.trim().trim_matches('"').strip_prefix(FAV_PREFIX) else { return false };
        let mut parts = body.split_whitespace();
        let Some(page) = parts.next() else { return true };
        let Some(host) = host_key(page) else { return true };
        let mut cands: Vec<String> = parts.map(|s| s.replace("\\/", "/")).filter(|s| s.starts_with("http")).take(MAX_CANDIDATES).collect();
        if let Some(o) = origin(page) {
            cands.push(format!("{o}/favicon.ico"));
        }
        self.views.insert(view, host.clone());
        if self.pending.contains(&host) {
            return true;
        }
        let fresh = match self.cache.get(&host) {
            Some(Some(_)) => true,
            Some(None) => false,
            None => self.load_disk(&host).is_some_and(|(_, fresh)| fresh),
        };
        if fresh {
            return true;
        }
        self.pending.insert(host.clone());
        let (tx, dir) = (self.tx.clone(), self.dir.clone());
        std::thread::spawn(move || {
            let found = cands.iter().find_map(|u| fetch(u).map(|rgba| (rgba, u.clone())));
            if let (Some((px, url)), Some(dir)) = (&found, &dir) {
                let stem = file_stem(&host);
                if std::fs::create_dir_all(dir).is_ok() {
                    let _ = std::fs::write(dir.join(format!("{stem}.png")), encode_png(STORE_PX, STORE_PX, px));
                    let _ = std::fs::write(dir.join(format!("{stem}.url")), url);
                }
            }
            let _ = tx.send((host, found));
            bifrost_app::dispatch::DispatchQueue::main().async_(|| {});
        });
        true
    }

    /// Collects finished fetches; true when something new can be drawn.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok((host, found)) = self.rx.try_recv() {
            self.pending.remove(&host);
            match found {
                Some((px, url)) => {
                    self.cache.insert(host, Some(Icon { img: to_image(STORE_PX, STORE_PX, &px), url, scaled: None }));
                    changed = true;
                }
                None => {
                    // keep a stale disk icon; remember the miss otherwise (no retry this session)
                    self.cache.entry(host).or_insert(None);
                }
            }
        }
        changed
    }
}

fn to_image(w: usize, h: usize, px: &[u8]) -> Image {
    let mut img = Image::new(w, h);
    for (d, p) in img.px.iter_mut().zip(px.chunks_exact(4)) {
        *d = rgba(p[0], p[1], p[2], p[3]);
    }
    img
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    let out = Command::new("curl")
        .args(["-sSfL", "-m", "8", "--max-redirs", "5", "--max-filesize", "1048576", "-A", crate::downloads::DEFAULT_UA])
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    decode_icon(&out.stdout)
}

/// PNG or ICO to straight RGBA at `STORE_PX` x `STORE_PX`.
pub fn decode_icon(bytes: &[u8]) -> Option<Vec<u8>> {
    let (w, h, px) = if bytes.starts_with(&PNG_MAGIC) { decode_png(bytes)? } else { decode_ico(bytes)? };
    Some(reduce(w, h, &px, STORE_PX))
}

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    use png::{BitDepth, ColorType, Transformations};
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut r = d.read_info().ok()?;
    let (w, h) = (r.info().width as usize, r.info().height as usize);
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return None;
    }
    let mut buf = vec![0u8; r.output_buffer_size()?];
    let out = r.next_frame(&mut buf).ok()?;
    if out.bit_depth != BitDepth::Eight {
        return None;
    }
    let buf = &buf[..out.buffer_size()];
    let px: Vec<u8> = match out.color_type {
        ColorType::Rgba => buf.to_vec(),
        ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ColorType::Indexed => return None,
    };
    (px.len() == w * h * 4).then_some((w, h, px))
}

fn le16(b: &[u8], i: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as usize)
}
fn le32(b: &[u8], i: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize)
}

/// The frame closest to `STORE_PX` (smallest at least that big, else the largest) of an ICO.
fn decode_ico(b: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    if le16(b, 0)? != 0 || le16(b, 2)? != 1 {
        return None;
    }
    let n = le16(b, 4)?.min(64);
    let mut frames: Vec<(usize, usize, usize)> = (0..n)
        .filter_map(|i| {
            let e = 6 + i * 16;
            let w = *b.get(e)? as usize;
            Some(((if w == 0 { 256 } else { w }), le32(b, e + 8)?, le32(b, e + 12)?))
        })
        .collect();
    frames.sort_by_key(|&(w, _, _)| if w >= STORE_PX { w - STORE_PX } else { 1000 + STORE_PX - w });
    frames.into_iter().find_map(|(_, size, off)| {
        let data = b.get(off..off.checked_add(size)?)?;
        if data.starts_with(&PNG_MAGIC) {
            decode_png(data)
        } else {
            decode_dib(data)
        }
    })
}

/// A BMP frame inside an ICO (32 or 24 bits per pixel, bottom-up, height counts the AND mask).
fn decode_dib(d: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let hdr = le32(d, 0)?;
    let w = le32(d, 4)?;
    let h = le32(d, 8)? / 2;
    let bpp = le16(d, 14)?;
    if hdr < 40 || w == 0 || h == 0 || w > 256 || h > 256 || le32(d, 16)? != 0 || (bpp != 32 && bpp != 24) {
        return None;
    }
    let base = hdr;
    let row = (w * bpp / 8).div_ceil(4) * 4;
    let mask_row = w.div_ceil(32) * 4;
    let mask_base = base + row * h;
    let mut px = vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        let sy = h - 1 - y;
        for x in 0..w {
            let o = base + sy * row + x * bpp / 8;
            let s = d.get(o..o + bpp / 8)?;
            let a = if bpp == 32 { s[3] } else { 255 };
            any_alpha |= a != 0;
            px[(y * w + x) * 4..][..4].copy_from_slice(&[s[2], s[1], s[0], a]);
        }
    }
    if bpp == 24 {
        for y in 0..h {
            let sy = h - 1 - y;
            for x in 0..w {
                let bit = d.get(mask_base + sy * mask_row + x / 8).map_or(0, |m| (m >> (7 - x % 8)) & 1);
                if bit == 1 {
                    px[(y * w + x) * 4 + 3] = 0;
                }
            }
        }
    } else if !any_alpha {
        for p in px.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    Some((w, h, px))
}

/// Area-average (shrink) or nearest (grow) to `n` x `n`, straight RGBA in and out; averages are alpha-weighted.
fn reduce(w: usize, h: usize, px: &[u8], n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        let (y0, y1) = (y * h / n, ((y + 1) * h / n).max(y * h / n + 1).min(h));
        for x in 0..n {
            let (x0, x1) = (x * w / n, ((x + 1) * w / n).max(x * w / n + 1).min(w));
            let (mut c, mut a, mut cnt) = ([0u32; 3], 0u32, 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &px[(sy * w + sx) * 4..][..4];
                    let al = p[3] as u32;
                    for k in 0..3 {
                        c[k] += p[k] as u32 * al;
                    }
                    a += al;
                    cnt += 1;
                }
            }
            let o = &mut out[(y * n + x) * 4..][..4];
            if a > 0 {
                for k in 0..3 {
                    o[k] = (c[k] / a) as u8;
                }
                o[3] = (a / cnt) as u8;
            }
        }
    }
    out
}

fn encode_png(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w as u32, h as u32);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    if let Ok(mut wr) = e.write_header() {
        let _ = wr.write_image_data(px);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_key_strips_www_and_credentials() {
        assert_eq!(host_key("https://User@WWW.Example.com:8080/a?b#c").as_deref(), Some("example.com:8080"));
        assert_eq!(host_key("about:blank"), None);
        assert_eq!(host_key("file:///x"), None);
    }

    #[test]
    fn png_round_trips_through_the_decoder_and_reduces() {
        let src: Vec<u8> = (0..64 * 64).flat_map(|_| [10u8, 200, 30, 255]).collect();
        let png = encode_png(64, 64, &src);
        let px = decode_icon(&png).unwrap();
        assert_eq!(px.len(), STORE_PX * STORE_PX * 4);
        assert_eq!(&px[..4], &[10, 200, 30, 255]);
    }

    #[test]
    fn ico_with_a_png_frame_decodes() {
        let png = encode_png(16, 16, &vec![255u8; 16 * 16 * 4]);
        let mut ico = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
        ico.extend((png.len() as u32).to_le_bytes());
        ico.extend(22u32.to_le_bytes());
        ico.extend(&png);
        assert_eq!(decode_icon(&ico).unwrap().len(), STORE_PX * STORE_PX * 4);
    }

    #[test]
    fn ico_with_a_32bit_bmp_frame_decodes() {
        let mut dib = Vec::new();
        for v in [40u32, 2, 4, 0] {
            dib.extend(v.to_le_bytes());
        }
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib.resize(40, 0);
        dib.extend([0, 0, 255, 255].repeat(4)); // red, opaque
        dib.extend([0u8; 8]); // AND mask
        let mut ico = vec![0, 0, 1, 0, 1, 0, 2, 2, 0, 0, 1, 0, 32, 0];
        ico.extend((dib.len() as u32).to_le_bytes());
        ico.extend(22u32.to_le_bytes());
        ico.extend(&dib);
        let px = decode_icon(&ico).unwrap();
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn garbage_is_refused() {
        assert!(decode_icon(b"<html>nope").is_none());
    }
}
