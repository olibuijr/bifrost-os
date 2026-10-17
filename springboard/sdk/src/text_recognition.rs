//! Text recognition for apps (`VNRecognizeTextRequest`, Live Text): the words in a picture, with where each sits and how
//! sure the reader is.
//!
//! The reading is done on this computer by the `tesseract` program (package `tesseract`, with `tesseract-data-<lang>`
//! for each language), which an app never talks to directly. [`Recognizer`] starts it with the picture on its standard
//! input, reads its tab-separated result and turns that into [`Recognition`]: lines of [`RecognizedWord`] values in
//! reading order, each with a rectangle in picture pixels (origin top-left) and a confidence from 0 to 1. Nothing is sent
//! anywhere and nothing is written to disk.
//!
//! The program is behind [`Runner`], so the parsing and the argument building are tested without it. Limits: the
//! picture must be a PNG, JPEG, GIF, BMP, TIFF or WebP file of at most [`MAX_IMAGE_BYTES`]; handwriting and text on a
//! strong curve read poorly; a language needs its data package, and [`Recognizer::languages`] lists what is installed.
//! When the program is missing every call fails with [`RecognitionError::EngineMissing`] rather than returning no text.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

/// The engine program.
pub const PROGRAM: &str = "tesseract";

/// The largest picture accepted, in bytes.
pub const MAX_IMAGE_BYTES: usize = 64 << 20;

/// Why a reading failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecognitionError {
    /// The engine program is not installed.
    EngineMissing,
    /// A language tag that has no data name (the tag is kept).
    UnknownLanguage(String),
    /// The bytes are not a picture format the engine reads, or there are none, or there are too many.
    UnsupportedImage,
    /// The engine ran and reported a failure (its last message).
    Failed(String),
    /// The engine's answer is not the table that was asked for.
    BadOutput,
}

/// A rectangle in picture pixels, origin top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Bounds {
    fn union(self, other: Bounds) -> Bounds {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.w).max(other.x + other.w);
        let bottom = (self.y + self.h).max(other.y + other.h);
        Bounds { x, y, w: right - x, h: bottom - y }
    }
}

/// One word.
#[derive(Clone, Debug, PartialEq)]
pub struct RecognizedWord {
    pub text: String,
    /// 0 to 1.
    pub confidence: f32,
    pub bounds: Bounds,
}

/// One line of words, left to right.
#[derive(Clone, Debug, PartialEq)]
pub struct RecognizedLine {
    pub words: Vec<RecognizedWord>,
    pub bounds: Bounds,
}

impl RecognizedLine {
    /// The words joined by single spaces.
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    /// The mean confidence of the words, 0 for an empty line.
    pub fn confidence(&self) -> f32 {
        if self.words.is_empty() {
            return 0.0;
        }
        self.words.iter().map(|w| w.confidence).sum::<f32>() / self.words.len() as f32
    }
}

/// Everything read from one picture, in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recognition {
    pub lines: Vec<RecognizedLine>,
}

