//! `textutil`: converts a text document between plain text, HTML, RTF and the open word processing formats
//! (`textutil -convert FORMAT FILE...`).
//!
//! Supported: `-convert {txt | html | rtf | docx | odt}`, `-output PATH` (one input file), `-stdout` (writes the
//! converted document to standard output, one input file). The input format comes from the file extension (`.txt`,
//! `.html`, `.htm`, `.rtf`, `.docx`, `.odt`) or, for other names, from the content (`{\rtf` starts an RTF file, a `<`
//! at the start an HTML file). A `.docx` or `.odt` file is a ZIP package (`bifrost files::zip`): its text is read from
//! `word/document.xml` or `content.xml`, and the packages written are the smallest valid ones (one body, one
//! paragraph per line, tabs kept). The old binary `.doc`, `rtfd` and `webarchive` formats are refused.
//!
//! The conversion goes through plain text: every format keeps its text and paragraph breaks, but not fonts, colours,
//! images or tables. Gap: no `-info`, `-stdin`, `-encoding` or `-extension` options, and the output is always
//! UTF-8 (HTML and RTF output use `\uN` escapes for characters outside ASCII).

use std::io::Write;
use std::path::Path;

pub const USAGE: &str = "usage: textutil -convert {txt | html | rtf | docx | odt} [-output PATH | -stdout] FILE...";

/// The formats the tool reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Txt,
    Html,
    Rtf,
    Docx,
    Odt,
}

impl Format {
    /// Reads a `-convert` argument. The word processing formats are named in the error.
    pub fn parse(s: &str) -> Result<Format, String> {
        match s.to_ascii_lowercase().as_str() {
            "txt" | "text" => Ok(Format::Txt),
            "html" => Ok(Format::Html),
            "rtf" => Ok(Format::Rtf),
            "docx" => Ok(Format::Docx),
            "odt" => Ok(Format::Odt),
            "doc" | "rtfd" | "webarchive" | "wordml" => {
                Err(format!("format '{s}' is not supported; use txt, html, rtf, docx or odt"))
            }
            _ => Err(format!("unknown format '{s}'; use txt, html, rtf, docx or odt")),
        }
    }

    /// The file extension the output gets when no `-output` is given.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Txt => "txt",
            Format::Html => "html",
            Format::Rtf => "rtf",
            Format::Docx => "docx",
            Format::Odt => "odt",
        }
    }
}

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub format: Format,
    pub output: Option<String>,
    pub stdout: bool,
    pub files: Vec<String>,
}

/// What the command line asks for: the help text, or a run.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// Reads the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut format: Option<Format> = None;
    let mut output = None;
    let mut stdout = false;
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "-help" | "--help" => return Ok(Parsed::Help),
            "-convert" => {
                let value = it.next().ok_or_else(|| format!("-convert needs a format\n{USAGE}"))?;
                format = Some(Format::parse(value)?);
            }
            "-output" => {
                let value = it.next().ok_or_else(|| format!("-output needs a path\n{USAGE}"))?;
                output = Some(value.clone());
            }
            "-stdout" => stdout = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unsupported option '{other}'\n{USAGE}"));
            }
            _ => files.push(arg.clone()),
        }
    }
    let format = format.ok_or_else(|| format!("-convert is required\n{USAGE}"))?;
    if files.is_empty() {
        return Err(format!("no input file\n{USAGE}"));
    }
    if (output.is_some() || stdout) && files.len() != 1 {
        return Err("-output and -stdout take one input file".to_string());
    }
    if output.is_some() && stdout {
        return Err("-output and -stdout cannot be combined".to_string());
    }
    Ok(Parsed::Run(Options { format, output, stdout, files }))
}

/// The format of an input file, from its extension, else from its first bytes.
pub fn detect(path: &str, data: &str) -> Result<Format, String> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "text" => return Ok(Format::Txt),
        "html" | "htm" => return Ok(Format::Html),
        "rtf" => return Ok(Format::Rtf),
        "docx" => return Ok(Format::Docx),
        "odt" => return Ok(Format::Odt),
        "doc" | "rtfd" | "webarchive" | "wordml" => {
            return Err(format!("{path}: word processing formats are not supported"));
        }
        _ => {}
    }
    let head = data.trim_start();
    if head.starts_with("{\\rtf") {
        Ok(Format::Rtf)
    } else if head.starts_with('<') {
        Ok(Format::Html)
    } else {
        Ok(Format::Txt)
    }
}

