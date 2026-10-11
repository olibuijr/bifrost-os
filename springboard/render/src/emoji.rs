//! Color emoji from a CBDT/CBLC bitmap font (Noto Color Emoji): `cmap`
//! (formats 4 and 12), CBLC index subtables (formats 1, 2, 3), CBDT glyph
//! formats 17, 18 and 19 (PNG), and the GSUB ligature lookups (type 4, also
//! through extension type 7) of the `ccmp`/`liga`/`rlig` features, which
//! build ZWJ sequences, skin tones, flags and keycaps. A sequence the font
//! has no ligature for falls back to its first glyph that has a bitmap.
//! `/system` is only ever read.

use crate::gfx::Image;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

/// The keyboard system package's emoji font (`is.olibuijr.keyboard`), tried before `PATHS`.
pub const PACKAGE_FONT: &str = "/data/local/springboard/apps/is.olibuijr.keyboard/current/assets/NotoColorEmoji.ttf";

/// Font files tried after the package font; each is loaded lazily. The system
/// `/system/fonts/NotoColorEmoji.ttf` is COLRv1 (no CBDT tables): it fails to parse and is skipped.
pub const PATHS: [&str; 2] = ["/system/fonts/NotoColorEmoji.ttf", "/system/fonts/NotoColorEmojiFlags.ttf"];

/// Rendered images kept (cluster, px).
pub const CACHE_CAP: usize = 256;

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// One parsed font file.
pub struct Face {
    data: Vec<u8>,
    cmap: Vec<(u16, usize)>, // (format, subtable offset), 12 first
    cbdt: usize,
    strike: Option<Strike>,
    /// ligature lookups (offsets of LookupType 4 subtables), in lookup order
    ligs: Vec<usize>,
}

#[derive(Clone, Copy)]
struct Strike {
    arr: usize,
    count: usize,
    ppem: u8,
}

impl Face {
    pub fn parse(data: Vec<u8>) -> Option<Face> {
        let n = u16_at(&data, 4)? as usize;
        let table = |tag: &[u8; 4]| -> Option<usize> {
            (0..n).find_map(|i| {
                let r = 12 + i * 16;
                (data.get(r..r + 4)? == tag).then(|| u32_at(&data, r + 8).map(|o| o as usize)).flatten()
            })
        };
        let cmap_t = table(b"cmap")?;
        let (cblc, cbdt) = (table(b"CBLC")?, table(b"CBDT")?);
        let mut cmap = vec![];
        for i in 0..u16_at(&data, cmap_t + 2)? as usize {
            let r = cmap_t + 4 + i * 8;
            let off = cmap_t + u32_at(&data, r + 4)? as usize;
            let fmt = u16_at(&data, off)?;
            if fmt == 4 || fmt == 12 {
                cmap.push((fmt, off));
            }
        }
        cmap.sort_by_key(|c| std::cmp::Reverse(c.0));
        if cmap.is_empty() {
            return None;
        }
        // the largest strike; PNG strikes are scaled down from it
        let sizes = u32_at(&data, cblc + 4)? as usize;
        let mut strike: Option<Strike> = None;
        for i in 0..sizes {
            let r = cblc + 8 + i * 48;
            let (arr, count, ppem) = (cblc + u32_at(&data, r)? as usize, u32_at(&data, r + 8)? as usize, *data.get(r + 44)?);
            if strike.is_none_or(|s| ppem > s.ppem) {
                strike = Some(Strike { arr, count, ppem });
            }
        }
        let gsub = table(b"GSUB");
        let mut face = Face { data, cmap, cbdt, strike, ligs: vec![] };
        face.ligs = face.gsub_ligatures(gsub).unwrap_or_default();
        Some(face)
    }

    /// Glyph for a code point, 0 when unmapped.
    pub fn glyph(&self, cp: u32) -> u16 {
        self.cmap.iter().find_map(|&(f, o)| self.cmap_lookup(f, o, cp)).unwrap_or(0)
    }

