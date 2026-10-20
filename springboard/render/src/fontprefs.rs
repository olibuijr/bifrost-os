//! Settings > Fonts: the UI fonts a user can replace with an installed font. An override is the path of a font
//! file, saved in the shell's `settings.ini` under the role's key; the shell and every app resolve their fonts
//! here, so they all follow it. A missing override, or one that does not parse, falls back to the packed asset.
//! The choices come from BifrOSt's own font library (`library`), not from whatever the host has installed.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A font the desktop draws with, each replaceable on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontRole {
    /// Every label, title and body text
    System,
    /// Terminals, code and text previews
    Mono,
}

impl FontRole {
    pub const ALL: [FontRole; 2] = [FontRole::System, FontRole::Mono];

    /// The packed asset used when nothing overrides it.
    pub fn file(self) -> &'static str {
        match self {
            FontRole::System => "SFUI.ttf",
            FontRole::Mono => "SFUIMono.ttf",
        }
    }

    /// The `settings.ini` key holding the override path.
    pub fn pref(self) -> &'static str {
        match self {
            FontRole::System => "font_system",
            FontRole::Mono => "font_mono",
        }
    }

    /// Position in `ALL`.
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            FontRole::System => "System Font",
            FontRole::Mono => "Monospaced Font",
        }
    }
}

/// The override saved for `role` in `dir/settings.ini`, if one is set.
pub fn font_override(dir: &str, role: FontRole) -> Option<String> {
    let ini = std::fs::read_to_string(format!("{dir}/settings.ini")).ok()?;
    ini.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim() == role.pref()).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The font file `role` draws with: the override when it exists, else the packed asset in `dir`.
pub fn font_path(dir: &str, role: FontRole) -> String {
    font_override(dir, role).filter(|p| Path::new(p).is_file()).unwrap_or_else(|| format!("{dir}/{}", role.file()))
}

/// Maps the font for `role`: the override when it parses as a font, else the packed asset.
pub fn map_font(dir: &str, role: FontRole) -> std::io::Result<&'static [u8]> {
    if let Some(path) = font_override(dir, role) {
        match crate::assets::map_file(&path) {
            Ok(data) if crate::font::Font::parse(data).is_some() => return Ok(data),
            Ok(_) => eprintln!("fonts: {path} is not a usable font; using {}", role.file()),
            Err(e) => eprintln!("fonts: {path}: {e}; using {}", role.file()),
        }
    }
    crate::assets::map_file(&format!("{dir}/{}", role.file()))
}

/// One installed font family, as Settings > Fonts offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub family: String,
    /// the file that stands for the family: a variable upright face, else the Regular one
    pub path: String,
    /// fixed pitch (offered for the Monospaced Font)
    pub mono: bool,
    /// carries a `wght` or other variation axis, so every weight renders
    pub variable: bool,
}

/// BifrOSt's font library in the data dir `dir`: the open fonts the system ships (packed by `tools/pack.py`,
/// seeded from the package-owned `/usr/share/springboard/system-assets/fonts`) and fonts that apps install. Settings offers only these,
/// never the host's or the distribution packages' fonts.
pub fn library(dir: &str) -> PathBuf {
    Path::new(dir).join("fonts")
}

/// The family name of the font file at `path`, if it is one the renderer can draw.
pub fn family(path: &str) -> Option<String> {
    probe(Path::new(path)).map(|f| f.family)
}

/// Every installed font family under `dirs` the renderer can draw, one entry per family, sorted by name.
/// Only the table directory and the small `name`, `post` and `OS/2` headers are read, never whole files.
pub fn installed(dirs: &[PathBuf]) -> Vec<Installed> {
    let mut faces = Vec::new();
    for dir in dirs {
        walk(dir, 0, &mut faces);
    }
    // one face per family: variable before static, upright before italic, Regular before other weights
    faces.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()).then(a.rank().cmp(&b.rank())).then(a.path.cmp(&b.path)));
    faces.dedup_by(|b, a| a.family == b.family);
    faces
        .into_iter()
        .map(|f| Installed { family: f.family, path: f.path, mono: f.mono, variable: f.variable })
        .collect()
}