impl Recognition {
    /// The lines joined by line breaks.
    pub fn text(&self) -> String {
        self.lines.iter().map(RecognizedLine::text).collect::<Vec<_>>().join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Only the words whose confidence is at least `minimum`; a line left with no word is dropped and the rectangle of
    /// a line that lost words is recomputed.
    pub fn at_least(&self, minimum: f32) -> Recognition {
        let mut lines = Vec::new();
        for line in &self.lines {
            let words: Vec<RecognizedWord> = line.words.iter().filter(|w| w.confidence >= minimum).cloned().collect();
            if let Some(bounds) = words.iter().map(|w| w.bounds).reduce(Bounds::union) {
                lines.push(RecognizedLine { words, bounds });
            }
        }
        Recognition { lines }
    }
}

/// Reads the table `tesseract ... tsv` prints: a header, then one row per page, block, paragraph, line and word with
/// `level page block paragraph line word left top width height confidence text`. Only word rows (level 5) with a text
/// and a non-negative confidence are kept; lines are the words that share a block, paragraph and line number, in the
/// order the engine listed them.
pub fn parse_tsv(tsv: &str) -> Result<Recognition, RecognitionError> {
    let mut rows = tsv.lines();
    match rows.next() {
        Some(header) if header.starts_with("level\t") => {}
        _ => return Err(RecognitionError::BadOutput),
    }
    let mut order: Vec<(u32, u32, u32)> = Vec::new();
    let mut lines: HashMap<(u32, u32, u32), RecognizedLine> = HashMap::new();
    for row in rows {
        let f: Vec<&str> = row.trim_end_matches('\r').split('\t').collect();
        if f.len() < 12 || f[0] != "5" {
            continue;
        }
        let num = |i: usize| f[i].parse::<u32>().ok();
        let px = |i: usize| f[i].parse::<f32>().ok();
        let (Some(block), Some(par), Some(line)) = (num(2), num(3), num(4)) else { continue };
        let (Some(x), Some(y), Some(w), Some(h), Some(conf)) = (px(6), px(7), px(8), px(9), px(10)) else { continue };
        let text = f[11].trim();
        if text.is_empty() || conf < 0.0 {
            continue;
        }
        let bounds = Bounds { x, y, w, h };
        let word = RecognizedWord { text: text.to_string(), confidence: (conf / 100.0).clamp(0.0, 1.0), bounds };
        let key = (block, par, line);
        match lines.get_mut(&key) {
            Some(l) => {
                l.bounds = l.bounds.union(bounds);
                l.words.push(word);
            }
            None => {
                order.push(key);
                lines.insert(key, RecognizedLine { words: vec![word], bounds });
            }
        }
    }
    Ok(Recognition { lines: order.into_iter().filter_map(|k| lines.remove(&k)).collect() })
}

/// The engine's data name for a language tag (`is`, `en-GB`, `zh-Hans`). A tag that already is a data name (`eng`,
/// `chi_sim`) is kept.
pub fn data_name(tag: &str) -> Option<String> {
    let lower = tag.trim().to_ascii_lowercase().replace('_', "-");
    let primary = lower.split('-').next().unwrap_or("");
    let name = match primary {
        "en" => "eng",
        "is" => "isl",
        "de" => "deu",
        "fr" => "fra",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "nl" => "nld",
        "sv" => "swe",
        "da" => "dan",
        "nb" | "nn" | "no" => "nor",
        "fi" => "fin",
        "pl" => "pol",
        "cs" => "ces",
        "tr" => "tur",
        "el" => "ell",
        "ru" => "rus",
        "uk" => "ukr",
        "ar" => "ara",
        "he" => "heb",
        "hi" => "hin",
        "ja" => "jpn",
        "ko" => "kor",
        "zh" => {
            let traditional = lower.contains("hant") || lower.ends_with("-tw") || lower.ends_with("-hk");
            return Some(if traditional { "chi_tra".to_string() } else { "chi_sim".to_string() });
        }
        _ => {
            // already a data name: three or more lower-case letters, optionally with underscores
            let raw = tag.trim();
            let plain = raw.len() >= 3 && raw.len() <= 12 && raw.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
            return plain.then(|| raw.to_string());
        }
    };
    Some(name.to_string())
}

/// The `-l` argument for several tags: data names joined by `+`, `eng` when there are none.
pub fn language_argument(tags: &[&str]) -> Result<String, RecognitionError> {
    if tags.is_empty() {
        return Ok("eng".to_string());
    }
    let mut names = Vec::new();
    for tag in tags {
        names.push(data_name(tag).ok_or_else(|| RecognitionError::UnknownLanguage((*tag).to_string()))?);
    }
    Ok(names.join("+"))
}

/// The data names in the output of `tesseract --list-langs` (a heading line, then one name per line).
pub fn parse_language_list(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "osd")
        .map(str::to_string)
        .collect()
}

/// True when the bytes start like a picture the engine reads.
pub fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || bytes.starts_with(b"BM")
        || bytes.starts_with(&[b'I', b'I', 42, 0])
        || bytes.starts_with(&[b'M', b'M', 0, 42])
        || (bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
}

