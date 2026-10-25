//! Dictionary results in Search. "define serendipity", "meaning of serendipity" and "serendipity definition" ask for a
//! definition; a plain single word ("serendipity") gets one too when an installed dictionary defines it, listed after
//! the other sections. The dictionaries are the Dictionary app's (`bifrost_ui::stardict::load_library` over the app's
//! data folder and the system StarDict folder), the same ones the Look Up popover reads.
//!
//! The library is loaded once, on a worker thread, when Search opens (`preload`); until it is in, no definition row is
//! shown and `take_fresh` tells the panel to search again when it lands. The row shows the start of the definition;
//! Space on it opens the Look Up popover with the whole definition and Return opens the word in Dictionary.

use bifrost_ui::stardict::Library;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// The longest preview of a definition shown on the row (characters, before an ellipsis).
pub const PREVIEW_MAX: usize = 160;
/// A plain word is looked up from this many characters, so single letters do not bring up definitions.
const WORD_MIN: usize = 3;
/// The longest term looked up.
const TERM_MAX: usize = 48;

/// What a definition row shows: the word, the dictionary it came from and the start of the definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub word: String,
    pub dictionary: String,
    pub preview: String,
}

/// The term `query` asks a definition for, and whether it asked explicitly ("define x") rather than being a plain word.
pub fn term_of(query: &str) -> Option<(String, bool)> {
    const PREFIXES: [&str; 6] = ["define ", "definition of ", "definition ", "meaning of ", "what does ", "dict "];
    const SUFFIXES: [&str; 3] = [" definition", " meaning", " mean"];
    // compared without case on the query itself (the affixes are ASCII), so a slice never splits a character
    let head = |s: &str, p: &str| s.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p));
    let tail = |s: &str, p: &str| s.len() > p.len() && s.get(s.len() - p.len()..).is_some_and(|t| t.eq_ignore_ascii_case(p));
    let strip = |t: &str| t.trim().trim_end_matches('?').trim().to_string();
    let q = strip(query);
    let mut term: Option<String> = PREFIXES.into_iter().find(|p| head(q.as_str(), *p)).map(|p| strip(&q[p.len()..]));
    // "what does x mean": the suffix goes too
    if let Some(t) = term.as_mut() {
        if let Some(s) = SUFFIXES.into_iter().find(|s| tail(t.as_str(), *s)) {
            *t = strip(&t[..t.len() - s.len()]);
        }
    }
    if term.is_none() {
        term = SUFFIXES.into_iter().find(|s| tail(q.as_str(), *s)).map(|s| strip(&q[..q.len() - s.len()]));
    }
    if let Some(t) = term {
        let t = t.trim_matches(|c: char| c == '"' || c == '\'' || c == '\u{201c}' || c == '\u{201d}').trim();
        return (!t.is_empty() && t.chars().count() <= TERM_MAX).then(|| (t.to_string(), true));
    }
    // a plain word: letters, with an inner hyphen or apostrophe ("well-being", "don't")
    let q = query.trim();
    let n = q.chars().count();
    let wordy = q.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\'') && q.chars().next().is_some_and(char::is_alphabetic);
    (wordy && (WORD_MIN..=TERM_MAX).contains(&n)).then(|| (q.to_string(), false))
}

