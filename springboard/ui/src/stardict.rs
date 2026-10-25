//! Dictionaries for the Dictionary app and the Look Up popover, in the StarDict layout (an `.ifo` description, an `.idx`
//! word index and a `.dict` data file, as dictd-style packages ship them). Parsing and lookup are pure; reading a
//! definition from the data file is the only I/O. A plain word list gives a spelling answer when no installed
//! dictionary defines a word.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The system's StarDict folder, read after the app's own dictionaries.
pub const SYSTEM_DICTIONARIES: &str = "/usr/share/stardict";
/// The system word list (one word per line), for spelling answers.
pub const WORD_LIST: &str = "/usr/share/dict/words";

/// Loads every dictionary an app's data folder and the system hold, and the word list. Shared by the Dictionary app
/// and the shell's Look Up popover, so both read the same dictionaries.
pub fn load_library(data_dir: &Path) -> Library {
    Library::load(&[data_dir.join("dictionaries"), PathBuf::from(SYSTEM_DICTIONARIES)], Path::new(WORD_LIST))
}

/// The first line of every `.ifo` file.
const IFO_MAGIC: &str = "StarDict's dict ifo file";
/// Largest definition read from the data file (bytes). A larger one is reported instead of read.
pub const DEFINITION_MAX: u64 = 1 << 20;

/// What the `.ifo` file says about a dictionary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub word_count: usize,
    /// Width of the offsets in the index: 32 or 64 bits.
    pub offset_bits: u32,
    /// The type every definition has, when the file names one (`m` plain text, `h` HTML).
    pub same_type: Option<char>,
}

/// Parses an `.ifo` file: the magic line, then `key=value` lines. A bookname is required; the rest have defaults.
pub fn parse_ifo(text: &str) -> Result<Info, String> {
    let mut lines = text.lines();
    if lines.next().unwrap_or("").trim() != IFO_MAGIC {
        return Err("not a StarDict .ifo file".to_string());
    }
    let mut name = None;
    let mut word_count = 0;
    let mut offset_bits = 32;
    let mut same_type = None;
    for line in lines {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "bookname" => name = Some(value.to_string()),
            "wordcount" => word_count = value.parse().unwrap_or(0),
            "idxoffsetbits" => {
                offset_bits = match value {
                    "32" => 32,
                    "64" => 64,
                    _ => return Err(format!("unsupported idxoffsetbits {value}")),
                }
            }
            "sametypesequence" => same_type = value.chars().next(),
            _ => {}
        }
    }
    let name = name.ok_or_else(|| "the .ifo file has no bookname".to_string())?;
    Ok(Info { name, word_count, offset_bits, same_type })
}

/// One index entry: a headword and where its definition sits in the data file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub word: String,
    pub offset: u64,
    pub size: u64,
}

/// Reads the `.idx` file: per entry, the headword (NUL-ended), the offset and the size, both big-endian.
pub fn parse_idx(bytes: &[u8], offset_bits: u32) -> Result<Vec<Entry>, String> {
    let width = if offset_bits == 64 { 8 } else { 4 };
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let nul = bytes[i..].iter().position(|&b| b == 0).ok_or("the index ends inside a headword")?;
        let word = String::from_utf8_lossy(&bytes[i..i + nul]).into_owned();
        let p = i + nul + 1;
        let end = p + width + 4;
        if end > bytes.len() {
            return Err("the index is cut short".to_string());
        }
        let offset = if width == 8 {
            u64::from_be_bytes(bytes[p..p + 8].try_into().unwrap_or([0; 8]))
        } else {
            u32::from_be_bytes(bytes[p..p + 4].try_into().unwrap_or([0; 4])) as u64
        };
        let size = u32::from_be_bytes(bytes[p + width..end].try_into().unwrap_or([0; 4])) as u64;
        out.push(Entry { word, offset, size });
        i = end;
    }
    Ok(out)
}

/// The key words are found and sorted by: lowercase, without surrounding spaces.
pub fn key_of(word: &str) -> String {
    word.trim().to_lowercase()
}

#[derive(Debug)]
struct Row {
    key: String,
    entry: Entry,
}

/// One opened dictionary: its description, its sorted index and the path of its data file.
#[derive(Debug)]
pub struct Dictionary {
    pub info: Info,
    rows: Vec<Row>,
    data_path: PathBuf,
}

