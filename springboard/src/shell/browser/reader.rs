//! Reader mode: finds the article in a page and rebuilds it as a small,
//! clean document (title, headings, paragraphs, lists, quotes, images),
//! styled by [`READER_CSS`] instead of the page's own sheets.
//!
//! The article is the `<article>`/`<main>` element with the most text, or
//! else the element that collects the most paragraph text (each long
//! paragraph scores its parent fully and its grandparent by half, minus
//! link text). Navigation, forms, ads and link farms are dropped.

use super::html::{self, Element, Node};
use std::collections::HashMap;

pub const READER_CSS: &str = "
html, body { background: #f7f3ea; color: #222 }
body { margin: 0; padding: 24px 20px }
.article { max-width: 680px; margin: 0 auto; font-size: 20px; line-height: 1.6 }
.article h1 { font-size: 32px; margin: 0 0 .6em }
.article h2 { font-size: 26px; margin: 1.2em 0 .5em }
.article h3 { font-size: 22px; margin: 1.1em 0 .5em }
.article p { margin: 0 0 1em }
.article a { color: #1a4f8b; text-decoration: none }
.article blockquote { margin: 1em 0; padding: 0 0 0 16px; border-left: 3px solid #c9bfa8; color: #444 }
.article pre { font-size: 16px; background: #ece6d8 }
";

/// Below this much paragraph text a page has no article worth a reader view.
const MIN_ARTICLE_CHARS: f32 = 140.0;
const NEGATIVE: &[&str] = &["comment", "sidebar", "footer", "nav", "menu", "promo", "share", "social", "related", "advert", "banner", "cookie", "popup", "newsletter", "subscribe", "breadcrumb", "widget"];
const DROP: &[&str] = &["script", "style", "nav", "aside", "footer", "form", "button", "input", "select", "textarea", "iframe", "svg", "noscript", "head", "template", "video", "audio", "canvas", "object", "embed", "link", "meta"];
const KEEP: &[&str] = &[
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "pre", "figure", "figcaption", "img", "hr", "br", "b", "strong", "i", "em", "a", "code", "sub", "sup", "small", "u", "mark", "dl", "dt", "dd",
];
const BLOCKISH: &[&str] = &["div", "section", "article", "main", "table", "tbody", "thead", "tr", "td", "th", "header", "center", "details", "summary", "address"];

#[derive(Clone, Debug, PartialEq)]
pub struct Article {
    pub title: String,
    pub nodes: Vec<Node>,
}

fn words_len(s: &str) -> usize {
    s.split_ascii_whitespace().map(|w| w.chars().count() + 1).sum::<usize>().saturating_sub(1)
}

fn text_len(n: &Node) -> usize {
    words_len(&n.text_content())
}

fn el_len(e: &Element) -> usize {
    words_len(&e.children.iter().map(Node::text_content).collect::<String>())
}

fn link_len(n: &Node) -> usize {
    match n {
        Node::Text(_) => 0,
        Node::Element(e) if e.tag == "a" => text_len(n),
        Node::Element(e) => e.children.iter().map(link_len).sum(),
    }
}

fn negative(e: &Element) -> bool {
    let hay = format!("{} {}", e.attr("class").unwrap_or(""), e.attr("id").unwrap_or("")).to_ascii_lowercase();
    NEGATIVE.iter().any(|n| hay.contains(n))
}

fn node_at<'a>(dom: &'a [Node], path: &[usize]) -> Option<&'a Element> {
    let (first, rest) = path.split_first()?;
    let e = dom.get(*first)?.element()?;
    if rest.is_empty() {
        Some(e)
    } else {
        node_at(&e.children, rest)
    }
}

fn score(nodes: &[Node], path: &mut Vec<usize>, scores: &mut HashMap<Vec<usize>, f32>) {
    for (i, n) in nodes.iter().enumerate() {
        let Node::Element(e) = n else { continue };
        path.push(i);
        if e.tag == "p" && path.len() >= 2 {
            let len = text_len(n);
            if len >= 25 {
                let density = link_len(n) as f32 / len as f32;
                let s = (len as f32 + n.text_content().matches(',').count() as f32 * 3.0) * (1.0 - density);
                *scores.entry(path[..path.len() - 1].to_vec()).or_default() += s;
                if path.len() >= 3 {
                    *scores.entry(path[..path.len() - 2].to_vec()).or_default() += s / 2.0;
                }
            }
        }
        score(&e.children, path, scores);
        path.pop();
    }
}

fn find_semantic<'a>(nodes: &'a [Node]) -> Option<&'a Element> {
    let mut all: Vec<&Element> = vec![];
    for tag in ["article", "main"] {
        html::find_all(nodes, tag, &mut all);
    }
    fn role_main<'a>(nodes: &'a [Node], out: &mut Vec<&'a Element>) {
        for n in nodes {
            if let Node::Element(e) = n {
                if e.attr("role") == Some("main") {
                    out.push(e);
                }
                role_main(&e.children, out);
            }
        }
    }
    role_main(nodes, &mut all);
    all.into_iter().filter(|e| !negative(e)).max_by_key(|e| el_len(e)).filter(|e| el_len(e) as f32 >= MIN_ARTICLE_CHARS && e.children.iter().map(link_len).sum::<usize>() * 2 < el_len(e))
}

