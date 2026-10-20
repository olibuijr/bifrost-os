//! Rewriting a received message: Message > Remove Attachments keeps the text and drops the files. The parts that
//! [`crate::body::attachments`] would list are cut out of the MIME tree; everything else (headers, text and HTML
//! alternatives, nested multiparts) stays as it was.

use crate::body::{take_part, MAX_DEPTH, MAX_PARTS, boundary, content_type, file_name, is_file_part, split_head, split_parts};
use crate::message::parse_headers;

/// The result of cutting the files out of one message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stripped {
    /// The message without its files, with CRLF line ends.
    pub raw: String,
    /// How many files were cut out. Zero means `raw` is the message as it was.
    pub removed: usize,
}

/// The message text up to and including the blank line that ends its headers, and the rest.
fn cut(raw: &str) -> (&str, &str) {
    let (head, body) = split_head(raw);
    let head_len = head.len();
    // `split_head` drops the separator: it is whatever lies between the head and the body
    let separator = &raw[head_len..raw.len() - body.len()];
    (&raw[..head_len + separator.len()], body)
}

/// The message with every file part removed. Each file leaves a text note in its place that names it, so the reader still
/// shows what was there. A multipart left with no part gets one empty text part, so it stays valid.
pub fn strip_attachments(raw: &str) -> Stripped {
    let mut removed = 0;
    let text = match strip_entity(raw, &mut removed, 0, &mut { MAX_PARTS }) {
        Some(text) => text,
        // the whole message is one file: keep its headers and give it an empty text body
        None => {
            removed = 1;
            let (head, _) = cut(raw);
            let mut headers: Vec<&str> = head.lines().filter(|l| !l.is_empty() && !is_content_header(l)).collect();
            headers.push("Content-Type: text/plain; charset=UTF-8");
            headers.push("");
            format!("{}\r\n", headers.join("\r\n"))
        }
    };
    if removed == 0 {
        return Stripped { raw: raw.to_string(), removed: 0 };
    }
    Stripped { raw: text, removed }
}

/// True for a header line that belongs to the body's description (Content-* lines and their folded continuations are
/// all dropped when the body is replaced).
fn is_content_header(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("content-") || line.starts_with(' ') || line.starts_with('\t')
}

/// The text part that takes the place of a removed file: a plain note with the file's name.
fn removed_note(file: &str) -> String {
    let (head, _) = cut(file);
    let name = file_name(&parse_headers(head)).unwrap_or_else(|| "file".to_string());
    format!("Content-Type: text/plain; charset=UTF-8\r\n\r\n[Attachment removed: {name}]\r\n")
}