impl Dictionary {
    /// Opens the dictionary described by `ifo_path`; its index and data file sit next to it with the same stem.
    pub fn open(ifo_path: &Path) -> Result<Dictionary, String> {
        let ifo = fs::read_to_string(ifo_path).map_err(|e| format!("{}: {e}", ifo_path.display()))?;
        let info = parse_ifo(&ifo).map_err(|e| format!("{}: {e}", ifo_path.display()))?;
        let idx_path = ifo_path.with_extension("idx");
        let data_path = ifo_path.with_extension("dict");
        let idx = fs::read(&idx_path).map_err(|e| format!("{}: {e}", idx_path.display()))?;
        if !data_path.is_file() {
            return Err(format!("{}: no data file beside it", ifo_path.display()));
        }
        Dictionary::from_parts(info, &idx, data_path)
    }

    /// Builds the dictionary from its parts, sorting the index for lookup.
    pub fn from_parts(info: Info, idx: &[u8], data_path: PathBuf) -> Result<Dictionary, String> {
        let entries = parse_idx(idx, info.offset_bits)?;
        let mut rows: Vec<Row> = entries.into_iter().map(|entry| Row { key: key_of(&entry.word), entry }).collect();
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(Dictionary { info, rows, data_path })
    }

    pub fn word_count(&self) -> usize {
        self.rows.len()
    }

    /// The entry for `word`, ignoring case and surrounding spaces.
    pub fn find(&self, word: &str) -> Option<&Entry> {
        let key = key_of(word);
        if key.is_empty() {
            return None;
        }
        let i = self.rows.partition_point(|r| r.key < key);
        self.rows.get(i).filter(|r| r.key == key).map(|r| &r.entry)
    }

    /// Up to `limit` headwords that start with `prefix`, in dictionary order.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<&str> {
        let key = key_of(prefix);
        if key.is_empty() {
            return Vec::new();
        }
        let start = self.rows.partition_point(|r| r.key < key);
        self.rows[start..].iter().take_while(|r| r.key.starts_with(&key)).take(limit).map(|r| r.entry.word.as_str()).collect()
    }

    /// The text of `entry`'s definition, read from the data file.
    pub fn read(&self, entry: &Entry) -> Result<String, String> {
        if entry.size > DEFINITION_MAX {
            return Err("the definition is too large to show".to_string());
        }
        let mut file = fs::File::open(&self.data_path).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(entry.offset)).map_err(|e| e.to_string())?;
        let mut block = vec![0u8; entry.size as usize];
        file.read_exact(&mut block).map_err(|e| e.to_string())?;
        Ok(plain_text(&block, self.info.same_type))
    }
}

/// The text of one stored definition. When the file names no type, each block starts with its type byte and
/// ends at a NUL.
pub fn plain_text(block: &[u8], same_type: Option<char>) -> String {
    let (kind, body) = match same_type {
        Some(t) => (t, block),
        None => match block.split_first() {
            Some((&t, rest)) => (t as char, rest.split(|&b| b == 0).next().unwrap_or_default()),
            None => return String::new(),
        },
    };
    let text = String::from_utf8_lossy(body);
    if kind == 'h' {
        strip_tags(&text)
    } else {
        text.trim().to_string()
    }
}

