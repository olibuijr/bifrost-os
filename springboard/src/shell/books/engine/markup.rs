//! A forgiving tag tokenizer for the XML-ish files inside an EPUB (XHTML,
//! OPF, NCX): good enough for well-formed books and tolerant of the sloppy
//! ones. It does not build a tree; callers walk the token list.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    Open { name: String, attrs: Vec<(String, String)>, selfclose: bool },
    Close(String),
    Text(String),
}

impl Tok {
    pub fn attr(&self, key: &str) -> Option<&str> {
        match self {
            Tok::Open { attrs, .. } => attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()),
            _ => None,
        }
    }
}

/// The part of a tag name after any `prefix:`.
pub fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, l)| l)
}

/// Decodes the named and numeric entities books actually use.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| entity(&rest[1..e]));
        match (end, decoded) {
            (Some(e), Some(c)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "deg" => '°',
        "middot" => '·',
        "bull" => '•',
        "laquo" => '«',
        "raquo" => '»',
        "eacute" => 'é',
        "egrave" => 'è',
        "aacute" => 'á',
        "agrave" => 'à',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "auml" => 'ä',
        "ntilde" => 'ñ',
        "ccedil" => 'ç',
        "thorn" => 'þ',
        "eth" => 'ð',
        "aelig" => 'æ',
        _ => return None,
    })
}

/// Splits `src` into tags and text. Comments, processing instructions and
/// doctypes vanish; CDATA becomes text; `<script>` and `<style>` bodies are
/// dropped.
pub fn tokens(src: &str) -> Vec<Tok> {
    let mut out = vec![];
    let mut rest = src;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            out.push(Tok::Text(decode_entities(rest)));
            break;
        };
        if lt > 0 {
            out.push(Tok::Text(decode_entities(&rest[..lt])));
        }
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map_or("", |e| &r[e + 3..]);
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let e = r.find("]]>").unwrap_or(r.len());
            out.push(Tok::Text(r[..e].to_string()));
            rest = r.get(e + 3..).unwrap_or("");
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
        } else {
            let Some(gt) = tag_end(rest) else { break };
            let body = &rest[1..gt];
            rest = &rest[gt + 1..];
            if let Some(name) = body.strip_prefix('/') {
                out.push(Tok::Close(name.trim().to_ascii_lowercase()));
                continue;
            }
            let selfclose = body.ends_with('/');
            let body = body.trim_end_matches('/');
            let (name, attrs) = parse_tag(body);
            let raw_text = !selfclose && (name == "script" || name == "style");
            out.push(Tok::Open { name: name.clone(), attrs, selfclose });
            if raw_text {
                let close = format!("</{name}");
                let lower = rest.to_ascii_lowercase();
                let at = lower.find(&close).unwrap_or(rest.len());
                rest = &rest[at..];
            }
        }
    }
    out
}

/// Index of the `>` that ends the tag at the start of `s`, skipping quoted values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_tag(body: &str) -> (String, Vec<(String, String)>) {
    let body = body.trim();
    let name_end = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
    let name = body[..name_end].to_ascii_lowercase();
    let mut attrs = vec![];
    let mut rest = body[name_end..].trim_start();
    while !rest.is_empty() {
        let key_end = rest.find(|c: char| c == '=' || c.is_whitespace()).unwrap_or(rest.len());
        let key = rest[..key_end].to_ascii_lowercase();
        rest = rest[key_end..].trim_start();
        let mut value = String::new();
        if let Some(r) = rest.strip_prefix('=') {
            let r = r.trim_start();
            if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let inner = &r[1..];
                let e = inner.find(q).unwrap_or(inner.len());
                value = decode_entities(&inner[..e]);
                rest = inner.get(e + 1..).unwrap_or("").trim_start();
            } else {
                let e = r.find(|c: char| c.is_whitespace()).unwrap_or(r.len());
                value = decode_entities(&r[..e]);
                rest = r[e..].trim_start();
            }
        }
        if !key.is_empty() {
            attrs.push((key, value));
        }
    }
    (name, attrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_named_numeric_and_unknown() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &#65;&#x42; &bogus; & x"), "a & b <c> AB &bogus; & x");
        assert_eq!(decode_entities("it&rsquo;s&nbsp;ok"), "it’s\u{a0}ok");
    }

    #[test]
    fn tags_attributes_comments_and_raw_text() {
        let t = tokens("<?xml version='1.0'?><!DOCTYPE html><p class=\"a b\" id='x'>Hi <b>there</b><!-- c --><br/></p><script>if (a<b) {}</script>tail");
        assert_eq!(t[0], Tok::Open { name: "p".into(), attrs: vec![("class".into(), "a b".into()), ("id".into(), "x".into())], selfclose: false });
        assert_eq!(t[1], Tok::Text("Hi ".into()));
        assert_eq!(t[3], Tok::Text("there".into()));
        assert!(t.contains(&Tok::Open { name: "br".into(), attrs: vec![], selfclose: true }));
        assert!(t.contains(&Tok::Close("script".into())));
        assert!(!t.iter().any(|x| matches!(x, Tok::Text(s) if s.contains("a<b"))));
        assert_eq!(t.last(), Some(&Tok::Text("tail".into())));
    }

    #[test]
    fn quoted_gt_does_not_end_a_tag() {
        let t = tokens("<a title=\"x>y\" href=z>t</a>");
        assert_eq!(t[0].attr("title"), Some("x>y"));
        assert_eq!(t[0].attr("href"), Some("z"));
    }

    #[test]
    fn local_names_drop_prefixes() {
        assert_eq!(local("dc:title"), "title");
        assert_eq!(local("p"), "p");
    }
}
