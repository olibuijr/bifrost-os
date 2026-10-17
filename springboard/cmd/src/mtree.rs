//! `mtree`: writes and checks file hierarchy specifications, in the style of the BSD `mtree(8)` tool as its man page
//! describes it. A spec has one entry per line, `name keyword=value ...`. `/set` sets defaults for the entries that
//! follow, `/unset` removes them, `..` leaves a directory, names and values may use `\\` and `\ooo` (octal) escapes,
//! `#` starts a comment and a trailing `\` continues the line.
//!
//! Supported keywords: `type`, `mode`, `uid`, `gid`, `size`, `link`, `time`, `nlink`, `sha1digest`, `sha256digest` and
//! `sha512digest`. Keywords such as `flags`, `uname` and `gname` are not implemented: `-c` refuses them and a spec that
//! lists them is checked without them (the names are reported on stderr).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use bifrost_core::{sha1, sha256, sha512};

/// Keywords `-c` writes when no `-k` list is given.
pub const DEFAULT_KEYWORDS: &[&str] = &["type", "mode", "uid", "gid", "size", "link", "time"];
/// Every keyword this tool can write or check.
pub const SUPPORTED_KEYWORDS: &[&str] = &[
    "type", "mode", "uid", "gid", "size", "link", "time", "nlink", "sha1digest", "sha256digest", "sha512digest",
];

pub const USAGE: &str = "usage: mtree -c [-p path] [-d] [-x] [-k keywords] [-K keywords]
       mtree [-p path] [-f spec] [-d] [-e] [-x] [-U | -u]

  -c   write a specification of the tree at -p to stdout
  -k   with -c: write these keywords instead of the defaults (comma separated)
  -K   with -c: add these keywords to the written ones (comma separated)
  -f   read the specification from this file (default stdin)
  -p   root of the tree (default .)
  -d   only directories are written or compared
  -e   do not report files in the tree that the specification does not list
  -x   do not descend into other file systems
  -U   change owner, group, mode and time to match the specification where possible
  -u   same as -U

Keywords: type, mode, uid, gid, size, link, time, nlink, sha1digest, sha256digest, sha512digest.
Exit status: 0 when the tree matches, 2 when it differs, 1 on error.";

/// Parsed command line.
pub enum Parsed {
    Help,
    Run(Options),
}

#[derive(Debug)]
pub struct Options {
    pub create: bool,
    pub extra_ok: bool,
    pub dirs_only: bool,
    pub fix: bool,
    pub one_fs: bool,
    pub root: PathBuf,
    pub spec_file: Option<PathBuf>,
    pub keywords: Option<Vec<String>>,
    pub extra_keywords: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            create: false,
            extra_ok: false,
            dirs_only: false,
            fix: false,
            one_fs: false,
            root: PathBuf::from("."),
            spec_file: None,
            keywords: None,
            extra_keywords: Vec::new(),
        }
    }
}

impl Options {
    /// Keywords `-c` writes. `type` is always first because the nesting of the spec depends on it.
    pub fn create_keywords(&self) -> Vec<String> {
        let base: Vec<String> = match &self.keywords {
            Some(list) => list.clone(),
            None => DEFAULT_KEYWORDS.iter().map(|k| k.to_string()).collect(),
        };
        let mut list = vec!["type".to_string()];
        for keyword in base.into_iter().chain(self.extra_keywords.iter().cloned()) {
            if !list.contains(&keyword) {
                list.push(keyword);
            }
        }
        list
    }
}

pub fn parse_args(args: &[String]) -> Result<Parsed, String> {
    let mut o = Options::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "-h" || arg == "--help" {
            return Ok(Parsed::Help);
        }
        let flags: Vec<char> = match arg.strip_prefix('-') {
            Some(f) if !f.is_empty() => f.chars().collect(),
            _ => return Err(format!("unexpected argument {arg}")),
        };
        let mut j = 0;
        while j < flags.len() {
            let c = flags[j];
            match c {
                'c' => o.create = true,
                'e' => o.extra_ok = true,
                'd' => o.dirs_only = true,
                'U' | 'u' => o.fix = true,
                'x' => o.one_fs = true,
                'p' | 'f' | 'K' | 'k' => {
                    let rest: String = flags[j + 1..].iter().collect();
                    let value = if rest.is_empty() {
                        i += 1;
                        args.get(i).cloned().ok_or_else(|| format!("-{c} needs an argument"))?
                    } else {
                        rest
                    };
                    match c {
                        'p' => o.root = PathBuf::from(value),
                        'f' => o.spec_file = Some(PathBuf::from(value)),
                        'K' => o.extra_keywords.extend(split_keywords(&value)?),
                        _ => o.keywords = Some(split_keywords(&value)?),
                    }
                    break;
                }
                other => return Err(format!("unknown option -{other}")),
            }
            j += 1;
        }
        i += 1;
    }
    Ok(Parsed::Run(o))
}

