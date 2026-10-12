//! Image metadata containers: 8BIM image resource blocks, IPTC-IIM datasets and simple XMP
//! properties (std only).
//!
//! Three containers carry the descriptive metadata of a picture:
//! - the XMP packet, an XML document with RDF properties (`dc:description`, `xmp:Rating`);
//! - the 8BIM image resource blocks of the APP13 segment, which hold the IPTC-IIM data under
//!   resource id `0x0404`;
//! - the IPTC-IIM stream itself: `0x1C record dataset length value` datasets, where record 2
//!   is the application record (caption, keywords, by-line and so on).
//! This module parses and writes those bytes. It reads no files, decodes no pixels and opens no
//! socket: the caller hands it the segment payloads it already has.

/// Resource id of the IPTC-IIM block inside the 8BIM resources.
pub const RESOURCE_IPTC: u16 = 0x0404;
/// IPTC record number of the application record (the one a photo's caption lives in).
pub const RECORD_APPLICATION: u8 = 2;
/// Application dataset 2:05 (object name, a short title).
pub const DS_OBJECT_NAME: u8 = 5;
/// Application dataset 2:25 (keyword, repeatable).
pub const DS_KEYWORDS: u8 = 25;
/// Application dataset 2:80 (by-line, the photographer).
pub const DS_BYLINE: u8 = 80;
/// Application dataset 2:105 (headline).
pub const DS_HEADLINE: u8 = 105;
/// Application dataset 2:116 (copyright notice).
pub const DS_COPYRIGHT: u8 = 116;
/// Application dataset 2:120 (caption or abstract).
pub const DS_CAPTION: u8 = 120;

/// Prefix of an APP13 payload written by the image editors (NUL terminated).
const PHOTOSHOP_HEADER: &[u8] = b"Photoshop 3.0\0";
/// Prefix of an APP1 payload that carries an XMP packet (NUL terminated).
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// Namespace prefixes the XMP writer declares, with their URIs.
const NAMESPACES: &[(&str, &str)] = &[
    ("dc", "http://purl.org/dc/elements/1.1/"),
    ("xmp", "http://ns.adobe.com/xap/1.0/"),
    ("photoshop", "http://ns.adobe.com/photoshop/1.0/"),
    ("Iptc4xmpCore", "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/"),
];
/// Properties of the `dc` namespace that the XML form stores as a language alternative.
const LANG_ALT: &[&str] = &["title", "description", "rights"];

/// One 8BIM image resource block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageResource {
    pub id: u16,
    pub name: String,
    pub data: Vec<u8>,
}

/// One IPTC-IIM dataset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IptcField {
    pub record: u8,
    pub dataset: u8,
    pub value: Vec<u8>,
}

/// Reads the 8BIM resource blocks in `buf`. Parsing stops at the first malformed block and
/// keeps the blocks read before it.
pub fn parse_resources(buf: &[u8]) -> Vec<ImageResource> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 7 <= buf.len() {
        if &buf[i..i + 4] != b"8BIM" {
            break;
        }
        let id = u16::from_be_bytes([buf[i + 4], buf[i + 5]]);
        let name_len = buf[i + 6] as usize;
        let name_end = i + 7 + name_len;
        if name_end > buf.len() {
            break;
        }
        let name = decode_text(&buf[i + 7..name_end]);
        // The pascal string (count byte plus name) is padded to an even length.
        let total = 1 + name_len;
        let size_at = i + 6 + total + total % 2;
        if size_at + 4 > buf.len() {
            break;
        }
        let size = u32::from_be_bytes([
            buf[size_at],
            buf[size_at + 1],
            buf[size_at + 2],
            buf[size_at + 3],
        ]) as usize;
        let start = size_at + 4;
        let end = match start.checked_add(size) {
            Some(end) if end <= buf.len() => end,
            _ => break,
        };
        out.push(ImageResource {
            id,
            name,
            data: buf[start..end].to_vec(),
        });
        // The data is padded to an even length as well.
        i = end + size % 2;
    }
    out
}

