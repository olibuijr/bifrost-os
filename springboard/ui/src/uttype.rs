//! Uniform type identifiers (`UniformTypeIdentifiers`): file extension to type, and the conformance tree
//! (`UTType.conforms(to:)`), so Files, drag and drop and pickers agree on what a file is.
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::{OnceLock, RwLock};

/// FNV-1a: the keys are short ASCII identifiers and extensions, so SipHash's DoS protection only costs time.
#[derive(Default)]
struct Fnv(u64);
impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut h = if self.0 == 0 { 0xcbf29ce484222325 } else { self.0 };
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
        self.0 = h;
    }
}
type Index = HashMap<&'static str, &'static Entry, BuildHasherDefault<Fnv>>;

pub(crate) struct Entry {
    pub(crate) id: &'static str,
    pub(crate) parent: &'static [&'static str],
    pub(crate) ext: &'static [&'static str],
    pub(crate) mime: &'static str,
    /// `UTType.localizedDescription`: what Files's Kind column shows.
    pub(crate) desc: &'static str,
}

/// Types an app declared (`uttype_declarations`): looked up after the built-in table, exported ones first.
static DECLARED: RwLock<Vec<&'static Entry>> = RwLock::new(Vec::new());
/// Dynamic types made for tags nobody declared; they only answer `entry`, never extension or MIME lookups.
static DYNAMIC: RwLock<Vec<&'static Entry>> = RwLock::new(Vec::new());

