//! EPUB loading: zip container, `META-INF/container.xml`, the OPF package
//! (metadata, manifest, spine), the table of contents (EPUB 3 nav document
//! or EPUB 2 NCX) and the spine's XHTML flowed into the shared page model.

use super::markup::{local, tokens, Tok};
use super::model::{Block, Book, TocEntry};
use super::xhtml::{self, Raw};
use super::zip::Zip;
use crate::gfx::Image;
use std::collections::HashMap;

/// Pictures wider than this are scaled down when loaded.
const MAX_IMAGE_W: usize = 1400;
const MAX_IMAGE_BYTES: usize = 12 << 20;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub href: String,
    pub media: String,
    pub props: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Package {
    pub title: String,
    pub author: String,
    pub items: Vec<Item>,
    pub spine: Vec<String>,
    pub toc_id: Option<String>,
    pub cover_id: Option<String>,
}

impl Package {
    fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
}

/// Title, author and cover, without flowing the text (for the library).
pub struct Meta {
    pub title: String,
    pub author: String,
    pub cover: Option<Image>,
}

/// Joins `href` onto the directory of `base` (a zip path), resolving `..`,
/// dropping the fragment and decoding `%XX`.
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = if href.starts_with('/') { vec![] } else { base.rsplit_once('/').map_or(vec![], |(d, _)| d.split('/').collect()) };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn fragment(href: &str) -> Option<&str> {
    href.split_once('#').map(|(_, f)| f).filter(|f| !f.is_empty())
}

/// The OPF path named by `container.xml`.
pub fn rootfile(container: &str) -> Option<String> {
    tokens(container).iter().find_map(|t| match t {
        Tok::Open { name, .. } if local(name) == "rootfile" => t.attr("full-path").map(str::to_string),
        _ => None,
    })
}

pub fn parse_opf(src: &str) -> Package {
    let mut p = Package::default();
    let mut want: Option<&'static str> = None;
    let mut cover_meta: Option<String> = None;
    for t in tokens(src) {
        match &t {
            Tok::Open { name, .. } => match local(name) {
                "title" if p.title.is_empty() => want = Some("title"),
                "creator" if p.author.is_empty() => want = Some("creator"),
                "item" => p.items.push(Item {
                    id: t.attr("id").unwrap_or("").into(),
                    href: t.attr("href").unwrap_or("").into(),
                    media: t.attr("media-type").unwrap_or("").into(),
                    props: t.attr("properties").unwrap_or("").into(),
                }),
                "itemref" => {
                    if let Some(id) = t.attr("idref") {
                        p.spine.push(id.into());
                    }
                }
                "spine" => p.toc_id = t.attr("toc").map(str::to_string),
                "meta" => {
                    if t.attr("name") == Some("cover") {
                        cover_meta = t.attr("content").map(str::to_string);
                    }
                }
                _ => {}
            },
            Tok::Text(s) => match want.take() {
                Some("title") => p.title = s.trim().to_string(),
                Some("creator") => p.author = s.trim().to_string(),
                _ => {}
            },
            Tok::Close(_) => want = None,
        }
    }
    p.cover_id = p.items.iter().find(|i| i.props.split_whitespace().any(|x| x == "cover-image")).map(|i| i.id.clone()).or(cover_meta);
    p
}

/// `(title, href)` of every TOC entry with its depth, from an NCX document.
pub fn parse_ncx(src: &str) -> Vec<(String, String, u8)> {
    let mut out = vec![];
    let mut depth = 0u8;
    let mut labels: Vec<String> = vec![];
    let mut in_label = false;
    let mut in_text = false;
    for t in tokens(src) {
        match &t {
            Tok::Open { name, .. } => match local(name) {
                "navpoint" => {
                    depth += 1;
                    labels.push(String::new());
                }
                "navlabel" => in_label = true,
                "text" if in_label => in_text = true,
                "content" => {
                    if let (Some(src), Some(label)) = (t.attr("src"), labels.last()) {
                        out.push((label.clone(), src.to_string(), depth.saturating_sub(1)));
                    }
                }
                _ => {}
            },
            Tok::Close(name) => match local(name) {
                "navpoint" => {
                    depth = depth.saturating_sub(1);
                    labels.pop();
                }
                "navlabel" => in_label = false,
                "text" => in_text = false,
                _ => {}
            },
            Tok::Text(s) => {
                if in_text {
                    if let Some(l) = labels.last_mut() {
                        l.push_str(s.trim());
                    }
                }
            }
        }
    }
    out
}