/// Appends one 8BIM resource block to `out`. A name longer than 255 bytes is cut.
pub fn write_resource(out: &mut Vec<u8>, id: u16, name: &str, data: &[u8]) {
    let name_bytes = &name.as_bytes()[..name.len().min(255)];
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(&id.to_be_bytes());
    out.push(name_bytes.len() as u8);
    out.extend_from_slice(name_bytes);
    if (1 + name_bytes.len()) % 2 == 1 {
        out.push(0);
    }
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// The first resource block with `id`, if there is one.
pub fn find_resource(resources: &[ImageResource], id: u16) -> Option<&ImageResource> {
    resources.iter().find(|r| r.id == id)
}

/// Reads the IPTC-IIM datasets in `buf`. Bytes that are not a dataset marker are skipped, and
/// parsing stops at a dataset whose length runs past the end of the buffer.
pub fn parse_iptc(buf: &[u8]) -> Vec<IptcField> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 5 <= buf.len() {
        if buf[i] != 0x1C {
            i += 1;
            continue;
        }
        let record = buf[i + 1];
        let dataset = buf[i + 2];
        let raw = u16::from_be_bytes([buf[i + 3], buf[i + 4]]);
        let (len, start) = if (raw & 0x8000) != 0 {
            // Extended length: the low 15 bits give the size of the length field that follows.
            let n = (raw & 0x7FFF) as usize;
            if n == 0 || n > 4 || i + 5 + n > buf.len() {
                break;
            }
            let mut l = 0usize;
            for k in 0..n {
                l = (l << 8) | buf[i + 5 + k] as usize;
            }
            (l, i + 5 + n)
        } else {
            (raw as usize, i + 5)
        };
        let end = match start.checked_add(len) {
            Some(end) if end <= buf.len() => end,
            _ => break,
        };
        out.push(IptcField {
            record,
            dataset,
            value: buf[start..end].to_vec(),
        });
        i = end;
    }
    out
}

/// Serialises datasets as an IPTC-IIM stream. Values over 32767 bytes use the extended length.
pub fn write_iptc(fields: &[IptcField]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in fields {
        out.push(0x1C);
        out.push(f.record);
        out.push(f.dataset);
        let n = f.value.len();
        if n <= 0x7FFF {
            out.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            out.extend_from_slice(&[0x80, 0x04]);
            out.extend_from_slice(&(n as u32).to_be_bytes());
        }
        out.extend_from_slice(&f.value);
    }
    out
}

/// Every application-record value of `dataset`, in stream order.
pub fn application_values(fields: &[IptcField], dataset: u8) -> Vec<String> {
    fields
        .iter()
        .filter(|f| f.record == RECORD_APPLICATION && f.dataset == dataset)
        .map(|f| decode_text(&f.value))
        .collect()
}

/// The first application-record value of `dataset`.
pub fn application_value(fields: &[IptcField], dataset: u8) -> Option<String> {
    application_values(fields, dataset).into_iter().next()
}

/// Replaces every application-record value of `dataset` with `text`.
pub fn set_application_text(fields: &mut Vec<IptcField>, dataset: u8, text: &str) {
    fields.retain(|f| !(f.record == RECORD_APPLICATION && f.dataset == dataset));
    add_application_text(fields, dataset, text);
}

/// Appends one application-record value of `dataset` (keywords repeat, so this adds).
pub fn add_application_text(fields: &mut Vec<IptcField>, dataset: u8, text: &str) {
    fields.push(IptcField {
        record: RECORD_APPLICATION,
        dataset,
        value: text.as_bytes().to_vec(),
    });
}

/// The IPTC datasets carried by an APP13 segment payload, or none when the payload has another
/// header or no IPTC resource.
pub fn iptc_from_app13(payload: &[u8]) -> Vec<IptcField> {
    if !payload.starts_with(PHOTOSHOP_HEADER) {
        return Vec::new();
    }
    let resources = parse_resources(&payload[PHOTOSHOP_HEADER.len()..]);
    match find_resource(&resources, RESOURCE_IPTC) {
        Some(r) => parse_iptc(&r.data),
        None => Vec::new(),
    }
}

