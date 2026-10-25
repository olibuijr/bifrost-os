//! The process-wide fallback faces: when the UI font has no glyph for a character, `Font::render_var` and
//! `Font::measure_var` draw that run with the first of these faces that has one, so a character any installed face
//! covers never renders as `.notdef`. The shell and every SDK app name the list once at start
//! (`install_with_system_lazy`); it is only read the first time a string needs a character the UI font lacks, so a
//! process that only draws Latin text never maps or parses the large script faces (the CJK collection is ~20 MB).
//! Until then the list is empty and text draws exactly as before.

use crate::font::Font;
use crate::fontobj::{FontManager, Scope};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

type Faces = Arc<Vec<Font<'static>>>;

static FACES: RwLock<Option<Faces>> = RwLock::new(None);

/// The library and language `install_with_system_lazy` named, loaded on the first `faces()` call.
static PENDING: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);
/// Set while `PENDING` holds a request, so `faces()` takes no lock in the common case.
static HAS_PENDING: AtomicBool = AtomicBool::new(false);

/// The installed fallback faces, in order. Empty before `install`. Loads a pending lazy install first.
pub fn faces() -> Faces {
    if HAS_PENDING.load(Ordering::Acquire) {
        load_pending();
    }
    FACES.read().ok().and_then(|g| g.clone()).unwrap_or_default()
}

fn load_pending() {
    // the lock is held while loading, so a second thread waits for the list instead of seeing it empty
    let Ok(mut p) = PENDING.lock() else { return };
    if let Some((library, lang)) = p.take() {
        install_with_system(&library, &lang);
    }
    HAS_PENDING.store(false, Ordering::Release);
}

/// Replaces the fallback faces (and drops a lazy install that has not loaded yet: the latest call wins).
pub fn install(faces: Vec<Font<'static>>) {
    HAS_PENDING.store(false, Ordering::Release);
    if let Ok(mut g) = FACES.write() {
        *g = Some(Arc::new(faces));
    }
}

/// Names the fallback list `install_with_system(library, lang)` builds, without loading it: the faces are mapped
/// and parsed the first time a string has a character the UI font lacks.
pub fn install_with_system_lazy(library: &Path, lang: &str) {
    if let Ok(mut p) = PENDING.lock() {
        *p = Some((library.to_path_buf(), lang.to_string()));
        HAS_PENDING.store(true, Ordering::Release);
    }
}

/// The user's language from the locale (`LC_ALL`, `LC_MESSAGES`, then `LANG`), as `default_cascade` takes it (`zh_TW`), or `en`.
pub fn system_lang() -> String {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| !v.is_empty()).unwrap_or_default();
    lang_of_locale(&raw)
}

/// `zh_TW.UTF-8@x` -> `zh_TW`; the C and POSIX locales and an empty one are `en`.
fn lang_of_locale(locale: &str) -> String {
    let l = locale.split(['.', '@']).next().unwrap_or("");
    if l.is_empty() || l == "C" || l == "POSIX" {
        "en".to_string()
    } else {
        l.to_string()
    }
}

/// True for the characters of the CJK scripts and their punctuation: Han (unified ideographs, extensions,
/// compatibility ideographs, radicals, strokes), Hiragana, Katakana, Bopomofo, Hangul (syllables and jamo), the CJK
/// symbols and punctuation block, enclosed and compatibility forms and the halfwidth and fullwidth forms. A run of
/// these draws from one CJK face, so a sentence never mixes the shapes of two fonts.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF // Hangul Jamo
        | 0x2E80..=0x2FDF // CJK radicals, Kangxi radicals
        | 0x2FF0..=0x2FFF // ideographic description
        | 0x3000..=0x303F // CJK symbols and punctuation
        | 0x3040..=0x309F // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x3100..=0x312F // Bopomofo
        | 0x3130..=0x318F // Hangul compatibility jamo
        | 0x3190..=0x31FF // kanbun, Bopomofo extended, CJK strokes, Katakana phonetic extensions
        | 0x3200..=0x33FF // enclosed CJK letters, CJK compatibility
        | 0x3400..=0x4DBF // Han extension A
        | 0x4E00..=0x9FFF // Han unified ideographs
        | 0xA960..=0xA97F // Hangul Jamo extended A
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xD7B0..=0xD7FF // Hangul Jamo extended B
        | 0xF900..=0xFAFF // CJK compatibility ideographs
        | 0xFE30..=0xFE4F // CJK compatibility forms
        | 0xFF00..=0xFFEF // halfwidth and fullwidth forms
        | 0x1B000..=0x1B16F // Kana supplement and extended
        | 0x20000..=0x3134F // Han extensions B to G, compatibility supplement
    )
}