/// One entity (a message or a part) without its files. None when the entity is itself a file.
///
/// An entity nested deeper than `MAX_DEPTH`, or past the part budget, is treated as a file and cut out, so a hostile
/// tree is neither followed nor kept.
fn strip_entity(entity: &str, removed: &mut usize, depth: usize, budget: &mut usize) -> Option<String> {
    if depth > MAX_DEPTH || !take_part(budget) {
        *removed += 1;
        return None;
    }
    let (head, body) = cut(entity);
    let headers = parse_headers(head);
    let kind = content_type(&headers);
    if let (true, Some(delim)) = (kind.starts_with("multipart/"), boundary(&headers)) {
        let mut kept: Vec<String> = Vec::new();
        for part in split_parts(body, &delim) {
            match strip_entity(part, removed, depth + 1, budget) {
                Some(text) => kept.push(text),
                None => kept.push(removed_note(part)),
            }
        }
        if kept.is_empty() {
            kept.push("Content-Type: text/plain; charset=UTF-8\r\n\r\n".to_string());
        }
        let mut out = String::from(head);
        for part in kept {
            out.push_str(&format!("--{delim}\r\n{part}\r\n"));
        }
        out.push_str(&format!("--{delim}--\r\n"));
        return Some(out);
    }
    if is_file_part(&headers) {
        *removed += 1;
        return None;
    }
    Some(entity.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{attachments, readable};

    const MIXED: &str = concat!(
        "From: ana@example.com\r\nSubject: Skjal\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n",
        "preamble\r\n",
        "--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nSee the file\r\n",
        "--B\r\nContent-Type: application/pdf; name=\"skjal.pdf\"\r\nContent-Disposition: attachment; filename=\"skjal.pdf\"\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\naGk=\r\n",
        "--B--\r\n",
    );

    #[test]
    fn the_text_stays_and_the_file_goes() {
        let out = strip_attachments(MIXED);
        assert_eq!(out.removed, 1);
        assert!(attachments(&out.raw).is_empty());
        assert_eq!(readable(&out.raw), "See the file");
        assert!(out.raw.starts_with("From: ana@example.com\r\nSubject: Skjal\r\n"));
        assert!(!out.raw.contains("aGk="));
        assert!(out.raw.contains("[Attachment removed: skjal.pdf]"), "the file's place keeps a note with its name");
        assert!(out.raw.ends_with("--B--\r\n"));
    }

    #[test]
    fn a_message_without_files_is_returned_as_it_was() {
        let plain = "Subject: x\r\n\r\nhello\r\n";
        assert_eq!(strip_attachments(plain), Stripped { raw: plain.to_string(), removed: 0 });
        let alt = "Content-Type: multipart/alternative; boundary=\"A\"\r\n\r\n--A\r\nContent-Type: text/plain\r\n\r\nhi\r\n--A--\r\n";
        assert_eq!(strip_attachments(alt).removed, 0);
        assert_eq!(strip_attachments(alt).raw, alt);
    }

    #[test]
    fn nested_files_are_removed_and_the_alternatives_stay() {
        let raw = concat!(
            "Subject: n\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n",
            "--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n",
            "--inner\r\nContent-Type: text/plain\r\n\r\nplain text\r\n",
            "--inner\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n--inner--\r\n",
            "--outer\r\nContent-Type: image/png; name=a.png\r\nContent-Transfer-Encoding: base64\r\n\r\niVBO\r\n",
            "--outer\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=n.txt\r\n\r\nnotes\r\n",
            "--outer--\r\n",
        );
        let out = strip_attachments(raw);
        assert_eq!(out.removed, 2);
        assert!(attachments(&out.raw).is_empty());
        assert_eq!(readable(&out.raw), "plain text");
        assert!(out.raw.contains("<p>html</p>"));
        assert!(out.raw.contains("[Attachment removed: a.png]") && out.raw.contains("[Attachment removed: n.txt]"));
    }

    #[test]
    fn deeply_nested_multiparts_are_cut_off_not_followed() {
        let mut raw = String::new();
        for i in 0..5000 {
            raw.push_str(&format!("Content-Type: multipart/mixed; boundary=b{i}\r\n\r\n--b{i}\r\n"));
        }
        raw.push_str("Content-Type: text/plain\r\n\r\ndeep\r\n");
        for i in (0..5000).rev() {
            raw.push_str(&format!("--b{i}--\r\n"));
        }
        assert_eq!(readable(&raw), "");
        assert!(attachments(&raw).is_empty());
        let out = strip_attachments(&raw);
        assert!(out.removed >= 1);
    }

    #[test]
    fn a_multipart_left_empty_keeps_one_empty_text_part() {
        let raw = "Content-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain; name=a.txt\r\n\r\nx\r\n--B--\r\n";
        let out = strip_attachments(raw);
        assert_eq!(out.removed, 1);
        assert!(out.raw.contains("--B\r\nContent-Type: text/plain; charset=UTF-8\r\n"));
        assert!(attachments(&out.raw).is_empty());
    }

    #[test]
    fn a_message_that_is_one_file_becomes_an_empty_text_message() {
        let raw = "Subject: pic\r\nContent-Type: image/png; name=a.png\r\nContent-Transfer-Encoding: base64\r\n\r\niVBO\r\n";
        let out = strip_attachments(raw);
        assert_eq!(out.removed, 1);
        assert_eq!(out.raw, "Subject: pic\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n");
    }
}
