//! A reading list: web pages saved to read later, newest first. It keeps the URL, title, preview text and the
//! time each item was added, refuses addresses it cannot open again, and round-trips through a tab-separated text
//! form for the app's own store. Pure model: the caller supplies the clock and the file.

/// One saved page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub url: String,
    pub title: String,
    pub preview: String,
    /// Seconds since the Unix epoch, as given by the caller.
    pub added: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Only http and https addresses with a host can be saved.
    UnsupportedUrl,
}

#[derive(Debug)]
pub struct ReadingList {
    items: Vec<Item>,
    limit: usize,
}

impl ReadingList {
    /// An empty list that keeps at most `limit` items; the oldest are dropped first.
    pub fn new(limit: usize) -> Self {
        ReadingList { items: Vec::new(), limit: limit.max(1) }
    }

    /// Whether `url` can be saved: an http or https address with a host, and no spaces or control characters.
    pub fn supports(url: &str) -> bool {
        let rest = match url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) {
            Some(rest) => rest,
            None => return false,
        };
        let host = rest.split(|c: char| c == '/' || c == '?' || c == '#').next().unwrap_or("");
        !host.is_empty() && !url.chars().any(|c| c.is_whitespace() || c.is_control())
    }

    /// Saves a page. A page already in the list moves to the top with the new title, preview and time.
    /// An empty title falls back to the address.
    pub fn add(&mut self, url: &str, title: &str, preview: &str, added: u64) -> Result<(), Error> {
        if !Self::supports(url) {
            return Err(Error::UnsupportedUrl);
        }
        self.items.retain(|item| item.url != url);
        let title = title.trim();
        self.items.insert(
            0,
            Item {
                url: url.to_string(),
                title: if title.is_empty() { url.to_string() } else { title.to_string() },
                preview: preview.trim().to_string(),
                added,
            },
        );
        self.items.truncate(self.limit);
        Ok(())
    }

    /// Removes the page at `url`. Returns whether it was in the list.
    pub fn remove(&mut self, url: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.url != url);
        self.items.len() != before
    }

    pub fn contains(&self, url: &str) -> bool {
        self.items.iter().any(|item| item.url == url)
    }

    /// Newest first.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// One line per item, newest first: `added<TAB>url<TAB>title<TAB>preview`. Tabs, line breaks and backslashes
    /// inside the fields are escaped, so each item is exactly one line.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                item.added,
                escape(&item.url),
                escape(&item.title),
                escape(&item.preview)
            ));
        }
        out
    }

    /// Reads `to_text` output. Malformed lines, unsupported addresses and repeated addresses are skipped, so a
    /// damaged store loses only those rows.
    pub fn from_text(text: &str, limit: usize) -> Self {
        let mut list = ReadingList::new(limit);
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 4 {
                continue;
            }
            let Ok(added) = fields[0].parse::<u64>() else { continue };
            let url = unescape(fields[1]);
            if !Self::supports(&url) || list.contains(&url) {
                continue;
            }
            list.items.push(Item { url, title: unescape(fields[2]), preview: unescape(fields[3]), added });
        }
        list.items.truncate(list.limit);
        list
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_items_come_first_and_a_repeat_moves_to_the_top() {
        let mut list = ReadingList::new(10);
        list.add("https://a.example/one", "One", "", 1).unwrap();
        list.add("https://b.example/two", "Two", "", 2).unwrap();
        list.add("https://a.example/one", "One again", "fresh", 3).unwrap();
        let urls: Vec<&str> = list.items().iter().map(|i| i.url.as_str()).collect();
        assert_eq!(urls, ["https://a.example/one", "https://b.example/two"]);
        assert_eq!(list.items()[0].title, "One again");
        assert_eq!(list.items()[0].added, 3);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn only_web_addresses_with_a_host_are_saved() {
        assert!(ReadingList::supports("https://example.com/a?b=c#d"));
        assert!(!ReadingList::supports("ftp://example.com/"));
        assert!(!ReadingList::supports("file:///home/user/a.html"));
        assert!(!ReadingList::supports("https:///no-host"));
        assert!(!ReadingList::supports("https://exa mple.com/"));
        let mut list = ReadingList::new(5);
        assert_eq!(list.add("mailto:a@example.com", "", "", 1), Err(Error::UnsupportedUrl));
        assert!(list.is_empty());
    }

    #[test]
    fn an_empty_title_falls_back_to_the_address() {
        let mut list = ReadingList::new(5);
        list.add("https://example.com/x", "   ", "  Summary text  ", 7).unwrap();
        assert_eq!(list.items()[0].title, "https://example.com/x");
        assert_eq!(list.items()[0].preview, "Summary text");
    }

    #[test]
    fn the_limit_drops_the_oldest_items() {
        let mut list = ReadingList::new(2);
        for n in 0..4u64 {
            list.add(&format!("https://example.com/{n}"), "", "", n).unwrap();
        }
        assert_eq!(list.len(), 2);
        assert!(!list.contains("https://example.com/0"));
        assert!(!list.contains("https://example.com/1"));
        assert!(list.contains("https://example.com/3"));
    }

    #[test]
    fn remove_reports_whether_the_item_was_there() {
        let mut list = ReadingList::new(5);
        list.add("https://example.com/x", "X", "", 1).unwrap();
        assert!(list.remove("https://example.com/x"));
        assert!(!list.remove("https://example.com/x"));
        assert!(list.is_empty());
    }

    #[test]
    fn the_text_form_round_trips_tabs_line_breaks_and_backslashes() {
        let mut list = ReadingList::new(5);
        list.add("https://example.com/p", "Tab\there", "line one\nline two \\ end", 42).unwrap();
        list.add("https://example.com/q", "Second", "", 41).unwrap();
        let text = list.to_text();
        assert_eq!(text.lines().count(), 2);
        let back = ReadingList::from_text(&text, 5);
        assert_eq!(back.items(), list.items());
    }

    #[test]
    fn damaged_rows_are_skipped_on_load() {
        let text = "10\thttps://example.com/ok\tOK\t\nnot a row\nx\thttps://example.com/bad-time\tT\t\n5\tftp://example.com/\tF\t\n11\thttps://example.com/ok\tDup\t\n";
        let list = ReadingList::from_text(text, 5);
        assert_eq!(list.len(), 1);
        assert_eq!(list.items()[0].title, "OK");
    }
}
