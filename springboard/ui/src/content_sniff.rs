//! Content sniffing: names the type of a byte stream from its leading bytes, for files whose extension is missing or
//! wrong. The answers are identifiers from `uttype`, so a sniffed type feeds the same description and conformance
//! lookups as a file extension does.

/// How many leading bytes the sniffer reads. Every signature below lies inside this window.
pub const SNIFF_LEN: usize = 512;

/// The type identifier for the leading bytes of a stream, or `None` when no signature matches.
///
/// Container formats that share one signature are named by their container: a ZIP archive may also be an office
/// document or a Java archive, so the caller refines that answer from the file name when it has one.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(SNIFF_LEN)];
    if head.is_empty() {
        return None;
    }
    binary_type(head).or_else(|| text_type(head))
}

/// True when `bytes` holds `needle` starting at `offset`.
fn at(bytes: &[u8], offset: usize, needle: &[u8]) -> bool {
    bytes.get(offset..offset + needle.len()) == Some(needle)
}

fn binary_type(b: &[u8]) -> Option<&'static str> {
    const SIGNATURES: &[(&[u8], &str)] = &[
        (b"\x89PNG\r\n\x1a\n", "public.png"),
        (b"\xff\xd8\xff", "public.jpeg"),
        (b"GIF87a", "com.compuserve.gif"),
        (b"GIF89a", "com.compuserve.gif"),
        (b"%PDF-", "com.adobe.pdf"),
        (b"PK\x03\x04", "public.zip-archive"),
        (b"\x1f\x8b", "org.gnu.gnu-zip-archive"),
        (b"\xfd7zXZ\x00", "org.tukaani.xz-archive"),
        (b"7z\xbc\xaf\x27\x1c", "org.7-zip.7-zip-archive"),
        (b"Rar!\x1a\x07", "com.rarlab.rar-archive"),
        (b"fLaC", "org.xiph.flac"),
        (b"OggS", "org.xiph.ogg-audio"),
        (b"ID3", "public.mp3"),
        (b"\x00\x01\x00\x00", "public.truetype-ttf-font"),
        (b"OTTO", "public.opentype-font"),
        (b"ttcf", "public.truetype-collection-font"),
    ];
    if let Some((_, id)) = SIGNATURES.iter().find(|(signature, _)| b.starts_with(signature)) {
        return Some(*id);
    }
    // A bzip2 stream is "BZh" followed by the block size digit, 1 to 9.
    if b.starts_with(b"BZh") && b.get(3).is_some_and(|d| (b'1'..=b'9').contains(d)) {
        return Some("public.bzip2-archive");
    }
    if at(b, 257, b"ustar") {
        return Some("public.tar-archive");
    }
    if at(b, 4, b"ftyp") {
        return Some(match b.get(8..12) {
            Some(b"M4A ") => "public.mpeg-4-audio",
            Some(b"qt  ") => "bifrost.player-movie",
            _ => "public.mpeg-4",
        });
    }
    if at(b, 0, b"RIFF") {
        return match b.get(8..12) {
            Some(b"WEBP") => Some("public.webp"),
            Some(b"WAVE") => Some("com.microsoft.waveform-audio"),
            _ => None,
        };
    }
    None
}

/// Markup and script types, then plain text: valid UTF-8 with no NUL byte.
fn text_type(b: &[u8]) -> Option<&'static str> {
    let start = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    let body = &b[start..];
    let lower: Vec<u8> = body[..body.len().min(64)].to_ascii_lowercase();
    if lower.starts_with(b"<?xml") {
        let xml_head = b[..b.len().min(1024)].to_ascii_lowercase();
        return Some(if contains(&xml_head, b"<svg") { "public.svg-image" } else { "public.xml" });
    }
    if lower.starts_with(b"<svg") {
        return Some("public.svg-image");
    }
    if lower.starts_with(b"<!doctype html") || lower.starts_with(b"<html") {
        return Some("public.html");
    }
    if body.starts_with(b"#!") {
        let line = body.split(|c| *c == b'\n').next().unwrap_or(body);
        let mut words = line[2..].split(|c| c.is_ascii_whitespace()).filter(|word| !word.is_empty());
        let first = words.next().unwrap_or_default();
        let basename = first.rsplit(|c| *c == b'/').next().unwrap_or(first);
        let interpreter = if basename == b"env" {
            words.find(|word| !word.starts_with(b"-" )).unwrap_or_default()
        } else {
            basename
        };
        let interpreter = interpreter.rsplit(|c| *c == b'/').next().unwrap_or(interpreter);
        if interpreter.starts_with(b"python") {
            return Some("public.python-script");
        }
        if interpreter.ends_with(b"sh") {
            return Some("public.shell-script");
        }
        return None;
    }
    let text = match std::str::from_utf8(b) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    (text && !b.contains(&0)).then_some("public.plain-text")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
