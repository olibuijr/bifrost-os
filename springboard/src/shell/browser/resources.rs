//! Subresource plumbing for `doc::Document`: request/response types, limits,
//! charset detection and decoding, image size sniffing, `srcset`, `@import`
//! scanning, a cookie jar, CORS-lite, import maps and a small ES module
//! rewriter. Everything here is pure and host-testable; no I/O.

use super::nav;
use super::style::{parse_stylesheet, MediaEnv, Origin};
use url::Url;

pub type ResourceId = u32;

/// What a request is for. Decides its priority.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ResourceKind {
    Stylesheet,
    Script,
    Module,
    Image,
    Font,
    /// A `fetch`/XHR request made by page script.
    Fetch,
}

impl ResourceKind {
    /// Lower is more urgent: css (blocks render) > scripts > script fetches > images > fonts.
    pub fn priority(self) -> u8 {
        match self {
            ResourceKind::Stylesheet => 0,
            ResourceKind::Script | ResourceKind::Module => 1,
            ResourceKind::Fetch => 2,
            ResourceKind::Image => 3,
            ResourceKind::Font => 4,
        }
    }
}

/// One request for the transport (the loader) to perform. Redirects are the transport's job.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceRequest {
    pub id: ResourceId,
    pub url: String,
    pub kind: ResourceKind,
    pub priority: u8,
    /// True for style sheets: rendering waits for them.
    pub render_blocking: bool,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// The transport's answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Final URL after redirects; empty keeps the request URL.
    pub url: String,
    pub error: Option<String>,
}

impl ResourceResponse {
    pub fn ok(content_type: &str, body: &[u8]) -> ResourceResponse {
        ResourceResponse {
            status: 200,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.to_vec(),
            ..ResourceResponse::default()
        }
    }

    pub fn failed(message: &str) -> ResourceResponse {
        ResourceResponse { error: Some(message.to_owned()), ..ResourceResponse::default() }
    }

    pub fn status(status: u16) -> ResourceResponse {
        ResourceResponse { status, ..ResourceResponse::default() }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> ResourceResponse {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// First header named `name`, case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }
}

/// Per-page caps; keeps a hostile or huge page inside the memory budget.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Requests the page may make in total.
    pub max_resources: usize,
    /// Sum of all response bodies accepted.
    pub max_total_bytes: usize,
    pub max_html_bytes: usize,
    pub max_css_bytes: usize,
    pub max_script_bytes: usize,
    /// Scripts and modules that may run.
    pub max_scripts: usize,
    pub max_images: usize,
    /// `@import` chain depth.
    pub max_import_depth: u8,
    /// Static `import` graph depth.
    pub max_module_depth: u8,
    /// Off keeps the page script-free (reader style).
    pub scripting: bool,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_resources: 150,
            max_total_bytes: 12 * 1024 * 1024,
            max_html_bytes: 3 * 1024 * 1024,
            max_css_bytes: 512 * 1024,
            max_script_bytes: 1536 * 1024,
            max_scripts: 80,
            max_images: 64,
            max_import_depth: 4,
            max_module_depth: 8,
            scripting: true,
        }
    }
}

/// What the page has spent so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Budget {
    pub requests: usize,
    pub images: usize,
    pub bytes: usize,
}

impl Budget {
    /// Counts one request; false when the cap is reached (nothing is counted then).
    pub fn admit(&mut self, kind: ResourceKind, lim: &Limits) -> bool {
        if self.requests >= lim.max_resources || (kind == ResourceKind::Image && self.images >= lim.max_images) {
            return false;
        }
        self.requests += 1;
        if kind == ResourceKind::Image {
            self.images += 1;
        }
        true
    }

    /// Counts accepted body bytes; false (nothing counted) when over the cap.
    pub fn add_bytes(&mut self, n: usize, lim: &Limits) -> bool {
        if self.bytes.saturating_add(n) > lim.max_total_bytes {
            return false;
        }
        self.bytes += n;
        true
    }
}

// ---- charset -------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Charset {
    Utf8,
    /// windows-1252; also what `iso-8859-1` and `latin1` mean on the web.
    Windows1252,
    Utf16Le,
    Utf16Be,
}

impl Charset {
    pub fn from_label(label: &str) -> Option<Charset> {
        match label.trim().trim_matches(|c: char| c == '"' || c == '\'').to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" | "unicode-1-1-utf-8" => Some(Charset::Utf8),
            "windows-1252" | "cp1252" | "iso-8859-1" | "iso8859-1" | "latin1" | "l1" | "us-ascii" | "ascii"
            | "x-cp1252" | "iso_8859-1" => Some(Charset::Windows1252),
            "utf-16" | "utf-16le" | "ucs-2" => Some(Charset::Utf16Le),
            "utf-16be" => Some(Charset::Utf16Be),
            _ => None,
        }
    }
}

/// BOM charset and its length in bytes.
pub fn sniff_bom(b: &[u8]) -> Option<(Charset, usize)> {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some((Charset::Utf8, 3))
    } else if b.starts_with(&[0xFF, 0xFE]) {
        Some((Charset::Utf16Le, 2))
    } else if b.starts_with(&[0xFE, 0xFF]) {
        Some((Charset::Utf16Be, 2))
    } else {
        None
    }
}

