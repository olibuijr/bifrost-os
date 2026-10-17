//! Text recognition for apps: the text half of an image analysis (`ImageAnalyzer` with the text analysis type, the text
//! items of `DataScannerViewController`, and Live Text selection). [`recognize`] reads the text in an image and gives it
//! back as [`Line`]s of [`Word`]s, each with the pixel rectangle it sits in and a confidence from 0 to 1, in reading order.
//!
//! The engine is the system's `tesseract` (package `tesseract` with the `tesseract-data-*` language packs), run on the
//! machine: the image is written to an owner-only temporary file, read by the engine and deleted, and nothing goes to
//! the network. [`parse_tsv`] reads the engine's table, so the parsing and the selection helpers ([`word_at`],
//! [`line_at`], [`find`], [`text`]) work on any recogniser's output and are tested without the engine.
//!
//! Limits: the languages are the packs installed (English is shipped); there is no handwriting model, no vertical text
//! and no per-character boxes; a very large image is refused ([`MAX_PIXELS`]) rather than shrunk.

use bifrost_ui::subject_lift::{PixelRect, Rgba};
use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

/// The recogniser program.
pub const TESSERACT: &str = "tesseract";
/// The largest image read, in pixels (about 64 megapixels).
pub const MAX_PIXELS: usize = 64 * 1024 * 1024;

/// One recognised word.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    /// 0 (a guess) to 1 (certain)
    pub confidence: f32,
    pub bounds: PixelRect,
}

/// One recognised line of text: the words left to right and the rectangle that holds them.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub words: Vec<Word>,
    pub bounds: PixelRect,
}

impl Line {
    /// The words joined by single spaces.
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    /// The mean confidence of the words.
    pub fn confidence(&self) -> f32 {
        if self.words.is_empty() {
            return 0.0;
        }
        self.words.iter().map(|w| w.confidence).sum::<f32>() / self.words.len() as f32
    }
}

/// Why recognition did not run or failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// the recogniser is not installed
    NotInstalled,
    /// a language is not a code of letters (`eng`, `deu`, `chi_sim`), joined by `+` for several
    BadLanguage(String),
    /// the image has more than [`MAX_PIXELS`] pixels
    TooLarge,
    Io(String),
    /// the recogniser ran and refused; the first line of what it said
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotInstalled => write!(f, "text recognition needs the tesseract package"),
            Error::BadLanguage(l) => write!(f, "{l} is not a language code"),
            Error::TooLarge => write!(f, "the image is too large to read"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Failed(e) => write!(f, "tesseract: {e}"),
        }
    }
}

/// Whether `code` names a language pack: lower-case letters and `_`, three to ten characters (`eng`, `chi_sim`).
pub fn valid_language(code: &str) -> bool {
    (3..=10).contains(&code.len()) && code.chars().all(|c| c.is_ascii_lowercase() || c == '_') && !code.starts_with('_')
}

/// The `-l` value for `languages` (`eng+deu`), or the first code that is not valid. No languages means English.
pub fn language_arg(languages: &[&str]) -> Result<String, Error> {
    if languages.is_empty() {
        return Ok("eng".to_string());
    }
    if let Some(bad) = languages.iter().find(|l| !valid_language(l)) {
        return Err(Error::BadLanguage(bad.to_string()));
    }
    Ok(languages.join("+"))
}

/// The image as a binary PPM (P6): the recogniser's simplest input. Transparent pixels are composited over white, so
/// text drawn on a transparent background is read as dark on light.
pub fn ppm_bytes(image: &Rgba) -> Vec<u8> {
    let (w, h) = (image.width(), image.height());
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.reserve(w * h * 3);
    for px in image.pixels().chunks_exact(4) {
        let a = px[3] as u32;
        for &c in &px[..3] {
            out.push(((c as u32 * a + 255 * (255 - a) + 127) / 255) as u8);
        }
    }
    out
}

/// Reads the engine's TSV table (`tesseract ... tsv`) into lines of words. Only word rows (level 5) with text and a
/// confidence of at least 0 are kept; words of one block, paragraph and line form a [`Line`]. A malformed row is skipped.
pub fn parse_tsv(tsv: &str) -> Vec<Line> {
    let mut lines: Vec<((u32, u32, u32), Line)> = Vec::new();
    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let num = |i: usize| cols[i].trim().parse::<u32>().ok();
        let (Some(block), Some(par), Some(line), Some(x), Some(y), Some(w), Some(h)) = (num(2), num(3), num(4), num(6), num(7), num(8), num(9)) else { continue };
        let Ok(conf) = cols[10].trim().parse::<f32>() else { continue };
        let text = cols[11..].join("\t");
        let text = text.trim();
        if conf < 0.0 || text.is_empty() {
            continue;
        }
        let bounds = PixelRect { x: x as usize, y: y as usize, w: w as usize, h: h as usize };
        let word = Word { text: text.to_string(), confidence: (conf / 100.0).clamp(0.0, 1.0), bounds };
        let key = (block, par, line);
        match lines.last_mut() {
            Some((k, l)) if *k == key => {
                l.bounds = union(l.bounds, bounds);
                l.words.push(word);
            }
            _ => lines.push((key, Line { words: vec![word], bounds })),
        }
    }
    lines.into_iter().map(|(_, l)| l).collect()
}