/// The XMP packet carried by an APP1 segment payload, if it is one and is valid UTF-8.
pub fn xmp_from_app1(payload: &[u8]) -> Option<&str> {
    let rest = payload.strip_prefix(XMP_HEADER)?;
    std::str::from_utf8(rest).ok()
}

/// The XMP value of `name` (for example `dc:description` or `xmp:Rating`). The property is read
/// as an attribute of an RDF description, or as an element. An element holding a language
/// alternative or a list gives its first item. XML entities are decoded.
pub fn xmp_value(packet: &str, name: &str) -> Option<String> {
    attr_value(packet, name)
        .map(unescape_xml)
        .or_else(|| element_text(packet, name))
}

/// Builds an XMP packet holding `props`, each a `(prefix:name, value)` pair. Pairs with a prefix
/// outside the declared namespaces are dropped. `dc:title`, `dc:description` and `dc:rights`
/// are written as language alternatives, as XMP requires.
pub fn xmp_packet(props: &[(&str, &str)]) -> String {
    let mut s = String::new();
    s.push_str("<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n");
    s.push_str("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n");
    s.push_str("<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n");
    s.push_str("<rdf:Description rdf:about=\"\"");
    for (prefix, uri) in NAMESPACES {
        s.push_str(&format!(" xmlns:{}=\"{}\"", prefix, uri));
    }
    s.push_str(">\n");
    for (name, value) in props {
        let (prefix, local) = match name.split_once(':') {
            Some(parts) => parts,
            None => continue,
        };
        if !NAMESPACES.iter().any(|(p, _)| *p == prefix) {
            continue;
        }
        let v = escape_xml(value);
        if prefix == "dc" && LANG_ALT.contains(&local) {
            s.push_str(&format!(
                "<{n}><rdf:Alt><rdf:li xml:lang=\"x-default\">{v}</rdf:li></rdf:Alt></{n}>\n",
                n = name,
                v = v
            ));
        } else {
            s.push_str(&format!("<{n}>{v}</{n}>\n", n = name, v = v));
        }
    }
    s.push_str("</rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>\n");
    s
}

/// The image caption: the XMP `dc:description` when it is present and not empty, else the
/// IPTC caption dataset 2:120.
pub fn caption(xmp: Option<&str>, iptc: &[IptcField]) -> Option<String> {
    xmp.and_then(|p| xmp_value(p, "dc:description"))
        .filter(|s| !s.is_empty())
        .or_else(|| application_value(iptc, DS_CAPTION).filter(|s| !s.is_empty()))
}

/// Decodes a text value: UTF-8 when it is valid, else ISO 8859-1 (one char per byte), since
/// older IPTC writers did not declare a character set. Trailing NUL padding is dropped.
fn decode_text(bytes: &[u8]) -> String {
    let trimmed = {
        let mut end = bytes.len();
        while end > 0 && bytes[end - 1] == 0 {
            end -= 1;
        }
        &bytes[..end]
    };
    match std::str::from_utf8(trimmed) {
        Ok(s) => s.to_string(),
        Err(_) => trimmed.iter().map(|&b| b as char).collect(),
    }
}

/// The value of attribute `name="..."` where the name starts after whitespace.
fn attr_value<'a>(packet: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{}=\"", name);
    let mut from = 0usize;
    while let Some(p) = packet[from..].find(&key) {
        let at = from + p;
        let s = at + key.len();
        if matches!(packet[..at].chars().last(), Some(c) if c.is_whitespace()) {
            let e = packet[s..].find('"')? + s;
            return Some(&packet[s..e]);
        }
        from = s;
    }
    None
}

