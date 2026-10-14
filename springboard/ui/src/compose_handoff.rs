//! Handing text and files to a mail compose window: building and reading `mailto:` URLs (recipients, cc, bcc,
//! subject, body) with percent-encoding, and the request a share sheet sends to a mail app (recipients, subject,
//! body and attachment paths). Pure std.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compose {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// Absolute paths of files to attach (never part of a `mailto:` URL).
    pub attachments: Vec<String>,
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 0 {
            let (h, l) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16));
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn valid_address(a: &str) -> bool {
    let a = a.trim();
    match a.split_once('@') {
        Some((l, d)) => !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.') && !a.contains(char::is_whitespace),
        None => false,
    }
}

impl Compose {
    /// The `mailto:` URL. Recipients are joined by commas in the path; the rest are query fields.
    pub fn to_url(&self) -> String {
        let mut url = format!("mailto:{}", self.to.iter().map(|a| encode(a).replace("%40", "@")).collect::<Vec<_>>().join(","));
        let mut q: Vec<String> = vec![];
        if !self.cc.is_empty() {
            q.push(format!("cc={}", self.cc.iter().map(|a| encode(a).replace("%40", "@")).collect::<Vec<_>>().join(",")));
        }
        if !self.bcc.is_empty() {
            q.push(format!("bcc={}", self.bcc.iter().map(|a| encode(a).replace("%40", "@")).collect::<Vec<_>>().join(",")));
        }
        if !self.subject.is_empty() {
            q.push(format!("subject={}", encode(&self.subject)));
        }
        if !self.body.is_empty() {
            q.push(format!("body={}", encode(&self.body)));
        }
        if !q.is_empty() {
            url.push('?');
            url.push_str(&q.join("&"));
        }
        url
    }

    /// Reads a `mailto:` URL. Addresses that do not look like addresses are dropped; unknown fields are ignored.
    pub fn from_url(url: &str) -> Option<Compose> {
        let rest = url.trim().strip_prefix("mailto:").or_else(|| url.trim().strip_prefix("MAILTO:"))?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let list = |s: &str| -> Vec<String> {
            decode(s).split(',').map(str::trim).filter(|a| valid_address(a)).map(str::to_string).collect()
        };
        let mut c = Compose { to: list(path), ..Compose::default() };
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            match k.to_ascii_lowercase().as_str() {
                "to" => c.to.extend(list(v)),
                "cc" => c.cc.extend(list(v)),
                "bcc" => c.bcc.extend(list(v)),
                "subject" => c.subject = decode(v),
                "body" => c.body = decode(v),
                _ => {}
            }
        }
        Some(c)
    }

    /// Whether the window has anything to show: a recipient, a subject, a body or an attachment.
    pub fn is_empty(&self) -> bool {
        self.to.is_empty() && self.cc.is_empty() && self.bcc.is_empty() && self.subject.is_empty() && self.body.is_empty() && self.attachments.is_empty()
    }

    /// Adds a shared file; relative paths and repeats are refused.
    pub fn attach(&mut self, path: &str) -> bool {
        if !path.starts_with('/') || self.attachments.iter().any(|p| p == path) {
            return false;
        }
        self.attachments.push(path.to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_round_trip() {
        let c = Compose {
            to: vec!["ann@x.is".into(), "bob@y.is".into()],
            cc: vec!["cy@z.is".into()],
            subject: "Lunch & learn?".into(),
            body: "Line one\nLine two".into(),
            ..Compose::default()
        };
        let url = c.to_url();
        assert!(url.starts_with("mailto:ann@x.is,bob@y.is?cc=cy@z.is&subject=Lunch%20%26%20learn%3F&body=Line%20one%0ALine%20two"));
        assert_eq!(Compose::from_url(&url), Some(c));
    }

    #[test]
    fn reads_foreign_urls() {
        let c = Compose::from_url("mailto:a@b.is?subject=Hi+there&BCC=c@d.is&x=1&to=e@f.is").unwrap();
        assert_eq!(c.to, vec!["a@b.is", "e@f.is"]);
        assert_eq!(c.bcc, vec!["c@d.is"]);
        assert_eq!(c.subject, "Hi there");
        assert_eq!(Compose::from_url("https://x.is"), None);
        assert!(Compose::from_url("mailto:").unwrap().is_empty());
        assert!(Compose::from_url("mailto:not-an-address").unwrap().to.is_empty());
    }

    #[test]
    fn attachments_are_absolute_and_unique() {
        let mut c = Compose::default();
        assert!(c.attach("/home/a/doc.pdf"));
        assert!(!c.attach("/home/a/doc.pdf"));
        assert!(!c.attach("doc.pdf"));
        assert!(!c.is_empty());
    }

    #[test]
    fn percent_decoding_edges() {
        assert_eq!(decode("a%2"), "a%2");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("%C3%A9"), "é");
    }
}
