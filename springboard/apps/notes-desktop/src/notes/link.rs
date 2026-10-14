//! Edit > Add Link: wraps the selected text (or the address itself) as a Markdown link, `[text](address)`.

/// The address as it is stored: trimmed, with `https://` in front when it names no scheme. None for an empty address
/// or one with spaces or parentheses, which would end the Markdown link early.
pub fn normalize(address: &str) -> Option<String> {
    let a = address.trim();
    if a.is_empty() || a.chars().any(|c| c.is_whitespace() || c == '(' || c == ')') {
        return None;
    }
    let has_scheme = a.split_once(':').is_some_and(|(s, rest)| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') && (rest.starts_with("//") || s == "mailto" || s == "tel"));
    Some(if has_scheme { a.to_string() } else { format!("https://{a}") })
}

/// The Markdown for a link. An empty `text` shows the address.
pub fn markdown(text: &str, address: &str) -> Option<String> {
    let url = normalize(address)?;
    let text: String = text.chars().filter(|c| *c != '\n' && *c != '[' && *c != ']').collect();
    let text = if text.trim().is_empty() { url.clone() } else { text };
    Some(format!("[{text}]({url})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_address_gets_a_scheme() {
        assert_eq!(normalize("example.com/a").as_deref(), Some("https://example.com/a"));
        assert_eq!(normalize(" http://x.org ").as_deref(), Some("http://x.org"));
        assert_eq!(normalize("mailto:a@b.c").as_deref(), Some("mailto:a@b.c"));
        assert_eq!(normalize("localhost:8080").as_deref(), Some("https://localhost:8080"));
    }

    #[test]
    fn addresses_that_would_break_the_link_are_refused() {
        for a in ["", "   ", "a b", "x(y)", "x)"] {
            assert_eq!(normalize(a), None, "{a:?}");
        }
    }

    #[test]
    fn the_selection_becomes_the_link_text() {
        assert_eq!(markdown("docs", "example.com").as_deref(), Some("[docs](https://example.com)"));
        assert_eq!(markdown("", "example.com").as_deref(), Some("[https://example.com](https://example.com)"));
        assert_eq!(markdown("a [b]\nc", "x.org").as_deref(), Some("[a bc](https://x.org)"));
        assert_eq!(markdown("t", "no way"), None);
    }
}