fn read(list: &RwLock<Vec<&'static Entry>>) -> Vec<&'static Entry> {
    list.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Replaces the app-declared set.
pub(crate) fn set_declared(entries: Vec<&'static Entry>) {
    *DECLARED.write().unwrap_or_else(|e| e.into_inner()) = entries;
}

/// Remembers a dynamic type; a second call with the same identifier keeps the first.
pub(crate) fn add_dynamic(e: &'static Entry) {
    let mut v = DYNAMIC.write().unwrap_or_else(|e| e.into_inner());
    if !v.iter().any(|d| d.id == e.id) {
        v.push(e);
    }
}

/// True for a type in the built-in (system-declared) table.
pub(crate) fn is_builtin(id: &str) -> bool {
    TYPES.iter().any(|e| e.id == id)
}

/// True for a dynamic type made by `add_dynamic`.
pub(crate) fn is_dynamic_entry(id: &str) -> bool {
    read(&DYNAMIC).iter().any(|e| e.id == id)
}

/// True when the identifier is built in or declared by the app.
pub(crate) fn is_known(id: &str) -> bool {
    entry(id).is_some()
}

const TYPES: &[Entry] = &[
    Entry { id: "public.data", parent: &[], ext: &[], mime: "application/octet-stream", desc: "Document" },
    Entry { id: "public.item", parent: &[], ext: &[], mime: "", desc: "Item" },
    Entry { id: "public.content", parent: &["public.item"], ext: &[], mime: "", desc: "Document" },
    Entry { id: "public.text", parent: &["public.data", "public.content"], ext: &[], mime: "text/plain", desc: "Text" },
    Entry { id: "public.plain-text", parent: &["public.text"], ext: &["txt", "text", "log", "nfo", "list", "env", "prop", "rc", "ini", "conf", "cfg"], mime: "text/plain", desc: "Plain Text" },
    Entry { id: "public.utf8-plain-text", parent: &["public.plain-text"], ext: &[], mime: "", desc: "Plain Text" },
    Entry { id: "public.url", parent: &["public.data"], ext: &[], mime: "", desc: "URL" },
    Entry { id: "public.file-url", parent: &["public.url"], ext: &[], mime: "", desc: "File URL" },
    Entry { id: "public.source-code", parent: &["public.plain-text"], ext: &[], mime: "text/plain", desc: "Source Code" },
    Entry { id: "public.rust-source", parent: &["public.source-code"], ext: &["rs"], mime: "text/rust", desc: "Rust Source" },
    Entry { id: "public.shell-script", parent: &["public.source-code"], ext: &["sh"], mime: "application/x-sh", desc: "Shell Script" },
    Entry { id: "public.python-script", parent: &["public.source-code"], ext: &["py"], mime: "text/x-python", desc: "Python Script" },
    Entry { id: "public.c-source", parent: &["public.source-code"], ext: &["c"], mime: "text/x-c", desc: "C Source" },
    Entry { id: "public.c-header", parent: &["public.source-code"], ext: &["h"], mime: "text/x-c", desc: "C Header" },
    Entry { id: "public.c-plus-plus-source", parent: &["public.source-code"], ext: &["cpp"], mime: "text/x-c++", desc: "C++ Source" },
    Entry { id: "public.c-plus-plus-header", parent: &["public.source-code"], ext: &["hpp"], mime: "text/x-c++", desc: "C++ Header" },
    Entry { id: "com.sun.java-source", parent: &["public.source-code"], ext: &["java"], mime: "text/x-java", desc: "Java Source" },
    Entry { id: "org.kotlinlang.kotlin-source", parent: &["public.source-code"], ext: &["kt"], mime: "text/x-kotlin", desc: "Kotlin Source" },
    Entry { id: "com.netscape.javascript-source", parent: &["public.source-code"], ext: &["js"], mime: "text/javascript", desc: "JavaScript" },
    Entry { id: "com.microsoft.typescript", parent: &["public.source-code"], ext: &["ts"], mime: "text/typescript", desc: "TypeScript" },
    Entry { id: "public.css", parent: &["public.source-code"], ext: &["css"], mime: "text/css", desc: "CSS Style Sheet" },
    Entry { id: "public.html", parent: &["public.text"], ext: &["html", "htm"], mime: "text/html", desc: "HTML Document" },
    Entry { id: "public.xml", parent: &["public.text"], ext: &["xml"], mime: "text/xml", desc: "XML Document" },
    Entry { id: "net.daringfireball.markdown", parent: &["public.plain-text"], ext: &["md", "markdown"], mime: "text/markdown", desc: "Markdown Document" },
    Entry { id: "public.json", parent: &["public.text"], ext: &["json"], mime: "application/json", desc: "JSON" },
    Entry { id: "public.yaml", parent: &["public.plain-text"], ext: &["yaml", "yml"], mime: "application/yaml", desc: "YAML Document" },
    Entry { id: "org.toml.toml", parent: &["public.plain-text"], ext: &["toml"], mime: "application/toml", desc: "TOML Document" },
    Entry { id: "public.comma-separated-values-text", parent: &["public.text"], ext: &["csv"], mime: "text/csv", desc: "CSV Document" },
    Entry { id: "public.tab-separated-values-text", parent: &["public.text"], ext: &["tsv"], mime: "text/tab-separated-values", desc: "TSV Document" },
    Entry { id: "public.subrip-subtitle", parent: &["public.plain-text"], ext: &["srt", "sub"], mime: "application/x-subrip", desc: "Subtitles" },
    Entry { id: "public.m3u-playlist", parent: &["public.text"], ext: &["m3u"], mime: "audio/x-mpegurl", desc: "Playlist" },
    Entry { id: "public.image", parent: &["public.data"], ext: &[], mime: "image/*", desc: "Image" },
    Entry { id: "public.png", parent: &["public.image"], ext: &["png"], mime: "image/png", desc: "PNG image" },
    Entry { id: "public.jpeg", parent: &["public.image"], ext: &["jpg", "jpeg"], mime: "image/jpeg", desc: "JPEG image" },
    Entry { id: "com.compuserve.gif", parent: &["public.image"], ext: &["gif"], mime: "image/gif", desc: "GIF image" },
    Entry { id: "public.webp", parent: &["public.image"], ext: &["webp"], mime: "image/webp", desc: "WebP image" },
    Entry { id: "public.svg-image", parent: &["public.image", "public.xml"], ext: &["svg"], mime: "image/svg+xml", desc: "SVG image" },
    Entry { id: "public.audiovisual-content", parent: &["public.data", "public.content"], ext: &[], mime: "", desc: "Audiovisual Content" },
    Entry { id: "public.audio", parent: &["public.audiovisual-content"], ext: &[], mime: "audio/*", desc: "Audio" },
    Entry { id: "public.mp3", parent: &["public.audio"], ext: &["mp3"], mime: "audio/mpeg", desc: "MP3 audio" },
    Entry { id: "public.mpeg-4-audio", parent: &["public.audio"], ext: &["m4a"], mime: "audio/mp4", desc: "MPEG-4 audio" },
    Entry { id: "public.aac-audio", parent: &["public.audio"], ext: &["aac"], mime: "audio/aac", desc: "AAC audio" },
    Entry { id: "org.xiph.flac", parent: &["public.audio"], ext: &["flac"], mime: "audio/flac", desc: "FLAC audio" },
    Entry { id: "com.microsoft.waveform-audio", parent: &["public.audio"], ext: &["wav"], mime: "audio/wav", desc: "Waveform audio" },
    Entry { id: "org.xiph.ogg-audio", parent: &["public.audio"], ext: &["ogg"], mime: "audio/ogg", desc: "Ogg audio" },
    Entry { id: "org.xiph.opus-audio", parent: &["public.audio"], ext: &["opus"], mime: "audio/opus", desc: "Opus audio" },
    Entry { id: "public.movie", parent: &["public.audiovisual-content"], ext: &[], mime: "video/*", desc: "Movie" },
    Entry { id: "public.mpeg-4", parent: &["public.movie"], ext: &["mp4", "m4v"], mime: "video/mp4", desc: "MPEG-4 movie" },
    Entry { id: "bifrost.player-movie", parent: &["public.movie"], ext: &["mov"], mime: "video/quicktime", desc: "QuickTime movie" },
    Entry { id: "com.adobe.pdf", parent: &["public.data", "public.content"], ext: &["pdf"], mime: "application/pdf", desc: "PDF document" },
    Entry { id: "public.archive", parent: &["public.data"], ext: &[], mime: "", desc: "Archive" },
    Entry { id: "public.zip-archive", parent: &["public.archive"], ext: &["zip"], mime: "application/zip", desc: "ZIP archive" },
    Entry { id: "public.tar-archive", parent: &["public.archive"], ext: &["tar"], mime: "application/x-tar", desc: "Tar archive" },
    Entry { id: "org.gnu.gnu-zip-archive", parent: &["public.archive"], ext: &["gz", "tgz"], mime: "application/gzip", desc: "GZip archive" },
    Entry { id: "org.tukaani.xz-archive", parent: &["public.archive"], ext: &["xz"], mime: "application/x-xz", desc: "XZ archive" },
    Entry { id: "public.bzip2-archive", parent: &["public.archive"], ext: &["bz2"], mime: "application/x-bzip2", desc: "Bzip2 archive" },
    Entry { id: "org.7-zip.7-zip-archive", parent: &["public.archive"], ext: &["7z"], mime: "application/x-7z-compressed", desc: "7-Zip archive" },
    Entry { id: "com.rarlab.rar-archive", parent: &["public.archive"], ext: &["rar"], mime: "application/vnd.rar", desc: "RAR archive" },
    Entry { id: "com.sun.java-archive", parent: &["public.zip-archive"], ext: &["jar"], mime: "application/java-archive", desc: "Java archive" },
    Entry { id: "com.android.package-archive", parent: &["public.zip-archive"], ext: &["apk"], mime: "application/vnd.android.package-archive", desc: "Android package" },
    Entry { id: "public.font", parent: &["public.data"], ext: &[], mime: "font/*", desc: "Font" },
    Entry { id: "public.truetype-ttf-font", parent: &["public.font"], ext: &["ttf"], mime: "font/ttf", desc: "TrueType font" },
    Entry { id: "public.opentype-font", parent: &["public.font"], ext: &["otf"], mime: "font/otf", desc: "OpenType font" },
    Entry { id: "public.truetype-collection-font", parent: &["public.font"], ext: &["ttc"], mime: "font/collection", desc: "Font collection" },
    Entry { id: "public.rtf", parent: &["public.text"], ext: &["rtf"], mime: "text/rtf", desc: "Rich Text" },
    Entry { id: "org.openxmlformats.wordprocessingml.document", parent: &["public.data", "public.content"], ext: &["docx"], mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document", desc: "Word document" },
    Entry { id: "org.openxmlformats.spreadsheetml.sheet", parent: &["public.data", "public.content"], ext: &["xlsx"], mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", desc: "Spreadsheet" },
    Entry { id: "org.openxmlformats.presentationml.presentation", parent: &["public.data", "public.content"], ext: &["pptx"], mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation", desc: "Presentation" },
    Entry { id: "org.oasis-open.opendocument.text", parent: &["public.data", "public.content"], ext: &["odt"], mime: "application/vnd.oasis.opendocument.text", desc: "OpenDocument text" },
    Entry { id: "bifrost.web-archive", parent: &["public.data", "public.content"], ext: &["webarchive"], mime: "application/x-webarchive", desc: "Web archive" },
    Entry { id: "org.oasis-open.opendocument.spreadsheet", parent: &["public.data", "public.content"], ext: &["ods"], mime: "application/vnd.oasis.opendocument.spreadsheet", desc: "OpenDocument spreadsheet" },
    Entry { id: "org.oasis-open.opendocument.presentation", parent: &["public.data", "public.content"], ext: &["odp"], mime: "application/vnd.oasis.opendocument.presentation", desc: "OpenDocument presentation" },
    Entry { id: "public.vcard", parent: &["public.data", "public.content"], ext: &["vcf", "vcard"], mime: "text/vcard", desc: "vCard" },
    Entry { id: "public.calendar-event", parent: &["public.data", "public.content"], ext: &["ics", "ical"], mime: "text/calendar", desc: "Calendar event" },
    Entry { id: "bifrost.text-clipping", parent: &["public.data", "public.content"], ext: &["textclipping"], mime: "", desc: "Text Clipping" },
    Entry { id: "bifrost.web-location", parent: &["public.data", "public.content"], ext: &["webloc"], mime: "", desc: "Web Location" },
    Entry { id: "org.debian.deb-archive", parent: &["public.archive"], ext: &["deb"], mime: "application/vnd.debian.binary-package", desc: "Debian package" },
    Entry { id: "org.rpm.rpm-archive", parent: &["public.archive"], ext: &["rpm"], mime: "application/x-rpm", desc: "RPM package" },
    Entry { id: "org.zstandard.zstd-archive", parent: &["public.archive"], ext: &["zst"], mime: "application/zstd", desc: "Zstandard archive" },
    Entry { id: "bifrost.3d-model", parent: &["public.data", "public.content"], ext: &[], mime: "model/*", desc: "3D Model" },
    Entry { id: "bifrost.wavefront-obj", parent: &["bifrost.3d-model"], ext: &["obj"], mime: "model/obj", desc: "Wavefront OBJ model" },
    Entry { id: "bifrost.stl-model", parent: &["bifrost.3d-model"], ext: &["stl"], mime: "model/stl", desc: "STL model" },
    Entry { id: "bifrost.ply-model", parent: &["bifrost.3d-model"], ext: &["ply"], mime: "model/ply", desc: "PLY model" },
    Entry { id: "public.folder", parent: &["public.item"], ext: &[], mime: "", desc: "Folder" },
];

/// Extension to entry, built once: a lookup is one hash instead of a scan of every type's extension list.
fn by_extension() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| TYPES.iter().flat_map(|e| e.ext.iter().map(move |x| (*x, e))).collect())
}

fn entry(id: &str) -> Option<&'static Entry> {
    static INDEX: OnceLock<Index> = OnceLock::new();
    if let Some(e) = INDEX.get_or_init(|| TYPES.iter().map(|e| (e.id, e)).collect()).get(id).copied() {
        return Some(e);
    }
    read(&DECLARED).into_iter().chain(read(&DYNAMIC)).find(|e| e.id == id)
}

/// The type for a file name or extension (case-insensitive); `None` for an unknown extension.
pub fn type_for_extension(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map_or(name, |(_, e)| e);
    let lower;
    let ext = if ext.bytes().any(|b| b.is_ascii_uppercase()) { lower = ext.to_ascii_lowercase(); lower.as_str() } else { ext };
    by_extension().get(ext).map(|e| e.id).or_else(|| read(&DECLARED).into_iter().find(|e| e.ext.contains(&ext)).map(|e| e.id))
}

/// `UTType.localizedDescription`: "PNG image", "Rust Source", "PDF document". Empty for an unknown identifier.
pub fn description(id: &str) -> &'static str {
    entry(id).map_or("", |e| e.desc)
}

/// `UTType(mimeType:)`: the most specific type whose MIME type is `mime` (case-insensitive, parameters ignored).
/// Wildcards such as "image/*" name the abstract type.
pub fn type_for_mime(mime: &str) -> Option<&'static str> {
    let essence = mime.split(';').next()?.trim().to_ascii_lowercase();
    if essence.is_empty() {
        return None;
    }
    TYPES.iter().find(|e| e.mime == essence).map(|e| e.id).or_else(|| read(&DECLARED).into_iter().find(|e| e.mime == essence).map(|e| e.id))
}