/// `charset=` of a Content-Type value.
pub fn charset_from_content_type(ct: &str) -> Option<Charset> {
    let lower = ct.to_ascii_lowercase();
    let at = lower.find("charset")?;
    let rest = lower[at + 7..].trim_start().strip_prefix('=')?;
    let label: String = rest
        .trim_start()
        .trim_start_matches(|c: char| c == '"' || c == '\'')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    Charset::from_label(&label)
}

/// `<meta charset>` or `<meta http-equiv=content-type content="...charset=">` within the first KiB.
/// A meta that names UTF-16 means UTF-8 (the page was readable as ASCII).
pub fn charset_from_meta(b: &[u8]) -> Option<Charset> {
    let head = &b[..b.len().min(1024)];
    let lower: String = head.iter().map(|c| c.to_ascii_lowercase() as char).collect();
    let mut from = 0;
    while let Some(p) = lower[from..].find("charset") {
        let at = from + p + 7;
        let rest = lower[at..].trim_start();
        if let Some(rest) = rest.strip_prefix('=') {
            let label: String = rest
                .trim_start()
                .trim_start_matches(|c: char| c == '"' || c == '\'')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if let Some(cs) = Charset::from_label(&label) {
                return Some(match cs {
                    Charset::Utf16Le | Charset::Utf16Be => Charset::Utf8,
                    c => c,
                });
            }
        }
        from = at;
    }
    None
}

/// Picks the charset: BOM, then Content-Type, then meta, else UTF-8.
/// The flag says whether something declared it (an undeclared UTF-8 that
/// fails to validate falls back to windows-1252).
pub fn detect_charset(b: &[u8], content_type: Option<&str>) -> (Charset, bool) {
    if let Some((cs, _)) = sniff_bom(b) {
        return (cs, true);
    }
    if let Some(cs) = content_type.and_then(charset_from_content_type) {
        return (cs, true);
    }
    if let Some(cs) = charset_from_meta(b) {
        return (cs, true);
    }
    (Charset::Utf8, false)
}

const W1252_HIGH: [u16; 32] = [
    0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D,
    0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
    0x0153, 0x9D, 0x017E, 0x0178,
];

fn decode_1252(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len());
    for &c in b {
        let ch = match c {
            0x80..=0x9F => char::from_u32(u32::from(W1252_HIGH[(c - 0x80) as usize])).unwrap_or('\u{FFFD}'),
            _ => char::from(c),
        };
        s.push(ch);
    }
    s
}

/// Decodes consuming the buffer; UTF-8 reuses it in place when valid, so the body is never held twice.
pub fn decode(mut bytes: Vec<u8>, cs: Charset, declared: bool) -> String {
    match cs {
        Charset::Utf8 => {
            if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                bytes.drain(..3);
            }
            match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => {
                    let b = e.into_bytes();
                    if declared {
                        String::from_utf8_lossy(&b).into_owned()
                    } else {
                        decode_1252(&b)
                    }
                }
            }
        }
        Charset::Windows1252 => decode_1252(&bytes),
        Charset::Utf16Le | Charset::Utf16Be => {
            let mut be = cs == Charset::Utf16Be;
            let mut skip = 0;
            if let Some((c, n)) = sniff_bom(&bytes) {
                if matches!(c, Charset::Utf16Le | Charset::Utf16Be) {
                    be = c == Charset::Utf16Be;
                    skip = n;
                }
            }
            let units: Vec<u16> = bytes[skip..]
                .chunks_exact(2)
                .map(|p| if be { u16::from_be_bytes([p[0], p[1]]) } else { u16::from_le_bytes([p[0], p[1]]) })
                .collect();
            String::from_utf16_lossy(&units)
        }
    }
}

/// Decodes a CSS or script body (BOM, Content-Type, else UTF-8 with a windows-1252 fallback).
pub fn decode_text(bytes: &[u8], content_type: Option<&str>) -> String {
    let (cs, declared) = detect_charset(&[], content_type);
    let (cs, declared) = match sniff_bom(bytes) {
        Some((c, _)) => (c, true),
        None => (cs, declared),
    };
    decode(bytes.to_vec(), cs, declared)
}

// ---- images --------------------------------------------------------------

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?])))
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?])))
}

fn le24(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*b.get(i)?) | u32::from(*b.get(i + 1)?) << 8 | u32::from(*b.get(i + 2)?) << 16)
}

/// Pixel size from the header of a PNG, GIF, JPEG, WebP or SVG, without decoding.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    let size = if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        (be32(b, 16)?, be32(b, 20)?)
    } else if b.starts_with(b"GIF8") {
        (le16(b, 6)?, le16(b, 8)?)
    } else if b.starts_with(&[0xFF, 0xD8]) {
        jpeg_size(b)?
    } else if b.len() >= 16 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        webp_size(b)?
    } else {
        svg_size(b)?
    };
    (size.0 > 0 && size.1 > 0).then_some(size)
}

fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
            return Some((be16(b, i + 7)?, be16(b, i + 5)?));
        }
        if m == 0xD8 || (0xD0..=0xD7).contains(&m) || m == 0x01 {
            i += 2;
            continue;
        }
        i += 2 + be16(b, i + 2)? as usize;
    }
    None
}