fn split_keywords(list: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for name in list.split(',').filter(|n| !n.is_empty()) {
        if !SUPPORTED_KEYWORDS.contains(&name) {
            return Err(format!("keyword {name} is not supported"));
        }
        out.push(name.to_string());
    }
    Ok(out)
}

/// One spec entry. `key` is the path relative to the root (empty for the root itself) and `keywords` holds the
/// normalized values, with the `/set` defaults already merged in.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: Vec<u8>,
    pub keywords: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Default)]
pub struct Spec {
    pub entries: Vec<Entry>,
    /// Keywords the spec names that this tool does not implement, each listed once.
    pub unsupported: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Difference {
    Missing(Vec<u8>),
    Extra(Vec<u8>),
    Changed {
        key: Vec<u8>,
        keyword: String,
        expected: Vec<u8>,
        received: Option<Vec<u8>>,
    },
}

pub fn format_difference(d: &Difference) -> String {
    match d {
        Difference::Missing(key) => format!("{}: missing", display_key(key)),
        Difference::Extra(key) => format!("{}: extra", display_key(key)),
        Difference::Changed { key, keyword, expected, received } => format!(
            "{}: {keyword} expected={} received={}",
            display_key(key),
            escape(expected),
            received.as_deref().map(escape).unwrap_or_else(|| "-".to_string())
        ),
    }
}

fn display_key(key: &[u8]) -> String {
    if key.is_empty() {
        ".".to_string()
    } else {
        format!("./{}", escape(key))
    }
}

/// Escapes a name or value for a spec: `\\` for backslash, `\ooo` for spaces, control bytes, `#` and non-ASCII.
pub fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b == b'\\' {
            out.push_str("\\\\");
        } else if b <= b' ' || b >= 0x7f || b == b'#' {
            out.push_str(&format!("\\{b:03o}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

/// Reverses `escape`, also accepting any `\ooo` octal byte.
pub fn unescape(text: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] != b'\\' {
            out.push(text[i]);
            i += 1;
            continue;
        }
        if text.get(i + 1) == Some(&b'\\') {
            out.push(b'\\');
            i += 2;
            continue;
        }
        match text.get(i + 1..i + 4) {
            Some(digits) if digits.iter().all(|b| (b'0'..=b'7').contains(b)) => {
                let value = digits.iter().fold(0u32, |acc, b| acc * 8 + (b - b'0') as u32);
                if value > 255 {
                    return Err(format!("octal escape out of range in {}", String::from_utf8_lossy(text)));
                }
                out.push(value as u8);
                i += 4;
            }
            _ => return Err(format!("bad escape in {}", String::from_utf8_lossy(text))),
        }
    }
    Ok(out)
}

/// Checks a keyword value from a spec and returns it in the form `keyword_value` produces for a file, so the two
/// sides compare byte for byte (`mode=0644` and `mode=644` are the same).
fn normalize(keyword: &str, value: &[u8]) -> Result<Vec<u8>, String> {
    if keyword == "link" {
        return Ok(value.to_vec());
    }
    let text = std::str::from_utf8(value).map_err(|_| format!("{keyword} value is not text"))?;
    let bad = || format!("bad {keyword} value {text}");
    Ok(match keyword {
        "type" => match text {
            "file" | "dir" | "link" | "fifo" | "char" | "block" | "socket" => value.to_vec(),
            _ => return Err(bad()),
        },
        "mode" => {
            let mode = u32::from_str_radix(text, 8).map_err(|_| bad())?;
            if mode > 0o7777 {
                return Err(bad());
            }
            format!("{mode:o}").into_bytes()
        }
        "uid" | "gid" | "size" | "nlink" => text.parse::<u64>().map_err(|_| bad())?.to_string().into_bytes(),
        "time" => {
            let (secs, nanos) = parse_time(text).ok_or_else(|| bad())?;
            format!("{secs}.{nanos:09}").into_bytes()
        }
        "sha1digest" | "sha256digest" | "sha512digest" => {
            let len = match keyword {
                "sha1digest" => 40,
                "sha256digest" => 64,
                _ => 128,
            };
            if text.len() != len || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(bad());
            }
            text.to_ascii_lowercase().into_bytes()
        }
        _ => return Err(format!("keyword {keyword} is not supported")),
    })
}

/// Parses `seconds` or `seconds.fraction` (up to nine digits) into whole seconds and nanoseconds.
fn parse_time(text: &str) -> Option<(i64, u32)> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    let secs: i64 = whole.parse().ok()?;
    if frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut nanos: u32 = if frac.is_empty() { 0 } else { frac.parse().ok()? };
    for _ in frac.len()..9 {
        nanos *= 10;
    }
    Some((secs, nanos))
}

