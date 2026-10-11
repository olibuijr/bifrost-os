//! PDF loading: file structure, page tree, text and images extracted in
//! reading order, then reflowed into the shared page model.

pub mod content;
pub mod doc;
pub mod font;
pub mod image;
pub mod obj;
mod tables;

use super::model::{Block, Book, TocEntry};
use content::{Extractor, Piece};
use doc::Doc;
use obj::{Dict, Obj};
use std::collections::{HashMap, HashSet};

/// Pages read before giving up, so a huge file cannot stall the app forever.
const MAX_PAGES: usize = 3000;

struct PageNode {
    num: Option<u32>,
    dict: Dict,
    resources: Dict,
}

fn walk(doc: &Doc, node: &Obj, inherited: &Dict, out: &mut Vec<PageNode>, seen: &mut HashSet<u32>, depth: u8) {
    if depth > 24 || out.len() >= MAX_PAGES {
        return;
    }
    let num = if let Obj::Ref(n, _) = node { Some(*n) } else { None };
    if let Some(n) = num {
        if !seen.insert(n) {
            return;
        }
    }
    let Obj::Dict(d) = doc.get(node) else { return };
    let resources = match doc.field(&d, "Resources") {
        Obj::Dict(r) => r,
        _ => inherited.clone(),
    };
    match doc.field(&d, "Kids") {
        Obj::Array(kids) => {
            for k in &kids {
                walk(doc, k, &resources, out, seen, depth + 1);
            }
        }
        _ => out.push(PageNode { num, dict: d, resources }),
    }
}

fn pages(doc: &Doc) -> Vec<PageNode> {
    let mut out = vec![];
    if let Some(cat) = doc.catalog() {
        if let Some(root) = cat.get("Pages") {
            walk(doc, root, &Dict::new(), &mut out, &mut HashSet::new(), 0);
        }
    }
    out
}

/// A PDF text string (UTF-16BE with BOM, else PDFDocEncoding ~ Latin-1).
pub fn text_string(b: &[u8]) -> String {
    if b.starts_with(&[0xfe, 0xff]) {
        let units: Vec<u16> = b[2..].chunks(2).filter(|c| c.len() == 2).map(|c| (c[0] as u16) << 8 | c[1] as u16).collect();
        String::from_utf16_lossy(&units)
    } else if b.starts_with(&[0xef, 0xbb, 0xbf]) {
        String::from_utf8_lossy(&b[3..]).into_owned()
    } else {
        b.iter().map(|&c| c as char).collect()
    }
}

