//! PDF loading for the Books reader. `bifrost_render::cg::pdf` reads the file (structure, page tree, text and
//! pictures in drawing order, outline); this module reflows those pieces into the shared page model.

use super::model::{Block, Book, TocEntry};
use bifrost_render::cg::pdf::doc::Doc;
use bifrost_render::cg::pdf::extract::{Extractor, Piece};
use bifrost_render::cg::pdf::page;
use std::collections::HashMap;

/// Title and author without reading the pages (for the library).
pub fn probe(data: Vec<u8>) -> Option<(String, String)> {
    let doc = Doc::open(data).ok()?;
    let info = page::info(&doc);
    Some((info.title, info.author))
}

/// Reflows the extracted pieces into blocks; returns them with the index of
/// the block each page starts at.
pub fn assemble(pieces: Vec<Piece>) -> (Vec<Block>, Vec<usize>) {
    // body size: the font size carrying the most characters
    let mut weight: HashMap<i32, usize> = HashMap::new();
    for p in &pieces {
        if let Piece::Line { size, text, .. } = p {
            *weight.entry((size * 2.0).round() as i32).or_default() += text.chars().count();
        }
    }
    let body = weight.iter().max_by_key(|(k, v)| (**v, **k)).map(|(k, _)| *k as f32 / 2.0).unwrap_or(12.0).max(1.0);
    // typical baseline step between consecutive body lines
    let mut steps: Vec<f32> = vec![];
    let mut prev: Option<(f32, f32)> = None;
    for p in &pieces {
        match p {
            Piece::Line { y, size, .. } => {
                if let Some((py, ps)) = prev {
                    let dy = py - y;
                    if dy > 0.0 && dy < 3.0 * body && (ps - size).abs() < 0.5 {
                        steps.push(dy);
                    }
                }
                prev = Some((*y, *size));
            }
            _ => prev = None,
        }
    }
    steps.sort_by(|a, b| a.total_cmp(b));
    let step = steps.get(steps.len() / 2).copied().unwrap_or(body * 1.2).max(body * 0.8);

    let mut blocks: Vec<Block> = vec![];
    let mut starts: Vec<usize> = vec![];
    // (text, heading size or 0 for body, last baseline)
    let mut para: Option<(String, f32, f32)> = None;
    let mut page_break = false;
    // a page's start is the block holding its first content, found lazily
    let mut need_start = false;

    let finish = |para: &mut Option<(String, f32, f32)>, blocks: &mut Vec<Block>| {
        if let Some((text, size, _)) = para.take() {
            if size > 0.0 {
                let r = size / body;
                blocks.push(Block::Heading(if r >= 1.8 { 1 } else if r >= 1.45 { 2 } else { 3 }, text));
            } else {
                blocks.push(Block::Para(text));
            }
        }
    };

    for p in pieces {
        match p {
            Piece::Page => {
                if need_start {
                    starts.push(blocks.len());
                }
                need_start = true;
                page_break = true;
            }
            Piece::Image(img) => {
                finish(&mut para, &mut blocks);
                if std::mem::take(&mut need_start) {
                    starts.push(blocks.len());
                }
                blocks.push(Block::Image(img));
            }
            Piece::Line { y, size, text } => {
                let text = text.trim().to_string();
                // bare page numbers are chrome
                if text.chars().count() <= 4 && text.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let heading = size >= body * 1.25 && text.chars().count() <= 120;
                let hsize = if heading { size } else { 0.0 };
                let joins = match &para {
                    None => false,
                    Some((t, ps, py)) => {
                        let same_kind = (*ps > 0.0) == heading && (!heading || (ps / size - 1.0).abs() < 0.08);
                        let ends_sentence = t.ends_with(['.', '!', '?', ':', '"', '”', ')']);
                        if !same_kind {
                            false
                        } else if page_break {
                            !ends_sentence && !heading
                        } else {
                            let dy = py - y;
                            dy > -0.5 * size && dy <= step * 1.45
                        }
                    }
                };
                if !joins {
                    finish(&mut para, &mut blocks);
                }
                if std::mem::take(&mut need_start) {
                    starts.push(blocks.len());
                }
                if !joins {
                    para = Some((text, hsize, y));
                } else if let Some((t, _, py)) = &mut para {
                    let hyphen = t.ends_with('-') && t.chars().rev().nth(1).is_some_and(char::is_alphabetic) && text.chars().next().is_some_and(char::is_lowercase);
                    if hyphen {
                        t.pop();
                    } else {
                        t.push(' ');
                    }
                    t.push_str(&text);
                    *py = y;
                }
                page_break = false;
            }
        }
    }
    finish(&mut para, &mut blocks);
    if need_start {
        starts.push(blocks.len());
    }
    (blocks, starts)
}


