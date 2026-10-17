//! Share as HTML: a note becomes one standalone HTML page. Block styles come from `md::classify` and inline styles from
//! `md::spans`, the same rules the editor shows. Every piece of text is escaped, and the page carries its own small
//! stylesheet, so it reads the same without a network or a font.

use springboard::shell::md::{self, Block, Style};

/// Escapes text for HTML element content and quoted attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// One line's inline content with its emphasis as elements. `content` is the line without its block marker.
fn inline(content: &str) -> String {
    md::spans(content, 0)
        .into_iter()
        .filter(|s| !s.marker)
        .map(|s| {
            let text = escape(&content.chars().skip(s.range.start).take(s.range.len()).collect::<String>());
            match s.style {
                Style::Plain => text,
                Style::Bold => format!("<strong>{text}</strong>"),
                Style::Italic => format!("<em>{text}</em>"),
                Style::Code => format!("<code>{text}</code>"),
                Style::Strike => format!("<s>{text}</s>"),
                Style::Underline => format!("<u>{text}</u>"),
                Style::Highlight => format!("<mark>{text}</mark>"),
                Style::Math => format!("<code class=\"math\">{text}</code>"),
            }
        })
        .collect()
}

/// The HTML page for a note: its title, then its Markdown as headings, paragraphs, lists, checklists, quotes and code.
pub fn render(title: &str, markdown: &str) -> String {
    let mut body = String::new();
    let mut list: Option<&'static str> = None;
    for line in markdown.split('\n') {
        let info = md::classify(line);
        let content: String = line.chars().skip(info.prefix).collect();
        let kind = match info.block {
            Block::Bullet | Block::Dashed | Block::Check(_) => Some("ul"),
            Block::Numbered(_) => Some("ol"),
            _ => None,
        };
        if kind != list {
            if let Some(open) = list {
                body.push_str(&format!("</{open}>\n"));
            }
            if let Some(open) = kind {
                body.push_str(&format!("<{open}>\n"));
            }
            list = kind;
        }
        match info.block {
            Block::Title => body.push_str(&format!("<h1>{}</h1>\n", inline(&content))),
            Block::Heading => body.push_str(&format!("<h2>{}</h2>\n", inline(&content))),
            Block::Subheading => body.push_str(&format!("<h3>{}</h3>\n", inline(&content))),
            Block::Bullet | Block::Dashed | Block::Numbered(_) => body.push_str(&format!("<li>{}</li>\n", inline(&content))),
            Block::Check(done) => {
                let checked = if done { " checked" } else { "" };
                body.push_str(&format!("<li class=\"check\"><input type=\"checkbox\" disabled{checked}> {}</li>\n", inline(&content)));
            }
            Block::Quote => body.push_str(&format!("<blockquote>{}</blockquote>\n", inline(&content))),
            Block::Mono => body.push_str(&format!("<pre><code>{}</code></pre>\n", escape(&content))),
            Block::Body => {
                if !content.trim().is_empty() {
                    body.push_str(&format!("<p>{}</p>\n", inline(&content)));
                }
            }
        }
    }
    if let Some(open) = list {
        body.push_str(&format!("</{open}>\n"));
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>\nbody {{ font-family: -apple-system, system-ui, sans-serif; line-height: 1.5; max-width: 40em; margin: 2em auto; padding: 0 1em; }}\ncode, pre {{ font-family: ui-monospace, monospace; }}\npre {{ background: #f4f4f4; padding: 0.5em; overflow-x: auto; }}\nblockquote {{ border-left: 3px solid #ccc; margin-left: 0; padding-left: 1em; color: #555; }}\nmark {{ background: #fff3a3; }}\nli.check {{ list-style: none; }}\n</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        escape(title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_in_every_position() {
        assert_eq!(escape("a < b & \"c\" 'd' > e"), "a &lt; b &amp; &quot;c&quot; &#39;d&#39; &gt; e");
        let page = render("<Title>", "A <script> line");
        assert!(page.contains("<title>&lt;Title&gt;</title>"));
        assert!(page.contains("<p>A &lt;script&gt; line</p>"), "{page}");
        assert!(!page.contains("<script>"));
    }

    #[test]
    fn headings_paragraphs_and_inline_styles_become_elements() {
        let page = render("Note", "# Head\n## Sub\n### Third\nplain **bold** and *italic* and `code`");
        assert!(page.contains("<h1>Head</h1>"));
        assert!(page.contains("<h2>Sub</h2>"));
        assert!(page.contains("<h3>Third</h3>"));
        assert!(page.contains("<p>plain <strong>bold</strong> and <em>italic</em> and <code>code</code></p>"), "{page}");
    }

    #[test]
    fn lists_group_their_items_and_close_at_the_end() {
        let page = render("L", "- one\n- two\n1. first\n- [x] done\n- [ ] open");
        assert!(page.contains("<ul>\n<li>one</li>\n<li>two</li>\n</ul>"), "{page}");
        assert!(page.contains("<ol>\n<li>first</li>\n</ol>"));
        assert!(page.contains("<li class=\"check\"><input type=\"checkbox\" disabled checked> done</li>"));
        assert!(page.contains("<input type=\"checkbox\" disabled> open</li>"));
    }

    #[test]
    fn a_page_is_a_complete_document() {
        let page = render("Note", "");
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("<meta charset=\"utf-8\">"));
        assert!(page.trim_end().ends_with("</html>"));
    }
}