/// `UTType.tags[.filenameExtension]`: every extension a type claims.
pub fn extensions(id: &str) -> &'static [&'static str] {
    entry(id).map_or(&[], |e| e.ext)
}

/// `UTType.preferredFilenameExtension`.
pub fn preferred_extension(id: &str) -> Option<&'static str> {
    entry(id).and_then(|e| e.ext.first().copied())
}

/// `UTType.conforms(to:)`: true when `id` is `ancestor` or descends from it.
pub fn conforms(id: &str, ancestor: &str) -> bool {
    if id == ancestor {
        return true;
    }
    entry(id).map_or(false, |e| e.parent.iter().any(|p| conforms(p, ancestor)))
}

/// How many parent links separate `id` from `ancestor` (0 when equal), taking the shortest path; `None` when it does not conform.
/// Launch Services prefers a handler registered for the most specific type, which is the smallest distance.
pub fn distance(id: &str, ancestor: &str) -> Option<usize> {
    if id == ancestor {
        return Some(0);
    }
    entry(id)?.parent.iter().filter_map(|p| distance(p, ancestor)).min().map(|d| d + 1)
}

/// The preferred MIME type, falling back through the parents.
pub fn mime(id: &str) -> &'static str {
    match entry(id) {
        Some(e) if !e.mime.is_empty() => e.mime,
        Some(e) => e.parent.iter().map(|p| mime(p)).find(|m| !m.is_empty()).unwrap_or(""),
        None => "",
    }
}