/// Removes HTML tags, keeps line breaks at block tags and decodes the entities dictionaries use.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        if matches!(name, "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            out.push('\n');
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let mut lines: Vec<&str> = Vec::new();
    for line in decoded.lines().map(str::trim) {
        let blank_before = lines.last().map_or(true, |p| p.is_empty());
        if line.is_empty() && blank_before {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

/// One dictionary's answer for a word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub dictionary: String,
    pub text: String,
}

/// Every dictionary the app can see, and the system word list.
#[derive(Debug, Default)]
pub struct Library {
    pub dictionaries: Vec<Dictionary>,
    /// Problems met while loading, each naming the file, so the app can say why a dictionary is missing.
    pub problems: Vec<String>,
    /// Sorted lowercase words of the system word list; empty when there is none.
    words: Vec<String>,
}

impl Library {
    /// Opens every `.ifo` dictionary in `dirs`, in order and by file name, and reads the word list when it exists.
    /// A directory that does not exist is skipped.
    pub fn load(dirs: &[PathBuf], word_list: &Path) -> Library {
        let mut lib = Library::default();
        for dir in dirs {
            let Ok(entries) = fs::read_dir(dir) else { continue };
            let mut ifos: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "ifo"))
                .collect();
            ifos.sort();
            for ifo in ifos {
                match Dictionary::open(&ifo) {
                    Ok(d) => lib.dictionaries.push(d),
                    Err(e) => lib.problems.push(e),
                }
            }
        }
        if let Ok(text) = fs::read_to_string(word_list) {
            let mut words: Vec<String> = text.lines().map(key_of).filter(|w| !w.is_empty()).collect();
            words.sort();
            words.dedup();
            lib.words = words;
        }
        lib
    }

    pub fn is_empty(&self) -> bool {
        self.dictionaries.is_empty()
    }

    /// Every dictionary's definition of `word`, in load order. Empty when none defines it.
    pub fn lookup(&self, word: &str) -> Vec<Hit> {
        self.dictionaries
            .iter()
            .filter_map(|d| {
                let entry = d.find(word)?;
                let text = d.read(entry).unwrap_or_else(|e| format!("(could not read this definition: {e})"));
                Some(Hit { dictionary: d.info.name.clone(), text })
            })
            .collect()
    }

    /// Up to `limit` distinct headwords that start with `prefix`, from every dictionary.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in &self.dictionaries {
            for w in d.suggest(prefix, limit) {
                if out.len() >= limit {
                    return out;
                }
                if !out.iter().any(|o| o == w) {
                    out.push(w.to_string());
                }
            }
        }
        out
    }

    /// Words in all loaded dictionaries, counted per dictionary.
    pub fn word_count(&self) -> usize {
        self.dictionaries.iter().map(Dictionary::word_count).sum()
    }

    /// True when the system word list has `word`, whether or not a dictionary defines it.
    pub fn contains_word(&self, word: &str) -> bool {
        self.words.binary_search(&key_of(word)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dictionary-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Index bytes for `(word, offset, size)` rows, with 32-bit offsets.
    fn idx(rows: &[(&str, u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        for &(word, offset, size) in rows {
            out.extend_from_slice(word.as_bytes());
            out.push(0);
            out.extend_from_slice(&offset.to_be_bytes());
            out.extend_from_slice(&size.to_be_bytes());
        }
        out
    }

    /// Writes a plain-text StarDict dictionary named `name` with `words` (headword, definition) into `dir`.
    fn write_dictionary(dir: &Path, stem: &str, name: &str, words: &[(&str, &str)]) -> PathBuf {
        let mut data = Vec::new();
        let mut index = Vec::new();
        for &(word, def) in words {
            index.extend(idx(&[(word, data.len() as u32, def.len() as u32)]));
            data.extend_from_slice(def.as_bytes());
        }
        let ifo = format!(
            "StarDict's dict ifo file\nversion=2.4.2\nbookname={name}\nwordcount={}\nidxfilesize={}\nsametypesequence=m\n",
            words.len(),
            index.len()
        );
        let ifo_path = dir.join(format!("{stem}.ifo"));
        fs::write(&ifo_path, ifo).unwrap();
        fs::write(dir.join(format!("{stem}.idx")), index).unwrap();
        fs::write(dir.join(format!("{stem}.dict")), data).unwrap();
        ifo_path
    }

    #[test]
    fn ifo_gives_name_count_and_type_and_rejects_other_files() {
        let info = parse_ifo("StarDict's dict ifo file\nversion=2.4.2\nbookname=Small\nwordcount=3\nsametypesequence=h\n").unwrap();
        assert_eq!(info, Info { name: "Small".into(), word_count: 3, offset_bits: 32, same_type: Some('h') });
        assert!(parse_ifo("hello\nbookname=x\n").is_err());
        assert!(parse_ifo("StarDict's dict ifo file\nwordcount=3\n").is_err());
        assert!(parse_ifo("StarDict's dict ifo file\nbookname=x\nidxoffsetbits=16\n").is_err());
    }

    #[test]
    fn index_entries_carry_word_offset_and_size() {
        let bytes = idx(&[("cat", 0, 3), ("dog", 3, 5)]);
        let entries = parse_idx(&bytes, 32).unwrap();
        assert_eq!(entries[1], Entry { word: "dog".into(), offset: 3, size: 5 });
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn a_cut_short_index_is_an_error() {
        let mut bytes = idx(&[("cat", 0, 3)]);
        bytes.truncate(bytes.len() - 1);
        assert!(parse_idx(&bytes, 32).is_err());
        assert!(parse_idx(b"no-end", 32).is_err());
    }

    #[test]
    fn sixty_four_bit_offsets_are_read_eight_bytes_wide() {
        let mut bytes = b"big\0".to_vec();
        bytes.extend_from_slice(&(1u64 << 40).to_be_bytes());
        bytes.extend_from_slice(&7u32.to_be_bytes());
        let entries = parse_idx(&bytes, 64).unwrap();
        assert_eq!(entries, vec![Entry { word: "big".into(), offset: 1 << 40, size: 7 }]);
    }

    #[test]
    fn find_ignores_case_and_suggest_lists_prefix_matches() {
        let info = Info { name: "t".into(), word_count: 3, offset_bits: 32, same_type: Some('m') };
        let d = Dictionary::from_parts(info, &idx(&[("Zebra", 0, 1), ("banana", 1, 1), ("Baboon", 2, 1)]), PathBuf::from("/none")).unwrap();
        assert_eq!(d.find("  BANANA ").map(|e| e.offset), Some(1));
        assert!(d.find("pear").is_none());
        assert!(d.find("  ").is_none());
        assert_eq!(d.suggest("ba", 10), vec!["Baboon", "banana"]);
        assert_eq!(d.suggest("", 10), Vec::<&str>::new());
        assert_eq!(d.suggest("b", 1).len(), 1);
        assert_eq!(d.word_count(), 3);
    }

    #[test]
    fn html_definitions_lose_their_tags_and_keep_breaks() {
        assert_eq!(strip_tags("<b>cat</b><br>a small &amp; furry animal"), "cat\na small & furry animal");
        assert_eq!(strip_tags("x and <p>one</p>\n\n\n<p>two</p>"), "x and\none\n\ntwo");
        assert_eq!(plain_text(b"h<i>word</i>", None), "word");
        assert_eq!(plain_text(b"mplain  text ", None), "plain  text");
        assert_eq!(plain_text(b"plain", Some('m')), "plain");
        assert_eq!(plain_text(b"", None), "");
    }

    #[test]
    fn a_library_reads_definitions_from_disk_and_reports_broken_dictionaries() {
        let dir = scratch("library");
        write_dictionary(&dir, "first", "First", &[("cat", "a small animal"), ("dog", "a loud animal")]);
        write_dictionary(&dir, "second", "Second", &[("cat", "a feline")]);
        fs::write(dir.join("broken.ifo"), "nope").unwrap();
        fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let lib = Library::load(&[dir.clone(), dir.join("missing")], Path::new("/no/such/words"));
        assert_eq!(lib.dictionaries.len(), 2);
        assert_eq!(lib.problems.len(), 1);
        assert_eq!(lib.word_count(), 3);
        assert_eq!(
            lib.lookup("cat"),
            vec![Hit { dictionary: "First".into(), text: "a small animal".into() }, Hit { dictionary: "Second".into(), text: "a feline".into() }]
        );
        assert!(lib.lookup("fish").is_empty());
        assert_eq!(lib.suggest("c", 5), vec!["cat".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The system dictionary (tools/build-wordnet-stardict.py) gives an inflected form an index row that points at its
    /// base form's definition block, and multi-word lemmas are headwords with spaces.
    #[test]
    fn the_wordnet_layout_shares_a_block_between_a_form_and_its_base() {
        let dir = scratch("wordnet");
        let goose = "noun\n1. web-footed long-necked birds\n   Synonyms: Anser";
        let ice = "noun\n1. frozen dessert";
        let mut data = goose.as_bytes().to_vec();
        data.extend_from_slice(ice.as_bytes());
        let g = goose.len() as u32;
        let index = idx(&[("geese", 0, g), ("goose", 0, g), ("ice cream", g, ice.len() as u32)]);
        let ifo = format!(
            "StarDict's dict ifo file\nversion=2.4.2\nbookname=WordNet 3.0 (English)\nwordcount=3\nidxfilesize={}\nsametypesequence=m\n",
            index.len()
        );
        fs::write(dir.join("wordnet.ifo"), ifo).unwrap();
        fs::write(dir.join("wordnet.idx"), index).unwrap();
        fs::write(dir.join("wordnet.dict"), data).unwrap();
        let lib = Library::load(&[dir.clone()], Path::new("/no/such/words"));
        assert!(lib.problems.is_empty(), "{:?}", lib.problems);
        let hit = |w: &str| lib.lookup(w).into_iter().map(|h| h.text).collect::<Vec<_>>();
        assert_eq!(hit("Geese"), vec![goose.to_string()]);
        assert_eq!(hit("goose"), hit("geese"));
        assert_eq!(hit("Ice Cream"), vec![ice.to_string()]);
        assert!(lib.lookup("ice").is_empty());
        assert_eq!(lib.suggest("ice", 5), vec!["ice cream".to_string()]);
        assert_eq!(lib.dictionaries[0].info.name, "WordNet 3.0 (English)");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_library_finds_nothing() {
        let lib = Library::load(&[], Path::new("/no/such/words"));
        assert!(lib.is_empty());
        assert!(lib.lookup("cat").is_empty());
        assert!(lib.suggest("c", 5).is_empty());
    }

    #[test]
    fn the_word_list_answers_spelling_only() {
        let dir = scratch("words");
        let list = dir.join("words");
        fs::write(&list, "Cat\ndog\n\ncat\n").unwrap();
        let lib = Library::load(&[], &list);
        assert!(lib.contains_word("CAT"));
        assert!(lib.contains_word("dog"));
        assert!(!lib.contains_word("bird"));
        assert!(!lib.contains_word(""));
        let _ = fs::remove_dir_all(&dir);
    }
}
