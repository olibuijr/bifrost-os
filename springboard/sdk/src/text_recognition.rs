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