/// `UTTagClass`: the kinds of tag a type can carry. The table records filename extensions and MIME types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagClass {
    FilenameExtension,
    MimeType,
}

/// `UTType.tags[class]`: the tags a type declares itself, not the ones it inherits. A type with no declared tag of
/// that class gives an empty list (use `mime` for the inherited MIME type).
pub fn tags(id: &str, class: TagClass) -> Vec<&'static str> {
    match class {
        TagClass::FilenameExtension => extensions(id).to_vec(),
        TagClass::MimeType => entry(id).map(|e| e.mime).filter(|m| !m.is_empty()).into_iter().collect(),
    }
}

/// The type that declares `tag` for `class`: an extension (case-insensitive, no dot) or a MIME type (parameters
/// ignored). `None` when no type declares it.
pub fn type_for_tag(class: TagClass, tag: &str) -> Option<&'static str> {
    match class {
        TagClass::FilenameExtension if tag.contains('.') || tag.is_empty() => None,
        TagClass::FilenameExtension => type_for_extension(tag),
        TagClass::MimeType => type_for_mime(tag),
    }
}

#[cfg(test)]
mod tag_class_tests {
    use super::*;

    #[test]
    fn a_type_lists_its_own_extensions_and_mime_type() {
        assert_eq!(tags("public.jpeg", TagClass::FilenameExtension), vec!["jpg", "jpeg"]);
        assert_eq!(tags("public.png", TagClass::MimeType), vec!["image/png"]);
        assert_eq!(tags("public.image", TagClass::MimeType), vec!["image/*"]);
        assert!(tags("public.image", TagClass::FilenameExtension).is_empty());
    }