fn webp_size(b: &[u8]) -> Option<(u32, u32)> {
    match b.get(12..16)? {
        b"VP8 " => Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF)),
        b"VP8L" => {
            let (b0, b1, b2, b3) = (
                u32::from(*b.get(21)?),
                u32::from(*b.get(22)?),
                u32::from(*b.get(23)?),
                u32::from(*b.get(24)?),
            );
            Some((1 + (b0 | (b1 & 0x3F) << 8), 1 + ((b1 >> 6) | b2 << 2 | (b3 & 0xF) << 10)))
        }
        b"VP8X" => Some((1 + le24(b, 24)?, 1 + le24(b, 27)?)),
        _ => None,
    }
}

fn svg_size(b: &[u8]) -> Option<(u32, u32)> {
    let head = String::from_utf8_lossy(&b[..b.len().min(2048)]).into_owned();
    let at = head.find("<svg")?;
    let tag = &head[at..head[at..].find('>').map_or(head.len(), |e| at + e)];
    let num = |name: &str| -> Option<f32> {
        let p = tag.find(&format!("{name}=\""))? + name.len() + 2;
        let v: String = tag[p..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        v.parse().ok()
    };
    if let (Some(w), Some(h)) = (num("width"), num("height")) {
        return Some((w.round() as u32, h.round() as u32));
    }
    let p = tag.find("viewBox=\"")? + 9;
    let parts: Vec<f32> = tag[p..].split('"').next()?.split(|c: char| c == ' ' || c == ',').filter_map(|s| s.parse().ok()).collect();
    (parts.len() == 4).then(|| (parts[2].round() as u32, parts[3].round() as u32))
}

/// Chooses one `srcset` candidate for a ~800 px wide, 1.5x device: the smallest
/// that is at least large enough, else the largest. Returns the unresolved URL.
pub fn pick_srcset(srcset: &str) -> Option<String> {
    let mut cands: Vec<(String, f32)> = Vec::new();
    for part in srcset.split(',') {
        let mut it = part.split_whitespace();
        let Some(url) = it.next() else { continue };
        if url.starts_with("data:") {
            continue;
        }
        let ratio = match it.next() {
            Some(d) if d.ends_with('w') => d[..d.len() - 1].parse::<f32>().ok().map(|w| w / 800.0),
            Some(d) if d.ends_with('x') => d[..d.len() - 1].parse::<f32>().ok().map(|x| x / 1.5),
            _ => Some(1.0 / 1.5),
        };
        if let Some(r) = ratio {
            cands.push((url.to_owned(), r));
        }
    }
    let enough = cands.iter().filter(|c| c.1 >= 1.0).min_by(|a, b| a.1.total_cmp(&b.1));
    let pick = enough.or_else(|| cands.iter().max_by(|a, b| a.1.total_cmp(&b.1)));
    pick.map(|c| c.0.clone())
}

/// Whether an `<source type>` names a format the image path can handle.
pub fn supported_image_type(t: &str) -> bool {
    let t = t.trim().to_ascii_lowercase();
    matches!(t.as_str(), "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" | "image/svg+xml")
}

/// Evaluates a `media` attribute against `env` using the style parser.
pub fn media_matches(media: &str, env: &MediaEnv) -> bool {
    let m = media.trim();
    if m.is_empty() || m.eq_ignore_ascii_case("all") {
        return true;
    }
    let sheet = parse_stylesheet(&format!("@media {m}{{a{{color:red}}}}"), Origin::Author);
    sheet.rules.first().is_some_and(|r| r.media.iter().all(|q| q.matches(env)))
}

// ---- css -----------------------------------------------------------------

fn skip_string(b: &[u8], i: usize) -> usize {
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() && b[j] != q {
        if b[j] == b'\\' {
            j += 1;
        }
        j += 1;
    }
    (j + 1).min(b.len())
}

/// `@import` rules of a style sheet as (url, media). Comments are skipped; `layer`/`supports()` conditions are ignored.
pub fn extract_imports(css: &str) -> Vec<(String, Option<String>)> {
    let b = css.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
            }
            b'"' | b'\'' => i = skip_string(b, i),
            b'@' if b.len() >= i + 7 && b[i + 1..i + 7].eq_ignore_ascii_case(b"import") => {
                let mut j = i + 7;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let mut spec = None;
                if b.len() >= j + 4 && b[j..j + 4].eq_ignore_ascii_case(b"url(") {
                    j += 4;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                        let end = skip_string(b, j);
                        spec = Some(css.get(j + 1..end.saturating_sub(1).max(j + 1)).unwrap_or("").to_owned());
                        j = end;
                    } else {
                        let end = css[j..].find(')').map_or(b.len(), |p| j + p);
                        spec = Some(css[j..end].trim().to_owned());
                        j = end;
                    }
                    j = css[j..].find(')').map_or(b.len(), |p| j + p + 1);
                } else if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                    let end = skip_string(b, j);
                    spec = Some(css.get(j + 1..end.saturating_sub(1).max(j + 1)).unwrap_or("").to_owned());
                    j = end;
                }
                let semi = css[j..].find(';').map_or(b.len(), |p| j + p);
                let rest = css[j..semi].trim();
                let lower = rest.to_ascii_lowercase();
                let media = if rest.is_empty() || lower.starts_with("layer") || lower.starts_with("supports") {
                    None
                } else {
                    Some(rest.to_owned())
                };
                if let Some(s) = spec.filter(|s| !s.is_empty()) {
                    out.push((s, media));
                }
                i = semi.max(i + 7);
            }
            _ => i += 1,
        }
    }
    out
}

// ---- cookies and CORS ----------------------------------------------------