/// Converts a document held as text. Text input is copied as it is; HTML and RTF go through their plain text. The ZIP
/// package formats are bytes, not text, so they go through [`to_plain`] and [`from_plain`]; here they pass through as
/// plain text.
pub fn convert(data: &str, from: Format, to: Format) -> String {
    if from == Format::Txt && to == Format::Txt {
        return data.to_string();
    }
    let text = match from {
        Format::Txt | Format::Docx | Format::Odt => data.to_string(),
        Format::Html => html_to_text(data),
        Format::Rtf => rtf_to_text(data),
    };
    match to {
        Format::Txt | Format::Docx | Format::Odt => text,
        Format::Html => text_to_html(&text),
        Format::Rtf => text_to_rtf(&text),
    }
}

/// The largest part of a package that is unpacked, so a crafted file cannot fill memory.
const MAX_PART: u64 = 64 << 20;

/// The plain text of a document's bytes in format `from`.
pub fn to_plain(bytes: &[u8], from: Format) -> Result<String, String> {
    match from {
        Format::Docx => Ok(docx_xml_to_text(&zip_part(bytes, "word/document.xml")?)),
        Format::Odt => Ok(odt_xml_to_text(&zip_part(bytes, "content.xml")?)),
        _ => {
            let decoded = String::from_utf8_lossy(bytes);
            let text = decoded.strip_prefix('\u{feff}').unwrap_or(&decoded);
            Ok(convert(text, from, Format::Txt))
        }
    }
}

/// The bytes of plain text written as format `to`.
pub fn from_plain(text: &str, to: Format) -> Result<Vec<u8>, String> {
    match to {
        Format::Docx => text_to_docx(text),
        Format::Odt => text_to_odt(text),
        _ => Ok(convert(text, Format::Txt, to).into_bytes()),
    }
}

/// One part of a ZIP package as text.
fn zip_part(bytes: &[u8], name: &str) -> Result<String, String> {
    let mut archive = springboard::files::zip::Archive::open(std::io::Cursor::new(bytes)).map_err(|e| format!("not a valid package: {e}"))?;
    let member = archive
        .members
        .iter()
        .find(|m| m.name == name && !m.is_dir)
        .cloned()
        .ok_or_else(|| format!("the package has no {name}"))?;
    if member.encrypted {
        return Err("the package is password protected".to_string());
    }
    if member.size > MAX_PART {
        return Err(format!("{name} is too large"));
    }
    let data = archive.read(&member).map_err(|e| format!("{name}: {e}"))?;
    Ok(String::from_utf8_lossy(&data).into_owned())
}

/// The next tag of `rest` (without its angle brackets) and the text before it; `None` at the end.
fn next_tag(rest: &str) -> Option<(&str, &str, &str)> {
    let lt = rest.find('<')?;
    let after = &rest[lt + 1..];
    let gt = after.find('>')?;
    Some((&rest[..lt], &after[..gt], &after[gt + 1..]))
}

/// The element name in a tag's inner text, and whether it closes (`/w:p`) or is empty (`w:tab/`).
fn xml_name(tag: &str) -> (&str, bool, bool) {
    let closing = tag.starts_with('/');
    let empty = tag.ends_with('/');
    let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
    (name, closing, empty)
}

/// The text of a `word/document.xml`: the runs of `w:t`, a line break at the end of each paragraph, tabs and breaks.
pub fn docx_xml_to_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some((_, tag, after)) = next_tag(rest) {
        rest = after;
        match xml_name(tag) {
            ("w:t", false, false) => {
                let end = rest.find("</w:t>").unwrap_or(rest.len());
                out.push_str(&decode_entities(&rest[..end]));
                rest = &rest[end..];
            }
            ("w:p", true, _) | ("w:p", false, true) => out.push('\n'),
            ("w:tab", false, _) => out.push('\t'),
            ("w:br" | "w:cr", false, _) => out.push('\n'),
            _ => {}
        }
    }
    out
}

