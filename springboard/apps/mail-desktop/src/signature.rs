//! Signatures: one per account, appended to a new message after the standard `-- ` delimiter line, and placed above
//! the quoted text in a reply. Pure functions over the compose body.
#![allow(dead_code)] // wired by the compose and mailbox UI in the next step

/// The delimiter line before a signature: two hyphens and a space.
pub const DELIMITER: &str = "-- ";

/// The body with `signature` appended after a delimiter. Empty signatures change nothing; a body that already
/// carries a signature is returned unchanged.
pub fn append(body: &str, signature: &str) -> String {
    let signature = signature.trim_end();
    if signature.trim().is_empty() || has_signature(body) {
        return body.to_string();
    }
    let body = body.trim_end_matches('\n');
    if body.is_empty() {
        format!("\n\n{DELIMITER}\n{signature}")
    } else {
        format!("{body}\n\n{DELIMITER}\n{signature}")
    }
}

/// The body for a reply: the signature goes between the cursor area (an empty line at the top) and the quoted
/// text, so typing happens above both. `quoted` is the already-quoted original.
pub fn reply_body(quoted: &str, signature: &str) -> String {
    let signature = signature.trim_end();
    if signature.trim().is_empty() {
        return format!("\n\n{quoted}");
    }
    format!("\n\n{DELIMITER}\n{signature}\n\n{quoted}")
}

/// True when a line of the body is exactly the delimiter (a trailing-space-less `--` counts, as many editors strip it).
pub fn has_signature(body: &str) -> bool {
    body.lines().any(|l| l == DELIMITER || l == "--")
}

/// The body without its signature (everything from the last delimiter line on).
pub fn strip(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    match lines.iter().rposition(|l| *l == DELIMITER || *l == "--") {
        Some(i) => lines[..i].join("\n").trim_end_matches('\n').to_string(),
        None => body.to_string(),
    }
}

/// The signature text of a body, if it has one.
pub fn extract(body: &str) -> Option<String> {
    let lines: Vec<&str> = body.lines().collect();
    let i = lines.iter().rposition(|l| *l == DELIMITER || *l == "--")?;
    Some(lines[i + 1..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_after_the_delimiter() {
        assert_eq!(append("Hello", "Ann\nShop"), "Hello\n\n-- \nAnn\nShop");
        assert_eq!(append("", "Ann"), "\n\n-- \nAnn");
        assert_eq!(append("Hello\n\n", "Ann\n\n"), "Hello\n\n-- \nAnn");
    }

    #[test]
    fn empty_or_repeated_signature_changes_nothing() {
        assert_eq!(append("Hello", "  \n"), "Hello");
        let once = append("Hello", "Ann");
        assert_eq!(append(&once, "Bob"), once);
    }

    #[test]
    fn reply_puts_signature_above_the_quote() {
        assert_eq!(reply_body("> hi", "Ann"), "\n\n-- \nAnn\n\n> hi");
        assert_eq!(reply_body("> hi", ""), "\n\n> hi");
    }

    #[test]
    fn strip_and_extract() {
        let body = append("Hello", "Ann\nShop");
        assert_eq!(strip(&body), "Hello");
        assert_eq!(extract(&body).as_deref(), Some("Ann\nShop"));
        assert_eq!(strip("no signature"), "no signature");
        assert_eq!(extract("no signature"), None);
    }
}
