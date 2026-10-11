//! XHTML subset to blocks: headings, paragraphs (div, li, blockquote, table
//! rows and friends flush like paragraphs), `<br>`, `<pre>` and images.
//! Inline markup is dropped, its text kept.

use super::markup::{local, tokens, Tok};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Raw {
    Heading(u8, String),
    Para(String),
    /// the `src` of an image, unresolved
    Image(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub blocks: Vec<Raw>,
    /// `id` attribute -> index of the block the id sits in or precedes
    pub anchors: Vec<(String, usize)>,
}

const BLOCKS: [&str; 22] = ["p", "div", "li", "blockquote", "tr", "section", "article", "ul", "ol", "table", "dl", "dt", "dd", "header", "footer", "figure", "figcaption", "aside", "body", "hr", "caption", "pre"];

struct State {
    out: Parsed,
    buf: String,
    heading: Option<u8>,
    bullet: bool,
    pre: bool,
}

impl State {
    fn flush(&mut self) {
        let text = self.buf.trim_matches(|c: char| c == ' ' || c == '\n' || c == '\u{a0}').to_string();
        self.buf.clear();
        let bullet = std::mem::take(&mut self.bullet);
        if text.is_empty() {
            return;
        }
        let text = if bullet { format!("• {text}") } else { text };
        match self.heading {
            Some(l) => self.out.blocks.push(Raw::Heading(l, text)),
            None => self.out.blocks.push(Raw::Para(text)),
        }
    }

    fn text(&mut self, t: &str) {
        if self.pre {
            self.buf.push_str(&t.replace('\r', ""));
            return;
        }
        for c in t.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                if !self.buf.is_empty() && !self.buf.ends_with([' ', '\n']) {
                    self.buf.push(' ');
                }
            } else {
                self.buf.push(c);
            }
        }
    }
}

pub fn parse(src: &str) -> Parsed {
    let mut st = State { out: Parsed::default(), buf: String::new(), heading: None, bullet: false, pre: false };
    let mut skip_head = 0usize;
    for tok in tokens(src) {
        match &tok {
            Tok::Open { name, selfclose, .. } => {
                let n = local(name);
                if n == "head" && !selfclose {
                    skip_head += 1;
                }
                if skip_head > 0 {
                    continue;
                }
                if let Some(id) = tok.attr("id") {
                    st.out.anchors.push((id.to_string(), st.out.blocks.len()));
                }
                let level = n.strip_prefix('h').and_then(|d| d.parse::<u8>().ok()).filter(|l| (1..=6).contains(l));
                if let Some(l) = level {
                    st.flush();
                    st.heading = Some(l);
                } else if n == "br" {
                    if !st.buf.is_empty() && !st.buf.ends_with('\n') {
                        st.buf.push('\n');
                    }
                } else if n == "img" || n == "image" {
                    st.flush();
                    let src = tok.attr("src").or_else(|| tok.attr("xlink:href")).or_else(|| tok.attr("href"));
                    if let Some(s) = src {
                        st.out.blocks.push(Raw::Image(s.to_string()));
                    }
                } else if BLOCKS.contains(&n) {
                    st.flush();
                    if n == "li" {
                        st.bullet = true;
                    }
                    if n == "pre" && !selfclose {
                        st.pre = true;
                    }
                }
            }
            Tok::Close(name) => {
                let n = local(name);
                if n == "head" {
                    skip_head = skip_head.saturating_sub(1);
                    continue;
                }
                if skip_head > 0 {
                    continue;
                }
                let is_heading = n.strip_prefix('h').and_then(|d| d.parse::<u8>().ok()).is_some_and(|l| (1..=6).contains(&l));
                if is_heading {
                    st.flush();
                    st.heading = None;
                } else if BLOCKS.contains(&n) {
                    st.flush();
                    if n == "pre" {
                        st.pre = false;
                    }
                }
            }
            Tok::Text(t) => {
                if skip_head == 0 {
                    st.text(t);
                }
            }
        }
    }
    st.flush();
    st.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_paragraphs_and_inline_markup() {
        let p = parse("<html><head><title>T</title></head><body><h1 id=\"top\">Chapter  One</h1><p>It was <em>a</em>\n dark &amp; stormy night.</p><p>Next</p></body></html>");
        assert_eq!(p.blocks, vec![Raw::Heading(1, "Chapter One".into()), Raw::Para("It was a dark & stormy night.".into()), Raw::Para("Next".into())]);
        assert_eq!(p.anchors, vec![("top".to_string(), 0)]);
    }

    #[test]
    fn lists_breaks_images_and_pre() {
        let p = parse("<body><ul><li>one</li><li>two</li></ul><p>a<br/>b</p><img src=\"../i/c.jpg\" alt=\"\"/><pre>x  y\n z</pre></body>");
        assert_eq!(p.blocks, vec![Raw::Para("• one".into()), Raw::Para("• two".into()), Raw::Para("a\nb".into()), Raw::Image("../i/c.jpg".into()), Raw::Para("x  y\n z".into())]);
    }

    #[test]
    fn anchors_point_at_the_block_they_precede_or_sit_in() {
        let p = parse("<body><p>a</p><span id=\"s\"></span><p id=\"p2\">b</p><div>c <a id=\"mid\">x</a></div></body>");
        let at = |id: &str| p.anchors.iter().find(|(k, _)| k == id).map(|(_, i)| *i);
        assert_eq!(at("s"), Some(1));
        assert_eq!(at("p2"), Some(1));
        assert_eq!(at("mid"), Some(2));
    }

    #[test]
    fn svg_image_href_and_scripts() {
        let p = parse("<body><svg><image xlink:href=\"cover.png\"/></svg><script>var x = '<p>no</p>';</script><p>yes</p></body>");
        assert_eq!(p.blocks, vec![Raw::Image("cover.png".into()), Raw::Para("yes".into())]);
    }
}