/// The text of an OpenDocument `content.xml`: the characters inside paragraphs and headings, a line break at the end
/// of each, and the space, tab and line-break elements.
pub fn odt_xml_to_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    let mut depth = 0usize;
    while let Some((text, tag, after)) = next_tag(rest) {
        rest = after;
        if depth > 0 {
            out.push_str(&decode_entities(text));
        }
        match xml_name(tag) {
            ("text:p" | "text:h", false, false) => depth += 1,
            ("text:p" | "text:h", false, true) => out.push('\n'),
            ("text:p" | "text:h", true, _) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    out.push('\n');
                }
            }
            ("text:tab", false, _) if depth > 0 => out.push('\t'),
            ("text:line-break", false, _) if depth > 0 => out.push('\n'),
            ("text:s", false, _) if depth > 0 => {
                let count = tag.split("text:c=\"").nth(1).and_then(|r| r.split('"').next()).and_then(|n| n.parse::<usize>().ok()).unwrap_or(1);
                out.push_str(&" ".repeat(count.min(1000)));
            }
            _ => {}
        }
    }
    out
}

const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// The smallest valid `.docx` holding `text`: one paragraph per line, tabs kept.
pub fn text_to_docx(text: &str) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    for line in text.lines() {
        if line.is_empty() {
            body.push_str("<w:p/>");
            continue;
        }
        body.push_str("<w:p>");
        for (i, part) in line.split('\t').enumerate() {
            if i > 0 {
                body.push_str("<w:r><w:tab/></w:r>");
            }
            if !part.is_empty() {
                body.push_str(&format!("<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", escape_html(part)));
            }
        }
        body.push_str("</w:p>");
    }
    let types = format!("{XML_HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>");
    let rels = format!("{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>");
    let document = format!("{XML_HEAD}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>");
    let mut zip = springboard::files::zip::Writer::new(Vec::new());
    for (name, data) in [("[Content_Types].xml", types), ("_rels/.rels", rels), ("word/document.xml", document)] {
        zip.add_bytes(name, data.as_bytes(), 0).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())
}

/// The smallest valid `.odt` holding `text`: the stored `mimetype` first, then the manifest and the content.
pub fn text_to_odt(text: &str) -> Result<Vec<u8>, String> {
    const MIME: &str = "application/vnd.oasis.opendocument.text";
    let mut body = String::new();
    for line in text.lines() {
        body.push_str("<text:p>");
        let mut prev_space = true;
        for c in line.chars() {
            match c {
                '\t' => {
                    body.push_str("<text:tab/>");
                    prev_space = false;
                }
                ' ' if prev_space => body.push_str("<text:s/>"),
                ' ' => {
                    body.push(' ');
                    prev_space = true;
                }
                c => {
                    body.push_str(&escape_html(&c.to_string()));
                    prev_space = false;
                }
            }
        }
        body.push_str("</text:p>");
    }
    let manifest = format!("{XML_HEAD}<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"{MIME}\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>");
    let content = format!("{XML_HEAD}<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" office:version=\"1.2\"><office:body><office:text>{body}</office:text></office:body></office:document-content>");
    let mut zip = springboard::files::zip::Writer::new(Vec::new());
    // the type marker must be stored, not deflated, so file-type sniffers can read it at a fixed offset
    zip.add_stored_stream("mimetype", std::io::Cursor::new(MIME.as_bytes()), MIME.len() as u64, 0, &mut |_| true).map_err(|e| e.to_string())?;
    zip.add_bytes("META-INF/manifest.xml", manifest.as_bytes(), 0).map_err(|e| e.to_string())?;
    zip.add_bytes("content.xml", content.as_bytes(), 0).map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())
}

/// Converts each input file and writes the result next to it (or to `-output`, or to standard output).
pub fn run(opts: &Options) -> Result<(), String> {
    for file in &opts.files {
        let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        let decoded = String::from_utf8_lossy(&bytes);
        let sniffed: &str = decoded.strip_prefix('\u{feff}').unwrap_or(&decoded);
        let from = detect(file, sniffed)?;
        let plain = to_plain(&bytes, from).map_err(|e| format!("{file}: {e}"))?;
        let converted = from_plain(&plain, opts.format)?;
        if opts.stdout {
            let mut out = std::io::stdout().lock();
            out.write_all(&converted).and_then(|()| out.flush()).map_err(|e| e.to_string())?;
            continue;
        }
        let target = match &opts.output {
            Some(path) => path.clone(),
            None => Path::new(file).with_extension(opts.format.ext()).to_string_lossy().into_owned(),
        };
        std::fs::write(&target, converted).map_err(|e| format!("{target}: {e}"))?;
    }
    Ok(())
}

