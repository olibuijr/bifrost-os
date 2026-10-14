//! Text form of a shared file list, so recents and favourites can be kept in a preferences file and read back. One line
//! per entry, newest first: `F` for a favourite or `R` for a recent, a tab, the key, a tab and the title. Backslashes,
//! tabs, line feeds and carriage returns inside a key or title are written as `\\`, `\t`, `\n` and `\r`. Lines that do
//! not parse are skipped when the text is read.

use crate::shared_list::{Item, SharedList};

/// Writes a list as text, one entry per line, in recency order.
pub fn encode(list: &SharedList) -> String {
    let mut out = String::new();
    for item in list.in_recency_order() {
        out.push(if item.favourite { 'F' } else { 'R' });
        out.push('\t');
        out.push_str(&escape(&item.key));
        out.push('\t');
        out.push_str(&escape(&item.title));
        out.push('\n');
    }
    out
}

/// Reads a list written by `encode`. The result keeps at most `capacity` recents.
pub fn decode(text: &str, capacity: usize) -> SharedList {
    let items: Vec<Item> = text
        .lines()
        .filter_map(|line| {
            let (flag, rest) = line.split_once('\t')?;
            let (key, title) = rest.split_once('\t')?;
            let favourite = match flag {
                "F" => true,
                "R" => false,
                _ => return None,
            };
            Some(Item { key: unescape(key), title: unescape(title), favourite })
        })
        .collect();
    SharedList::from_recency_order(capacity, items)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
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