#[derive(Clone, Debug)]
struct Cookie {
    scope: String,
    host_only: bool,
    path: String,
    name: String,
    value: String,
    secure: bool,
    http_only: bool,
}

const MAX_COOKIES: usize = 300;

/// Cookies per host/domain. No clock: `Max-Age<=0` deletes, other lifetimes are session-long.
#[derive(Clone, Debug, Default)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> CookieJar {
        CookieJar::default()
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// Stores one `Set-Cookie` header value received from `url`.
    pub fn set_from_header(&mut self, url: &str, header: &str) {
        let Ok(u) = Url::parse(url) else { return };
        let Some(host) = u.host_str().map(str::to_ascii_lowercase) else { return };
        let mut parts = header.split(';');
        let Some(first) = parts.next() else { return };
        let Some((name, value)) = first.split_once('=') else { return };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() {
            return;
        }
        let up = u.path();
        let mut c = Cookie {
            scope: host.clone(),
            host_only: true,
            path: match up.rfind('/') {
                Some(p) if p > 0 && up.starts_with('/') => up[..p].to_owned(),
                _ => "/".to_owned(),
            },
            name: name.to_owned(),
            value: value.to_owned(),
            secure: false,
            http_only: false,
        };
        let mut expired = false;
        for a in parts {
            let (k, v) = a.split_once('=').unwrap_or((a, ""));
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match k.as_str() {
                "domain" => {
                    let d = v.trim_start_matches('.').to_ascii_lowercase();
                    if !d.is_empty() && (host == d || host.ends_with(&format!(".{d}"))) {
                        c.scope = d;
                        c.host_only = false;
                    }
                }
                "path" if v.starts_with('/') => c.path = v.to_owned(),
                "max-age" => {
                    if v.parse::<i64>().is_ok_and(|n| n <= 0) {
                        expired = true;
                    }
                }
                "secure" => c.secure = true,
                "httponly" => c.http_only = true,
                _ => {}
            }
        }
        self.cookies
            .retain(|o| !(o.scope == c.scope && o.host_only == c.host_only && o.path == c.path && o.name == c.name));
        if !expired && self.cookies.len() < MAX_COOKIES {
            self.cookies.push(c);
        }
    }

    fn applicable(&self, url: &str, include_http_only: bool) -> Vec<&Cookie> {
        let Ok(u) = Url::parse(url) else { return Vec::new() };
        let Some(host) = u.host_str().map(str::to_ascii_lowercase) else { return Vec::new() };
        let https = u.scheme() == "https";
        let path = u.path();
        let mut v: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| {
                let host_ok = if c.host_only { c.scope == host } else { host == c.scope || host.ends_with(&format!(".{}", c.scope)) };
                let path_ok = c.path == "/"
                    || path == c.path
                    || (path.starts_with(&c.path) && (c.path.ends_with('/') || path[c.path.len()..].starts_with('/')));
                host_ok && path_ok && (!c.secure || https) && (include_http_only || !c.http_only)
            })
            .collect();
        v.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        v
    }

    /// The `Cookie` request header value for `url`, if any cookie applies.
    pub fn header_for(&self, url: &str) -> Option<String> {
        let v = self.applicable(url, true);
        (!v.is_empty()).then(|| v.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; "))
    }

    /// What `document.cookie` reads for `url` (no HttpOnly cookies).
    pub fn document_cookie(&self, url: &str) -> String {
        self.applicable(url, false).iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; ")
    }
}

/// `scheme://host[:port]` of a URL ("null" when it has none).
pub fn origin_of(url: &str) -> String {
    Url::parse(url).map_or_else(|_| "null".to_owned(), |u| u.origin().ascii_serialization())
}

/// CORS-lite: same origin passes; cross-origin needs `Access-Control-Allow-Origin` of `*` or the page origin.
pub fn cors_allows(page_origin: &str, url: &str, resp_headers: &[(String, String)]) -> bool {
    if origin_of(url) == page_origin {
        return true;
    }
    resp_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("access-control-allow-origin"))
        .is_some_and(|(_, v)| v.trim() == "*" || v.trim() == page_origin)
}

// ---- import maps ---------------------------------------------------------

/// `<script type=importmap>`: top-level `imports` only (no scopes, no integrity).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportMap {
    pub imports: Vec<(String, String)>,
}

enum Json {
    Obj(Vec<(String, Json)>),
    Str(String),
    Other,
}

struct JsonParser<'a> {
    b: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn string(&mut self) -> Option<String> {
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
                            let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            self.i += 4;
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(c),
            }
        }
        None
    }

    fn value(&mut self, depth: u8) -> Option<Json> {
        if depth > 8 {
            return None;
        }
        self.ws();
        match *self.b.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    match *self.b.get(self.i)? {
                        b'}' => {
                            self.i += 1;
                            return Some(Json::Obj(items));
                        }
                        b',' => self.i += 1,
                        b'"' => {
                            let k = self.string()?;
                            self.ws();
                            if *self.b.get(self.i)? != b':' {
                                return None;
                            }
                            self.i += 1;
                            items.push((k, self.value(depth + 1)?));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                loop {
                    self.ws();
                    match *self.b.get(self.i)? {
                        b']' => {
                            self.i += 1;
                            return Some(Json::Other);
                        }
                        b',' => self.i += 1,
                        _ => {
                            self.value(depth + 1)?;
                        }
                    }
                }
            }
            b'"' => self.string().map(Json::Str),
            _ => {
                while self.i < self.b.len() && !matches!(self.b[self.i], b',' | b'}' | b']') && !self.b[self.i].is_ascii_whitespace() {
                    self.i += 1;
                }
                Some(Json::Other)
            }
        }
    }
}