/// Elements that start or end a line in HTML.
const BLOCK_TAGS: &[&str] = &[
    "p", "div", "br", "li", "ul", "ol", "tr", "table", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "pre",
    "section", "article", "header", "footer", "hr",
];

/// The plain text of an HTML document: tags are dropped, script, style and comments are skipped, entities are
/// decoded, whitespace runs collapse to one space and block elements become line breaks.
pub fn html_to_text(src: &str) -> String {
    let mut out = String::new();
    let mut skipping: Option<String> = None;
    let mut pos = 0;
    while pos < src.len() {
        let rest = &src[pos..];
        if let Some(open) = skipping.clone() {
            // Inside script or style: nothing shows until the matching closing tag.
            let lower = rest.to_ascii_lowercase();
            match lower.find(format!("</{open}").as_str()) {
                Some(at) => {
                    pos += at;
                    skipping = None;
                }
                None => break,
            }
            continue;
        }
        let Some(lt) = rest.find('<') else {
            push_text(&mut out, rest);
            break;
        };
        push_text(&mut out, &rest[..lt]);
        let after = &rest[lt..];
        if after.starts_with("<!--") {
            let end = after.find("-->").map_or(after.len(), |e| e + 3);
            pos += lt + end;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        pos += lt + gt + 1;
        let (closing, name) = tag_name(&after[1..gt]);
        if !closing && (name == "script" || name == "style" || name == "head") {
            skipping = Some(name);
            continue;
        }
        if BLOCK_TAGS.contains(&name.as_str()) {
            out.push('\n');
        }
    }
    tidy_lines(&out)
}

/// The lower-case name of an HTML tag and whether it is a closing tag (`/p`, `</p>`).
fn tag_name(inner: &str) -> (bool, String) {
    let inner = inner.trim_start();
    let (closing, body) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, inner),
    };
    let name: String = body.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    (closing, name.to_ascii_lowercase())
}

/// Adds the decoded text of an HTML run to `out`, collapsing whitespace to one space.
fn push_text(out: &mut String, raw: &str) {
    for c in decode_entities(raw).chars() {
        if c.is_ascii_whitespace() {
            match out.chars().last() {
                None | Some(' ') | Some('\n') => {}
                Some(_) => out.push(' '),
            }
        } else {
            out.push(c);
        }
    }
}

/// Replaces the named and numeric character references in `s`. An unknown reference stays as it is.
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let decoded = tail
            .find(';')
            .filter(|&end| end <= 10)
            .and_then(|end| entity(&tail[1..end]).map(|c| (c, end)));
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Trims the spaces at both ends of each line, drops repeated blank lines and the blank lines at the start and the
/// end, and ends a non-empty text with one newline.
fn tidy_lines(s: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in s.split('\n') {
        let t = line.trim_matches(' ');
        if t.is_empty() && lines.last().map_or(true, |l| l.is_empty()) {
            continue;
        }
        lines.push(t);
    }
    let start = lines.iter().position(|l| !l.is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| !l.is_empty()).map_or(start, |e| e + 1);
    if start >= end {
        return String::new();
    }
    let mut out = lines[start..end].join("\n");
    out.push('\n');
    out
}

/// The text as a minimal HTML page: each line is a paragraph, an empty line is an empty paragraph.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::from("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n");
    for line in text.lines() {
        if line.is_empty() {
            out.push_str("<p><br></p>\n");
        } else {
            out.push_str("<p>");
            out.push_str(&escape_html(line));
            out.push_str("</p>\n");
        }
    }
    out.push_str("</body>\n</html>\n");
    out
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The text as an RTF document: braces and backslashes are escaped, line breaks become `\par`, and characters outside
/// ASCII become `\uN?` (N is the UTF-16 code unit, signed, as RTF expects).
pub fn text_to_rtf(text: &str) -> String {
    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0 Sans;}}\\f0\\fs24 ");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\par\n"),
            '\r' => {}
            '\t' => out.push_str("\\tab "),
            c if c.is_ascii() => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    out.push('}');
    out
}

