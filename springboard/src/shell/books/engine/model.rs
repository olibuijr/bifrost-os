//! The page model both formats load into: a flat list of blocks (headings,
//! paragraphs, pictures) plus a table of contents. Pagination happens later,
//! at the reader's font size, so reflow never touches the format code.

use crate::gfx::Image;

#[derive(Debug)]
pub enum Block {
    /// level 1..=6
    Heading(u8, String),
    Para(String),
    Image(Image),
}

impl Block {
    pub fn text(&self) -> Option<&str> {
        match self {
            Block::Heading(_, t) | Block::Para(t) => Some(t),
            Block::Image(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocEntry {
    pub title: String,
    /// index into `Book::blocks`
    pub block: usize,
    /// nesting depth, 0 = top
    pub level: u8,
}

#[derive(Debug, Default)]
pub struct Book {
    pub title: String,
    pub author: String,
    pub blocks: Vec<Block>,
    pub toc: Vec<TocEntry>,
    pub cover: Option<Image>,
}

/// A search match: where it is and a line of context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub block: usize,
    /// char offset of the match inside the block's text
    pub offset: usize,
    pub snippet: String,
}

impl Book {
    /// Case-insensitive search through all text, at most `limit` hits.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        search(&self.blocks, query, limit)
    }

    /// The last TOC entry at or before `block`, for the "current chapter" label.
    pub fn chapter_at(&self, block: usize) -> Option<&TocEntry> {
        self.toc.iter().filter(|e| e.block <= block).last()
    }
}

pub fn search(blocks: &[Block], query: &str, limit: usize) -> Vec<Hit> {
    let q: Vec<char> = query.trim().chars().flat_map(|c| c.to_lowercase()).collect();
    let mut hits = vec![];
    if q.is_empty() {
        return hits;
    }
    for (bi, b) in blocks.iter().enumerate() {
        let Some(text) = b.text() else { continue };
        let chars: Vec<char> = text.chars().collect();
        // lowercase per char keeps indices aligned with `chars` for the common
        // one-to-one case; the rare multi-char lowercase just keeps its first char
        let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
        let mut i = 0;
        while i + q.len() <= lower.len() {
            if lower[i..i + q.len()] == q[..] {
                let from = i.saturating_sub(24);
                let to = (i + q.len() + 40).min(chars.len());
                let mut snippet: String = chars[from..to].iter().collect();
                if from > 0 {
                    snippet.insert(0, '…');
                }
                if to < chars.len() {
                    snippet.push('…');
                }
                hits.push(Hit { block: bi, offset: i, snippet: snippet.replace('\n', " ") });
                if hits.len() >= limit {
                    return hits;
                }
                i += q.len();
            } else {
                i += 1;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Book {
        Book {
            blocks: vec![Block::Heading(1, "One".into()), Block::Para("The Quick brown fox, the end".into()), Block::Image(Image::new(1, 1)), Block::Para("another THE".into())],
            toc: vec![TocEntry { title: "One".into(), block: 0, level: 0 }, TocEntry { title: "Two".into(), block: 3, level: 0 }],
            ..Default::default()
        }
    }

    #[test]
    fn search_is_case_insensitive_and_reports_offsets() {
        let hits = book().search("the", 10);
        assert_eq!(hits.iter().map(|h| (h.block, h.offset)).collect::<Vec<_>>(), [(1, 0), (1, 21), (3, 3), (3, 8)]);
        assert!(hits[0].snippet.starts_with("The Quick"));
        assert_eq!(book().search("the", 2).len(), 2);
        assert!(book().search("  ", 5).is_empty());
        assert!(book().search("zebra", 5).is_empty());
    }

    #[test]
    fn chapter_lookup_uses_the_last_entry_before_the_block() {
        let b = book();
        assert_eq!(b.chapter_at(2).unwrap().title, "One");
        assert_eq!(b.chapter_at(3).unwrap().title, "Two");
    }
}