/// TOC entries from an EPUB 3 navigation document's `<nav epub:type="toc">`.
pub fn parse_nav(src: &str) -> Vec<(String, String, u8)> {
    let mut out = vec![];
    let mut in_toc = false;
    let mut depth = 0u8;
    let mut href: Option<String> = None;
    let mut label = String::new();
    for t in tokens(src) {
        match &t {
            Tok::Open { name, .. } => match local(name) {
                "nav" => {
                    let ty = t.attr("epub:type").or_else(|| t.attr("type")).unwrap_or("");
                    in_toc = ty.split_whitespace().any(|x| x == "toc");
                }
                "ol" if in_toc => depth += 1,
                "a" if in_toc => {
                    href = t.attr("href").map(str::to_string);
                    label.clear();
                }
                _ => {}
            },
            Tok::Close(name) => match local(name) {
                "nav" => in_toc = false,
                "ol" if in_toc => depth = depth.saturating_sub(1),
                "a" if in_toc => {
                    if let Some(h) = href.take() {
                        let l = label.split_whitespace().collect::<Vec<_>>().join(" ");
                        if !l.is_empty() {
                            out.push((l, h, depth.saturating_sub(1)));
                        }
                    }
                }
                _ => {}
            },
            Tok::Text(s) => {
                if href.is_some() {
                    label.push_str(s);
                }
            }
        }
    }
    out
}

fn read_package(zip: &Zip) -> Option<(String, Package)> {
    let container = String::from_utf8_lossy(&zip.read("META-INF/container.xml")?).into_owned();
    let opf_path = rootfile(&container)?;
    let opf = String::from_utf8_lossy(&zip.read(&opf_path)?).into_owned();
    Some((opf_path, parse_opf(&opf)))
}

fn load_image(zip: &Zip, path: &str) -> Option<Image> {
    let bytes = zip.read(path)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let img = crate::cg::imageio::decode(&bytes)?;
    Some(shrink(img))
}

/// Scales an image down to `MAX_IMAGE_W` wide, keeping its aspect.
pub fn shrink(img: Image) -> Image {
    if img.w <= MAX_IMAGE_W || img.w == 0 {
        return img;
    }
    let h = (img.h * MAX_IMAGE_W / img.w).max(1);
    img.resized(MAX_IMAGE_W, h)
}

/// Title, author and cover of an EPUB.
pub fn probe(data: Vec<u8>) -> Option<Meta> {
    let zip = Zip::new(data)?;
    let (opf_path, pkg) = read_package(&zip)?;
    let cover = pkg.cover_id.as_deref().and_then(|id| pkg.item(id)).and_then(|i| load_image(&zip, &resolve(&opf_path, &i.href)));
    Some(Meta { title: pkg.title, author: pkg.author, cover })
}

/// Loads a whole EPUB.
pub fn load(data: Vec<u8>) -> Result<Book, String> {
    let zip = Zip::new(data).ok_or("not an EPUB (bad zip)")?;
    let (opf_path, pkg) = read_package(&zip).ok_or("no package document")?;
    let mut book = Book { title: pkg.title.clone(), author: pkg.author.clone(), ..Default::default() };
    book.cover = pkg.cover_id.as_deref().and_then(|id| pkg.item(id)).and_then(|i| load_image(&zip, &resolve(&opf_path, &i.href)));

    let mut doc_start: HashMap<String, usize> = HashMap::new();
    let mut anchors: HashMap<(String, String), usize> = HashMap::new();
    for idref in &pkg.spine {
        let Some(item) = pkg.item(idref) else { continue };
        let path = resolve(&opf_path, &item.href);
        let Some(bytes) = zip.read(&path) else { continue };
        let parsed = xhtml::parse(&String::from_utf8_lossy(&bytes));
        let base = book.blocks.len();
        doc_start.insert(path.clone(), base);
        for (id, i) in parsed.anchors {
            anchors.insert((path.clone(), id), base + i);
        }
        for raw in parsed.blocks {
            match raw {
                Raw::Heading(l, t) => book.blocks.push(Block::Heading(l, t)),
                Raw::Para(t) => book.blocks.push(Block::Para(t)),
                Raw::Image(src) => {
                    if let Some(img) = load_image(&zip, &resolve(&path, &src)) {
                        book.blocks.push(Block::Image(img));
                    }
                }
            }
        }
    }

    // table of contents: the nav document if there is one, else the NCX
    let nav = pkg.items.iter().find(|i| i.props.split_whitespace().any(|x| x == "nav"));
    let mut raw_toc: Vec<(String, String, u8)> = vec![];
    let mut toc_base = opf_path.clone();
    if let Some(n) = nav {
        toc_base = resolve(&opf_path, &n.href);
        if let Some(b) = zip.read(&toc_base) {
            raw_toc = parse_nav(&String::from_utf8_lossy(&b));
        }
    }
    if raw_toc.is_empty() {
        let ncx = pkg.toc_id.as_deref().and_then(|id| pkg.item(id)).or_else(|| pkg.items.iter().find(|i| i.media == "application/x-dtbncx+xml"));
        if let Some(n) = ncx {
            toc_base = resolve(&opf_path, &n.href);
            if let Some(b) = zip.read(&toc_base) {
                raw_toc = parse_ncx(&String::from_utf8_lossy(&b));
            }
        }
    }
    for (title, href, level) in raw_toc {
        let path = resolve(&toc_base, &href);
        let block = fragment(&href).and_then(|f| anchors.get(&(path.clone(), f.to_string())).copied()).or_else(|| doc_start.get(&path).copied());
        if let Some(block) = block {
            book.toc.push(TocEntry { title, block: block.min(book.blocks.len().saturating_sub(1)), level });
        }
    }
    if book.toc.is_empty() {
        book.toc = fallback_toc(&book.blocks, &doc_start);
    }
    if book.blocks.is_empty() {
        return Err("the book has no readable text".into());
    }
    Ok(book)
}

