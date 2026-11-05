//! Links: text can point at an address (Edit > Add Link), and the file formats carry it (an RTF hyperlink field, an HTML
//! anchor, a docx hyperlink, an OpenDocument anchor). A character holds a small number (`Style::link`) and the table
//! here turns it back into the address, so the many cells of a link share one string and reading a file needs no
//! table of its own. Numbers are never reused, so a link survives undo and redo.

use std::sync::Mutex;

/// The addresses seen so far; a link number is the position plus one (0 is no link).
static TABLE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The longest address a link may have.
pub const MAX_LEN: usize = 2048;

/// The number for an address, adding it to the table the first time. 0 when the table is full.
pub fn intern(url: &str) -> u16 {
    let mut table = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(at) = table.iter().position(|u| u == url) {
        return (at + 1) as u16;
    }
    if table.len() >= u16::MAX as usize - 1 {
        return 0;
    }
    table.push(url.to_string());
    table.len() as u16
}

/// The address a link number stands for.
pub fn url(id: u16) -> Option<String> {
    let at = (id as usize).checked_sub(1)?;
    TABLE.lock().unwrap_or_else(|e| e.into_inner()).get(at).cloned()
}

/// The address to store for what the user typed or a file named, or `None` when it is not one TextEdit links to.
/// Web, file transfer, mail and phone addresses are accepted; a bare host name becomes a web address and a bare mail
/// address a mail link. Schemes that run code or read local files, blanks and characters that break the file formats are
/// refused.
pub fn normalize(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() || text.len() > MAX_LEN {
        return None;
    }
    if text.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '\\' | '{' | '}')) {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    for scheme in ["http://", "https://", "ftp://"] {
        if lower.starts_with(scheme) {
            return (text.len() > scheme.len()).then(|| text.to_string());
        }
    }
    for scheme in ["mailto:", "tel:"] {
        if lower.starts_with(scheme) {
            return (text.len() > scheme.len()).then(|| text.to_string());
        }
    }
    // any other scheme with an authority, or a scheme that runs code or reads local data, is refused
    if lower.contains("://") || ["javascript:", "data:", "file:", "vbscript:", "about:"].iter().any(|s| lower.starts_with(s)) {
        return None;
    }
    if text.contains('@') && !text.contains('/') {
        return Some(format!("mailto:{text}"));
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or("");
    let host_like = (host.contains('.') && !host.starts_with('.') && !host.ends_with('.')) || host.starts_with("localhost");
    host_like.then(|| format!("https://{text}"))
}

/// Smart Links: the address in a word the user has just finished typing. Returns where the address starts in the word and
/// how long it is (in characters, so quotes or brackets around it and punctuation after it are left out), with the
/// address to link to. Web addresses (`http://`, `https://`, `ftp://`, `www.`), `mailto:` addresses and mail addresses
/// are recognised; a bare word like `example.com` is not, since it may be a file name or a sentence end.
pub fn smart_link(word: &str) -> Option<(usize, usize, String)> {
    let chars: Vec<char> = word.chars().collect();
    let opening = |c: char| matches!(c, '(' | '[' | '{' | '<' | '"' | '\'');
    let closing = |c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>' | '"' | '\'');
    let mut start = 0;
    while start < chars.len() && opening(chars[start]) {
        start += 1;
    }
    let mut end = chars.len();
    while end > start && closing(chars[end - 1]) {
        end -= 1;
    }
    if end <= start {
        return None;
    }
    let candidate: String = chars[start..end].iter().collect();
    let lower = candidate.to_ascii_lowercase();
    let address = if ["http://", "https://", "ftp://", "mailto:", "www."].iter().any(|p| lower.starts_with(p)) {
        normalize(&candidate)?
    } else if is_mail_address(&candidate) {
        normalize(&candidate)?
    } else {
        return None;
    };
    Some((start, end - start, address))
}

/// Whether text is one mail address: a name, an at sign and a dotted host.
fn is_mail_address(text: &str) -> bool {
    let Some((name, host)) = text.split_once('@') else { return false };
    !name.is_empty()
        && !host.contains('@')
        && !text.contains('/')
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
}

/// The address of an RTF hyperlink field instruction (`HYPERLINK "address"`), when it is one TextEdit links to.
pub fn from_field_instruction(instruction: &str) -> Option<String> {
    let text = instruction.trim();
    if !text.get(..9)?.eq_ignore_ascii_case("hyperlink") {
        return None;
    }
    let rest = text[9..].trim_start();
    let address = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => rest.split_whitespace().next()?,
    };
    normalize(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_keeps_one_number() {
        let a = intern("https://example.com/a");
        let b = intern("https://example.com/b");
        assert!(a != 0 && b != 0 && a != b);
        assert_eq!(intern("https://example.com/a"), a, "the same address is the same number");
        assert_eq!(url(a).as_deref(), Some("https://example.com/a"));
        assert_eq!(url(0), None, "0 is no link");
        assert_eq!(url(u16::MAX), None);
    }

    #[test]
    fn web_mail_and_phone_addresses_are_accepted_as_written() {
        assert_eq!(normalize(" https://example.com/x?y=1#z ").as_deref(), Some("https://example.com/x?y=1#z"));
        assert_eq!(normalize("HTTP://Example.com").as_deref(), Some("HTTP://Example.com"));
        assert_eq!(normalize("ftp://files.example.com/a").as_deref(), Some("ftp://files.example.com/a"));
        assert_eq!(normalize("mailto:ada@example.com").as_deref(), Some("mailto:ada@example.com"));
        assert_eq!(normalize("tel:+3545551234").as_deref(), Some("tel:+3545551234"));
    }

    #[test]
    fn a_bare_host_or_mail_address_gets_its_scheme() {
        assert_eq!(normalize("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(normalize("example.com:8080/path").as_deref(), Some("https://example.com:8080/path"));
        assert_eq!(normalize("localhost:3000").as_deref(), Some("https://localhost:3000"));
        assert_eq!(normalize("ada@example.com").as_deref(), Some("mailto:ada@example.com"));
    }

    #[test]
    fn unsafe_or_meaningless_addresses_are_refused() {
        for bad in [
            "",
            "   ",
            "hello",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,x",
            "gopher://x.example",
            "https://",
            "two words.com",
            "https://a.example/\"quoted\"",
            "https://a.example/<b>",
            "https://a.example/{x}",
            "https://a.example/\\x",
            ".com",
        ] {
            assert_eq!(normalize(bad), None, "{bad:?}");
        }
        assert_eq!(normalize(&format!("https://a.example/{}", "x".repeat(MAX_LEN))), None, "too long");
    }

    #[test]
    fn smart_links_find_the_address_in_a_finished_word() {
        let found = smart_link;
        assert_eq!(found("https://example.com/a"), Some((0, 21, "https://example.com/a".to_string())));
        assert_eq!(found("www.example.com"), Some((0, 15, "https://www.example.com".to_string())));
        assert_eq!(found("ada@example.com"), Some((0, 15, "mailto:ada@example.com".to_string())));
        assert_eq!(found("mailto:ada@example.com"), Some((0, 22, "mailto:ada@example.com".to_string())));
        // punctuation around the address is not part of it
        assert_eq!(found("(https://example.com/a)."), Some((1, 21, "https://example.com/a".to_string())));
        assert_eq!(found("\"www.example.com\","), Some((1, 15, "https://www.example.com".to_string())));
        // a bare host, a plain word and a half-typed address are left alone
        for word in ["example.com", "hello", "www.", "https://", "a@b", "@example.com", "ada@@example.com", "ada@example.", "", "()"] {
            assert_eq!(found(word), None, "{word:?}");
        }
        assert_eq!(found("javascript:alert(1)"), None);
    }

    #[test]
    fn a_field_instruction_names_its_address() {
        assert_eq!(from_field_instruction(" HYPERLINK \"https://example.com/a\" ").as_deref(), Some("https://example.com/a"));
        assert_eq!(from_field_instruction("hyperlink example.org").as_deref(), Some("https://example.org"));
        assert_eq!(from_field_instruction("PAGE"), None);
        assert_eq!(from_field_instruction("HYPERLINK \"javascript:x\""), None);
        assert_eq!(from_field_instruction("HYPERLINK"), None);
    }
}