/// The plain text of an RTF document. Destination groups (font and colour tables, stylesheets, pictures, `{\*...}`)
/// are skipped, `\par` and `\line` become line breaks, `\'hh` and `\uN` become characters (the fallback character
/// after `\uN` is skipped), and other control words are dropped.
pub fn rtf_to_text(src: &str) -> String {
    const SKIPPED_GROUPS: &[&str] = &["fonttbl", "colortbl", "stylesheet", "info", "pict", "header", "footer"];
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::new();
    let mut depth = 0usize;
    let mut skip_depth: Option<usize> = None;
    let mut uc_skip = 1usize;
    let mut pending_skip = 0usize;
    let mut high_surrogate: Option<u32> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '{' => {
                depth += 1;
                i += 1;
                if skip_depth.is_none() && group_is_skipped(&chars, i, SKIPPED_GROUPS) {
                    skip_depth = Some(depth);
                }
            }
            '}' => {
                if skip_depth == Some(depth) {
                    skip_depth = None;
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            '\\' => {
                let Some(&next) = chars.get(i + 1) else { break };
                let visible = skip_depth.is_none();
                if next.is_ascii_alphabetic() {
                    let mut j = i + 1;
                    let mut word = String::new();
                    while j < chars.len() && chars[j].is_ascii_alphabetic() {
                        word.push(chars[j]);
                        j += 1;
                    }
                    let mut param: Option<i64> = None;
                    let mut k = j;
                    if k < chars.len() && chars[k] == '-' {
                        k += 1;
                    }
                    let digits_start = k;
                    while k < chars.len() && chars[k].is_ascii_digit() {
                        k += 1;
                    }
                    if k > digits_start {
                        let text: String = chars[j..k].iter().collect();
                        param = text.parse::<i64>().ok();
                        j = k;
                    }
                    if j < chars.len() && chars[j] == ' ' {
                        j += 1;
                    }
                    i = j;
                    match word.as_str() {
                        "uc" => uc_skip = param.unwrap_or(1).max(0) as usize,
                        "u" => {
                            if let (Some(n), true) = (param, visible) {
                                let code = if n < 0 { (n + 65536) as u32 } else { n as u32 };
                                push_code(&mut out, code, &mut high_surrogate);
                            }
                            pending_skip = uc_skip;
                        }
                        "par" | "line" if visible => out.push('\n'),
                        "tab" if visible => out.push('\t'),
                        "emdash" if visible => out.push('\u{2014}'),
                        "endash" if visible => out.push('\u{2013}'),
                        "lquote" if visible => out.push('\u{2018}'),
                        "rquote" if visible => out.push('\u{2019}'),
                        "ldblquote" if visible => out.push('\u{201c}'),
                        "rdblquote" if visible => out.push('\u{201d}'),
                        "bullet" if visible => out.push('\u{2022}'),
                        _ => {}
                    }
                } else if next == '\'' {
                    let hex: String = chars.iter().skip(i + 2).take(2).collect();
                    i += 4;
                    if let (true, Ok(byte)) = (visible, u8::from_str_radix(&hex, 16)) {
                        if pending_skip == 0 {
                            out.push(char::from(byte));
                        }
                    }
                    if pending_skip > 0 {
                        pending_skip -= 1;
                    }
                } else {
                    i += 2;
                    if visible && pending_skip == 0 {
                        match next {
                            '\\' | '{' | '}' => out.push(next),
                            '~' => out.push('\u{a0}'),
                            '\n' | '\r' => out.push('\n'),
                            _ => {}
                        }
                    }
                }
            }
            '\r' | '\n' => i += 1,
            _ => {
                if skip_depth.is_none() {
                    if pending_skip > 0 {
                        pending_skip -= 1;
                    } else {
                        out.push(c);
                    }
                }
                i += 1;
            }
        }
    }
    out
}