/// The text of the first element `<name>...</name>`.
fn element_text(packet: &str, name: &str) -> Option<String> {
    let open = format!("<{}", name);
    let close = format!("</{}>", name);
    let mut from = 0usize;
    while let Some(p) = packet[from..].find(&open) {
        let after = from + p + open.len();
        if matches!(
            packet[after..].chars().next(),
            Some('>') | Some(' ') | Some('\n') | Some('\r') | Some('\t')
        ) {
            let gt = after + packet[after..].find('>')?;
            let end = gt + 1 + packet[gt + 1..].find(&close)?;
            return Some(first_item(&packet[gt + 1..end]));
        }
        from = after;
    }
    None
}

/// The first `rdf:li` item of `inner`, or all of `inner` when it holds no list, trimmed and
/// decoded.
fn first_item(inner: &str) -> String {
    let body: &str = match inner.find("<rdf:li") {
        Some(p) => {
            let rest = &inner[p..];
            match rest.find('>') {
                Some(gt) => {
                    let t = &rest[gt + 1..];
                    let e = t.find("</rdf:li>").unwrap_or(t.len());
                    &t[..e]
                }
                None => "",
            }
        }
        None => inner,
    };
    unescape_xml(body.trim())
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_block_parses_from_hand_written_bytes() {
        // "8BIM", id 0x0404, empty name (count 0 plus one pad byte), size 4, data 1 2 3 4.
        let buf = [
            b'8', b'B', b'I', b'M', 0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 1, 2, 3, 4,
        ];
        let r = parse_resources(&buf);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, 0x0404);
        assert_eq!(r[0].name, "");
        assert_eq!(r[0].data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn odd_sized_resource_is_padded_before_the_next_block() {
        // First block: id 0x03ED, size 3 (data 7 8 9) plus one pad byte, 16 bytes in all.
        // Second block starts at byte 16: id 0x0404, size 2 (data AA BB).
        let mut buf = Vec::new();
        write_resource(&mut buf, 0x03ED, "", &[7, 8, 9]);
        write_resource(&mut buf, 0x0404, "", &[0xAA, 0xBB]);
        assert_eq!(buf.len(), 16 + 14);
        let r = parse_resources(&buf);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].data, vec![7, 8, 9]);
        assert_eq!(r[1].id, 0x0404);
        assert_eq!(find_resource(&r, RESOURCE_IPTC).unwrap().data, vec![0xAA, 0xBB]);
        // A name of two bytes makes the pascal string 3 bytes, padded to 4.
        let mut named = Vec::new();
        write_resource(&mut named, 0x0001, "ab", &[5]);
        assert_eq!(named.len(), 4 + 2 + 1 + 2 + 1 + 4 + 1 + 1);
        assert_eq!(parse_resources(&named)[0].name, "ab");
    }

    #[test]
    fn iptc_short_and_extended_lengths() {
        // Dataset 2:120, length 5, "Hello".
        let short = [0x1C, 0x02, 0x78, 0x00, 0x05, b'H', b'e', b'l', b'l', b'o'];
        let f = parse_iptc(&short);
        assert_eq!(f.len(), 1);
        assert_eq!(application_value(&f, DS_CAPTION), Some("Hello".to_string()));
        // Dataset 2:25 with an extended length: 0x8002 says two length bytes follow, 0x0003.
        let ext = [0x1C, 0x02, 0x19, 0x80, 0x02, 0x00, 0x03, b'a', b'b', b'c'];
        let g = parse_iptc(&ext);
        assert_eq!(g.len(), 1);
        assert_eq!(application_values(&g, DS_KEYWORDS), vec!["abc".to_string()]);
        // A dataset whose length runs past the end is dropped, and so is everything after it.
        let cut = [0x1C, 0x02, 0x78, 0x00, 0x09, b'x'];
        assert!(parse_iptc(&cut).is_empty());
    }

    #[test]
    fn iptc_text_falls_back_to_latin1_when_not_utf8() {
        // 0xC9 alone is not valid UTF-8; as ISO 8859-1 it is U+00C9.
        let buf = [0x1C, 0x02, 0x78, 0x00, 0x01, 0xC9];
        let f = parse_iptc(&buf);
        assert_eq!(application_value(&f, DS_CAPTION), Some("\u{c9}".to_string()));
    }

    #[test]
    fn iptc_round_trip_and_set_replaces_values() {
        let mut fields = Vec::new();
        add_application_text(&mut fields, DS_KEYWORDS, "a");
        add_application_text(&mut fields, DS_KEYWORDS, "b");
        set_application_text(&mut fields, DS_CAPTION, "X");
        set_application_text(&mut fields, DS_CAPTION, "Y");
        assert_eq!(application_values(&fields, DS_CAPTION), vec!["Y".to_string()]);
        assert_eq!(
            application_values(&fields, DS_KEYWORDS),
            vec!["a".to_string(), "b".to_string()]
        );
        let bytes = write_iptc(&fields);
        // Each dataset is 5 bytes of header plus its value: 6 + 6 + 6 = 18 bytes.
        assert_eq!(bytes.len(), 6 + 6 + 6);
        assert_eq!(parse_iptc(&bytes), fields);
    }

    #[test]
    fn app13_payload_yields_the_caption_and_xmp_wins_over_iptc() {
        // The IPTC block holds one caption dataset "Hi": 5 header bytes plus 2 value bytes.
        let mut fields = Vec::new();
        add_application_text(&mut fields, DS_CAPTION, "Hi");
        let iptc = write_iptc(&fields);
        assert_eq!(iptc.len(), 7);
        let mut payload = PHOTOSHOP_HEADER.to_vec();
        write_resource(&mut payload, RESOURCE_IPTC, "", &iptc);
        let from_app13 = iptc_from_app13(&payload);
        assert_eq!(application_value(&from_app13, DS_CAPTION), Some("Hi".to_string()));
        assert_eq!(caption(None, &from_app13), Some("Hi".to_string()));

        let xmp = xmp_packet(&[("dc:description", "From XMP")]);
        assert_eq!(caption(Some(xmp.as_str()), &from_app13), Some("From XMP".to_string()));
        // An empty XMP description falls back to the IPTC caption.
        let empty = "<dc:description></dc:description>";
        assert_eq!(caption(Some(empty), &from_app13), Some("Hi".to_string()));
        // A payload with another header carries no IPTC.
        assert!(iptc_from_app13(b"Other\0\0\0").is_empty());
        // An APP1 payload with the XMP header yields the packet.
        let mut app1 = XMP_HEADER.to_vec();
        app1.extend_from_slice(b"<a/>");
        assert_eq!(xmp_from_app1(&app1), Some("<a/>"));
    }

    #[test]
    fn xmp_attribute_element_and_language_alternative() {
        let packet = "<rdf:Description rdf:about=\"\" xmp:Rating=\"5\" photoshop:City=\"Reykjavik\">\
            <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">A &amp; B</rdf:li></rdf:Alt></dc:description>\
            </rdf:Description>";
        assert_eq!(xmp_value(packet, "xmp:Rating"), Some("5".to_string()));
        assert_eq!(xmp_value(packet, "photoshop:City"), Some("Reykjavik".to_string()));
        assert_eq!(xmp_value(packet, "dc:description"), Some("A & B".to_string()));
        assert_eq!(xmp_value(packet, "dc:title"), None);
    }

    #[test]
    fn xmp_writer_escapes_and_reads_back() {
        let packet = xmp_packet(&[
            ("dc:description", "Ben & <Jerry>"),
            ("xmp:Rating", "3"),
            ("bogus:x", "1"),
        ]);
        assert!(packet.contains(
            "<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">Ben &amp; &lt;Jerry&gt;</rdf:li></rdf:Alt></dc:description>"
        ));
        assert_eq!(xmp_value(&packet, "dc:description"), Some("Ben & <Jerry>".to_string()));
        assert_eq!(xmp_value(&packet, "xmp:Rating"), Some("3".to_string()));
        assert_eq!(xmp_value(&packet, "bogus:x"), None);
    }
}