/// The face of a collection file to draw with for `lang`: the face whose family is earliest in the language's
/// cascade (`Noto Sans CJK SC` for `zh`, `KR` for `ko`), else the first face. A single font is face 0.
pub fn collection_face_for(data: &[u8], lang: &str) -> usize {
    let n = crate::fontinfo::collection_len(data);
    if n <= 1 {
        return 0;
    }
    let families: Vec<String> = (0..n).map(|i| crate::fontinfo::FaceInfo::parse_face(data, i).map(|f| f.family).unwrap_or_default()).collect();
    pick_face(&families, lang)
}

/// The index of the family earliest in `lang`'s cascade, else 0.
fn pick_face(families: &[String], lang: &str) -> usize {
    crate::fontobj::default_cascade(lang).iter().find_map(|want| families.iter().position(|f| f.eq_ignore_ascii_case(want))).unwrap_or(0)
}

/// Installs every face of `mgr`, in the manager's fallback order for `lang`, as the process fallback list.
/// Returns how many faces went in.
pub fn install_from_manager(mgr: &FontManager, lang: &str) -> usize {
    let faces: Vec<Font<'static>> = mgr
        .fallback_order(&[], lang, None)
        .into_iter()
        .filter_map(|i| {
            let data = mgr.static_bytes(i)?;
            // a collection (the CJK `.ttc`) draws with the face for the language
            Font::parse_face(data, collection_face_for(data, lang))
        })
        .collect();
    let n = faces.len();
    install(faces);
    n
}

/// Registers the fonts under `dir` (the font library), then installs them as the fallback list. Returns the count.
pub fn install_from_dir(dir: &std::path::Path, lang: &str) -> usize {
    let mut m = FontManager::new();
    m.scan_dir(dir, 2);
    install_from_manager(&m, lang)
}

/// The script fonts the packages BifrOSt installs (`packaging/arch/runtime-packages.txt`) provide, by the family
/// name the fallback cascade asks for (`fontobj::default_cascade`). Each file registers as one face under the name of
/// its first face (`Noto Sans CJK JP` for the CJK collection); `install_from_manager` then draws a collection with
/// the face for the language (`collection_face_for`: SC for Chinese, KR for Korean). A missing file is skipped.
pub const SYSTEM_FALLBACK: &[(&str, &str)] = &[
    ("Noto Sans", "/usr/share/fonts/noto/NotoSans-Regular.ttf"),
    ("Noto Sans Arabic", "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf"),
    ("Noto Naskh Arabic", "/usr/share/fonts/noto/NotoNaskhArabic-Regular.ttf"),
    ("Noto Sans Hebrew", "/usr/share/fonts/noto/NotoSansHebrew-Regular.ttf"),
    ("Noto Sans Devanagari", "/usr/share/fonts/noto/NotoSansDevanagari-Regular.ttf"),
    ("Noto Sans Thai", "/usr/share/fonts/noto/NotoSansThai-Regular.ttf"),
    ("Noto Sans CJK JP", "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
    ("Noto Sans Symbols", "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf"),
    ("Noto Sans Symbols 2", "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf"),
    ("Noto Sans Math", "/usr/share/fonts/noto/NotoSansMath-Regular.ttf"),
    ("DejaVu Sans", "/usr/share/fonts/TTF/DejaVuSans.ttf"),
];

/// Registers the library fonts under `library`, then each existing `SYSTEM_FALLBACK` file, and installs the list.
/// The library comes first, so the packed UI faces keep their order. Returns the count of fallback faces.
pub fn install_with_system(library: &std::path::Path, lang: &str) -> usize {
    let mut m = FontManager::new();
    m.scan_dir(library, 2);
    for (_, path) in SYSTEM_FALLBACK {
        if std::path::Path::new(path).is_file() {
            let _ = m.register_url(path, Scope::System);
        }
    }
    install_from_manager(&m, lang)
}

/// Splits `text` into runs that share a face. `pick` says, per character, which fallback face draws it (`None`
/// keeps the character on the main face). Neighbours with the same pick join one run.
pub fn split(text: &str, mut pick: impl FnMut(char) -> Option<usize>) -> Vec<(Option<usize>, &str)> {
    let mut runs: Vec<(Option<usize>, usize, usize)> = Vec::new();
    for (i, ch) in text.char_indices() {
        let f = pick(ch);
        let end = i + ch.len_utf8();
        match runs.last_mut() {
            Some(r) if r.0 == f => r.2 = end,
            _ => runs.push((f, i, end)),
        }
    }
    runs.into_iter().map(|(f, s, e)| (f, &text[s..e])).collect()
}

/// A run of text and the index (into the returned faces) of the face that draws it; `None` is the main face.
pub type Runs<'t> = Vec<(Option<usize>, &'t str)>;

