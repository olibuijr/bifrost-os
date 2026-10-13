//! A small XML reader for the office document formats: the markup becomes a flat list of tokens (open and close tags with
//! their attributes, and text with its entities decoded), and `escape` writes text back. It is not a validating parser:
//! comments, processing instructions and doctype lines are skipped, CDATA is text, and an unclosed tag ends the input.

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// An opening tag; `empty` is true for `<a/>`, which has no close token.
    Open { name: String, attrs: Vec<(String, String)>, empty: bool },
    Close(String),
    Text(String),
}

/// The value of attribute `key` (with its prefix, as written).
pub fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Text for element content or an attribute value: `&`, `<`, `>` and `"` as entities.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out += "&amp;",
            '<' => out += "&lt;",
            '>' => out += "&gt;",
            '"' => out += "&quot;",
            c => out.push(c),
        }
    }
    out
}

/// Decodes the five named entities and numeric references; an unknown entity stays as written.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match tail.find(';').filter(|&end| end <= 10) {
            Some(end) => {
                let name = &tail[1..end];
                let decoded = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => name
                        .strip_prefix("#x")
                        .or_else(|| name.strip_prefix("#X"))
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                        .and_then(char::from_u32),
                };
                match decoded {
                    Some(c) => {
                        out.push(c);
                        rest = &tail[end + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &tail[1..];
                    }
                }
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

fn starts(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, p)| chars.get(at + k) == Some(&p))
}

/// The index of the first `pat` at or after `from`, or the length when there is none.
fn find(chars: &[char], from: usize, pat: &str) -> usize {
    let mut i = from;
    while i < chars.len() {
        if starts(chars, i, pat) {
            return i;
        }
        i += 1;
    }
    chars.len()
}

fn skip_past(chars: &[char], from: usize, pat: &str) -> usize {
    (find(chars, from, pat) + pat.chars().count()).min(chars.len())
}

/// The tokens of an XML document.
pub fn tokens(xml: &str) -> Vec<Token> {
    let chars: Vec<char> = xml.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < n {
        if chars[i] != '<' {
            text.push(chars[i]);
            i += 1;
            continue;
        }
        if !text.is_empty() {
            out.push(Token::Text(unescape(&text)));
            text.clear();
        }
        if starts(&chars, i, "<?") {
            i = skip_past(&chars, i, "?>");
        } else if starts(&chars, i, "<!--") {
            i = skip_past(&chars, i, "-->");
        } else if starts(&chars, i, "<![CDATA[") {
            let end = find(&chars, i + 9, "]]>");
            out.push(Token::Text(chars[i + 9..end.max(i + 9)].iter().collect()));
            i = (end + 3).min(n);
        } else if starts(&chars, i, "<!") {
            i = skip_past(&chars, i, ">");
        } else if starts(&chars, i, "</") {
            let end = find(&chars, i, ">");
            let name: String = chars[i + 2..end.max(i + 2)].iter().collect();
            out.push(Token::Close(name.trim().to_string()));
            i = (end + 1).min(n);
        } else {
            i = open_tag(&chars, i, &mut out);
        }
    }
    if !text.is_empty() {
        out.push(Token::Text(unescape(&text)));
    }
    out
}

/// Reads the opening tag at `at` (a `<`), pushes its token, and returns the index after it.
fn open_tag(chars: &[char], at: usize, out: &mut Vec<Token>) -> usize {
    let n = chars.len();
    let mut j = at + 1;
    let mut name = String::new();
    while j < n && !chars[j].is_whitespace() && chars[j] != '/' && chars[j] != '>' {
        name.push(chars[j]);
        j += 1;
    }
    let mut attrs = Vec::new();
    let mut empty = false;
    while j < n {
        while j < n && chars[j].is_whitespace() {
            j += 1;
        }
        if j >= n {
            break;
        }
        match chars[j] {
            '>' => {
                j += 1;
                break;
            }
            '/' => {
                empty = true;
                j += 1;
            }
            _ => {
                let mut key = String::new();
                while j < n && !chars[j].is_whitespace() && !matches!(chars[j], '=' | '>' | '/') {
                    key.push(chars[j]);
                    j += 1;
                }
                while j < n && chars[j].is_whitespace() {
                    j += 1;
                }
                let mut value = String::new();
                if j < n && chars[j] == '=' {
                    j += 1;
                    while j < n && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if j < n && (chars[j] == '"' || chars[j] == '\'') {
                        let quote = chars[j];
                        j += 1;
                        while j < n && chars[j] != quote {
                            value.push(chars[j]);
                            j += 1;
                        }
                        j = (j + 1).min(n);
                    }
                }
                if key.is_empty() {
                    // a stray character: step over it so the loop always advances
                    j += 1;
                } else {
                    attrs.push((key, unescape(&value)));
                }
            }
        }
    }
    out.push(Token::Open { name, attrs, empty });
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_attributes_text_and_entities_become_tokens() {
        let t = tokens("<?xml version=\"1.0\"?><a x=\"1 &amp; 2\" y='q'><b/>t&lt;&#65;&#x42;</a>");
        assert_eq!(
            t,
            vec![
                Token::Open { name: "a".into(), attrs: vec![("x".into(), "1 & 2".into()), ("y".into(), "q".into())], empty: false },
                Token::Open { name: "b".into(), attrs: vec![], empty: true },
                Token::Text("t<AB".into()),
                Token::Close("a".into()),
            ]
        );
    }

    #[test]
    fn comments_doctype_and_cdata_are_handled() {
        let t = tokens("<!DOCTYPE a><a><!-- note --><![CDATA[1 < 2]]></a>");
        assert_eq!(t[1], Token::Text("1 < 2".into()));
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn escaping_round_trips_and_an_unknown_entity_stays() {
        assert_eq!(unescape(&escape("a<b>&\"c\"")), "a<b>&\"c\"");
        assert_eq!(unescape("fish & chips &nosuch; &"), "fish & chips &nosuch; &");
    }

    #[test]
    fn a_truncated_document_ends_without_panicking() {
        let t = tokens("<a href=\"x");
        assert_eq!(t.len(), 1);
        assert_eq!(tokens("<a><b").len(), 2);
        assert!(tokens("</").len() <= 1);
    }

    #[test]
    fn attr_looks_up_by_prefixed_name() {
        let t = tokens("<w:b w:val=\"0\"/>");
        let Token::Open { attrs, .. } = &t[0] else { panic!("an open tag") };
        assert_eq!(attr(attrs, "w:val"), Some("0"));
        assert_eq!(attr(attrs, "val"), None);
    }
}