/// Loads a whole PDF.
pub fn load(data: Vec<u8>) -> Result<Book, String> {
    let doc = Doc::open(data)?;
    let nodes = page::pages(&doc);
    if nodes.is_empty() {
        return Err("the PDF has no pages".into());
    }
    let mut ex = Extractor::new(&doc);
    for p in &nodes {
        ex.page(p);
    }
    let (blocks, mut starts) = assemble(std::mem::take(&mut ex.pieces));
    if blocks.is_empty() {
        return Err("no readable text in this PDF (a scan without text?)".into());
    }
    starts.resize(nodes.len(), blocks.len().saturating_sub(1));
    let info = page::info(&doc);
    let mut book = Book { title: info.title, author: info.author, blocks, ..Default::default() };
    let last = book.blocks.len() - 1;
    let page_of: HashMap<u32, usize> = nodes.iter().enumerate().filter_map(|(i, p)| p.num.map(|num| (num, i))).collect();
    for item in page::outline(&doc) {
        if let Some(&pi) = page_of.get(&item.dest.page) {
            book.toc.push(TocEntry { title: item.title, block: starts[pi].min(last), level: item.level });
        }
    }
    if book.toc.is_empty() {
        let step = (nodes.len() / 20).max(1).max(if nodes.len() > 20 { 5 } else { 1 });
        book.toc = (0..nodes.len()).step_by(step).map(|i| TocEntry { title: format!("Page {}", i + 1), block: starts[i].min(last), level: 0 }).collect();
    }
    Ok(book)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::cg::pdf::doc::tests::tiny_pdf;
    use bifrost_render::cg::pdf::page::text_string;

    fn line(y: f32, size: f32, t: &str) -> Piece {
        Piece::Line { y, size, text: t.into() }
    }

    fn texts(b: &[Block]) -> Vec<String> {
        b.iter().filter_map(|x| x.text().map(str::to_string)).collect()
    }

    #[test]
    fn paragraphs_headings_hyphens_and_page_numbers() {
        let pieces = vec![
            Piece::Page,
            line(740.0, 24.0, "A Title"),
            line(700.0, 12.0, "First line of the para-"),
            line(686.0, 12.0, "graph continues here."),
            line(656.0, 12.0, "Second paragraph."),
            line(60.0, 12.0, "7"),
            Piece::Page,
            line(740.0, 12.0, "starts mid sentence and"),
            line(726.0, 12.0, "ends."),
        ];
        let (blocks, starts) = assemble(pieces);
        assert_eq!(texts(&blocks), ["A Title", "First line of the paragraph continues here.", "Second paragraph.", "starts mid sentence and ends."]);
        assert!(matches!(blocks[0], Block::Heading(1, _)));
        assert_eq!(starts, vec![0, 3]);
    }

    #[test]
    fn a_sentence_end_closes_the_paragraph_at_a_page_break() {
        let (blocks, _) = assemble(vec![Piece::Page, line(700.0, 12.0, "Done."), Piece::Page, line(700.0, 12.0, "New page.")]);
        assert_eq!(texts(&blocks), ["Done.", "New page."]);
    }

    #[test]
    fn loads_text_from_a_tiny_pdf() {
        let b = load(tiny_pdf("BT /F1 12 Tf 72 700 Td (Hello PDF) Tj 0 -14 Td (second line) Tj ET")).unwrap();
        assert_eq!(texts(&b.blocks), ["Hello PDF second line"]);
        assert_eq!(b.toc.len(), 1);
    }

    #[test]
    fn errors_are_reported() {
        assert!(load(b"%PDF-1.4 garbage".to_vec()).is_err());
        assert!(load(tiny_pdf("")).unwrap_err().contains("no readable text"));
    }

    #[test]
    fn text_strings() {
        assert_eq!(text_string(&[0xfe, 0xff, 0, 0x48, 0, 0x69]), "Hi");
        assert_eq!(text_string(b"Plain"), "Plain");
    }
}