    #[test]
    fn inherited_mime_types_are_not_declared_tags() {
        // The abstract audiovisual type declares no MIME type; `mime` falls back to the parent's.
        assert!(tags("public.audiovisual-content", TagClass::MimeType).is_empty());
        assert_eq!(mime("public.audiovisual-content"), "application/octet-stream");
    }

    #[test]
    fn a_tag_maps_back_to_its_type() {
        assert_eq!(type_for_tag(TagClass::FilenameExtension, "JPG"), Some("public.jpeg"));
        assert_eq!(type_for_tag(TagClass::MimeType, "IMAGE/PNG; charset=x"), Some("public.png"));
        assert_eq!(type_for_tag(TagClass::MimeType, "image/*"), Some("public.image"));
    }

    #[test]
    fn a_dotted_or_unknown_tag_names_no_type() {
        assert_eq!(type_for_tag(TagClass::FilenameExtension, "a.jpg"), None);
        assert_eq!(type_for_tag(TagClass::FilenameExtension, ""), None);
        assert_eq!(type_for_tag(TagClass::FilenameExtension, "zzz"), None);
        assert_eq!(type_for_tag(TagClass::MimeType, "x/unknown"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extensions_map_case_insensitively() {
        assert_eq!(type_for_extension("Photo.JPG"), Some("public.jpeg"));
        assert_eq!(type_for_extension("png"), Some("public.png"));
        assert_eq!(type_for_extension("a.unknown"), None);
    }
    #[test]
    fn conformance_walks_the_tree() {
        assert!(conforms("public.png", "public.image"));
        assert!(conforms("public.png", "public.data"));
        assert!(conforms("public.rust-source", "public.text"));
        assert!(conforms("public.mp3", "public.content"));
        assert!(!conforms("public.png", "public.audio"));
        assert!(!conforms("public.folder", "public.data"));
    }
    #[test]
    fn descriptions_and_preferred_extensions() {
        assert_eq!(description("public.png"), "PNG image");
        assert_eq!(description(type_for_extension("main.rs").unwrap()), "Rust Source");
        assert_eq!(preferred_extension("public.jpeg"), Some("jpg"));
        assert_eq!(description("no.such.type"), "");
    }
    #[test]
    fn mime_types_map_back_to_types_and_extensions() {
        assert_eq!(type_for_mime("image/PNG; charset=x"), Some("public.png"));
        assert_eq!(type_for_mime("video/mp4"), Some("public.mpeg-4"));
        assert_eq!(type_for_mime("image/*"), Some("public.image"));
        assert_eq!(type_for_mime(""), None);
        assert_eq!(type_for_mime("x/unknown"), None);
        assert_eq!(extensions("public.jpeg"), ["jpg", "jpeg"]);
        assert!(extensions("public.image").is_empty());
    }
    #[test]
    fn an_extension_belongs_to_one_type() {
        let mut seen = std::collections::HashSet::new();
        for e in TYPES {
            for x in e.ext {
                assert!(seen.insert(*x), "{x} listed twice");
            }
        }
        assert!(TYPES.iter().all(|e| !e.desc.is_empty()), "every type has a description");
    }
    #[test]
    fn mesh_files_resolve_to_3d_model_types() {
        assert_eq!(type_for_extension("cube.OBJ"), Some("bifrost.wavefront-obj"));
        assert_eq!(type_for_extension("part.stl"), Some("bifrost.stl-model"));
        assert_eq!(type_for_extension("scan.ply"), Some("bifrost.ply-model"));
        assert!(conforms("bifrost.wavefront-obj", "bifrost.3d-model"));
        assert!(conforms("bifrost.ply-model", "public.data"));
        assert_eq!(mime("bifrost.stl-model"), "model/stl");
    }
    #[test]
    fn mime_falls_back() {
        assert_eq!(mime("public.png"), "image/png");
        assert_eq!(mime("public.audiovisual-content"), "application/octet-stream");
    }
    #[test]
    fn every_parent_exists_and_there_are_no_cycles() {
        for e in TYPES {
            for p in e.parent {
                assert!(entry(p).is_some(), "{} -> {p}", e.id);
                assert!(!conforms(p, e.id), "cycle at {}", e.id);
            }
        }
    }
}

#[cfg(test)]
mod pasteboard_type_tests {
    use super::*;

    #[test]
    fn pasteboard_types_conform() {
        assert!(conforms("public.utf8-plain-text", "public.text"));
        assert!(conforms("public.utf8-plain-text", "public.data"));
        assert!(conforms("public.file-url", "public.url"));
        assert!(!conforms("public.url", "public.text"));
        assert_eq!(type_for_mime(""), None);
    }
}