impl ImportMap {
    pub fn parse(text: &str) -> ImportMap {
        let mut p = JsonParser { b: text.as_bytes(), i: 0 };
        let mut map = ImportMap::default();
        if let Some(Json::Obj(top)) = p.value(0) {
            for (k, v) in top {
                if k == "imports" {
                    if let Json::Obj(items) = v {
                        for (name, target) in items {
                            if let Json::Str(t) = target {
                                map.imports.push((name, t));
                            }
                        }
                    }
                }
            }
        }
        map
    }

    pub fn merge(&mut self, other: ImportMap) {
        for (k, v) in other.imports {
            if !self.imports.iter().any(|(ek, _)| *ek == k) {
                self.imports.push((k, v));
            }
        }
    }

    /// Resolves a specifier found in a module at `base`; bare specifiers with no mapping fail.
    pub fn resolve(&self, spec: &str, base: &str) -> Option<String> {
        if let Some((_, v)) = self.imports.iter().find(|(k, _)| k == spec) {
            return nav::resolve(base, v);
        }
        let prefix = self.imports.iter().filter(|(k, _)| k.ends_with('/') && spec.starts_with(k.as_str())).max_by_key(|(k, _)| k.len());
        if let Some((k, v)) = prefix {
            return nav::resolve(base, &format!("{v}{}", &spec[k.len()..]));
        }
        if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') || spec.contains("://") {
            return nav::resolve(base, spec);
        }
        None
    }
}

// ---- ES modules ----------------------------------------------------------

/// A static module statement found at statement start. `(a, b)` pairs are
/// `(imported_or_local, alias)`.
#[derive(Clone, Debug, PartialEq)]
pub enum ModStmt {
    Import { start: usize, end: usize, spec: String, default: Option<String>, named: Vec<(String, String)> },
    /// `named` is None for `export * from`.
    ExportFrom { start: usize, end: usize, spec: String, named: Option<Vec<(String, String)>> },
    ExportNamed { start: usize, end: usize, named: Vec<(String, String)> },
    /// Covers `export default`.
    ExportDefault { start: usize, end: usize },
    /// Covers `export` plus the declaration keyword.
    ExportDecl { start: usize, end: usize, keyword: String },
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) == Some(&b'/') && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else if b.get(i) == Some(&b'/') && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else {
            return i;
        }
    }
}

fn parse_names(inner: &str) -> Vec<(String, String)> {
    inner
        .split(',')
        .filter_map(|p| {
            let p = p.trim();
            if p.is_empty() {
                return None;
            }
            let mut w = p.split_whitespace();
            let a = w.next()?.to_owned();
            match (w.next(), w.next()) {
                (Some("as"), Some(b)) => Some((a, b.to_owned())),
                _ => Some((a.clone(), a)),
            }
        })
        .collect()
}

fn read_string(src: &str, i: usize) -> Option<(String, usize)> {
    let b = src.as_bytes();
    if !matches!(b.get(i), Some(b'"') | Some(b'\'')) {
        return None;
    }
    let end = skip_string(b, i);
    Some((src.get(i + 1..end.saturating_sub(1).max(i + 1))?.to_owned(), end))
}

fn after_semi(b: &[u8], i: usize) -> usize {
    let j = skip_ws(b, i);
    if b.get(j) == Some(&b';') {
        j + 1
    } else {
        i
    }
}

fn parse_import(src: &str, start: usize, p: usize) -> Option<ModStmt> {
    let b = src.as_bytes();
    let j = skip_ws(b, p);
    if matches!(b.get(j), Some(b'(') | Some(b'.') | None) {
        return None;
    }
    if let Some((spec, end)) = read_string(src, j) {
        return Some(ModStmt::Import { start, end: after_semi(b, end), spec, default: None, named: Vec::new() });
    }
    let mut k = j;
    while k < b.len() && !matches!(b[k], b'"' | b'\'') {
        if matches!(b[k], b';' | b'=' | b'(') || k - j > 4000 {
            return None;
        }
        k += 1;
    }
    let clause = src[j..k].trim_end();
    let clause = clause.strip_suffix("from")?.trim();
    let (spec, end) = read_string(src, k)?;
    let mut default = None;
    let mut named = Vec::new();
    let mut rest = clause;
    if !rest.starts_with('{') && !rest.starts_with('*') {
        let (d, r) = rest.split_once(',').unwrap_or((rest, ""));
        default = Some(d.trim().to_owned());
        rest = r.trim();
    }
    if let Some(r) = rest.strip_prefix('{') {
        named = parse_names(r.split('}').next().unwrap_or(""));
    } else if let Some(r) = rest.strip_prefix('*') {
        let name = r.trim().strip_prefix("as")?.trim();
        named.push(("*".to_owned(), name.to_owned()));
    }
    Some(ModStmt::Import { start, end: after_semi(b, end), spec, default, named })
}