/// Splits the spec into logical lines, joining lines that end in `\`. Each comes with its first physical line number.
fn logical_lines(text: &[u8]) -> Vec<(usize, Vec<u8>)> {
    let mut lines = Vec::new();
    let mut pending: Option<(usize, Vec<u8>)> = None;
    for (n, raw) in text.split(|&b| b == b'\n').enumerate() {
        let (start, mut buf) = pending.take().unwrap_or((n + 1, Vec::new()));
        buf.extend_from_slice(raw);
        if buf.last() == Some(&b'\\') {
            buf.pop();
            pending = Some((start, buf));
        } else {
            lines.push((start, buf));
        }
    }
    if let Some(p) = pending {
        lines.push(p);
    }
    lines
}

/// Reads `keyword=value` tokens into `out`. Keywords this tool does not implement are recorded and skipped.
fn read_pairs<'a>(
    tokens: impl Iterator<Item = &'a [u8]>,
    line: usize,
    out: &mut BTreeMap<String, Vec<u8>>,
    unsupported: &mut Vec<String>,
) -> Result<(), String> {
    for token in tokens {
        let eq = token
            .iter()
            .position(|&b| b == b'=')
            .ok_or_else(|| format!("line {line}: {} is not keyword=value", String::from_utf8_lossy(token)))?;
        let keyword = std::str::from_utf8(&token[..eq]).map_err(|_| format!("line {line}: keyword is not text"))?;
        if !SUPPORTED_KEYWORDS.contains(&keyword) {
            if !unsupported.iter().any(|k| k.as_str() == keyword) {
                unsupported.push(keyword.to_string());
            }
            continue;
        }
        let value = unescape(&token[eq + 1..]).map_err(|e| format!("line {line}: {e}"))?;
        let normalized = normalize(keyword, &value).map_err(|e| format!("line {line}: {e}"))?;
        out.insert(keyword.to_string(), normalized);
    }
    Ok(())
}

fn join_frames(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for frame in frames.iter().filter(|f| !f.is_empty()) {
        if !out.is_empty() {
            out.push(b'/');
        }
        out.extend_from_slice(frame);
    }
    out
}