    fn cmap_lookup(&self, fmt: u16, o: usize, cp: u32) -> Option<u16> {
        let d = &self.data;
        let g = if fmt == 12 {
            let n = u32_at(d, o + 12)? as usize;
            let (mut lo, mut hi) = (0, n);
            let mut found = None;
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = o + 16 + mid * 12;
                let (s, e, g) = (u32_at(d, r)?, u32_at(d, r + 4)?, u32_at(d, r + 8)?);
                if cp < s {
                    hi = mid;
                } else if cp > e {
                    lo = mid + 1;
                } else {
                    found = Some(g + (cp - s));
                    break;
                }
            }
            found? as u16
        } else {
            if cp > 0xFFFF {
                return None;
            }
            let segx2 = u16_at(d, o + 6)? as usize;
            let (ends, starts) = (o + 14, o + 16 + segx2);
            let (deltas, ranges) = (starts + segx2, starts + 2 * segx2);
            let i = (0..segx2 / 2).find(|&i| u16_at(d, ends + i * 2).is_some_and(|e| e as u32 >= cp))?;
            let start = u16_at(d, starts + i * 2)? as u32;
            if cp < start {
                return None;
            }
            let (delta, ro) = (u16_at(d, deltas + i * 2)?, u16_at(d, ranges + i * 2)? as usize);
            if ro == 0 {
                (cp as u16).wrapping_add(delta)
            } else {
                let g = u16_at(d, ranges + i * 2 + ro + (cp - start) as usize * 2)?;
                if g == 0 {
                    return None;
                }
                g.wrapping_add(delta)
            }
        };
        (g != 0).then_some(g)
    }

    /// PNG bytes of a glyph's bitmap in the chosen strike.
    pub fn bitmap(&self, gid: u16) -> Option<&[u8]> {
        let d = &self.data;
        let st = self.strike?;
        for i in 0..st.count {
            let e = st.arr + i * 8;
            let (first, last, add) = (u16_at(d, e)?, u16_at(d, e + 2)?, u32_at(d, e + 4)? as usize);
            if gid < first || gid > last {
                continue;
            }
            let sub = st.arr + add;
            let (ifmt, imgfmt, base) = (u16_at(d, sub)?, u16_at(d, sub + 2)?, u32_at(d, sub + 4)? as usize);
            let k = (gid - first) as usize;
            let (start, end) = match ifmt {
                1 => (u32_at(d, sub + 8 + k * 4)? as usize, u32_at(d, sub + 12 + k * 4)? as usize),
                3 => (u16_at(d, sub + 8 + k * 2)? as usize, u16_at(d, sub + 10 + k * 2)? as usize),
                2 => {
                    let sz = u32_at(d, sub + 8)? as usize;
                    (k * sz, (k + 1) * sz)
                }
                _ => return None,
            };
            if end <= start {
                return None; // no bitmap for this glyph
            }
            let g = self.cbdt + base + start;
            let skip = match imgfmt {
                17 => 5,
                18 => 8,
                19 => 0,
                _ => return None,
            };
            let len = u32_at(d, g + skip)? as usize;
            return d.get(g + skip + 4..g + skip + 4 + len);
        }
        None
    }

    fn gsub_ligatures(&self, gsub: Option<usize>) -> Option<Vec<usize>> {
        let d = &self.data;
        let g = gsub?;
        let (flist, llist) = (g + u16_at(d, g + 6)? as usize, g + u16_at(d, g + 8)? as usize);
        let mut idx = vec![];
        for i in 0..u16_at(d, flist)? as usize {
            let r = flist + 2 + i * 6;
            if matches!(d.get(r..r + 4)?, b"ccmp" | b"liga" | b"rlig") {
                let ft = flist + u16_at(d, r + 4)? as usize;
                for j in 0..u16_at(d, ft + 2)? as usize {
                    idx.push(u16_at(d, ft + 4 + j * 2)?);
                }
            }
        }
        idx.sort_unstable();
        idx.dedup();
        let mut out = vec![];
        for li in idx {
            let lt = llist + u16_at(d, llist + 2 + li as usize * 2)? as usize;
            let ty = u16_at(d, lt)?;
            for s in 0..u16_at(d, lt + 4)? as usize {
                let mut st = lt + u16_at(d, lt + 6 + s * 2)? as usize;
                let mut t = ty;
                if t == 7 {
                    t = u16_at(d, st + 2)?;
                    st += u32_at(d, st + 4)? as usize;
                }
                if t == 4 {
                    out.push(st);
                }
            }
        }
        Some(out)
    }

    fn coverage(&self, o: usize, gid: u16) -> Option<usize> {
        let d = &self.data;
        match u16_at(d, o)? {
            1 => {
                let n = u16_at(d, o + 2)? as usize;
                (0..n).find(|&i| u16_at(d, o + 4 + i * 2) == Some(gid))
            }
            2 => {
                for i in 0..u16_at(d, o + 2)? as usize {
                    let r = o + 4 + i * 6;
                    let (s, e, si) = (u16_at(d, r)?, u16_at(d, r + 2)?, u16_at(d, r + 4)? as usize);
                    if gid >= s && gid <= e {
                        return Some(si + (gid - s) as usize);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Applies one ligature subtable over `gids`.
    fn apply_lig(&self, st: usize, gids: &mut Vec<u16>) -> Option<()> {
        let d = &self.data;
        let cov = st + u16_at(d, st + 2)? as usize;
        let mut i = 0;
        while i < gids.len() {
            let hit = self.coverage(cov, gids[i]).and_then(|ci| {
                let set = st + u16_at(d, st + 6 + ci * 2)? as usize;
                for l in 0..u16_at(d, set)? as usize {
                    let lig = set + u16_at(d, set + 2 + l * 2)? as usize;
                    let (out, nc) = (u16_at(d, lig)?, u16_at(d, lig + 2)? as usize);
                    if nc >= 1 && i + nc <= gids.len() && (1..nc).all(|c| u16_at(d, lig + 4 + (c - 1) * 2) == Some(gids[i + c])) {
                        return Some((out, nc));
                    }
                }
                None
            });
            if let Some((out, nc)) = hit {
                gids[i] = out;
                gids.drain(i + 1..i + nc);
            }
            i += 1;
        }
        Some(())
    }

    /// The glyph for a whole cluster (ligatures applied), if it has a bitmap.
    pub fn cluster_glyph(&self, cluster: &str) -> Option<u16> {
        let mut gids: Vec<u16> = cluster.chars().map(|c| self.glyph(c as u32)).collect();
        if gids.iter().all(|&g| g == 0) {
            return None;
        }
        for &st in &self.ligs {
            self.apply_lig(st, &mut gids);
        }
        if gids.len() == 1 && gids[0] != 0 && self.bitmap(gids[0]).is_some() {
            return Some(gids[0]);
        }
        // no ligature: the base emoji, i.e. the first glyph with a bitmap
        gids.into_iter().find(|&g| g != 0 && self.bitmap(g).is_some())
    }
}

/// Font files in search order: the package font first, then the `/system` paths.
pub fn search_paths(package_font: &Path) -> Vec<PathBuf> {
    std::iter::once(package_font.to_path_buf()).chain(PATHS.iter().map(PathBuf::from)).collect()
}

/// Parses every readable CBDT font among `paths`, in order.
pub fn load_faces(paths: &[PathBuf]) -> Vec<Face> {
    paths.iter().filter_map(|p| std::fs::read(p).ok().and_then(Face::parse)).collect()
}

/// `None` until first use or after a reload; `Some` (possibly empty) once searched.
static FACES: RwLock<Option<Arc<Vec<Face>>>> = RwLock::new(None);
/// Bumped by every reload so text caches elsewhere can drop images with stale emoji.
static GENERATION: AtomicU32 = AtomicU32::new(0);

fn faces() -> Arc<Vec<Face>> {
    if let Some(f) = FACES.read().ok().and_then(|g| g.clone()) {
        return f;
    }
    let loaded = Arc::new(load_faces(&search_paths(Path::new(PACKAGE_FONT))));
    match FACES.write() {
        Ok(mut g) => g.get_or_insert(loaded).clone(),
        Err(_) => loaded,
    }
}

/// Searches again (after the keyboard package installed or updated) and drops rendered emoji.
pub fn reload() {
    reload_from(Path::new(PACKAGE_FONT));
}

pub fn reload_from(package_font: &Path) {
    let loaded = Arc::new(load_faces(&search_paths(package_font)));
    if let Ok(mut g) = FACES.write() {
        *g = Some(loaded);
    }
    cache().remove_all();
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Changes whenever the emoji font changed.
pub fn generation() -> u32 {
    GENERATION.load(Ordering::Relaxed)
}

#[cfg(any(test, feature = "testkit"))]
thread_local! {
    pub static FORCE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// True when an emoji font is installed (the keyboard's emoji page needs it).
pub fn available() -> bool {
    #[cfg(any(test, feature = "testkit"))]
    if let Some(v) = FORCE.with(|f| f.get()) {
        return v;
    }
    !faces().is_empty()
}

/// Code points that draw as emoji when the UI font has no glyph.
pub fn is_emoji_char(c: char) -> bool {
    matches!(c as u32,
        0x203C | 0x2049 | 0x20E3 | 0x2122 | 0x2139 | 0x2194..=0x21AA | 0x231A..=0x23FF | 0x24C2 | 0x25AA..=0x25FE
        | 0x2600..=0x27BF | 0x2934 | 0x2935 | 0x2B05..=0x2B55 | 0x3030 | 0x303D | 0x3297 | 0x3299
        | 0x1F000..=0x1FAFF | 0xFE0F | 0x200D)
}

/// Splits `text` into runs: `(run, true)` for an emoji cluster the font can
/// draw, `(run, false)` for everything else. `has_glyph` tells which chars the
/// UI font already covers (those stay text).
pub fn segments<'a>(text: &'a str, has_glyph: &dyn Fn(char) -> bool) -> Vec<(&'a str, bool)> {
    let mut out: Vec<(&str, bool)> = vec![];
    if !available() || !text.chars().any(|c| is_emoji_char(c) && c as u32 > 0x7F) {
        return vec![(text, false)];
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let (mut i, mut plain) = (0, 0usize);
    let push_plain = |out: &mut Vec<(&'a str, bool)>, from: usize, to: usize| {
        if to > from {
            out.push((&text[from..to], false));
        }
    };
    while i < chars.len() {
        let c = chars[i].1;
        let starts = (is_emoji_char(c) && c as u32 > 0x7F && !has_glyph(c) && !matches!(c as u32, 0xFE0F | 0x200D | 0x20E3))
            || (c.is_ascii_digit() || c == '#' || c == '*') && chars.get(i + 1).is_some_and(|n| n.1 == '\u{20E3}' || (n.1 == '\u{FE0F}' && chars.get(i + 2).is_some_and(|m| m.1 == '\u{20E3}')));
        if !starts {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let regional = |c: char| (0x1F1E6..=0x1F1FF).contains(&(c as u32));
        if regional(c) && chars.get(j).is_some_and(|n| regional(n.1)) {
            j += 1;
        }
        while let Some(&(_, n)) = chars.get(j) {
            let u = n as u32;
            if matches!(u, 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F) {
                j += 1;
            } else if u == 0x200D && chars.get(j + 1).is_some_and(|m| is_emoji_char(m.1)) {
                j += 2;
            } else {
                break;
            }
        }
        let (from, to) = (chars[i].0, chars.get(j).map_or(text.len(), |c| c.0));
        if has_cluster(&text[from..to]) {
            push_plain(&mut out, plain, from);
            out.push((&text[from..to], true));
            plain = to;
        }
        i = j;
    }
    push_plain(&mut out, plain, text.len());
    out
}

fn has_cluster(cluster: &str) -> bool {
    #[cfg(any(test, feature = "testkit"))]
    if FORCE.with(|f| f.get()) == Some(true) {
        return true;
    }
    faces().iter().any(|f| f.cluster_glyph(cluster).is_some())
}

/// Area-average (down) or bilinear (up) resample of premultiplied pixels to `w` x `h`.
pub fn resample(src: &Image, w: usize, h: usize) -> Image {
    let mut out = Image::new(w.max(1), h.max(1));
    if src.w == 0 || src.h == 0 {
        return out;
    }
    let (fx, fy) = (src.w as f32 / out.w as f32, src.h as f32 / out.h as f32);
    for y in 0..out.h {
        for x in 0..out.w {
            let (mut acc, mut wsum) = ([0f32; 4], 0f32);
            let (x0, x1) = (x as f32 * fx, ((x + 1) as f32 * fx).max(x as f32 * fx + 1.0));
            let (y0, y1) = (y as f32 * fy, ((y + 1) as f32 * fy).max(y as f32 * fy + 1.0));
            let mut sy = y0.floor() as usize;
            while (sy as f32) < y1 && sy < src.h {
                let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                let mut sx = x0.floor() as usize;
                while (sx as f32) < x1 && sx < src.w {
                    let wx = (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    let p = src.px[sy * src.w + sx];
                    let k = wx * wy;
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += ((p >> (8 * c)) & 255) as f32 * k;
                    }
                    wsum += k;
                    sx += 1;
                }
                sy += 1;
            }
            if wsum > 0.0 {
                let c = |i: usize| (acc[i] / wsum + 0.5).min(255.0) as u8;
                out.px[y * out.w + x] = crate::gfx::pack(c(0), c(1), c(2), c(3));
            }
        }
    }
    out
}

/// Rendered clusters: at most `CACHE_CAP` entries, costed in bytes, dropped under memory pressure.
fn cache() -> &'static crate::purgeable::Cache<(String, u32), Image> {
    static C: OnceLock<crate::purgeable::Cache<(String, u32), Image>> = OnceLock::new();
    C.get_or_init(|| crate::purgeable::Cache::new(CACHE_CAP, 0))
}

/// Renders one cluster from `faces` at `px` wide (height keeps the strike's aspect).
pub fn render_with(faces: &[Face], cluster: &str, px: f32) -> Option<Image> {
    let w = px.round().max(1.0) as usize;
    faces.iter().find_map(|f| {
        let png = f.bitmap(f.cluster_glyph(cluster)?)?;
        let img = crate::imgdec::decode(png)?;
        let h = (img.h as f32 * w as f32 / img.w as f32).round().max(1.0) as usize;
        Some(resample(&img, w, h))
    })
}

/// The cluster as a premultiplied RGBA image `px` wide, cached per (cluster, px).
pub fn render(text_cluster: &str, px: f32) -> Option<Image> {
    let key = (text_cluster.to_string(), px.round() as u32);
    if let Some(i) = cache().get(&key) {
        return Some(i);
    }
    let img = render_with(&faces(), text_cluster, px)?;
    cache().set(key, img.clone(), img.px.len() * 4);
    Some(img)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn be16(v: &mut Vec<u8>, x: u16) {
        v.extend(x.to_be_bytes());
    }
    fn be32(v: &mut Vec<u8>, x: u32) {
        v.extend(x.to_be_bytes());
    }

    /// A font with U+1F600 -> gid 1 and U+1F601 -> gid 2 (cmap 12), CBLC format 1
    /// and 3 subtables, CBDT formats 17 and 18, and a GSUB ligature
    /// 1F600 + 1F601 -> gid 3 (ccmp).
    pub fn synthetic() -> Vec<u8> {
        let png = crate::imgdec::tests::png_bytes(8, 4, [255, 0, 0, 255]);
        let mut cbdt = vec![0, 3, 0, 0];
        let o1 = cbdt.len();
        cbdt.extend([4, 8, 0, 0, 8]);
        be32(&mut cbdt, png.len() as u32);
        cbdt.extend(&png);
        let o2 = cbdt.len();
        cbdt.extend([0; 8]);
        be32(&mut cbdt, png.len() as u32);
        cbdt.extend(&png);
        let o3 = cbdt.len();
        cbdt.extend([0; 8]);
        be32(&mut cbdt, png.len() as u32);
        cbdt.extend(&png);
        let end = cbdt.len();
        // CBLC: one size, two index subtables (gid 1 via format 1, gids 2..3 via format 3)
        let mut cblc = vec![0, 3, 0, 0];
        be32(&mut cblc, 1);
        let arr_off = 8 + 48;
        be32(&mut cblc, arr_off);
        be32(&mut cblc, 0);
        be32(&mut cblc, 2);
        be32(&mut cblc, 0);
        cblc.extend([0; 24]);
        be16(&mut cblc, 1);
        be16(&mut cblc, 3);
        cblc.extend([136, 136, 32, 1]);
        assert_eq!(cblc.len(), arr_off as usize);
        // array: 2 entries * 8 bytes, then subtables
        let sub1 = 16u32;
        let sub2 = sub1 + 8 + 8;
        for (f, l, off) in [(1u16, 1u16, sub1), (2, 3, sub2)] {
            be16(&mut cblc, f);
            be16(&mut cblc, l);
            be32(&mut cblc, off);
        }
        be16(&mut cblc, 1);
        be16(&mut cblc, 17);
        be32(&mut cblc, 0);
        be32(&mut cblc, o1 as u32);
        be32(&mut cblc, o2 as u32);
        be16(&mut cblc, 3);
        be16(&mut cblc, 18);
        be32(&mut cblc, 0);
        be16(&mut cblc, o2 as u16);
        be16(&mut cblc, o3 as u16);
        be16(&mut cblc, end as u16);
        // cmap format 12: 1F600..1F601 -> 1
        let mut cmap = vec![];
        be16(&mut cmap, 0);
        be16(&mut cmap, 1);
        be16(&mut cmap, 3);
        be16(&mut cmap, 10);
        be32(&mut cmap, 12);
        be16(&mut cmap, 12);
        be16(&mut cmap, 0);
        be32(&mut cmap, 28);
        be32(&mut cmap, 0);
        be32(&mut cmap, 1);
        for x in [0x1F600u32, 0x1F601, 1] {
            be32(&mut cmap, x);
        }
        // GSUB
        let mut gs = vec![];
        be32(&mut gs, 0x0001_0000);
        be16(&mut gs, 10);
        be16(&mut gs, 12);
        be16(&mut gs, 12 + 14);
        be16(&mut gs, 0); // empty script list
        be16(&mut gs, 1); // feature list: count 1
        gs.extend(b"ccmp");
        be16(&mut gs, 8);
        be16(&mut gs, 0); // feature: params, 1 lookup
        be16(&mut gs, 1);
        be16(&mut gs, 0);
        // lookup list at 12+12: offsets
        be16(&mut gs, 1);
        be16(&mut gs, 4);
        be16(&mut gs, 4);
        be16(&mut gs, 0);
        be16(&mut gs, 1);
        be16(&mut gs, 8);
        // ligature subtable: format1, coverage off, 1 set
        be16(&mut gs, 1);
        be16(&mut gs, 8);
        be16(&mut gs, 1);
        be16(&mut gs, 14);
        // coverage at 8: format 1, 1 glyph (gid 1)
        be16(&mut gs, 1);
        be16(&mut gs, 1);
        be16(&mut gs, 1);
        // ligature set at 14: 1 lig at offset 4
        be16(&mut gs, 1);
        be16(&mut gs, 4);
        be16(&mut gs, 3);
        be16(&mut gs, 2);
        be16(&mut gs, 2);
        let tables: [(&[u8; 4], Vec<u8>); 4] = [(b"CBDT", cbdt), (b"CBLC", cblc), (b"GSUB", gs), (b"cmap", cmap)];
        let mut out = vec![0, 1, 0, 0];
        be16(&mut out, 4);
        out.extend([0; 6]);
        let mut off = 12 + 16 * 4;
        let mut body: Vec<u8> = vec![];
        for (tag, t) in &tables {
            out.extend(*tag);
            be32(&mut out, 0);
            be32(&mut out, off as u32);
            be32(&mut out, t.len() as u32);
            body.extend(t);
            off += t.len();
        }
        out.extend(body);
        out
    }

    #[test]
    fn cmap_lookup_format12() {
        let f = Face::parse(synthetic()).unwrap();
        assert_eq!(f.glyph(0x1F600), 1);
        assert_eq!(f.glyph(0x1F601), 2);
        assert_eq!(f.glyph(0x1F602), 0);
        assert_eq!(f.glyph(0x41), 0);
    }

    #[test]
    fn cblc_cbdt_formats_1_3_and_17_18() {
        let f = Face::parse(synthetic()).unwrap();
        for g in [1u16, 2, 3] {
            let png = f.bitmap(g).expect("bitmap");
            assert_eq!(&png[1..4], b"PNG");
        }
        assert!(f.bitmap(4).is_none());
    }

    #[test]
    fn ligature_and_fallback() {
        let f = Face::parse(synthetic()).unwrap();
        assert_eq!(f.cluster_glyph("\u{1F600}\u{1F601}"), Some(3));
        assert_eq!(f.cluster_glyph("\u{1F601}\u{1F600}"), Some(2), "no ligature: base glyph");
        assert_eq!(f.cluster_glyph("x"), None);
    }

    #[test]
    fn render_scales_to_width() {
        let f = Face::parse(synthetic()).unwrap();
        let img = render_with(&[f], "\u{1F600}", 4.0).unwrap();
        assert_eq!((img.w, img.h), (4, 2));
        assert_eq!(img.px[0], crate::gfx::pack(255, 0, 0, 255));
    }

    #[test]
    fn cache_is_bounded() {
        let c = crate::purgeable::Cache::new(3, 0);
        for i in 0..10u32 {
            c.set((format!("e{i}"), 16), Image::new(1, 1), 4);
            assert!(c.len() <= 3);
        }
        assert!(c.get(&("e9".into(), 16)).is_some());
        assert!(c.get(&("e0".into(), 16)).is_none());
    }

    #[test]
    fn segments_split_text_and_emoji() {
        FORCE.with(|f| f.set(Some(true)));
        let s = segments("hi 👍🏽 ok", &|_| false);
        FORCE.with(|f| f.set(None));
        assert_eq!(s, [("hi ", false), ("👍🏽", true), (" ok", false)]);
    }

    #[test]
    fn unavailable_means_plain_text() {
        FORCE.with(|f| f.set(Some(false)));
        assert_eq!(segments("a😀", &|_| false), [("a😀", false)]);
        FORCE.with(|f| f.set(None));
    }

    #[test]
    fn package_font_is_searched_first_and_loaded_from_there() {
        let pkg = Path::new("/x/assets/NotoColorEmoji.ttf");
        let paths = search_paths(pkg);
        assert_eq!(paths[0], pkg);
        assert_eq!(paths[1..], PATHS.map(PathBuf::from));
        let dir = std::env::temp_dir().join(format!("system-pkgs-emoji-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let font = dir.join("NotoColorEmoji.ttf");
        std::fs::write(&font, synthetic()).unwrap();
        let faces = load_faces(&search_paths(&font));
        assert_eq!(faces.len(), 1, "the package font parses; the missing /system paths are skipped");
        assert!(faces[0].cluster_glyph("\u{1F600}").is_some());
        // an unparsable file (COLRv1-style, no CBDT) is ignored
        std::fs::write(&font, b"not a font").unwrap();
        assert!(load_faces(&search_paths(&font)).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reload_replaces_the_faces_and_bumps_the_generation() {
        let _emoji = crate::tests::emoji_lock();
        let dir = std::env::temp_dir().join(format!("system-pkgs-emoji-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let font = dir.join("f.ttf");
        std::fs::write(&font, synthetic()).unwrap();
        let g = generation();
        reload_from(&font);
        assert!(generation() > g && available());
        reload_from(&dir.join("missing.ttf"));
        assert!(!available(), "no package font and no CBDT font in /system");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