fn info(doc: &Doc) -> (String, String) {
    let get = |k: &str| match doc.trailer.get("Info").map(|o| doc.get(o)) {
        Some(Obj::Dict(d)) => match doc.field(&d, k) {
            Obj::Str(s) => text_string(&s).trim().to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    (get("Title"), get("Author"))
}

/// Title and author without reading the pages (for the library).
pub fn probe(data: Vec<u8>) -> Option<(String, String)> {
    let doc = Doc::open(data).ok()?;
    Some(info(&doc))
}

/// Resolves a destination (array, or a name looked up in the catalog) to a page object number.
fn dest_page(doc: &Doc, cat: &Dict, dest: &Obj, depth: u8) -> Option<u32> {
    match doc.get(dest) {
        Obj::Array(a) => match a.first()? {
            Obj::Ref(n, _) => Some(*n),
            _ => None,
        },
        Obj::Dict(d) => dest_page(doc, cat, d.get("D")?, depth + 1),
        Obj::Name(n) if depth < 3 => named_dest(doc, cat, n.as_bytes()).and_then(|o| dest_page(doc, cat, &o, depth + 1)),
        Obj::Str(s) if depth < 3 => named_dest(doc, cat, &s).and_then(|o| dest_page(doc, cat, &o, depth + 1)),
        _ => None,
    }
}

fn named_dest(doc: &Doc, cat: &Dict, name: &[u8]) -> Option<Obj> {
    if let Obj::Dict(d) = doc.field(cat, "Dests") {
        if let Some(o) = d.get(&String::from_utf8_lossy(name).into_owned()) {
            return Some(o.clone());
        }
    }
    let names = doc.field(cat, "Names");
    let tree = doc.field(names.dict()?, "Dests");
    name_tree(doc, &tree, name, 0)
}

fn name_tree(doc: &Doc, node: &Obj, name: &[u8], depth: u8) -> Option<Obj> {
    if depth > 8 {
        return None;
    }
    let d = node.dict()?;
    if let Obj::Array(pairs) = doc.field(d, "Names") {
        for kv in pairs.chunks(2) {
            if let (Obj::Str(k), Some(v)) = (doc.get(&kv[0]), kv.get(1)) {
                if k == name {
                    return Some(v.clone());
                }
            }
        }
    }
    if let Obj::Array(kids) = doc.field(d, "Kids") {
        for k in &kids {
            if let Some(o) = name_tree(doc, &doc.get(k), name, depth + 1) {
                return Some(o);
            }
        }
    }
    None
}

/// Outline entries as `(title, page object number, level)`.
fn outline(doc: &Doc, cat: &Dict) -> Vec<(String, u32, u8)> {
    let mut out = vec![];
    let Obj::Dict(root) = doc.field(cat, "Outlines") else { return out };
    let mut seen = HashSet::new();
    fn level(doc: &Doc, cat: &Dict, first: &Obj, lvl: u8, out: &mut Vec<(String, u32, u8)>, seen: &mut HashSet<u32>) {
        let mut cur = first.clone();
        while out.len() < 3000 {
            if let Obj::Ref(n, _) = &cur {
                if !seen.insert(*n) {
                    return;
                }
            }
            let Obj::Dict(item) = doc.get(&cur) else { return };
            let title = match doc.field(&item, "Title") {
                Obj::Str(s) => text_string(&s).trim().to_string(),
                _ => String::new(),
            };
            let dest = match item.get("Dest") {
                Some(d) => Some(d.clone()),
                None => doc.field(&item, "A").dict().and_then(|a| a.get("D").cloned()),
            };
            if let (false, Some(page)) = (title.is_empty(), dest.and_then(|d| dest_page(doc, cat, &d, 0))) {
                out.push((title, page, lvl));
            }
            if let Some(child) = item.get("First") {
                if lvl < 6 {
                    level(doc, cat, child, lvl + 1, out, seen);
                }
            }
            match item.get("Next") {
                Some(n) => cur = n.clone(),
                None => return,
            }
        }
    }
    if let Some(first) = root.get("First") {
        level(doc, cat, first, 0, &mut out, &mut seen);
    }
    out
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
    let nodes = pages(&doc);
    if nodes.is_empty() {
        return Err("the PDF has no pages".into());
    }
    let mut ex = Extractor::new(&doc);
    for n in &nodes {
        ex.page(&n.dict, &n.resources);
    }
    let (blocks, mut starts) = assemble(std::mem::take(&mut ex.pieces));
    if blocks.is_empty() {
        return Err("no readable text in this PDF (a scan without text?)".into());
    }
    starts.resize(nodes.len(), blocks.len().saturating_sub(1));
    let (title, author) = info(&doc);
    let mut book = Book { title, author, blocks, ..Default::default() };
    let last = book.blocks.len() - 1;
    let page_of: HashMap<u32, usize> = nodes.iter().enumerate().filter_map(|(i, n)| n.num.map(|num| (num, i))).collect();
    if let Some(cat) = doc.catalog() {
        for (t, page, level) in outline(&doc, &cat) {
            if let Some(&pi) = page_of.get(&page) {
                book.toc.push(TocEntry { title: t, block: starts[pi].min(last), level });
            }
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
    use doc::tests::tiny_pdf;

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
