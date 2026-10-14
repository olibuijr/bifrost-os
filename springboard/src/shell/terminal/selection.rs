//! Actions on the selected text: web search and man page lookup.

/// The search page URL for `text`, percent-encoded; None when the text is blank. Long selections are cut to 200 characters.
pub fn web_search_url(text: &str) -> Option<String> {
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
    if text.is_empty() {
        return None;
    }
    let mut q = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => q.push(b as char),
            b' ' => q.push('+'),
            b => q.push_str(&format!("%{b:02X}")),
        }
    }
    Some(format!("https://duckduckgo.com/?q={q}"))
}

/// The man page argv for a selected word such as `ls`, `printf(3)` or `2 open`; None when it is not a page name.
pub fn man_argv(text: &str) -> Option<Vec<String>> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let name_ok = |w: &str| !w.is_empty() && !w.starts_with('-') && w.chars().all(|c| c.is_alphanumeric() || "._+-:".contains(c));
    let section_ok = |w: &str| !w.is_empty() && w.len() <= 3 && w.chars().next().is_some_and(|c| c.is_ascii_digit());
    match words.as_slice() {
        [one] => {
            if let Some((name, rest)) = one.split_once('(') {
                let section = rest.strip_suffix(')')?;
                return (name_ok(name) && section_ok(section)).then(|| vec!["man".into(), section.into(), name.into()]);
            }
            name_ok(one).then(|| vec!["man".into(), (*one).into()])
        }
        [section, name] if section_ok(section) && name_ok(name) => Some(vec!["man".into(), (*section).into(), (*name).into()]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_url_encodes_and_trims() {
        assert_eq!(web_search_url("  a b&c\n").as_deref(), Some("https://duckduckgo.com/?q=a+b%26c"));
        assert_eq!(web_search_url(" \n "), None);
        assert!(web_search_url(&"x".repeat(500)).unwrap().len() < 260);
    }

    #[test]
    fn man_argv_reads_names_and_sections() {
        assert_eq!(man_argv("ls"), Some(vec!["man".into(), "ls".into()]));
        assert_eq!(man_argv("printf(3)"), Some(vec!["man".into(), "3".into(), "printf".into()]));
        assert_eq!(man_argv("2 open"), Some(vec!["man".into(), "2".into(), "open".into()]));
        assert_eq!(man_argv("-k x"), None);
        assert_eq!(man_argv("rm -rf /"), None);
        assert_eq!(man_argv("a;b"), None);
        assert_eq!(man_argv(""), None);
    }
}