fn keep_attrs(e: &Element) -> Vec<(String, String)> {
    let wanted: &[&str] = match e.tag.as_str() {
        "a" => &["href"],
        "img" => &["src", "data-src", "alt", "width", "height"],
        "ol" => &["start"],
        _ => &[],
    };
    e.attrs.iter().filter(|(k, _)| wanted.contains(&k.as_str())).cloned().collect()
}

fn has_content(e: &Element) -> bool {
    if matches!(e.tag.as_str(), "img" | "hr" | "br") {
        return true;
    }
    e.children.iter().any(|c| match c {
        Node::Text(t) => !t.trim().is_empty(),
        Node::Element(x) => has_content(x),
    })
}

fn clean(nodes: &[Node], out: &mut Vec<Node>) {
    for n in nodes {
        match n {
            Node::Text(t) => out.push(Node::Text(t.clone())),
            Node::Element(e) => {
                if DROP.contains(&e.tag.as_str()) || negative(e) {
                    continue;
                }
                let mut kids = Vec::new();
                clean(&e.children, &mut kids);
                let tag = if KEEP.contains(&e.tag.as_str()) {
                    e.tag.as_str()
                } else if BLOCKISH.contains(&e.tag.as_str()) {
                    "div"
                } else {
                    // spans and unknown inline wrappers dissolve into their text
                    out.extend(kids);
                    continue;
                };
                let el = Element { tag: tag.to_string(), attrs: keep_attrs(e), children: kids };
                if !has_content(&el) {
                    continue;
                }
                let node = Node::Element(el);
                let (len, links) = (text_len(&node), link_len(&node));
                if matches!(tag, "div" | "ul" | "ol") && len > 0 && len < 600 && links as f32 / len as f32 > 0.7 {
                    continue;
                }
                out.push(node);
            }
        }
    }
}

/// The article of a parsed page, or None when it has none worth reading.
pub fn extract(dom: &[Node]) -> Option<Article> {
    let root: Vec<Node> = match find_semantic(dom) {
        Some(e) => e.children.clone(),
        None => {
            let mut scores = HashMap::new();
            score(dom, &mut Vec::new(), &mut scores);
            let (path, best) = scores.into_iter().filter(|(p, _)| node_at(dom, p).is_some_and(|e| !negative(e))).max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
            if best < MIN_ARTICLE_CHARS {
                return None;
            }
            node_at(dom, &path)?.children.clone()
        }
    };
    let mut nodes = Vec::new();
    clean(&root, &mut nodes);
    let page_title = html::title(dom);
    let first_h1 = nodes.iter().find_map(|n| n.element().filter(|e| e.tag == "h1").map(|_| n.text_content().split_ascii_whitespace().collect::<Vec<_>>().join(" ")));
    let title = first_h1.clone().or(page_title).unwrap_or_default();
    if first_h1.is_none() && !title.is_empty() {
        nodes.insert(0, Node::Element(Element { tag: "h1".into(), attrs: vec![], children: vec![Node::Text(title.clone())] }));
    }
    nodes.iter().any(|n| text_len(n) > 0).then_some(Article { title, nodes })
}