fn parse_export(src: &str, start: usize, p: usize) -> Option<ModStmt> {
    let b = src.as_bytes();
    let j = skip_ws(b, p);
    let word_end = |from: usize| {
        let mut e = from;
        while e < b.len() && is_ident(b[e]) {
            e += 1;
        }
        e
    };
    match b.get(j)? {
        b'{' => {
            let close = j + src[j..].find('}')?;
            let named = parse_names(&src[j + 1..close]);
            let k = skip_ws(b, close + 1);
            if b.get(k..k + 4) == Some(&b"from"[..]) {
                let (spec, end) = read_string(src, skip_ws(b, k + 4))?;
                Some(ModStmt::ExportFrom { start, end: after_semi(b, end), spec, named: Some(named) })
            } else {
                Some(ModStmt::ExportNamed { start, end: after_semi(b, close + 1), named })
            }
        }
        b'*' => {
            let mut k = skip_ws(b, j + 1);
            if b.get(k..k + 2) == Some(&b"as"[..]) {
                k = skip_ws(b, word_end(skip_ws(b, k + 2)));
            }
            if b.get(k..k + 4) != Some(&b"from"[..]) {
                return None;
            }
            let (spec, end) = read_string(src, skip_ws(b, k + 4))?;
            Some(ModStmt::ExportFrom { start, end: after_semi(b, end), spec, named: None })
        }
        c if is_ident(*c) => {
            let e = word_end(j);
            let kw = &src[j..e];
            if kw == "default" {
                Some(ModStmt::ExportDefault { start, end: e })
            } else if matches!(kw, "const" | "let" | "var" | "function" | "class" | "async") {
                Some(ModStmt::ExportDecl { start, end: e, keyword: kw.to_owned() })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Finds static `import`/`export` statements (not `import()` or `import.meta`).
pub fn scan_module(src: &str) -> Vec<ModStmt> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    let mut at_start = true;
    while i < n {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < n && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i = src[i + 2..].find("*/").map_or(n, |p| i + 2 + p + 2);
        } else if matches!(c, b'"' | b'\'' | b'`') {
            i = skip_string(b, i);
            at_start = false;
        } else if matches!(c, b'\n' | b';' | b'{' | b'}') {
            at_start = true;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if is_ident(c) {
            let mut e = i;
            while e < n && is_ident(b[e]) {
                e += 1;
            }
            let word = &src[i..e];
            let stmt = if at_start && word == "import" {
                parse_import(src, i, e)
            } else if at_start && word == "export" {
                parse_export(src, i, e)
            } else {
                None
            };
            match stmt {
                Some(s) => {
                    i = match &s {
                        ModStmt::Import { end, .. }
                        | ModStmt::ExportFrom { end, .. }
                        | ModStmt::ExportNamed { end, .. }
                        | ModStmt::ExportDefault { end, .. }
                        | ModStmt::ExportDecl { end, .. } => *end,
                    };
                    out.push(s);
                    at_start = matches!(out.last(), Some(ModStmt::Import { .. } | ModStmt::ExportFrom { .. } | ModStmt::ExportNamed { .. }));
                }
                None => {
                    i = e;
                    at_start = false;
                }
            }
        } else {
            at_start = false;
            i += 1;
        }
    }
    out
}

/// Distinct static dependency specifiers in source order.
pub fn module_deps(src: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for s in scan_module(src) {
        let spec = match s {
            ModStmt::Import { spec, .. } | ModStmt::ExportFrom { spec, .. } => spec,
            _ => continue,
        };
        if !v.contains(&spec) {
            v.push(spec);
        }
    }
    v
}

/// Turns a module into a classic script so `JsPage::run_script` can run it:
/// import/export syntax is removed and bindings travel through the global
/// scope (`var`), the default export as `__def_<self_idx>`. Dependencies must
/// have run first. Namespace imports (`* as ns`) become an empty object.
pub fn rewrite_module(src: &str, self_idx: usize, dep: &dyn Fn(&str) -> Option<usize>) -> String {
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    let def = |i: usize| format!("__def_{i}");
    let alias = |orig: &str, name: &str, d: Option<usize>| -> String {
        if orig == "default" {
            match d {
                Some(d) if name != "default" => format!("var {name} = {};", def(d)),
                _ => String::new(),
            }
        } else if orig == "*" {
            format!("var {name} = {{}};")
        } else if orig != name {
            format!("var {name} = {orig};")
        } else {
            String::new()
        }
    };
    for s in scan_module(src) {
        let (start, end, rep) = match s {
            ModStmt::Import { start, end, spec, default, named } => {
                let d = dep(&spec);
                let mut r = String::new();
                if let (Some(name), Some(d)) = (default, d) {
                    r.push_str(&format!("var {name} = {};", def(d)));
                }
                for (o, n) in named {
                    r.push_str(&alias(&o, &n, d));
                }
                (start, end, r)
            }
            ModStmt::ExportFrom { start, end, spec, named } => {
                let d = dep(&spec);
                let mut r = String::new();
                for (o, n) in named.unwrap_or_default() {
                    if n == "default" {
                        if let Some(d) = d {
                            r.push_str(&format!("var {} = {};", def(self_idx), if o == "default" { def(d) } else { o.clone() }));
                        }
                    } else {
                        r.push_str(&alias(&o, &n, d));
                    }
                }
                (start, end, r)
            }
            ModStmt::ExportNamed { start, end, named } => {
                let mut r = String::new();
                for (o, n) in named {
                    if n == "default" {
                        r.push_str(&format!("var {} = {o};", def(self_idx)));
                    } else if o != n {
                        r.push_str(&format!("var {n} = {o};"));
                    }
                }
                (start, end, r)
            }
            ModStmt::ExportDefault { start, end } => (start, end, format!("var {} =", def(self_idx))),
            ModStmt::ExportDecl { start, end, keyword } => {
                let kw = if keyword == "const" || keyword == "let" { "var".to_owned() } else { keyword };
                (start, end, kw)
            }
        };
        out.push_str(&src[at..start]);
        out.push_str(&rep);
        at = end;
    }
    out.push_str(&src[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> MediaEnv {
        MediaEnv { width: 1200.0, height: 1920.0, dark: false }
    }

    #[test]
    fn charset_labels_and_bom() {
        assert_eq!(Charset::from_label("ISO-8859-1"), Some(Charset::Windows1252));
        assert_eq!(Charset::from_label(" \"UTF-8\" "), Some(Charset::Utf8));
        assert_eq!(Charset::from_label("klingon"), None);
        assert_eq!(sniff_bom(&[0xFF, 0xFE, 0x41, 0]), Some((Charset::Utf16Le, 2)));
        assert_eq!(sniff_bom(&[0xEF, 0xBB, 0xBF, 0x41]), Some((Charset::Utf8, 3)));
        assert_eq!(sniff_bom(b"abc"), None);
    }

    #[test]
    fn charset_detection_precedence() {
        assert_eq!(detect_charset(b"<meta charset=windows-1252>", Some("text/html; charset=utf-8")).0, Charset::Utf8);
        assert_eq!(detect_charset(b"<meta charset=windows-1252>", None).0, Charset::Windows1252);
        assert_eq!(detect_charset(b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=iso-8859-1\">", None).0, Charset::Windows1252);
        assert_eq!(detect_charset(&[0xFF, 0xFE, b'a', 0], Some("text/html; charset=utf-8")).0, Charset::Utf16Le);
        assert_eq!(detect_charset(b"<p>hi", None), (Charset::Utf8, false));
        // a meta naming utf-16 cannot be right for ASCII-readable bytes
        assert_eq!(detect_charset(b"<meta charset=utf-16>", None).0, Charset::Utf8);
    }

    #[test]
    fn decode_variants() {
        assert_eq!(decode(b"caf\xC3\xA9".to_vec(), Charset::Utf8, true), "caf\u{e9}");
        assert_eq!(decode(b"\xEF\xBB\xBFhi".to_vec(), Charset::Utf8, true), "hi");
        assert_eq!(decode(b"caf\xE9".to_vec(), Charset::Windows1252, true), "caf\u{e9}");
        assert_eq!(decode(b"\x80\x93".to_vec(), Charset::Windows1252, true), "\u{20AC}\u{201C}");
        // undeclared invalid utf-8 falls back to 1252, declared one is lossy
        assert_eq!(decode(b"caf\xE9".to_vec(), Charset::Utf8, false), "caf\u{e9}");
        assert_eq!(decode(b"caf\xE9".to_vec(), Charset::Utf8, true), "caf\u{FFFD}");
        assert_eq!(decode(vec![0xFF, 0xFE, b'h', 0, b'i', 0], Charset::Utf16Le, true), "hi");
        assert_eq!(decode(vec![0xFE, 0xFF, 0, b'h', 0, b'i'], Charset::Utf16Be, true), "hi");
        assert_eq!(decode(vec![0, b'h', 0, b'i'], Charset::Utf16Be, true), "hi");
    }

    #[test]
    fn decode_text_uses_content_type() {
        assert_eq!(decode_text(b"caf\xE9", Some("text/css; charset=iso-8859-1")), "caf\u{e9}");
        assert_eq!(decode_text(b"\xEF\xBB\xBFa{}", None), "a{}");
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v
    }

    #[test]
    fn image_sizes() {
        assert_eq!(image_size(&png(640, 480)), Some((640, 480)));
        assert_eq!(image_size(&[b'G', b'I', b'F', b'8', b'9', b'a', 10, 0, 20, 0]), Some((10, 20)));
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0, 50, 0, 70, 3];
        assert_eq!(image_size(&jpeg), Some((70, 50)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend_from_slice(&[10, 0, 0, 0, 0, 0, 0, 0]); // chunk size + flags
        webp.extend_from_slice(&[99, 0, 0, 49, 0, 0]); // w-1 = 99, h-1 = 49
        assert_eq!(image_size(&webp), Some((100, 50)));
        assert_eq!(image_size(b"<svg xmlns=\"x\" width=\"30\" height=\"40\"></svg>"), Some((30, 40)));
        assert_eq!(image_size(b"<svg viewBox=\"0 0 24 12\"></svg>"), Some((24, 12)));
        assert_eq!(image_size(b"hello"), None);
        assert_eq!(image_size(&png(0, 5)), None);
    }

    #[test]
    fn srcset_choice() {
        assert_eq!(pick_srcset("a.png 400w, b.png 900w, c.png 2000w").as_deref(), Some("b.png"));
        assert_eq!(pick_srcset("a.png 100w, b.png 200w").as_deref(), Some("b.png"));
        assert_eq!(pick_srcset("a.png 1x, b.png 2x, c.png 3x").as_deref(), Some("b.png"));
        assert_eq!(pick_srcset("only.png").as_deref(), Some("only.png"));
        assert_eq!(pick_srcset("   "), None);
    }

    #[test]
    fn media_attribute() {
        assert!(media_matches("", &env()));
        assert!(media_matches("all", &env()));
        assert!(media_matches("screen and (min-width: 600px)", &env()));
        assert!(!media_matches("print", &env()));
        assert!(!media_matches("(max-width: 500px)", &env()));
    }

    #[test]
    fn imports_extracted() {
        let css = "/* @import 'no.css'; */ @import url(a.css); @IMPORT url(\"b.css\") screen; @import 'c.css' layer(x);\nbody{}";
        let v = extract_imports(css);
        assert_eq!(
            v,
            vec![("a.css".into(), None), ("b.css".into(), Some("screen".into())), ("c.css".into(), None)]
        );
        assert!(extract_imports("a{content:'@import x'}").is_empty());
    }

    #[test]
    fn cookies_scope_and_delete() {
        let mut j = CookieJar::new();
        j.set_from_header("http://a.test/dir/page", "a=1");
        j.set_from_header("http://a.test/", "b=2; Path=/; HttpOnly");
        j.set_from_header("http://www.a.test/", "c=3; Domain=a.test; Path=/");
        assert_eq!(j.len(), 3);
        // a=1 has path /dir
        assert_eq!(j.header_for("http://a.test/dir/x").as_deref(), Some("a=1; b=2; c=3"));
        assert_eq!(j.header_for("http://a.test/other").as_deref(), Some("b=2; c=3"));
        assert_eq!(j.document_cookie("http://a.test/other"), "c=3");
        assert_eq!(j.header_for("http://other.test/"), None);
        j.set_from_header("http://a.test/", "b=; Max-Age=0; Path=/");
        assert_eq!(j.header_for("http://a.test/other").as_deref(), Some("c=3"));
        j.set_from_header("http://a.test/", "s=1; Secure; Path=/");
        assert_eq!(j.header_for("http://a.test/").as_deref(), Some("c=3"));
        assert_eq!(j.header_for("https://a.test/").as_deref(), Some("c=3; s=1"));
    }

    #[test]
    fn cors_rules() {
        let o = origin_of("http://a.test/p");
        assert_eq!(o, "http://a.test");
        assert!(cors_allows(&o, "http://a.test/x", &[]));
        assert!(!cors_allows(&o, "http://b.test/x", &[]));
        assert!(cors_allows(&o, "http://b.test/x", &[("Access-Control-Allow-Origin".into(), "*".into())]));
        assert!(cors_allows(&o, "http://b.test/x", &[("access-control-allow-origin".into(), "http://a.test".into())]));
        assert!(!cors_allows(&o, "http://b.test/x", &[("access-control-allow-origin".into(), "http://c.test".into())]));
    }

    #[test]
    fn import_map_resolution() {
        let m = ImportMap::parse(r#"{"imports": {"lib": "/vendor/lib.js", "u/": "https://cdn.test/u/", "x": ["ignored"]}, "scopes": {}}"#);
        assert_eq!(m.imports.len(), 2);
        let base = "http://a.test/app/main.js";
        assert_eq!(m.resolve("lib", base).as_deref(), Some("http://a.test/vendor/lib.js"));
        assert_eq!(m.resolve("u/z.js", base).as_deref(), Some("https://cdn.test/u/z.js"));
        assert_eq!(m.resolve("./rel.js", base).as_deref(), Some("http://a.test/app/rel.js"));
        assert_eq!(m.resolve("bare", base), None);
        assert!(ImportMap::parse("not json").imports.is_empty());
    }

    #[test]
    fn module_scan_and_rewrite() {
        let src = "import d, {a, b as c} from './dep.js';\nimport './side.js';\nconst m = await import('./dyn.js');\nexport const x = 1;\nexport default function f() {}\nexport { x as y };\nexport * from './re.js';\n// import 'no.js'\nconst s = \"import 'str.js'\";";
        assert_eq!(module_deps(src), vec!["./dep.js", "./side.js", "./re.js"]);
        let out = rewrite_module(src, 3, &|s| match s {
            "./dep.js" => Some(1),
            "./re.js" => Some(2),
            _ => None,
        });
        assert!(out.contains("var d = __def_1;"), "{out}");
        assert!(out.contains("var c = b;"), "{out}");
        assert!(out.contains("var x = 1;"), "{out}");
        assert!(out.contains("var __def_3 = function f() {}"), "{out}");
        assert!(out.contains("var y = x;"), "{out}");
        assert!(out.contains("import('./dyn.js')"), "{out}");
        assert!(!out.contains("export"), "{out}");
        assert!(!out.contains("import d"), "{out}");
    }

    #[test]
    fn budget_caps() {
        let lim = Limits { max_resources: 2, max_images: 1, max_total_bytes: 10, ..Limits::default() };
        let mut b = Budget::default();
        assert!(b.admit(ResourceKind::Image, &lim));
        assert!(!b.admit(ResourceKind::Image, &lim));
        assert!(b.admit(ResourceKind::Script, &lim));
        assert!(!b.admit(ResourceKind::Script, &lim));
        assert!(b.add_bytes(6, &lim));
        assert!(!b.add_bytes(5, &lim));
        assert_eq!(b.bytes, 6);
    }

    #[test]
    fn priorities_order() {
        assert!(ResourceKind::Stylesheet.priority() < ResourceKind::Script.priority());
        assert!(ResourceKind::Script.priority() < ResourceKind::Image.priority());
        assert!(ResourceKind::Image.priority() < ResourceKind::Font.priority());
    }
}
