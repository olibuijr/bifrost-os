//! Quotations for the general pasteboard: the sentence a find match sits in, with its citation (the book's title, its
//! author and the chapter the text is in). Books has no free text selection yet, so the quotation comes from the match
//! the last find went to, while it is on the page.

use super::engine::{Block, Book};
use super::store::Span;
use bifrost_app::pasteboard_types::{Board, Item, STRING};
use bifrost_app::Ctx;

/// A sentence longer than this many characters is quoted as the match alone.
pub const MAX_CHARS: usize = 400;

/// The sentence around the match `span` (the match alone when the sentence is too long). None when the span is not in
/// the book's text.
pub fn sentence(blocks: &[Block], span: Span) -> Option<String> {
    let chars: Vec<char> = blocks.get(span.block)?.text()?.chars().collect();
    if span.offset >= chars.len() {
        return None;
    }
    let match_end = (span.offset + span.len).min(chars.len());
    let is_end = |c: char| matches!(c, '.' | '!' | '?' | '\u{2026}');
    let mut start = span.offset;
    while start > 0 && !is_end(chars[start - 1]) {
        start -= 1;
    }
    let mut end = match_end;
    while end < chars.len() && !is_end(chars[end]) {
        end += 1;
    }
    if end < chars.len() {
        end += 1;
    }
    let quote: String = if end - start > MAX_CHARS { chars[span.offset..match_end].iter().collect() } else { chars[start..end].iter().collect() };
    let quote = quote.trim();
    (!quote.is_empty()).then(|| quote.to_string())
}

/// The chapter the block `block` is in: the last table of contents entry at or before it.
pub fn chapter(book: &Book, block: usize) -> Option<&str> {
    book.toc.iter().filter(|t| t.block <= block).max_by_key(|t| t.block).map(|t| t.title.trim()).filter(|t| !t.is_empty())
}

/// "Title, Author, Chapter": the parts that are known, in that order.
pub fn citation(book: &Book, span: Span) -> String {
    let parts = [Some(book.title.trim()), Some(book.author.trim()), chapter(book, span.block)];
    parts.into_iter().flatten().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ")
}

/// The text to copy for the match `span`: the quoted sentence in curly quotes, then the citation on its own line.
pub fn quotation(book: &Book, span: Span) -> Option<String> {
    let text = sentence(&book.blocks, span)?;
    Some(format!("\u{201c}{text}\u{201d}\n\u{2014} {}", citation(book, span)))
}

/// Writes `text` to the general board as one plain-text item (the shared clipboard). The reply is not needed.
pub fn copy(ctx: &Ctx, text: &str) -> u32 {
    Board::general().write(ctx, &[Item::default().with(STRING, text.as_bytes())])
}