struct Face {
    family: String,
    style: String,
    path: String,
    mono: bool,
    variable: bool,
}

impl Face {
    fn rank(&self) -> (bool, bool, bool) {
        let s = self.style.to_lowercase();
        (s.contains("italic") || s.contains("oblique"), !self.variable, s != "regular")
    }
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Face>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth < 6 {
                walk(&path, depth + 1, out);
            }
        } else if path.extension().and_then(|x| x.to_str()).is_some_and(|x| ["ttf", "otf", "ttc"].iter().any(|e| x.eq_ignore_ascii_case(e))) {
            if let Some(face) = probe(&path) {
                out.push(face);
            }
        }
    }
}

/// Reads just enough of an sfnt file to name it and to know the renderer can draw it.
fn probe(path: &Path) -> Option<Face> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    // a collection ('ttcf'): the first face is named and checked, its directory sits at the first offset
    if &head[..4] == b"ttcf" {
        let mut first = [0u8; 4];
        f.read_exact(&mut first).ok()?;
        f.seek(SeekFrom::Start(u32::from_be_bytes(first) as u64)).ok()?;
        f.read_exact(&mut head).ok()?;
    }
    // TrueType or CFF outlines; other containers are not single fonts
    if !matches!(&head[..4], [0, 1, 0, 0] | b"OTTO" | b"true") {
        return None;
    }
    let n = u16::from_be_bytes([head[4], head[5]]) as usize;
    let mut recs = vec![0u8; 16 * n];
    f.read_exact(&mut recs).ok()?;
    let table = |tag: &[u8; 4]| {
        recs.as_chunks::<16>().0.iter().find(|r| &r[..4] == tag).map(|r| (u32::from_be_bytes([r[8], r[9], r[10], r[11]]) as u64, u32::from_be_bytes([r[12], r[13], r[14], r[15]]) as usize))
    };
    let has = |tag: &[u8; 4]| table(tag).is_some();
    if !(has(b"head") && has(b"hhea") && has(b"maxp") && has(b"cmap") && (has(b"CFF ") || (has(b"glyf") && has(b"loca")))) {
        return None;
    }
    let mut read = |(off, len): (u64, usize), max: usize| -> Option<Vec<u8>> {
        let mut buf = vec![0u8; len.min(max)];
        f.seek(SeekFrom::Start(off)).ok()?;
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let name = read(table(b"name")?, 1 << 20)?;
    let family = name_string(&name, 16).or_else(|| name_string(&name, 1))?;
    // variable builds often name themselves "<Family> Variable"; the picker shows the family
    let family = family.strip_suffix(" Variable").map_or(family.clone(), str::to_string);
    let style = name_string(&name, 17).or_else(|| name_string(&name, 2)).unwrap_or_default();
    // fixed pitch: post.isFixedPitch, or a monospaced PANOSE proportion in OS/2
    let fixed = table(b"post").and_then(|t| read(t, 16)).is_some_and(|p| p.len() >= 16 && p[12..16] != [0; 4]);
    let panose = table(b"OS/2").and_then(|t| read(t, 36)).is_some_and(|o| o.len() >= 36 && o[32] == 2 && o[35] == 9);
    // a family name starting with '.' is a hidden system face
    (!family.starts_with('.')).then(|| Face { family, style, path: path.to_string_lossy().into_owned(), mono: fixed || panose, variable: has(b"fvar") && has(b"gvar") })
}

/// The user's own font folder, relative to the home directory. Only this folder is ever rebuilt by `refresh_user_cache`.
pub const USER_FONTS: &str = ".local/share/fonts";
/// The shortest gap between two font cache rebuilds, so a burst of installs or removals rebuilds once.
pub const REFRESH_GAP: Duration = Duration::from_secs(10);

static LAST_REFRESH: Mutex<Option<Instant>> = Mutex::new(None);