/// The reader document for `article`, ready for the style engine.
pub fn document(article: &Article) -> Vec<Node> {
    let body = Element {
        tag: "body".into(),
        attrs: vec![],
        children: vec![Node::Element(Element { tag: "div".into(), attrs: vec![("class".into(), "article".into())], children: article.nodes.clone() })],
    };
    vec![Node::Element(body)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOREM: &str = "The quick brown fox jumps over the lazy dog while the rest of the village sleeps, and nobody notices.";

    fn page() -> Vec<Node> {
        html::parse(&format!(
            "<html><head><title>Fox News</title><script>track()</script></head><body>\
             <nav><ul><li><a href=/>Home</a></li><li><a href=/a>About</a></li></ul></nav>\
             <div class=content><h1>The Fox</h1><p>{LOREM}</p><p>{LOREM} <span>More</span> <a href=/x>link</a>.</p>\
             <div class=share>Share this</div><ul><li>one</li><li>two</li></ul><img src=a.png alt=pic></div>\
             <div id=footer><p>copyright and legal things nobody wants to read in reader mode ok</p></div></body></html>"
        ))
    }

    fn all_text(a: &Article) -> String {
        a.nodes.iter().map(Node::text_content).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn the_densest_text_block_is_the_article() {
        let a = extract(&page()).expect("an article");
        let text = all_text(&a);
        assert!(text.contains("quick brown fox"));
        assert!(text.contains("More link."));
        assert!(!text.contains("Home") && !text.contains("copyright") && !text.contains("Share this") && !text.contains("track"));
        assert_eq!(a.title, "The Fox");
    }

    #[test]
    fn semantic_article_wins_and_spans_dissolve() {
        let dom = html::parse(&format!("<div><p>{LOREM}</p></div><article><h1>T</h1><p>{LOREM} <b>bold</b> and <span class=x>plain</span> text here.</p><p>{LOREM}</p></article>"));
        let a = extract(&dom).unwrap();
        assert_eq!(a.nodes.len(), 3);
        let p = a.nodes[1].element().unwrap();
        assert!(p.children.iter().all(|c| !matches!(c, Node::Element(e) if e.tag == "span")));
        assert!(p.children.iter().any(|c| matches!(c, Node::Element(e) if e.tag == "b")));
    }

    #[test]
    fn a_missing_h1_is_added_from_the_page_title() {
        let dom = html::parse(&format!("<title>Page Title</title><article><p>{LOREM}</p><p>{LOREM}</p></article>"));
        let a = extract(&dom).unwrap();
        assert_eq!(a.title, "Page Title");
        assert_eq!(a.nodes[0].element().unwrap().tag, "h1");
        assert_eq!(a.nodes[0].text_content(), "Page Title");
    }

    #[test]
    fn link_farms_and_empty_blocks_are_dropped() {
        let dom = html::parse(&format!(
            "<article><p>{LOREM}</p><p>{LOREM}</p><div><a href=1>one</a> <a href=2>two</a> <a href=3>three</a></div><p>   </p><div><div></div></div></article>"
        ));
        let a = extract(&dom).unwrap();
        assert!(a.nodes.iter().all(|n| n.element().is_some_and(|e| e.tag != "div")), "{:?}", a.nodes);
    }

    #[test]
    fn short_pages_have_no_article() {
        assert_eq!(extract(&html::parse("<p>Hello there, this is short.</p><p>Tiny.</p>")), None);
        assert_eq!(extract(&html::parse("")), None);
    }

    #[test]
    fn images_and_links_keep_only_their_useful_attributes() {
        let dom = html::parse(&format!("<article><p>{LOREM}</p><p>{LOREM} <a href=/y class=zz onclick=bad()>y</a></p><img src=p.jpg width=9 style=x class=c></article>"));
        let a = extract(&dom).unwrap();
        let p = a.nodes[1].element().unwrap();
        let link = p.children.iter().find_map(|c| c.element().filter(|e| e.tag == "a")).unwrap();
        assert_eq!(link.attrs, vec![("href".to_string(), "/y".to_string())]);
        let img = a.nodes[2].element().unwrap();
        assert_eq!(img.attrs, vec![("src".to_string(), "p.jpg".to_string()), ("width".to_string(), "9".to_string())]);
    }

    #[test]
    fn the_reader_document_is_styled_by_the_reader_sheet() {
        use crate::shell::browser::css;
        let a = extract(&page()).unwrap();
        let mut rules = css::parse_stylesheet(css::USER_AGENT);
        rules.extend(css::parse_stylesheet(READER_CSS));
        let tree = css::style_tree(&document(&a), &rules);
        assert_eq!(tree[0].tag(), Some("body"));
        assert_eq!(tree[0].style.background, Some(css::Color::rgb(0xf7, 0xf3, 0xea)));
        let article = &tree[0].children()[0];
        assert_eq!(article.style.font_size, 20.0);
        assert_eq!(article.style.max_width, css::Len::Px(680.0));
    }
}
