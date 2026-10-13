//! Signatures (Mailbox > Edit Signature). Each account has one signature, added to the end of a message when it is sent,
//! after the usual `-- ` line. They are kept in the app's data directory, one account a line: the account id, a tab, and
//! the signature with `\` written `\\` and line breaks written `\n`.

use std::collections::HashMap;
use std::path::Path;

/// File name of the signatures in the app's data directory.
pub const FILE: &str = "signatures.txt";

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\\n")
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

pub fn parse(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (id, signature) = line.split_once('\t')?;
            (!id.is_empty()).then(|| (id.to_string(), unescape(signature)))
        })
        .collect()
}

pub fn serialize(signatures: &HashMap<String, String>) -> String {
    let mut ids: Vec<&String> = signatures.keys().filter(|id| !id.contains(['\t', '\r', '\n'])).collect();
    ids.sort();
    ids.into_iter().filter(|id| !signatures[*id].trim().is_empty()).map(|id| format!("{id}\t{}\n", escape(&signatures[id]))).collect()
}

pub fn load(path: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(path: &Path, signatures: &HashMap<String, String>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serialize(signatures)).map_err(|e| e.to_string())
}

/// `body` with the signature after a `-- ` line. A blank signature leaves the body as it is.
pub fn append(body: &str, signature: &str) -> String {
    let signature = signature.trim_end();
    if signature.trim().is_empty() {
        return body.to_string();
    }
    format!("{}\n\n-- \n{signature}", body.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_follows_the_delimiter_line() {
        assert_eq!(append("Hello\n", "Ana\nSales"), "Hello\n\n-- \nAna\nSales");
        assert_eq!(append("Hello", "  \n"), "Hello");
        assert_eq!(append("", "Ana"), "\n\n-- \nAna");
    }

    #[test]
    fn signatures_with_line_breaks_and_backslashes_round_trip() {
        let mut all = HashMap::new();
        all.insert("ana-example-com".to_string(), "Ana\\Sales\nPhone 5".to_string());
        all.insert("empty".to_string(), "  ".to_string());
        let back = parse(&serialize(&all));
        assert_eq!(back.len(), 1, "a blank signature is not kept");
        assert_eq!(back["ana-example-com"], "Ana\\Sales\nPhone 5");
    }
}
