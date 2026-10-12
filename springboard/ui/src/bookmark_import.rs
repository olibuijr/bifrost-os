//! Reads a bookmark export in the HTML form that browsers write: a nested list of folders (a heading and a list
//! block) and links (anchors with an address). The result is a tree of folders and links, or the flat list of the
//! links with their folder paths. Pure parser: the caller reads the file and stores what it keeps. Tags, comments
//! and attribute quoting are handled, entities in titles and addresses are decoded, and links with no address are
//! skipped. Anything that is not a folder or a link is ignored.

/// One entry of the export, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Folder { title: String, children: Vec<Node> },
    Link { title: String, url: String },
}

/// A link with the titles of the folders around it, outermost first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub folders: Vec<String>,
    pub title: String,
    pub url: String,
}

/// What the text being read belongs to.
enum Target {
    Folder,
    Link(String),
}

struct Frame {
    title: String,
    children: Vec<Node>,
}

/// Parses an export into its top-level entries. Unclosed folders are closed at the end of the input. A stray list
/// close is ignored. A link title that is empty falls back to the address.
pub fn parse(html: &str) -> Vec<Node> {
    let mut stack = vec![Frame { title: String::new(), children: Vec::new() }];
    let mut heading: Option<String> = None;
    let mut collecting: Option<(Target, String)> = None;
    // the file's outermost list has no heading: its entries belong to the root, not to an untitled folder
    let mut root_list_seen = false;
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        if let Some((_, text)) = collecting.as_mut() {
            text.push_str(&rest[..start]);
        }
        let after = &rest[start..];
        if after.starts_with("<!--") {
            match after[4..].find("-->") {
                Some(end) => rest = &after[4 + end + 3..],
                None => return finish(stack),
            }
            continue;
        }
        let end = match tag_end(after) {
            Some(end) => end,
            None => break,
        };
        let tag = &after[1..end];
        rest = &after[end + 1..];
        let closing = tag.starts_with('/');
        let body = if closing { &tag[1..] } else { tag };
        let name = body.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_ascii_lowercase();
        match (closing, name.as_str()) {
            (false, "h3") => collecting = Some((Target::Folder, String::new())),
            (true, "h3") => {
                if let Some((Target::Folder, text)) = collecting.take() {
                    heading = Some(decode(text.trim()));
                }
            }
            (false, "dl") => {
                if heading.is_none() && stack.len() == 1 && !root_list_seen {
                    root_list_seen = true;
                    continue;
                }
                let title = heading.take().unwrap_or_default();
                stack.push(Frame { title, children: Vec::new() });
            }
            (true, "dl") => {
                heading = None;
                if stack.len() > 1 {
                    let frame = stack.pop().expect("frame above the root");
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(Node::Folder { title: frame.title, children: frame.children });
                    }
                }
            }
            (false, "a") => {
                collecting = href(body).map(|href| (Target::Link(href), String::new()));
            }
            (true, "a") => {
                if let Some((Target::Link(url), text)) = collecting.take() {
                    let url = decode(url.trim());
                    if !url.is_empty() {
                        let title = decode(text.trim());
                        let title = if title.is_empty() { url.clone() } else { title };
                        if let Some(frame) = stack.last_mut() {
                            frame.children.push(Node::Link { title, url });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    finish(stack)
}

/// The links of a tree with their folder paths, in file order.
pub fn bookmarks(nodes: &[Node]) -> Vec<Bookmark> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    collect(nodes, &mut path, &mut out);
    out
}

fn collect(nodes: &[Node], path: &mut Vec<String>, out: &mut Vec<Bookmark>) {
    for node in nodes {
        match node {
            Node::Folder { title, children } => {
                path.push(title.clone());
                collect(children, path, out);
                path.pop();
            }
            Node::Link { title, url } => {
                out.push(Bookmark { folders: path.clone(), title: title.clone(), url: url.clone() });
            }
        }
    }
}

/// Closes every folder still open and returns the top-level entries.
fn finish(mut stack: Vec<Frame>) -> Vec<Node> {
    while stack.len() > 1 {
        let frame = stack.pop().expect("frame above the root");
        if let Some(parent) = stack.last_mut() {
            parent.children.push(Node::Folder { title: frame.title, children: frame.children });
        }
    }
    stack.pop().map(|root| root.children).unwrap_or_default()
}

/// The index of the `>` that closes the tag at the start of `s`. A `>` inside a quoted attribute value does not count.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '>' => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// The value of the `href` attribute in a tag body, quoted or bare. Matching ignores case.
fn href(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let at = lower.find("href=")?;
    let value = body[at + 5..].trim_start();
    if let Some(quoted) = value.strip_prefix('"') {
        quoted.split('"').next().map(str::to_string)
    } else if let Some(quoted) = value.strip_prefix('\'') {
        quoted.split('\'').next().map(str::to_string)
    } else {
        Some(value.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_string())
    }
}

/// Decodes the named entities that exports use and numeric character references. Anything else is left as written.
fn decode(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let decoded = tail.find(';').filter(|&e| e <= 10).and_then(|e| entity(&tail[1..e]).map(|c| (c, e)));
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &tail[e + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(title: &str, url: &str) -> Node {
        Node::Link { title: title.to_string(), url: url.to_string() }
    }

    fn folder(title: &str, children: Vec<Node>) -> Node {
        Node::Folder { title: title.to_string(), children }
    }

    const EXPORT: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<TITLE>Bookmarks</TITLE>
<DL><p>
    <DT><H3 ADD_DATE="1">Work</H3>
    <DL><p>
        <DT><A HREF="https://docs.test/a?x=1&amp;y=2">Plan &amp; notes</A>
        <DD>description text
        <DT><H3>Nested</H3>
        <DL><p>
            <DT><A HREF="https://deep.test/">Deep</A>
        </DL><p>
    </DL><p>
    <DT><A HREF="https://home.test/">Home</A>
    <DT><A HREF="">Empty address</A>
    <DT><A HREF="https://blank.test/"></A>
</DL><p>
"#;

    #[test]
    fn the_folders_and_links_keep_their_order_and_nesting() {
        let expected = vec![
            folder(
                "Work",
                vec![
                    link("Plan & notes", "https://docs.test/a?x=1&y=2"),
                    folder("Nested", vec![link("Deep", "https://deep.test/")]),
                ],
            ),
            link("Home", "https://home.test/"),
            link("https://blank.test/", "https://blank.test/"),
        ];
        assert_eq!(parse(EXPORT), expected);
    }

    #[test]
    fn the_flat_list_gives_each_link_its_folder_path() {
        let tree = parse(EXPORT);
        let flat = bookmarks(&tree);
        assert_eq!(flat.len(), 4);
        assert_eq!(flat[0].folders, vec!["Work".to_string()]);
        assert_eq!(flat[0].title, "Plan & notes");
        assert_eq!(flat[1].folders, vec!["Work".to_string(), "Nested".to_string()]);
        assert_eq!(flat[1].url, "https://deep.test/");
        assert!(flat[2].folders.is_empty());
        assert_eq!(flat[2].title, "Home");
        assert_eq!(flat[3].title, "https://blank.test/");
    }

    #[test]
    fn entities_decode_once_and_unknown_ones_stay_as_written() {
        assert_eq!(decode("a &amp;amp; b &#65;&#x42; &bogus; &lt;"), "a &amp; b AB &bogus; <");
        assert_eq!(decode("no entities & here"), "no entities & here");
        assert_eq!(decode("&quot;q&quot; &apos;"), "\"q\" '");
    }

    #[test]
    fn comments_are_skipped_and_unclosed_folders_are_closed_at_the_end() {
        // the folder F is never closed; it still holds Q, and the commented-out list is ignored
        let html = "<!-- <DL> --> <DL><DT><H3>F</H3><DL><DT><A HREF='https://q.test/'>Q</A>";
        assert_eq!(parse(html), vec![folder("F", vec![link("Q", "https://q.test/")])]);
    }

    #[test]
    fn a_gt_inside_a_quoted_address_does_not_end_the_tag() {
        let html = r#"<DL><DT><A HREF="https://r.test/?a>b">R</A></DL>"#;
        assert_eq!(parse(html), vec![link("R", "https://r.test/?a>b")]);
    }

    #[test]
    fn a_stray_list_close_is_ignored() {
        let html = r#"</DL></DL><DT><A HREF="https://s.test/">S</A>"#;
        assert_eq!(parse(html), vec![link("S", "https://s.test/")]);
    }
}