fn union(a: PixelRect, b: PixelRect) -> PixelRect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    let (right, bottom) = ((a.x + a.w).max(b.x + b.w), (a.y + a.h).max(b.y + b.h));
    PixelRect { x, y, w: right - x, h: bottom - y }
}

fn inside(r: PixelRect, x: usize, y: usize) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

/// The text of all lines, one line per row, in reading order.
pub fn text(lines: &[Line]) -> String {
    lines.iter().map(Line::text).collect::<Vec<_>>().join("\n")
}

/// The line holding the pixel (`x`, `y`), if any.
pub fn line_at(lines: &[Line], x: usize, y: usize) -> Option<&Line> {
    lines.iter().find(|l| inside(l.bounds, x, y))
}

/// The word under the pixel (`x`, `y`), with its line: what a tap on recognised text selects.
pub fn word_at(lines: &[Line], x: usize, y: usize) -> Option<(&Line, &Word)> {
    let line = line_at(lines, x, y)?;
    line.words.iter().find(|w| inside(w.bounds, x, y)).map(|w| (line, w))
}

/// The words whose text contains `query`, ignoring case: the highlights of a find in the image.
pub fn find<'a>(lines: &'a [Line], query: &str) -> Vec<&'a Word> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    lines.iter().flat_map(|l| l.words.iter()).filter(|w| w.text.to_lowercase().contains(&q)).collect()
}