/// Whether the group that starts at `chars[start]` (just after its `{`) is a destination the text does not show.
fn group_is_skipped(chars: &[char], start: usize, names: &[&str]) -> bool {
    if chars.get(start) != Some(&'\\') {
        return false;
    }
    if chars.get(start + 1) == Some(&'*') {
        return true;
    }
    let word: String = chars[start + 1..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
    names.contains(&word.as_str())
}

/// Appends the character for a `\u` code, pairing UTF-16 surrogates across two codes.
fn push_code(out: &mut String, code: u32, high_surrogate: &mut Option<u32>) {
    if (0xD800..0xDC00).contains(&code) {
        *high_surrogate = Some(code);
        return;
    }
    let combined = match (*high_surrogate, (0xDC00..0xE000).contains(&code)) {
        (Some(high), true) => Some(0x10000 + ((high - 0xD800) << 10) + (code - 0xDC00)),
        _ => None,
    };
    *high_surrogate = None;
    if let Some(c) = char::from_u32(combined.unwrap_or(code)) {
        out.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_reads_convert_output_and_files() {
        let Parsed::Run(o) = parse(&args(&["-convert", "html", "-output", "a.html", "a.txt"])).unwrap() else {
            panic!("expected a run")
        };
        assert_eq!(o.format, Format::Html);
        assert_eq!(o.output.as_deref(), Some("a.html"));
        assert!(!o.stdout);
        assert_eq!(o.files, vec!["a.txt".to_string()]);
    }

    #[test]
    fn parse_refuses_bad_command_lines() {
        assert!(parse(&args(&["a.txt"])).is_err(), "-convert is required");
        assert!(parse(&args(&["-convert", "txt"])).is_err(), "a file is required");
        assert!(parse(&args(&["-convert", "doc", "a.txt"])).is_err());
        assert!(parse(&args(&["-convert", "rtfd", "a.txt"])).is_err());
        assert!(parse(&args(&["-convert", "txt", "-info", "a.txt"])).is_err());
        assert!(parse(&args(&["-convert", "txt", "-output", "o.txt", "a", "b"])).is_err());
        assert!(parse(&args(&["-convert", "txt", "-stdout", "-output", "o.txt", "a"])).is_err());
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse(&args(&["-h"])).unwrap(), Parsed::Help);
    }

    #[test]
    fn detect_uses_extension_then_content() {
        assert_eq!(detect("x.rtf", ""), Ok(Format::Rtf));
        assert_eq!(detect("x.HTM", ""), Ok(Format::Html));
        assert_eq!(detect("x.bin", "{\\rtf1 hi}"), Ok(Format::Rtf));
        assert_eq!(detect("x.bin", "  <p>hi</p>"), Ok(Format::Html));
        assert_eq!(detect("x.bin", "plain"), Ok(Format::Txt));
        assert!(detect("x.doc", "").is_err());
        assert_eq!(detect("x.docx", ""), Ok(Format::Docx));
        assert_eq!(detect("x.ODT", ""), Ok(Format::Odt));
    }

    #[test]
    fn html_keeps_paragraphs_and_drops_tags() {
        assert_eq!(html_to_text("<p>Hello <b>world</b></p><p>Second</p>"), "Hello world\n\nSecond\n");
        assert_eq!(html_to_text("a<br>b"), "a\nb\n");
        assert_eq!(html_to_text("  one\n   two  "), "one two\n");
    }

    #[test]
    fn html_skips_script_style_and_comments() {
        assert_eq!(html_to_text("<style>p{}</style><!-- hidden --><script>x<y</script><p>A</p>"), "A\n");
    }

    #[test]
    fn html_decodes_entities() {
        assert_eq!(html_to_text("&amp; &lt;tag&gt; &#65; &#x42; &quot;q&quot; &bogus;"), "& <tag> A B \"q\" &bogus;\n");
    }

    #[test]
    fn rtf_reads_text_and_skips_tables() {
        let rtf = "{\\rtf1\\ansi{\\fonttbl{\\f0 Sans;}}\\f0\\fs24 Hello\\par World \\'e9\\u8364?}";
        assert_eq!(rtf_to_text(rtf), "Hello\nWorld \u{e9}\u{20ac}");
    }

    #[test]
    fn rtf_skips_starred_destinations_and_joins_surrogates() {
        assert_eq!(rtf_to_text("{\\rtf1{\\*\\generator Word;}Ok}"), "Ok");
        assert_eq!(rtf_to_text("{\\rtf1 \\u-10179?\\u-8704?}"), "\u{1f600}");
    }

    #[test]
    fn rtf_escapes_are_read_back() {
        assert_eq!(rtf_to_text("{\\rtf1 a\\{b\\}\\\\c}"), "a{b}\\c");
    }

    #[test]
    fn text_to_rtf_round_trips() {
        let text = "Caf\u{e9} \u{20ac} {x}\\y\nline two\t\u{1f600}";
        let rtf = text_to_rtf(text);
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.contains("a\\{") || rtf.contains("\\{x\\}"));
        assert_eq!(rtf_to_text(&rtf), text);
    }

    #[test]
    fn text_to_html_escapes_and_keeps_blank_lines() {
        let html = text_to_html("a<b\n\nc & d");
        assert!(html.contains("<p>a&lt;b</p>\n"));
        assert!(html.contains("<p><br></p>\n"));
        assert!(html.contains("<p>c &amp; d</p>\n"));
        assert!(html.ends_with("</body>\n</html>\n"));
    }

    #[test]
    fn convert_routes_between_formats() {
        assert_eq!(convert("<p>Hi</p>", Format::Html, Format::Txt), "Hi\n");
        assert_eq!(convert("plain", Format::Txt, Format::Txt), "plain");
        let rtf = convert("Hi", Format::Txt, Format::Rtf);
        assert_eq!(convert(&rtf, Format::Rtf, Format::Txt), "Hi");
        assert!(convert("Hi", Format::Txt, Format::Html).contains("<p>Hi</p>"));
    }

    #[test]
    fn a_docx_round_trips_its_text_paragraphs_tabs_and_escapes() {
        let text = "Hello & <world>\n\ncol1\tcol2\nCaf\u{e9} \u{20ac}\n";
        let package = from_plain(text, Format::Docx).unwrap();
        assert_eq!(&package[..2], b"PK");
        assert_eq!(to_plain(&package, Format::Docx).unwrap(), text);
    }

    #[test]
    fn an_odt_round_trips_spaces_tabs_and_keeps_the_mimetype_first_and_stored() {
        let text = "a  b\n   lead\ttab & <x>\n";
        let package = from_plain(text, Format::Odt).unwrap();
        assert_eq!(to_plain(&package, Format::Odt).unwrap(), text);
        // the first member is `mimetype`, stored, so its text sits at a fixed offset after the 30-byte header
        assert_eq!(&package[30..38], b"mimetype");
        assert_eq!(&package[38..38 + 39], b"application/vnd.oasis.opendocument.text");
    }

    #[test]
    fn word_xml_from_other_writers_is_read() {
        let docx = "<w:document><w:body><w:p><w:pPr/><w:r><w:t>One</w:t></w:r><w:r><w:t xml:space=\"preserve\"> two</w:t></w:r></w:p>\
<w:p><w:r><w:t>A&amp;B</w:t><w:br/><w:t>next</w:t></w:r></w:p></w:body></w:document>";
        assert_eq!(docx_xml_to_text(docx), "One two\nA&B\nnext\n");
        let odt = "<office:text><text:h text:outline-level=\"1\">Title</text:h><text:p>x<text:s text:c=\"3\"/>y<text:line-break/>z</text:p><text:p/></office:text>";
        assert_eq!(odt_xml_to_text(odt), "Title\nx   y\nz\n\n");
    }

    #[test]
    fn a_package_without_the_text_part_or_not_a_package_is_an_error() {
        assert!(to_plain(b"not a zip", Format::Docx).unwrap_err().contains("not a valid package"));
        let odt = from_plain("x\n", Format::Odt).unwrap();
        assert!(to_plain(&odt, Format::Docx).unwrap_err().contains("no word/document.xml"));
    }

    #[test]
    fn plain_formats_still_convert_through_the_byte_functions() {
        assert_eq!(from_plain("Hi\n", Format::Html).unwrap(), convert("Hi\n", Format::Txt, Format::Html).into_bytes());
        assert_eq!(to_plain(b"\xef\xbb\xbf<p>Hi</p>", Format::Html).unwrap(), "Hi\n");
        assert_eq!(to_plain(b"keep as is", Format::Txt).unwrap(), "keep as is");
    }

    #[test]
    fn tidy_drops_leading_and_trailing_blank_lines() {
        assert_eq!(tidy_lines("\n\n a \n\n\n b \n\n"), "a\n\nb\n");
        assert_eq!(tidy_lines("\n  \n"), "");
    }
}