/// True when a rebuild last run at `last` may run again at `now`.
pub fn refresh_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= REFRESH_GAP)
}

/// Rebuilds the fontconfig cache of the user's font folder so other apps see an install or a removal. Runs
/// `fc-cache` on `user_dir` only (no root, no system folders) and at most once per `REFRESH_GAP`: a call inside the
/// gap returns `Ok(false)` and does nothing. `user_dir` must end in `.local/share/fonts`.
pub fn refresh_user_cache(user_dir: &Path) -> Result<bool, String> {
    if !user_dir.ends_with(USER_FONTS) {
        return Err(format!("Only the user's font folder is refreshed, not {}.", user_dir.display()));
    }
    let now = Instant::now();
    let mut last = LAST_REFRESH.lock().unwrap_or_else(|e| e.into_inner());
    if !refresh_due(*last, now) {
        return Ok(false);
    }
    *last = Some(now);
    drop(last);
    let status = Command::new("fc-cache").arg(user_dir).status().map_err(|e| format!("Cannot run fc-cache: {e}"))?;
    if status.success() {
        Ok(true)
    } else {
        Err(format!("fc-cache exited with {status}"))
    }
}

/// Name `id` from a `name` table: Windows Unicode (English first), else Unicode, else Mac Roman.
fn name_string(t: &[u8], id: u16) -> Option<String> {
    let u16_at = |o: usize| t.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let count = u16_at(2)? as usize;
    let strings = u16_at(4)? as usize;
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let r = 6 + 12 * i;
        let (platform, lang, nid) = (u16_at(r)?, u16_at(r + 4)?, u16_at(r + 6)?);
        if nid != id {
            continue;
        }
        let (len, off) = (u16_at(r + 8)? as usize, u16_at(r + 10)? as usize);
        let Some(raw) = t.get(strings + off..strings + off + len) else { continue };
        let (rank, text) = match platform {
            0 | 3 => (if platform == 3 && lang == 0x409 { 0 } else { 1 }, String::from_utf16_lossy(&raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>())),
            1 => (2, raw.iter().map(|&b| b as char).collect()),
            _ => continue,
        };
        if best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, s)| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fontprefs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A minimal sfnt with the tables `probe` checks, named `family`/`style`.
    fn sfnt(family: &str, style: &str, fixed: bool, variable: bool) -> Vec<u8> {
        let utf16 = |s: &str| s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<u8>>();
        let (fam, sty) = (utf16(family), utf16(style));
        let mut name = vec![0, 0, 0, 2, 0, 30];
        for (id, off, len) in [(1u16, 0usize, fam.len()), (2, fam.len(), sty.len())] {
            for v in [3u16, 1, 0x409, id, len as u16, off as u16] {
                name.extend(v.to_be_bytes());
            }
        }
        name.extend(&fam);
        name.extend(&sty);
        let mut post = vec![0u8; 32];
        post[15] = fixed as u8;
        let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"cmap", vec![0; 4]), (b"glyf", vec![0; 4]), (b"head", vec![0; 4]), (b"hhea", vec![0; 4]), (b"loca", vec![0; 4]), (b"maxp", vec![0; 4]), (b"name", name), (b"post", post)];
        if variable {
            tables.push((b"fvar", vec![0; 4]));
            tables.push((b"gvar", vec![0; 4]));
        }
        let mut out = vec![0, 1, 0, 0];
        out.extend((tables.len() as u16).to_be_bytes());
        out.extend([0; 6]);
        let mut off = 12 + 16 * tables.len();
        let mut body: Vec<u8> = Vec::new();
        for (tag, data) in &tables {
            out.extend(*tag);
            out.extend([0; 4]);
            out.extend((off as u32).to_be_bytes());
            out.extend((data.len() as u32).to_be_bytes());
            body.extend(data);
            off += data.len();
        }
        out.extend(body);
        out
    }

    #[test]
    fn installed_lists_one_face_per_family_preferring_variable_upright() {
        let d = tmp("scan");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("Inter-Bold.ttf"), sfnt("Inter", "Bold", false, false)).unwrap();
        std::fs::write(d.join("Inter-Regular.ttf"), sfnt("Inter", "Regular", false, false)).unwrap();
        std::fs::write(d.join("sub/InterVariable-Italic.ttf"), sfnt("Inter", "Italic", false, true)).unwrap();
        std::fs::write(d.join("sub/InterVariable.ttf"), sfnt("Inter Variable", "Regular", false, true)).unwrap();
        std::fs::write(d.join("Code-Regular.otf"), sfnt("Code", "Regular", true, false)).unwrap();
        std::fs::write(d.join("Hidden.ttf"), sfnt(".Hidden", "Regular", false, false)).unwrap();
        std::fs::write(d.join("notes.ttf"), b"not a font").unwrap();
        let got = installed(&[d.clone()]);
        let names: Vec<_> = got.iter().map(|f| (f.family.as_str(), f.path.rsplit('/').next().unwrap(), f.mono, f.variable)).collect();
        assert_eq!(names, [("Code", "Code-Regular.otf", true, false), ("Inter", "InterVariable.ttf", false, true)]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A font collection holding one face: the header, then the face with its table offsets moved past the header.
    fn ttc(family: &str) -> Vec<u8> {
        let mut face = sfnt(family, "Regular", false, false);
        let n = u16::from_be_bytes([face[4], face[5]]) as usize;
        for i in 0..n {
            let at = 12 + 16 * i + 8;
            let off = u32::from_be_bytes(face[at..at + 4].try_into().unwrap()) + 16;
            face[at..at + 4].copy_from_slice(&off.to_be_bytes());
        }
        let mut out = b"ttcf".to_vec();
        out.extend([0, 1, 0, 0]);
        out.extend(1u32.to_be_bytes());
        out.extend(16u32.to_be_bytes());
        out.extend(face);
        out
    }

    #[test]
    fn a_collection_lists_its_first_face() {
        let d = tmp("ttc");
        std::fs::write(d.join("Mincho.ttc"), ttc("Mincho")).unwrap();
        let faces = installed(std::slice::from_ref(&d));
        assert_eq!(faces.iter().map(|f| f.family.as_str()).collect::<Vec<_>>(), ["Mincho"]);
        assert!(faces[0].path.ends_with("Mincho.ttc"));
    }

    #[test]
    fn the_cache_refresh_is_rate_limited_and_limited_to_the_user_folder() {
        let now = Instant::now();
        assert!(refresh_due(None, now), "the first refresh always runs");
        assert!(!refresh_due(Some(now), now + REFRESH_GAP / 2), "a burst waits out the gap");
        assert!(refresh_due(Some(now), now + REFRESH_GAP), "the gap has passed");
        assert!(refresh_user_cache(Path::new("/usr/share/fonts")).is_err(), "system folders are never rebuilt");
        assert!(refresh_user_cache(Path::new("/home/u/Fonts")).is_err());
    }

    #[test]
    fn overrides_resolve_from_settings_and_fall_back_to_the_asset() {
        let d = tmp("resolve");
        let dir = d.to_str().unwrap();
        assert_eq!(font_path(dir, FontRole::Mono), format!("{dir}/SFUIMono.ttf"), "no settings: the packed asset");
        let font = d.join("Mine.ttf");
        std::fs::write(&font, b"x").unwrap();
        std::fs::write(d.join("settings.ini"), format!("wallpaper=lake\nfont_mono = {}\nfont_system=/nonexistent.ttf\n", font.display())).unwrap();
        assert_eq!(font_override(dir, FontRole::Mono).as_deref(), font.to_str());
        assert_eq!(font_path(dir, FontRole::Mono), font.to_str().unwrap());
        assert_eq!(font_path(dir, FontRole::System), format!("{dir}/SFUI.ttf"), "a missing file falls back");
        std::fs::write(d.join("SFUIMono.ttf"), b"asset").unwrap();
        assert_eq!(map_font(dir, FontRole::Mono).unwrap(), b"asset", "an override that does not parse falls back");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