/// How the engine splits the page into text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Columns, paragraphs and lines are found.
    Page,
    /// One block of text.
    Block,
    /// One line of text.
    Line,
}

impl Layout {
    fn mode(self) -> &'static str {
        match self {
            Layout::Page => "3",
            Layout::Block => "6",
            Layout::Line => "7",
        }
    }
}

/// One reading.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    /// The encoded picture.
    pub image: &'a [u8],
    /// Language tags; none means English.
    pub languages: &'a [&'a str],
    pub layout: Layout,
    /// Words below this confidence (0 to 1) are dropped.
    pub minimum_confidence: f32,
}

impl<'a> Request<'a> {
    pub fn new(image: &'a [u8]) -> Request<'a> {
        Request { image, languages: &[], layout: Layout::Page, minimum_confidence: 0.0 }
    }
}

/// What a finished engine run left.
#[derive(Clone, Debug, Default)]
pub struct RunOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub success: bool,
}

/// Runs the engine program with `stdin` as its input.
pub trait Runner {
    fn run(&self, program: &str, args: &[String], stdin: &[u8]) -> Result<RunOutput, RecognitionError>;
}

/// Starts the real program.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[String], stdin: &[u8]) -> Result<RunOutput, RecognitionError> {
        let mut child = Command::new(program)
            .args(args)
            .env("OMP_THREAD_LIMIT", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    RecognitionError::EngineMissing
                } else {
                    RecognitionError::Failed(e.to_string())
                }
            })?;
        let pipe = child.stdin.take();
        let input = stdin.to_vec();
        // written on its own thread so a large picture and a large answer cannot block each other
        let writer = std::thread::spawn(move || {
            if let Some(mut pipe) = pipe {
                let _ = pipe.write_all(&input);
            }
        });
        let out = child.wait_with_output().map_err(|e| RecognitionError::Failed(e.to_string()))?;
        let _ = writer.join();
        Ok(RunOutput { stdout: out.stdout, stderr: out.stderr, success: out.status.success() })
    }
}

/// Reads text from pictures through a [`Runner`].
#[derive(Clone, Debug, Default)]
pub struct Recognizer<R: Runner = SystemRunner> {
    runner: R,
}

impl Recognizer<SystemRunner> {
    /// A recognizer that starts the installed `tesseract` program.
    pub fn system() -> Recognizer<SystemRunner> {
        Recognizer { runner: SystemRunner }
    }
}

impl<R: Runner> Recognizer<R> {
    pub fn with_runner(runner: R) -> Recognizer<R> {
        Recognizer { runner }
    }

    /// The arguments of one reading.
    fn arguments(languages: &str, layout: Layout) -> Vec<String> {
        ["stdin", "stdout", "-l", languages, "--psm", layout.mode(), "tsv"].iter().map(|s| s.to_string()).collect()
    }

    /// Reads the picture. The languages are checked and the picture is sniffed before anything is started.
    pub fn recognize(&self, request: &Request) -> Result<Recognition, RecognitionError> {
        if request.image.is_empty() || request.image.len() > MAX_IMAGE_BYTES || !looks_like_image(request.image) {
            return Err(RecognitionError::UnsupportedImage);
        }
        let languages = language_argument(request.languages)?;
        let out = self.runner.run(PROGRAM, &Self::arguments(&languages, request.layout), request.image)?;
        if !out.success {
            let message = String::from_utf8_lossy(&out.stderr);
            let last = message.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("the engine failed");
            return Err(RecognitionError::Failed(last.trim().to_string()));
        }
        let table = String::from_utf8(out.stdout).map_err(|_| RecognitionError::BadOutput)?;
        Ok(parse_tsv(&table)?.at_least(request.minimum_confidence))
    }