pub fn parse_spec(text: &[u8]) -> Result<Spec, String> {
    let mut spec = Spec::default();
    let mut defaults: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    // One frame per open directory: the name it was entered by ("" for the root ".").
    let mut frames: Vec<Vec<u8>> = Vec::new();
    for (line, content) in logical_lines(text) {
        let tokens: Vec<&[u8]> = content.split(|b| b.is_ascii_whitespace()).filter(|t| !t.is_empty()).collect();
        let Some(&first) = tokens.first() else { continue };
        if first[0] == b'#' {
            continue;
        }
        if first == &b"/set"[..] {
            read_pairs(tokens[1..].iter().copied(), line, &mut defaults, &mut spec.unsupported)?;
            continue;
        }
        if first == &b"/unset"[..] {
            for token in &tokens[1..] {
                if let Ok(keyword) = std::str::from_utf8(token) {
                    defaults.remove(keyword);
                }
            }
            continue;
        }
        let name = unescape(first).map_err(|e| format!("line {line}: {e}"))?;
        if name == b".." {
            if frames.pop().is_none() {
                return Err(format!("line {line}: .. without an open directory"));
            }
            continue;
        }
        let mut keywords = defaults.clone();
        read_pairs(tokens[1..].iter().copied(), line, &mut keywords, &mut spec.unsupported)?;
        let parent = join_frames(&frames);
        let (key, frame) = if name == b"." {
            (parent, Vec::new())
        } else {
            if name.contains(&b'/') {
                return Err(format!("line {line}: name {} contains /", escape(&name)));
            }
            let mut key = parent;
            if !key.is_empty() {
                key.push(b'/');
            }
            key.extend_from_slice(&name);
            (key, name)
        };
        if keywords.get("type") == Some(&b"dir".to_vec()) {
            frames.push(frame);
        }
        spec.entries.push(Entry { key, keywords });
    }
    Ok(spec)
}

/// What the walk saw. Every directory gets an `Up` after its children, whether or not it was descended into, so that
/// the spec written by `create` always closes what it opens.
enum Event {
    Enter { name: Vec<u8>, key: Vec<u8>, path: PathBuf, meta: fs::Metadata },
    Up,
}

fn io_error(path: &Path, e: std::io::Error) -> String {
    format!("{}: {e}", path.display())
}

fn walk(root: &Path, one_fs: bool, dirs_only: bool) -> Result<Vec<Event>, String> {
    let meta = fs::symlink_metadata(root).map_err(|e| io_error(root, e))?;
    let is_dir = meta.is_dir();
    let dev = meta.dev();
    let mut events = vec![Event::Enter { name: b".".to_vec(), key: Vec::new(), path: root.to_path_buf(), meta }];
    if is_dir {
        walk_children(root, &[], dev, one_fs, dirs_only, &mut events)?;
        events.push(Event::Up);
    }
    Ok(events)
}

fn walk_children(
    dir: &Path,
    key: &[u8],
    dev: u64,
    one_fs: bool,
    dirs_only: bool,
    events: &mut Vec<Event>,
) -> Result<(), String> {
    let mut entries: Vec<(Vec<u8>, PathBuf)> = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| io_error(dir, e))? {
        let entry = entry.map_err(|e| io_error(dir, e))?;
        entries.push((entry.file_name().into_vec(), entry.path()));
    }
    entries.sort();
    for (name, path) in entries {
        let meta = fs::symlink_metadata(&path).map_err(|e| io_error(&path, e))?;
        if dirs_only && !meta.is_dir() {
            continue;
        }
        let is_dir = meta.is_dir();
        let descend = is_dir && !(one_fs && meta.dev() != dev);
        let mut child_key = key.to_vec();
        if !child_key.is_empty() {
            child_key.push(b'/');
        }
        child_key.extend_from_slice(&name);
        events.push(Event::Enter { name, key: child_key.clone(), path: path.clone(), meta });
        if is_dir {
            if descend {
                walk_children(&path, &child_key, dev, one_fs, dirs_only, events)?;
            }
            events.push(Event::Up);
        }
    }
    Ok(())
}

fn kind_name(kind: &fs::FileType) -> &'static str {
    if kind.is_dir() {
        "dir"
    } else if kind.is_symlink() {
        "link"
    } else if kind.is_file() {
        "file"
    } else if kind.is_fifo() {
        "fifo"
    } else if kind.is_char_device() {
        "char"
    } else if kind.is_block_device() {
        "block"
    } else if kind.is_socket() {
        "socket"
    } else {
        "unknown"
    }
}

fn digest_hex(keyword: &str, data: &[u8]) -> String {
    match keyword {
        "sha1digest" => sha256::hex(&sha1::digest(data)),
        "sha256digest" => sha256::hex(&sha256::digest(data)),
        _ => sha256::hex(&sha512::digest512(data)),
    }
}