/// Whether the recogniser is installed.
pub fn available() -> bool {
    Command::new(TESSERACT).arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The temporary file for one run: owner-only, made fresh (never an existing file), and removed by its guard.
struct TempImage(PathBuf);

impl TempImage {
    fn write(bytes: &[u8]) -> Result<TempImage, Error> {
        let name = format!("bifrost-ocr-{}-{}.ppm", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| Error::Io(format!("cannot make a temporary image: {e}")))?;
        let guard = TempImage(path);
        file.write_all(bytes).map_err(|e| Error::Io(format!("cannot write the temporary image: {e}")))?;
        Ok(guard)
    }
}

impl Drop for TempImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Reads the text in `image` in the given languages (English when none), in reading order.
pub fn recognize(image: &Rgba, languages: &[&str]) -> Result<Vec<Line>, Error> {
    let lang = language_arg(languages)?;
    if image.width().saturating_mul(image.height()) > MAX_PIXELS {
        return Err(Error::TooLarge);
    }
    let file = TempImage::write(&ppm_bytes(image))?;
    let out = Command::new(TESSERACT)
        .arg(&file.0)
        .args(["stdout", "-l", &lang, "--psm", "3", "tsv"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Error::NotInstalled } else { Error::Io(e.to_string()) })?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        return Err(Error::Failed(said.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string()));
    }
    Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext";

    fn table() -> String {
        [
            HEADER,
            "1\t1\t0\t0\t0\t0\t0\t0\t200\t100\t-1\t",
            "4\t1\t1\t1\t1\t0\t10\t10\t90\t20\t-1\t",
            "5\t1\t1\t1\t1\t1\t10\t10\t40\t20\t96.5\tHello",
            "5\t1\t1\t1\t1\t2\t60\t12\t40\t18\t88\tworld",
            "4\t1\t1\t1\t2\t0\t10\t40\t60\t20\t-1\t",
            "5\t1\t1\t1\t2\t1\t10\t40\t60\t20\t91\tSecond",
            "5\t1\t2\t1\t1\t1\t120\t70\t30\t15\t-1\t",
            "5\t1\t2\t1\t1\t2\t120\t70\t30\t15\t50\t  ",
            "5\t1\t2\t1\t2\t1\t120\t90\t30\t15\t77\tEnd",
        ]
        .join("\n")
    }

    #[test]
    fn the_engine_table_becomes_lines_of_words_in_reading_order() {
        let lines = parse_tsv(&table());
        assert_eq!(lines.len(), 3, "structure rows, a negative confidence and a blank word are not words");
        assert_eq!(lines[0].text(), "Hello world");
        assert_eq!(lines[0].words[0].bounds, PixelRect { x: 10, y: 10, w: 40, h: 20 });
        assert!((lines[0].words[0].confidence - 0.965).abs() < 1e-6);
        assert_eq!(lines[0].bounds, PixelRect { x: 10, y: 10, w: 90, h: 20 }, "the line holds both words");
        assert_eq!(lines[1].text(), "Second");
        assert_eq!(lines[2].text(), "End");
        assert_eq!(text(&lines), "Hello world\nSecond\nEnd");
        assert!((lines[0].confidence() - (0.965 + 0.88) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn malformed_and_empty_tables_give_no_lines() {
        assert!(parse_tsv("").is_empty());
        assert!(parse_tsv(HEADER).is_empty());
        assert!(parse_tsv(&format!("{HEADER}\n5\t1\t1\t1\t1\t1\tx\t10\t40\t20\t96\tbad")).is_empty(), "a non-number is skipped");
        assert!(parse_tsv(&format!("{HEADER}\n5\t1\t1")).is_empty(), "a short row is skipped");
        assert_eq!(Line { words: vec![], bounds: PixelRect { x: 0, y: 0, w: 0, h: 0 } }.confidence(), 0.0);
    }

    #[test]
    fn a_word_keeps_a_tab_inside_its_text() {
        let lines = parse_tsv(&format!("{HEADER}\n5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t90\ta\tb"));
        assert_eq!(lines[0].text(), "a\tb");
    }

    #[test]
    fn a_tap_selects_the_word_and_line_under_it() {
        let lines = parse_tsv(&table());
        let (line, word) = word_at(&lines, 70, 20).expect("a word");
        assert_eq!(word.text, "world");
        assert_eq!(line.text(), "Hello world");
        assert_eq!(word_at(&lines, 55, 15), None, "between two words is the line but no word");
        assert_eq!(line_at(&lines, 55, 15).map(Line::text), Some("Hello world".to_string()));
        assert!(line_at(&lines, 500, 500).is_none());
        assert!(word_at(&lines, 10, 10).is_some(), "the top left pixel is inside");
        assert!(word_at(&lines, 50, 10).is_none(), "the right edge is outside the word");
    }

    #[test]
    fn find_highlights_words_that_hold_the_query() {
        let lines = parse_tsv(&table());
        let hits = find(&lines, "  WOR ");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "world");
        assert!(find(&lines, "").is_empty());
        assert!(find(&lines, "zzz").is_empty());
        assert_eq!(find(&lines, "e").len(), 3, "Hello, Second and End");
    }

    #[test]
    fn languages_are_codes_joined_for_the_engine() {
        assert_eq!(language_arg(&[]), Ok("eng".to_string()));
        assert_eq!(language_arg(&["eng", "deu"]), Ok("eng+deu".to_string()));
        assert_eq!(language_arg(&["chi_sim"]), Ok("chi_sim".to_string()));
        assert_eq!(language_arg(&["eng", "--oem"]), Err(Error::BadLanguage("--oem".into())));
        assert_eq!(language_arg(&["EN"]), Err(Error::BadLanguage("EN".into())));
        assert!(!valid_language("_eng") && !valid_language("") && !valid_language("english_us_extra"));
    }

    #[test]
    fn an_image_is_handed_over_as_ppm_over_white() {
        // one opaque red pixel, one fully transparent pixel, one half transparent black pixel
        let image = Rgba::new(3, 1, vec![255, 0, 0, 255, 10, 20, 30, 0, 0, 0, 0, 128]).unwrap();
        let ppm = ppm_bytes(&image);
        let header = b"P6\n3 1\n255\n";
        assert_eq!(&ppm[..header.len()], header);
        let body = &ppm[header.len()..];
        assert_eq!(body.len(), 9);
        assert_eq!(&body[..3], &[255, 0, 0]);
        assert_eq!(&body[3..6], &[255, 255, 255], "transparent becomes white");
        assert!((120..=135).contains(&body[6]), "half-transparent black is mid grey: {}", body[6]);
    }

    #[test]
    fn a_bad_language_is_refused_before_anything_runs() {
        let image = Rgba::new(1, 1, vec![0, 0, 0, 255]).unwrap();
        assert_eq!(recognize(&image, &["--version"]), Err(Error::BadLanguage("--version".into())));
        assert_eq!(Error::NotInstalled.to_string(), "text recognition needs the tesseract package");
    }

    #[test]
    fn a_blank_page_has_no_text_when_the_engine_is_installed() {
        if !available() {
            return;
        }
        let image = Rgba::new(64, 64, vec![255; 64 * 64 * 4]).unwrap();
        let lines = recognize(&image, &[]).expect("the engine ran");
        assert!(text(&lines).trim().is_empty(), "{lines:?}");
    }
}