/// The start of a definition for the row: its lines joined, runs of spaces made one, cut at `max` characters.
pub fn preview(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max {
        return joined;
    }
    let cut: String = joined.chars().take(max).collect();
    // end on a whole word when there is one in the last part
    let cut = match cut.rfind(' ') {
        Some(i) if i > max / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{}\u{2026}", cut.trim_end_matches([',', ';', ':', '.']))
}

/// The first definition `library` has for `term`, previewed.
pub fn define_in(library: &Library, term: &str) -> Option<Definition> {
    let hit = library.lookup(term).into_iter().next()?;
    Some(Definition { word: term.to_string(), dictionary: hit.dictionary, preview: preview(&hit.text, PREVIEW_MAX) })
}

/// The definition row for `query`: the installed dictionaries' first definition of its term. An explicit request for a
/// word no dictionary has still gets a row that looks the word up in Dictionary; a plain word without one gets none.
/// None too while the library is still loading. The bool says the request was explicit (the row goes first).
pub fn lookup(query: &str) -> Option<(Definition, bool)> {
    let (term, explicit) = term_of(query)?;
    let found = cached(&term);
    match found {
        Some(def) => Some((def, explicit)),
        None if explicit => Some((Definition { word: term, dictionary: String::new(), preview: String::new() }, true)),
        None => None,
    }
}

enum Load {
    Idle,
    Loading,
    Ready(Arc<Library>),
}

static LIBRARY: Mutex<Load> = Mutex::new(Load::Idle);
/// Set by the loader when the library lands; taken by the panel to search again.
static FRESH: AtomicBool = AtomicBool::new(false);
/// How many times the library was read from disk (once per run; the tests check a second open does not read again).
static LOADS: AtomicUsize = AtomicUsize::new(0);
/// The last term looked up and its answer, so the frames of one query do not read the data file again.
static LAST: Mutex<Option<(String, Option<Definition>)>> = Mutex::new(None);

/// Starts loading the dictionaries of the Dictionary app's data folder `data_dir` (and the system's), once per run.
pub fn preload(data_dir: PathBuf) {
    let mut state = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    if !matches!(*state, Load::Idle) {
        return;
    }
    *state = Load::Loading;
    drop(state);
    crate::bare::sys::spawn_named("quicksearch-dictionary", move || {
        LOADS.fetch_add(1, Ordering::Relaxed);
        // tests never read the host's dictionaries, so a word typed in a test gets the same rows on every machine
        #[cfg(test)]
        let library = Library::load(&[data_dir.join("dictionaries")], std::path::Path::new("/nonexistent/words"));
        #[cfg(not(test))]
        let library = bifrost_ui::stardict::load_library(&data_dir);
        install(library);
    });
}

/// Makes `library` the one Search reads (the loader, and tests).
pub fn install(library: Library) {
    *LIBRARY.lock().unwrap_or_else(|e| e.into_inner()) = Load::Ready(Arc::new(library));
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = None;
    FRESH.store(true, Ordering::Release);
}

/// True once after the library landed: the panel searches again so a definition can show.
pub fn take_fresh() -> bool {
    FRESH.swap(false, Ordering::AcqRel)
}

/// How many times the library was read from disk this run.
pub fn loads() -> usize {
    LOADS.load(Ordering::Relaxed)
}

/// The whole first definition of `term` for the Look Up popover, from the library Search reads: `None` while it is
/// still loading (call `preload` first), `Some(None)` when no installed dictionary defines the word.
pub fn definition(term: &str) -> Option<Option<String>> {
    let lib = library()?;
    Some(lib.lookup(term).into_iter().next().map(|hit| hit.text))
}

/// The library when it has loaded.
fn library() -> Option<Arc<Library>> {
    match &*LIBRARY.lock().unwrap_or_else(|e| e.into_inner()) {
        Load::Ready(l) => Some(Arc::clone(l)),
        _ => None,
    }
}

/// `define_in` over the loaded library, remembered for the last term.
fn cached(term: &str) -> Option<Definition> {
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, d)) = last.as_ref() {
        if t == term {
            return d.clone();
        }
    }
    let lib = library()?;
    let found = define_in(&lib, term);
    *last = Some((term.to_string(), found.clone()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn define_and_meaning_ask_explicitly_and_a_plain_word_implicitly() {
        assert_eq!(term_of("define weather"), Some(("weather".into(), true)));
        assert_eq!(term_of("Define Weather"), Some(("Weather".into(), true)));
        assert_eq!(term_of("definition of serendipity"), Some(("serendipity".into(), true)));
        assert_eq!(term_of("meaning of life"), Some(("life".into(), true)));
        assert_eq!(term_of("what does ephemeral mean?"), Some(("ephemeral".into(), true)));
        assert_eq!(term_of("ephemeral meaning"), Some(("ephemeral".into(), true)));
        assert_eq!(term_of("ephemeral definition"), Some(("ephemeral".into(), true)));
        assert_eq!(term_of("define \"ice cream\""), Some(("ice cream".into(), true)));
        assert_eq!(term_of("weather"), Some(("weather".into(), false)));
        assert_eq!(term_of("well-being"), Some(("well-being".into(), false)));
        assert_eq!(term_of("kaffi\u{f0}"), Some(("kaffi\u{f0}".into(), false)));
    }

    #[test]
    fn short_words_numbers_sums_and_phrases_are_not_looked_up() {
        assert_eq!(term_of("ab"), None);
        assert_eq!(term_of("e2e"), None);
        assert_eq!(term_of("2+2*3"), None);
        assert_eq!(term_of("10 km in miles"), None);
        assert_eq!(term_of("define ?"), None);
        assert_eq!(term_of("define"), Some(("define".into(), false)), "the bare word is a plain word");
        assert_eq!(term_of(" "), None);
        assert_eq!(term_of("-dash"), None);
    }

    #[test]
    fn a_preview_joins_lines_and_ends_on_a_word_with_an_ellipsis() {
        assert_eq!(preview("n.\n  the state of the\tatmosphere", 100), "n. the state of the atmosphere");
        assert_eq!(preview("one two three four five", 12), "one two\u{2026}");
        assert_eq!(preview("abcdefghijklmnop", 5), "abcde\u{2026}");
        assert_eq!(preview("", 10), "");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qs-dict-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A one-word StarDict dictionary in `dir`: `weather` defined as `text`.
    fn one_word(dir: &Path, text: &str) {
        let mut idx = b"weather\0".to_vec();
        idx.extend(0u32.to_be_bytes());
        idx.extend((text.len() as u32).to_be_bytes());
        fs::write(dir.join("test.idx"), &idx).unwrap();
        fs::write(dir.join("test.dict"), text).unwrap();
        fs::write(dir.join("test.ifo"), format!("StarDict's dict ifo file\nversion=2.4.2\nbookname=Test Dictionary\nwordcount=1\nidxfilesize={}\nsametypesequence=m\n", idx.len())).unwrap();
    }

    #[test]
    fn the_library_is_read_once_however_often_it_is_asked_for() {
        let dir = scratch("once");
        fs::create_dir_all(dir.join("dictionaries")).unwrap();
        one_word(&dir.join("dictionaries"), "n. the atmosphere");
        let before = loads();
        preload(dir.clone());
        // a second open while it loads, and after, starts no second read
        preload(dir.clone());
        let start = std::time::Instant::now();
        while definition("weather").is_none() && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::yield_now();
        }
        assert!(definition("weather").is_some(), "the library came in");
        preload(dir.clone());
        assert!(loads() - before <= 1, "read {} times", loads() - before);
        let after = loads();
        preload(dir.clone());
        assert_eq!(loads(), after);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_defined_word_gets_its_dictionary_and_preview_and_an_unknown_one_nothing() {
        let dir = scratch("define");
        one_word(&dir, "n. the state of the\natmosphere at a place");
        let lib = Library::load(&[dir.clone()], &dir.join("no-words"));
        let def = define_in(&lib, "Weather").expect("defined");
        assert_eq!(def.dictionary, "Test Dictionary");
        assert_eq!(def.preview, "n. the state of the atmosphere at a place");
        assert_eq!(define_in(&lib, "climate"), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