/// The normalized value of one keyword for a file. `meta` must come from `symlink_metadata`. Returns `None` when the
/// keyword does not apply to this kind of file (`link` on a regular file, a digest on a directory).
fn keyword_value(keyword: &str, path: &Path, meta: &fs::Metadata) -> Result<Option<Vec<u8>>, String> {
    let kind = meta.file_type();
    let value = match keyword {
        "type" => kind_name(&kind).as_bytes().to_vec(),
        "mode" => format!("{:o}", meta.mode() & 0o7777).into_bytes(),
        "uid" => meta.uid().to_string().into_bytes(),
        "gid" => meta.gid().to_string().into_bytes(),
        "nlink" => meta.nlink().to_string().into_bytes(),
        "size" => meta.len().to_string().into_bytes(),
        "time" => format!("{}.{:09}", meta.mtime(), meta.mtime_nsec()).into_bytes(),
        "link" => {
            if !kind.is_symlink() {
                return Ok(None);
            }
            fs::read_link(path).map_err(|e| io_error(path, e))?.into_os_string().into_vec()
        }
        "sha1digest" | "sha256digest" | "sha512digest" => {
            if !kind.is_file() {
                return Ok(None);
            }
            let data = fs::read(path).map_err(|e| io_error(path, e))?;
            digest_hex(keyword, &data).into_bytes()
        }
        _ => return Err(format!("keyword {keyword} is not supported")),
    };
    Ok(Some(value))
}

fn current(keyword: &str, path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| io_error(path, e))?;
    keyword_value(keyword, path, &meta)
}

/// Writes a spec for the tree at `opts.root`. Keywords are written after the entry name; the type is set with `/set`
/// whenever it changes, so most entries carry only the other keywords.
pub fn create(opts: &Options) -> Result<Vec<u8>, String> {
    let keywords = opts.create_keywords();
    for keyword in &keywords {
        if !SUPPORTED_KEYWORDS.contains(&keyword.as_str()) {
            return Err(format!("keyword {keyword} is not supported"));
        }
    }
    let events = walk(&opts.root, opts.one_fs, opts.dirs_only)?;
    let mut out = b"# mtree specification, written by mtree -c\n".to_vec();
    let mut set_type: Option<Vec<u8>> = None;
    for event in &events {
        match event {
            Event::Up => out.extend_from_slice(b"..\n"),
            Event::Enter { name, path, meta, .. } => {
                let kind = keyword_value("type", path, meta)?.unwrap_or_default();
                if set_type.as_deref() != Some(kind.as_slice()) {
                    out.extend_from_slice(format!("/set type={}\n", escape(&kind)).as_bytes());
                    set_type = Some(kind);
                }
                let mut line = escape(name).into_bytes();
                for keyword in keywords.iter().filter(|k| k.as_str() != "type") {
                    if let Some(value) = keyword_value(keyword, path, meta)? {
                        line.extend_from_slice(format!(" {keyword}={}", escape(&value)).as_bytes());
                    }
                }
                line.push(b'\n');
                out.extend_from_slice(&line);
            }
        }
    }
    Ok(out)
}

/// Sets one keyword on the file to the expected value where the system allows it. Returns whether it did.
fn fix(keyword: &str, path: &Path, expected: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(expected) else { return false };
    let Ok(meta) = fs::symlink_metadata(path) else { return false };
    let is_link = meta.file_type().is_symlink();
    match keyword {
        "uid" | "gid" => {
            let Ok(id) = text.parse::<u32>() else { return false };
            let (uid, gid) = if keyword == "uid" { (Some(id), None) } else { (None, Some(id)) };
            std::os::unix::fs::lchown(path, uid, gid).is_ok()
        }
        "mode" if !is_link => {
            let Ok(mode) = u32::from_str_radix(text, 8) else { return false };
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).is_ok()
        }
        "time" if !is_link => {
            let Some((secs, nanos)) = parse_time(text) else { return false };
            let Ok(secs) = u64::try_from(secs) else { return false };
            let when = UNIX_EPOCH + Duration::new(secs, nanos);
            fs::File::open(path)
                .and_then(|file| file.set_times(fs::FileTimes::new().set_modified(when)))
                .is_ok()
        }
        _ => false,
    }
}