/// One entry per spine document, named after its first heading.
fn fallback_toc(blocks: &[Block], doc_start: &HashMap<String, usize>) -> Vec<TocEntry> {
    let mut starts: Vec<usize> = doc_start.values().copied().filter(|&s| s < blocks.len()).collect();
    starts.sort_unstable();
    starts.dedup();
    starts
        .iter()
        .enumerate()
        .map(|(n, &s)| {
            let title = blocks[s..].iter().find_map(|b| if let Block::Heading(_, t) = b { Some(t.clone()) } else { None }).unwrap_or_else(|| format!("Section {}", n + 1));
            TocEntry { title, block: s, level: 0 }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::zip::tests::build;
    use super::*;

    const CONTAINER: &str = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    const OPF: &str = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Tiny Book</dc:title><dc:creator>A. Writer</dc:creator></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="ch%201.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#;
    const NAV: &str = r##"<html xmlns:epub="x"><body><nav epub:type="toc"><ol><li><a href="ch%201.xhtml">First</a></li><li><a href="text/ch2.xhtml#mid">Second</a><ol><li><a href="text/ch2.xhtml">Nested</a></li></ol></li></ol></nav></body></html>"##;
    const CH1: &str = "<html><body><h1>First</h1><p>Hello world.</p></body></html>";
    const CH2: &str = "<html><body><p>Intro</p><h2 id=\"mid\">Middle</h2><p>The end.</p></body></html>";

    fn tiny() -> Vec<u8> {
        build(&[
            ("mimetype", 0, b"application/epub+zip", 20),
            ("META-INF/container.xml", 0, CONTAINER.as_bytes(), CONTAINER.len()),
            ("OEBPS/content.opf", 0, OPF.as_bytes(), OPF.len()),
            ("OEBPS/nav.xhtml", 0, NAV.as_bytes(), NAV.len()),
            ("OEBPS/ch 1.xhtml", 0, CH1.as_bytes(), CH1.len()),
            ("OEBPS/text/ch2.xhtml", 0, CH2.as_bytes(), CH2.len()),
        ])
    }

    #[test]
    fn resolves_relative_paths() {
        assert_eq!(resolve("OEBPS/content.opf", "text/a.xhtml#x"), "OEBPS/text/a.xhtml");
        assert_eq!(resolve("OEBPS/text/a.xhtml", "../img/c%20d.png"), "OEBPS/img/c d.png");
        assert_eq!(resolve("content.opf", "a.xhtml"), "a.xhtml");
        assert_eq!(resolve("OEBPS/a.xhtml", "/abs/b.png"), "abs/b.png");
    }

    #[test]
    fn container_and_opf() {
        assert_eq!(rootfile(CONTAINER).as_deref(), Some("OEBPS/content.opf"));
        let p = parse_opf(OPF);
        assert_eq!((p.title.as_str(), p.author.as_str()), ("Tiny Book", "A. Writer"));
        assert_eq!(p.spine, ["c1", "c2"]);
        assert_eq!(p.items.len(), 3);
        assert_eq!(p.items[0].props, "nav");
    }

    #[test]
    fn ncx_nesting() {
        let ncx = r#"<ncx><navMap><navPoint id="a"><navLabel><text>One</text></navLabel><content src="a.xhtml"/><navPoint id="b"><navLabel><text>One.1</text></navLabel><content src="a.xhtml#s"/></navPoint></navPoint><navPoint id="c"><navLabel><text>Two</text></navLabel><content src="b.xhtml"/></navPoint></navMap></ncx>"#;
        assert_eq!(parse_ncx(ncx), vec![("One".into(), "a.xhtml".into(), 0), ("One.1".into(), "a.xhtml#s".into(), 1), ("Two".into(), "b.xhtml".into(), 0)]);
    }

    #[test]
    fn loads_a_whole_tiny_epub() {
        let b = load(tiny()).unwrap();
        assert_eq!((b.title.as_str(), b.author.as_str()), ("Tiny Book", "A. Writer"));
        let texts: Vec<_> = b.blocks.iter().filter_map(Block::text).collect();
        assert_eq!(texts, ["First", "Hello world.", "Intro", "Middle", "The end."]);
        assert_eq!(b.toc.iter().map(|e| (e.title.as_str(), e.block, e.level)).collect::<Vec<_>>(), [("First", 0, 0), ("Second", 3, 0), ("Nested", 2, 1)]);
    }

    #[test]
    fn broken_input_is_an_error_not_a_panic() {
        assert!(load(b"nope".to_vec()).is_err());
        let z = build(&[("mimetype", 0, b"x", 1)]);
        assert!(load(z).is_err());
        assert!(probe(tiny()).is_some());
    }
}