    /// The installed language data names.
    pub fn languages(&self) -> Result<Vec<String>, RecognitionError> {
        let out = self.runner.run(PROGRAM, &["--list-langs".to_string()], &[])?;
        // older versions print the list on the error stream
        let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
        if !out.success {
            return Err(RecognitionError::Failed("the language list could not be read".into()));
        }
        Ok(parse_language_list(&String::from_utf8_lossy(&text)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const HEADER: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n";

    fn sample() -> String {
        format!(
            "{HEADER}\
1\t1\t0\t0\t0\t0\t0\t0\t200\t100\t-1\t\n\
5\t1\t1\t1\t1\t1\t10\t10\t40\t20\t96.5\tHello\n\
5\t1\t1\t1\t1\t2\t60\t12\t50\t18\t91.0\tworld\n\
5\t1\t1\t1\t2\t1\t10\t40\t30\t20\t30.0\tnoise\n\
5\t1\t1\t1\t2\t2\t45\t40\t20\t20\t-1\t \n"
        )
    }

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0];

    struct Fake {
        answer: RefCell<Option<Result<RunOutput, RecognitionError>>>,
        seen: RefCell<Vec<(String, Vec<String>, usize)>>,
    }

    impl Fake {
        fn answering(r: Result<RunOutput, RecognitionError>) -> Fake {
            Fake { answer: RefCell::new(Some(r)), seen: RefCell::new(Vec::new()) }
        }
    }

    impl Runner for &Fake {
        fn run(&self, program: &str, args: &[String], stdin: &[u8]) -> Result<RunOutput, RecognitionError> {
            self.seen.borrow_mut().push((program.to_string(), args.to_vec(), stdin.len()));
            self.answer.borrow_mut().take().unwrap_or(Err(RecognitionError::BadOutput))
        }
    }

    fn ok(text: &str) -> Result<RunOutput, RecognitionError> {
        Ok(RunOutput { stdout: text.as_bytes().to_vec(), stderr: Vec::new(), success: true })
    }

    #[test]
    fn words_are_grouped_into_lines_in_the_engine_order() {
        let r = parse_tsv(&sample()).unwrap();
        assert_eq!(r.lines.len(), 2);
        assert_eq!(r.lines[0].text(), "Hello world");
        assert_eq!(r.lines[1].text(), "noise");
        assert_eq!(r.text(), "Hello world\nnoise");
        assert!(!r.is_empty());
    }

    #[test]
    fn a_line_rectangle_covers_its_words_and_confidence_is_scaled_to_one() {
        let r = parse_tsv(&sample()).unwrap();
        assert_eq!(r.lines[0].bounds, Bounds { x: 10.0, y: 10.0, w: 100.0, h: 20.0 });
        assert!((r.lines[0].words[0].confidence - 0.965).abs() < 1e-6);
        assert!((r.lines[0].confidence() - 0.9375).abs() < 1e-6);
    }

    #[test]
    fn rows_without_text_or_confidence_and_non_word_levels_are_skipped() {
        let r = parse_tsv(&sample()).unwrap();
        assert_eq!(r.lines[1].words.len(), 1);
        assert_eq!(parse_tsv(HEADER).unwrap(), Recognition::default());
    }

    #[test]
    fn a_table_without_the_header_is_refused() {
        assert_eq!(parse_tsv(""), Err(RecognitionError::BadOutput));
        assert_eq!(parse_tsv("not a table\n"), Err(RecognitionError::BadOutput));
    }

    #[test]
    fn low_confidence_words_are_dropped_and_empty_lines_with_them() {
        let r = parse_tsv(&sample()).unwrap().at_least(0.5);
        assert_eq!(r.lines.len(), 1);
        let one = parse_tsv(&sample()).unwrap().at_least(0.95);
        assert_eq!(one.lines[0].text(), "Hello");
        assert_eq!(one.lines[0].bounds, Bounds { x: 10.0, y: 10.0, w: 40.0, h: 20.0 });
    }

    #[test]
    fn language_tags_map_to_data_names() {
        assert_eq!(data_name("is").as_deref(), Some("isl"));
        assert_eq!(data_name("en-GB").as_deref(), Some("eng"));
        assert_eq!(data_name("zh-Hans").as_deref(), Some("chi_sim"));
        assert_eq!(data_name("zh-Hant").as_deref(), Some("chi_tra"));
        assert_eq!(data_name("chi_sim").as_deref(), Some("chi_sim"));
        assert_eq!(data_name("../etc"), None);
        assert_eq!(data_name("x"), None);
        assert_eq!(language_argument(&[]).unwrap(), "eng");
        assert_eq!(language_argument(&["is", "en"]).unwrap(), "isl+eng");
        assert_eq!(language_argument(&["is", "??"]), Err(RecognitionError::UnknownLanguage("??".into())));
    }

    #[test]
    fn the_language_list_skips_its_heading_and_the_orientation_data() {
        let text = "List of available languages in \"/usr/share/tessdata/\" (3):\neng\nisl\nosd\n";
        assert_eq!(parse_language_list(text), vec!["eng".to_string(), "isl".to_string()]);
    }

    #[test]
    fn only_picture_formats_are_accepted() {
        assert!(looks_like_image(PNG));
        assert!(looks_like_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(looks_like_image(b"RIFF\x10\0\0\0WEBPVP8 "));
        assert!(!looks_like_image(b"%PDF-1.7"));
        assert!(!looks_like_image(b""));
    }

    #[test]
    fn a_reading_passes_the_picture_on_stdin_with_the_asked_languages_and_layout() {
        let fake = Fake::answering(ok(&sample()));
        let recognizer = Recognizer::with_runner(&fake);
        let mut request = Request::new(PNG);
        request.languages = &["is", "en"];
        request.layout = Layout::Line;
        request.minimum_confidence = 0.5;
        let r = recognizer.recognize(&request).unwrap();
        assert_eq!(r.text(), "Hello world");
        let seen = fake.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "tesseract");
        assert_eq!(seen[0].1, ["stdin", "stdout", "-l", "isl+eng", "--psm", "7", "tsv"]);
        assert_eq!(seen[0].2, PNG.len());
    }

    #[test]
    fn bad_input_is_refused_before_the_engine_starts() {
        let fake = Fake::answering(ok(&sample()));
        let recognizer = Recognizer::with_runner(&fake);
        assert_eq!(recognizer.recognize(&Request::new(b"not a picture")), Err(RecognitionError::UnsupportedImage));
        assert_eq!(recognizer.recognize(&Request::new(&[])), Err(RecognitionError::UnsupportedImage));
        let mut request = Request::new(PNG);
        request.languages = &["??"];
        assert_eq!(recognizer.recognize(&request), Err(RecognitionError::UnknownLanguage("??".into())));
        assert!(fake.seen.borrow().is_empty());
    }

    #[test]
    fn a_missing_engine_and_a_failed_run_are_errors_not_empty_text() {
        let missing = Fake::answering(Err(RecognitionError::EngineMissing));
        assert_eq!(Recognizer::with_runner(&missing).recognize(&Request::new(PNG)), Err(RecognitionError::EngineMissing));
        let failed = Fake::answering(Ok(RunOutput { stdout: Vec::new(), stderr: b"Error opening data file\nFailed loading language 'isl'\n".to_vec(), success: false }));
        assert_eq!(
            Recognizer::with_runner(&failed).recognize(&Request::new(PNG)),
            Err(RecognitionError::Failed("Failed loading language 'isl'".into()))
        );
        let garbled = Fake::answering(Ok(RunOutput { stdout: vec![0xFF, 0xFE], stderr: Vec::new(), success: true }));
        assert_eq!(Recognizer::with_runner(&garbled).recognize(&Request::new(PNG)), Err(RecognitionError::BadOutput));
    }

    #[test]
    fn installed_languages_come_from_either_stream() {
        let on_stderr = Fake::answering(Ok(RunOutput { stdout: Vec::new(), stderr: b"List of available languages (1):\neng\n".to_vec(), success: true }));
        assert_eq!(Recognizer::with_runner(&on_stderr).languages().unwrap(), vec!["eng".to_string()]);
        let seen = on_stderr.seen.borrow();
        assert_eq!(seen[0].1, ["--list-langs"]);
    }
}
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