/// How `text` splits between `primary` and the fallback faces, or None when `primary` draws all of it (the
/// common case, found without allocating).
pub fn runs<'t>(primary: &Font<'_>, text: &'t str) -> Option<(Faces, Runs<'t>)> {
    // default-ignorable characters draw nothing on any face (`Font::shape_run`), so they never start a fallback run
    let skip = |c: char| c.is_ascii() || c.is_whitespace() || c.is_control() || crate::font::is_default_ignorable(c);
    // checked before the faces are fetched, so text the UI font covers never triggers the lazy load
    if text.chars().all(|c| skip(c) || primary.glyph(c) != 0) {
        return None;
    }
    let faces = faces();
    if faces.is_empty() {
        return None;
    }
    // the face that draws CJK text: the first that has the most common ideograph, so Han, kana, Hangul and CJK
    // punctuation stay on one face instead of taking a stray glyph from an earlier symbol or Latin face
    let cjk_face = faces.iter().position(|f| f.glyph('\u{4E00}') != 0);
    let r = split(text, |c| {
        if skip(c) || primary.glyph(c) != 0 {
            return None;
        }
        if is_cjk(c) {
            if let Some(i) = cjk_face.filter(|&i| faces[i].glyph(c) != 0) {
                return Some(i);
            }
        }
        faces.iter().position(|f| f.glyph(c) != 0)
    });
    if r.iter().all(|x| x.0.is_none()) {
        return None;
    }
    Some((faces, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held by the tests that replace the process-wide list, so they never see each other's faces.
    static GLOBAL: Mutex<()> = Mutex::new(());

    fn global() -> std::sync::MutexGuard<'static, ()> {
        GLOBAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sf() -> &'static [u8] {
        Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice())
    }

    #[test]
    fn split_merges_neighbours_and_covers_the_text() {
        let picks = |c: char| match c {
            'b' | 'c' => Some(0),
            'x' => Some(1),
            _ => None,
        };
        let r = split("abcxxa", picks);
        assert_eq!(r, vec![(None, "a"), (Some(0), "bc"), (Some(1), "xx"), (None, "a")]);
        assert!(split("", picks).is_empty());
        assert_eq!(r.iter().map(|x| x.1).collect::<String>(), "abcxxa");
    }

    #[test]
    fn no_fallback_faces_or_full_coverage_means_no_runs() {
        let _g = global();
        let data = sf();
        let f = Font::parse(data).unwrap();
        // the list may be shared with other tests in this process; whatever it holds, covered text yields None
        assert!(runs(&f, "Hello world").is_none());
        assert!(runs(&f, "").is_none());
    }

    #[test]
    fn system_fallback_names_the_primary_face_of_each_language() {
        use crate::fontobj::default_cascade;
        let has = |family: &str| SYSTEM_FALLBACK.iter().any(|(f, _)| *f == family);
        // the first cascade entry of each language that BifrOSt ships a package for
        for (lang, family) in [("en", "Noto Sans"), ("ja", "Noto Sans CJK JP"), ("ar", "Noto Sans Arabic"), ("he", "Noto Sans Hebrew"), ("hi", "Noto Sans Devanagari"), ("th", "Noto Sans Thai")] {
            assert_eq!(default_cascade(lang)[0], family, "{lang}");
            assert!(has(family), "{lang} needs {family} in SYSTEM_FALLBACK");
        }
        // the symbol and math families at the end of every cascade are installed too
        for family in ["Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Sans Math"] {
            assert!(has(family), "{family}");
        }
    }

    #[test]
    fn system_fallback_files_are_package_paths_without_duplicates() {
        let mut seen = Vec::new();
        for (family, path) in SYSTEM_FALLBACK {
            assert!(path.starts_with("/usr/share/fonts/"), "{path}");
            assert!(!seen.contains(family), "duplicate family {family}");
            seen.push(*family);
        }
    }

    #[test]
    fn install_with_system_with_an_empty_library_reports_what_it_installed() {
        let _g = global();
        // an empty library: the count is exactly the system files present on this machine, never a panic
        let dir = std::env::temp_dir().join("bifrost-fallback-empty-test");
        let _ = std::fs::create_dir_all(&dir);
        let present = SYSTEM_FALLBACK.iter().filter(|(_, p)| std::path::Path::new(p).is_file()).count();
        let n = install_with_system(&dir, "en");
        // a file the parser rejects is not counted, so the bound is one-sided
        assert!(n <= present);
        install(Vec::new());
    }

    /// A collection of `fonts`, each face's table offsets moved to where it lands in the file.
    fn ttc(fonts: &[&[u8]]) -> Vec<u8> {
        let mut starts = Vec::new();
        let mut pos = 12 + 4 * fonts.len();
        for f in fonts {
            starts.push(pos);
            pos += f.len();
        }
        let mut out = b"ttcf".to_vec();
        out.extend([0, 1, 0, 0]);
        out.extend((fonts.len() as u32).to_be_bytes());
        for s in &starts {
            out.extend((*s as u32).to_be_bytes());
        }
        for (f, s) in fonts.iter().zip(&starts) {
            let mut f = f.to_vec();
            for i in 0..u16::from_be_bytes([f[4], f[5]]) as usize {
                let at = 12 + 16 * i + 8;
                let off = u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) + *s as u32;
                f[at..at + 4].copy_from_slice(&off.to_be_bytes());
            }
            out.extend(f);
        }
        out
    }

    #[test]
    fn a_collection_parses_every_face_at_its_own_directory() {
        let cff = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-cff.otf")).unwrap();
        let ui = sf();
        let file = ttc(&[&cff[..], ui]);
        assert_eq!(crate::fontinfo::collection_len(&file), 2);
        assert_eq!(crate::fontinfo::collection_len(ui), 1);
        // the first face is what `parse` opens, the second only through `parse_face`
        let first = Font::parse(&file).expect("a collection parses");
        let second = Font::parse_face(&file, 1).expect("face 1");
        assert!(Font::parse_face(&file, 2).is_none());
        assert!(Font::parse_face(ui, 1).is_none());
        let alone = Font::parse(ui).unwrap();
        assert_eq!(second.num_glyphs(), alone.num_glyphs());
        assert_eq!(second.glyph('H'), alone.glyph('H'));
        assert_ne!(second.glyph('H'), 0);
        assert_eq!(second.advance(second.glyph('H')), alone.advance(alone.glyph('H')));
        assert_eq!(first.num_glyphs(), Font::parse(&cff).unwrap().num_glyphs());
        // the two faces share the bytes but never a render cache entry
        assert_ne!(first.ident(), second.ident());
        // it draws the same ink as the standalone face, shaping tables included
        let st = crate::font::Style { px: 20.0, embolden: 0.0, color: 0xFFFF_FFFF, shadow: false };
        assert_eq!(second.render("Hi", &st).px, alone.render("Hi", &st).px);
        let names = |d: &[u8], i| crate::fontinfo::FaceInfo::parse_face(d, i).unwrap().family;
        assert_eq!(names(&file, 1), names(ui, 0));
        // no cascade family among them: the first face
        assert_eq!(collection_face_for(&file, "zh"), 0);
        assert_eq!(collection_face_for(ui, "zh"), 0);
    }

    #[test]
    fn the_cjk_collection_face_follows_the_language() {
        let fams: Vec<String> = ["Noto Sans CJK JP", "Noto Sans CJK KR", "Noto Sans CJK SC", "Noto Sans CJK TC", "Noto Sans CJK HK", "Noto Sans Mono CJK JP"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_face(&fams, "ja"), 0);
        assert_eq!(pick_face(&fams, "ko"), 1);
        assert_eq!(pick_face(&fams, "zh"), 2);
        assert_eq!(pick_face(&fams, "zh_CN"), 2);
        assert_eq!(pick_face(&fams, "zh_TW"), 3);
        // every other language uses the cascade's default CJK face
        assert_eq!(pick_face(&fams, "en"), 0);
        assert_eq!(pick_face(&fams, "is"), 0);
        assert_eq!(pick_face(&["Other".to_string()], "zh"), 0);
    }

    #[test]
    fn the_locale_names_the_language() {
        assert_eq!(lang_of_locale("zh_TW.UTF-8"), "zh_TW");
        assert_eq!(lang_of_locale("ko_KR.UTF-8@euro"), "ko_KR");
        assert_eq!(lang_of_locale("is_IS"), "is_IS");
        assert_eq!(lang_of_locale("C.UTF-8"), "en");
        assert_eq!(lang_of_locale("POSIX"), "en");
        assert_eq!(lang_of_locale(""), "en");
    }

    #[test]
    fn script_detection_covers_han_kana_hangul_and_cjk_punctuation() {
        // Han (unified, extension A and B, compatibility), Hiragana, Katakana (and halfwidth), Hangul, Bopomofo
        for c in ['中', '文', '\u{3400}', '\u{20000}', '\u{F900}', 'あ', 'ん', 'カ', 'ー', 'ｶ', '한', '글', 'ᄀ', 'ㄱ', 'ㄅ'] {
            assert!(is_cjk(c), "{c:?} U+{:04X}", c as u32);
        }
        // CJK punctuation and fullwidth forms
        for c in ['。', '、', '「', '」', '『', '〜', '\u{3000}', '，', '！', '（', '：'] {
            assert!(is_cjk(c), "{c:?} U+{:04X}", c as u32);
        }
        for c in ['a', 'Z', '1', 'é', 'Ж', 'ا', 'א', 'क', 'ก', '→', '€', '…', '“', '\u{1F600}'] {
            assert!(!is_cjk(c), "{c:?} U+{:04X}", c as u32);
        }
    }

    #[test]
    fn a_lazy_install_loads_on_the_first_uncovered_character_only() {
        let _g = global();
        let dir = std::env::temp_dir().join(format!("bifrost-fallback-lazy-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = Font::parse(sf()).unwrap();
        install_with_system_lazy(&dir, "en");
        // covered text decides without the faces, so the request stays pending
        assert!(runs(&f, "Hello").is_none());
        assert!(HAS_PENDING.load(Ordering::Acquire));
        // a character the UI font lacks loads the list once
        let _ = runs(&f, "中文");
        assert!(!HAS_PENDING.load(Ordering::Acquire));
        assert!(PENDING.lock().unwrap().is_none());
        install(Vec::new());
    }

    /// With the system CJK collection installed (package noto-fonts-cjk), a Chinese, Japanese and Korean line
    /// draws from it with no missing glyph, as one run. Skipped where the package is absent.
    #[test]
    fn the_system_cjk_face_covers_chinese_japanese_and_korean() {
        let _g = global();
        let path = SYSTEM_FALLBACK.iter().find(|(f, _)| *f == "Noto Sans CJK JP").unwrap().1;
        let Ok(data) = std::fs::read(path) else {
            eprintln!("skipped: {path} is not installed");
            return;
        };
        let data: &'static [u8] = Box::leak(data.into_boxed_slice());
        let face = Font::parse_face(data, collection_face_for(data, "zh")).expect("the CJK face parses");
        assert_eq!(crate::fontinfo::FaceInfo::parse_face(data, collection_face_for(data, "zh")).unwrap().family, "Noto Sans CJK SC");
        let text = "你好，世界。ひらがなカタカナ「한국어」";
        for c in text.chars() {
            assert_ne!(face.glyph(c), 0, "{c:?}");
        }
        let ui = Font::parse(sf()).unwrap();
        install(vec![face]);
        let (_, r) = runs(&ui, text).expect("the UI font lacks CJK");
        assert_eq!(r, vec![(Some(0), text)]);
        install(Vec::new());
    }

    #[test]
    fn a_character_no_face_covers_stays_on_the_main_face() {
        let _g = global();
        let data = sf();
        let f = Font::parse(data).unwrap();
        install(vec![Font::parse(data).unwrap()]);
        assert!(runs(&f, "a\u{10FFFE}b").is_none());
        install(Vec::new());
    }
}