/// Compares the tree at `opts.root` with `spec`. Only keywords present in a spec entry are compared. With `opts.fix`,
/// owner, mode and time are set to the spec's values where possible and the fixed differences are not reported.
pub fn check(opts: &Options, spec: &Spec) -> Result<Vec<Difference>, String> {
    let mut found: BTreeMap<Vec<u8>, PathBuf> = BTreeMap::new();
    for event in walk(&opts.root, opts.one_fs, opts.dirs_only)? {
        if let Event::Enter { key, path, .. } = event {
            found.insert(key, path);
        }
    }
    let mut diffs = Vec::new();
    let mut listed: BTreeSet<Vec<u8>> = BTreeSet::new();
    for entry in &spec.entries {
        let is_dir = entry.keywords.get("type") == Some(&b"dir".to_vec());
        if opts.dirs_only && !is_dir {
            continue;
        }
        listed.insert(entry.key.clone());
        let Some(path) = found.get(&entry.key) else {
            diffs.push(Difference::Missing(entry.key.clone()));
            continue;
        };
        for (keyword, expected) in &entry.keywords {
            let mut received = current(keyword, path)?;
            if opts.fix && received.as_ref() != Some(expected) && fix(keyword, path, expected) {
                received = current(keyword, path)?;
            }
            if received.as_ref() != Some(expected) {
                diffs.push(Difference::Changed {
                    key: entry.key.clone(),
                    keyword: keyword.clone(),
                    expected: expected.clone(),
                    received,
                });
            }
        }
    }
    if !opts.extra_ok {
        for key in found.keys() {
            if !listed.contains(key) {
                diffs.push(Difference::Extra(key.clone()));
            }
        }
    }
    Ok(diffs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    use std::time::SystemTime;

    /// A temporary directory removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("bifrost-mtree-{name}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_tree(root: &Path) {
        fs::create_dir_all(root.join("sub dir")).unwrap();
        fs::write(root.join("a b"), b"hello\n").unwrap();
        fs::write(root.join("sub dir").join("#file"), b"").unwrap();
        fs::set_permissions(root.join("a b"), fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(root.join("sub dir"), fs::Permissions::from_mode(0o755)).unwrap();
        symlink("a b", root.join("link")).unwrap();
    }

    fn names(spec: &Spec) -> Vec<String> {
        spec.entries.iter().map(|e| String::from_utf8_lossy(&e.key).into_owned()).collect()
    }

    #[test]
    fn escape_and_unescape_round_trip() {
        let raw = b"a b\\c#\td\x7f\xff";
        let text = escape(raw);
        assert_eq!(text, "a\\040b\\\\c\\043\\011d\\177\\377");
        assert_eq!(unescape(text.as_bytes()).unwrap(), raw.to_vec());
        assert!(unescape(b"\\12").is_err());
        assert!(unescape(b"\\777").is_err());
        assert!(unescape(b"\\q").is_err());
    }

    #[test]
    fn mode_values_compare_numerically() {
        assert_eq!(normalize("mode", b"0644").unwrap(), normalize("mode", b"644").unwrap());
        assert_eq!(normalize("mode", b"644").unwrap(), b"644".to_vec());
        assert!(normalize("mode", b"9").is_err());
        assert_eq!(normalize("time", b"1700000000.5").unwrap(), b"1700000000.500000000".to_vec());
    }

    #[test]
    fn parse_spec_handles_set_unset_dotdot_and_continuation() {
        let text = b"# a comment\n\
/set type=file uid=0 gid=0\n\
one mode=600 size=3 \\\n    time=1700000000.5\n\
/unset uid gid\n\
sub type=dir mode=755\n  inner type=file\n..\n\
two\\040x type=file flags=uchg\n";
        let spec = parse_spec(text).unwrap();
        assert_eq!(names(&spec), ["one", "sub", "sub/inner", "two x"]);
        let one = &spec.entries[0].keywords;
        assert_eq!(one["mode"], b"600".to_vec());
        assert_eq!(one["uid"], b"0".to_vec());
        assert_eq!(one["time"], b"1700000000.500000000".to_vec());
        assert!(!spec.entries[1].keywords.contains_key("uid"));
        assert_eq!(spec.unsupported, vec!["flags".to_string()]);
        assert!(parse_spec(b"..\n").is_err());
    }

    #[test]
    fn create_then_check_round_trip() {
        let scratch = Scratch::new("roundtrip");
        sample_tree(&scratch.0);
        let mut opts = Options { root: scratch.0.clone(), ..Options::default() };
        opts.extra_keywords = vec!["nlink".into(), "sha256digest".into(), "sha512digest".into()];
        let text = create(&opts).unwrap();
        let spec = parse_spec(&text).unwrap();
        assert!(spec.unsupported.is_empty());
        assert_eq!(names(&spec), ["", "a b", "link", "sub dir", "sub dir/#file"]);
        assert!(check(&opts, &spec).unwrap().is_empty());
    }

    #[test]
    fn spec_escapes_awkward_names_and_link_targets() {
        let scratch = Scratch::new("escape");
        sample_tree(&scratch.0);
        let opts = Options { root: scratch.0.clone(), ..Options::default() };
        let text = String::from_utf8(create(&opts).unwrap()).unwrap();
        assert!(text.contains("\na\\040b type=file"), "{text}");
        assert!(text.contains("\\043file"), "{text}");
        assert!(text.contains("link=a\\040b"), "{text}");
        assert!(text.contains("/set type=dir\n"), "{text}");
    }

    #[test]
    fn check_reports_missing_extra_and_changed() {
        let scratch = Scratch::new("changes");
        sample_tree(&scratch.0);
        let opts = Options { root: scratch.0.clone(), ..Options::default() };
        let spec = parse_spec(&create(&opts).unwrap()).unwrap();

        fs::remove_file(scratch.0.join("a b")).unwrap();
        fs::write(scratch.0.join("new"), b"x").unwrap();
        fs::set_permissions(scratch.0.join("sub dir"), fs::Permissions::from_mode(0o700)).unwrap();

        let diffs = check(&opts, &spec).unwrap();
        assert!(diffs.contains(&Difference::Missing(b"a b".to_vec())));
        assert!(diffs.contains(&Difference::Extra(b"new".to_vec())));
        assert!(diffs.iter().any(|d| matches!(
            d,
            Difference::Changed { key, keyword, .. } if key.as_slice() == b"sub dir" && keyword.as_str() == "mode"
        )));

        let lenient = Options { extra_ok: true, ..Options::default() };
        let diffs = check(&Options { root: opts.root.clone(), ..lenient }, &spec).unwrap();
        assert!(!diffs.iter().any(|d| matches!(d, Difference::Extra(_))));
        assert!(diffs.contains(&Difference::Missing(b"a b".to_vec())));
    }

    #[test]
    fn fix_restores_mode() {
        let scratch = Scratch::new("fix");
        sample_tree(&scratch.0);
        let opts = Options { root: scratch.0.clone(), ..Options::default() };
        let spec = parse_spec(&create(&opts).unwrap()).unwrap();
        fs::set_permissions(scratch.0.join("a b"), fs::Permissions::from_mode(0o600)).unwrap();
        let fixing = Options { fix: true, ..opts };
        assert!(check(&fixing, &spec).unwrap().is_empty());
        assert_eq!(fs::metadata(scratch.0.join("a b")).unwrap().mode() & 0o7777, 0o644);
    }

    #[test]
    fn dirs_only_writes_and_checks_directories() {
        let scratch = Scratch::new("dirs");
        sample_tree(&scratch.0);
        let opts = Options { root: scratch.0.clone(), dirs_only: true, ..Options::default() };
        let spec = parse_spec(&create(&opts).unwrap()).unwrap();
        assert_eq!(names(&spec), ["", "sub dir"]);
        assert!(check(&opts, &spec).unwrap().is_empty());
    }

    #[test]
    fn parse_args_reads_flags_and_refuses_unsupported_keywords() {
        let args: Vec<String> = ["-cx", "-p", "/x", "-K", "nlink"].iter().map(|s| s.to_string()).collect();
        match parse_args(&args).unwrap() {
            Parsed::Run(o) => {
                assert!(o.create && o.one_fs);
                assert_eq!(o.root, PathBuf::from("/x"));
                assert!(o.create_keywords().contains(&"nlink".to_string()));
                assert_eq!(o.create_keywords()[0], "type");
            }
            Parsed::Help => panic!("expected a run"),
        }
        let bad: Vec<String> = ["-K", "flags"].iter().map(|s| s.to_string()).collect();
        assert!(parse_args(&bad).is_err());
    }
}
