//! Elements panel of the Web Inspector: a read-only DOM tree, fetched through injected script
//! (`control::js_tree`) and flattened into rows. Rows show the tag with its attributes; a tap folds or unfolds
//! a branch, and a tap on an element selects it: the page gets a highlight overlay and the panel shows its box and
//! attributes (`pagetools.rs`). A selector in the filter field keeps only the matching elements and their ancestors.
use std::collections::HashSet;

use bifrost_app::kit::ui::system::{SYSTEM_BROWN, SYSTEM_PURPLE};
use bifrost_app::kit::ui::{LABEL, LABEL_2, TINT, accent_now};
use bifrost_core::json::{self, Json};

use crate::control::{Cell, Row};

/// Most elements the panel asks the page for.
pub const MAX_NODES: usize = 1500;
/// Branches deeper than this start folded.
const OPEN_DEPTH: usize = 3;
/// Indent per level, points.
pub const INDENT_PT: f32 = 12.0;
/// Cell text prefix: disclosure triangle (`c` closed, `o` open, `l` leaf) and indent guides, drawn by the panel.
pub const MARK: &str = "\u{3}";
/// Cell text prefix: syntax runs, each `kind` char plus text, separated by `SEP`.
pub const RUNS: &str = "\u{2}";
pub const SEP: char = '\u{1f}';
/// Space the triangle takes before the tag text.
const TRI_PT: f32 = 12.0;

/// Colour of a run kind: `t` tag, `a` attribute name, `v` attribute value, anything else punctuation and text.
pub fn run_color(kind: char) -> u32 {
    match kind {
        't' => SYSTEM_PURPLE.now(),
        'a' => SYSTEM_BROWN.now(),
        'v' => TINT,
        'x' => LABEL.now(),
        _ => LABEL_2.now(),
    }
}

fn runs_open_tag(n: &Node) -> Vec<(char, String)> {
    let mut r = vec![('p', "<".to_string()), ('t', n.tag.clone())];
    let mut attr = |k: &str, v: &str| {
        r.push(('p', " ".into()));
        r.push(('a', k.to_string()));
        r.push(('p', "=\"".into()));
        r.push(('v', v.to_string()));
        r.push(('p', "\"".into()));
    };
    if !n.id.is_empty() {
        attr("id", &n.id);
    }
    if !n.class.is_empty() {
        attr("class", &n.class);
    }
    for (k, v) in &n.attrs {
        attr(k, v);
    }
    r.push(('p', ">".into()));
    r
}

pub fn encode(runs: Vec<(char, String)>) -> String {
    let body: Vec<String> = runs.into_iter().map(|(k, t)| format!("{k}{t}")).collect();
    format!("{RUNS}{}", body.join(&SEP.to_string()))
}

fn close_runs(tag: &str) -> Vec<(char, String)> {
    vec![('p', "</".into()), ('t', tag.to_string()), ('p', ">".into())]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub tag: String,
    /// element-index path from `<html>` (`0.1.3`), the address `pagetools::js_highlight` takes; empty if unknown
    pub path: String,
    pub id: String,
    pub class: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Node>,
    pub matched: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub root: Option<Node>,
    pub nodes: usize,
    pub truncated: bool,
}

fn node_from(v: &Json) -> Node {
    let s = |k: &str| v.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let attrs = match v.get("a") {
        Some(Json::Obj(fields)) => fields.iter().map(|(k, val)| (k.clone(), val.as_str().unwrap_or("").to_string())).collect(),
        _ => Vec::new(),
    };
    let children = match v.get("k") {
        Some(Json::Arr(items)) => items.iter().map(node_from).collect(),
        _ => Vec::new(),
    };
    Node { tag: s("t"), path: s("p"), id: s("i"), class: s("c"), attrs, text: s("x"), children, matched: v.get("m").is_some() }
}

/// Reads the JSON string `control::js_tree` returns. `Err` carries the page's message (a bad selector, say).
pub fn parse(raw: &str) -> Result<Tree, String> {
    let v = json::parse(raw)?;
    if let Some(e) = v.get("error").and_then(Json::as_str) {
        return Err(e.to_string());
    }
    Ok(Tree {
        root: v.get("root").filter(|r| matches!(r, Json::Obj(_))).map(node_from),
        nodes: v.get("nodes").and_then(Json::as_u64).unwrap_or(0) as usize,
        truncated: matches!(v.get("truncated"), Some(Json::Bool(true))),
    })
}

/// `<div id="a" class="b" href="...">`
pub fn open_tag(n: &Node) -> String {
    let mut s = format!("<{}", n.tag);
    if !n.id.is_empty() {
        s.push_str(&format!(" id=\"{}\"", n.id));
    }
    if !n.class.is_empty() {
        s.push_str(&format!(" class=\"{}\"", n.class));
    }
    for (k, v) in &n.attrs {
        s.push_str(&format!(" {k}=\"{v}\""));
    }
    s.push('>');
    s
}

/// Opens the branch `key` (a node path) whatever its default fold state is.
pub fn unfold(toggled: &mut HashSet<String>, key: &str) {
    if key.matches('.').count() >= OPEN_DEPTH {
        toggled.insert(key.to_string());
    } else {
        toggled.remove(key);
    }
}

fn is_folded(key: &str, depth: usize, toggled: &HashSet<String>) -> bool {
    (depth >= OPEN_DEPTH) != toggled.contains(key)
}

fn walk(n: &Node, depth: usize, key: String, toggled: &HashSet<String>, selected: Option<&str>, out: &mut Vec<Row>) {
    let has_kids = !n.children.is_empty();
    let folded = has_kids && is_folded(&key, depth, toggled);
    let mark = match (has_kids, folded) {
        (false, _) => "l",
        (true, true) => "c",
        (true, false) => "o",
    };
    let leaf_text = !has_kids && !n.text.is_empty();
    let mut runs = runs_open_tag(n);
    if leaf_text {
        runs.push(('x', n.text.clone()));
        runs.extend(close_runs(&n.tag));
    } else if folded {
        runs.push(('p', "...".into()));
        runs.extend(close_runs(&n.tag));
    }
    let x = 10.0 + depth as f32 * INDENT_PT;
    let is_sel = selected.is_some_and(|p| !p.is_empty() && p == n.path);
    out.push(Row {
        cells: vec![
            Cell { x, w: TRI_PT, text: format!("{MARK}{mark}"), color: LABEL_2.now() },
            Cell { x: x + TRI_PT, w: 4000.0, text: encode(runs), color: if n.matched || is_sel { accent_now() } else { LABEL.now() } },
        ],
        copy: open_tag(n),
        toggle: if has_kids { Some(key.clone()) } else { None },
        select: if n.path.is_empty() { None } else { Some(n.path.clone()) },
    });
    if has_kids && !folded {
        if !n.text.is_empty() {
            out.push(Row {
                cells: vec![
                    Cell { x: x + INDENT_PT, w: TRI_PT, text: format!("{MARK}l"), color: LABEL_2.now() },
                    Cell { x: x + INDENT_PT + TRI_PT, w: 4000.0, text: format!("\"{}\"", n.text), color: LABEL_2.now() },
                ],
                copy: n.text.clone(),
                toggle: None,
                select: None,
            });
        }
        for (i, c) in n.children.iter().enumerate() {
            let ck = if c.path.is_empty() { format!("{key}.{i}") } else { c.path.clone() };
            walk(c, depth + 1, ck, toggled, selected, out);
        }
        out.push(Row {
            cells: vec![
                Cell { x, w: TRI_PT, text: format!("{MARK}l"), color: LABEL_2.now() },
                Cell { x: x + TRI_PT, w: 4000.0, text: encode(close_runs(&n.tag)), color: LABEL_2.now() },
            ],
            copy: format!("</{}>", n.tag),
            toggle: None,
            select: None,
        });
    }
}

/// The rows of a tree; `toggled` holds the branches the user flipped from their default fold state.
pub fn rows(tree: &Tree, toggled: &HashSet<String>, selected: Option<&str>) -> Vec<Row> {
    let mut out = Vec::new();
    if let Some(root) = &tree.root {
        walk(root, 0, "0".into(), toggled, selected, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_and_folds() {
        let raw = r#"{"root":{"t":"html","k":[{"t":"body","i":"b","k":[{"t":"p","x":"hi","m":1}]}]},"nodes":3,"truncated":false}"#;
        let tree = parse(raw).unwrap();
        let rows_open = rows(&tree, &HashSet::new(), None);
        assert_eq!(rows_open.len(), 5);
        assert!(rows_open[2].cells[1].text.contains("hi"));
        let mut t = HashSet::new();
        t.insert("0.0".to_string());
        assert_eq!(rows(&tree, &t, None).len(), 3);
        assert!(parse(r#"{"error":"bad"}"#).is_err());
    }

    fn leaf(tag: &str, text: &str) -> Node {
        Node { tag: tag.into(), text: text.into(), ..Node::default() }
    }

    #[test]
    fn a_selected_or_matched_row_is_drawn_in_the_chosen_accent_not_system_blue() {
        use bifrost_app::kit::appearance::Accent;
        let tree = Tree { root: Some(Node { tag: "div".into(), path: "0".into(), ..Node::default() }), ..Tree::default() };
        let (accent, selected, plain) = crate::tokens::with_accent(Accent::Green, || {
            (bifrost_app::kit::ui::accent_now(), rows(&tree, &HashSet::new(), Some("0")), rows(&tree, &HashSet::new(), None))
        });
        assert_eq!(selected[0].cells[1].color, accent);
        assert_ne!(selected[0].cells[1].color, TINT);
        assert_ne!(plain[0].cells[1].color, accent, "an unselected row keeps the label colour");
    }

    #[test]
    fn attribute_values_keep_the_literal_syntax_colour() {
        assert_eq!(run_color('v'), TINT);
    }

    #[test]
    fn open_tag_runs_and_text() {
        let n = Node { tag: "a".into(), id: "x".into(), class: "c".into(), attrs: vec![("href".into(), "/".into())], ..Node::default() };
        assert_eq!(open_tag(&n), "<a id=\"x\" class=\"c\" href=\"/\">");
        let kinds: String = runs_open_tag(&n).iter().map(|r| r.0).collect();
        assert_eq!(kinds, "ptpapvppapvppapvpp");
        let enc = encode(close_runs("a"));
        assert!(enc.starts_with(RUNS));
        assert_eq!(enc[RUNS.len()..].split(SEP).collect::<Vec<_>>(), vec!["p</", "ta", "p>"]);
    }

    #[test]
    fn leaf_with_text_closes_on_one_row_and_folded_branch_shows_dots() {
        let tree = Tree { root: Some(Node { tag: "div".into(), path: "0".into(), children: vec![leaf("p", "hi")], ..Node::default() }), ..Tree::default() };
        let r = rows(&tree, &HashSet::new(), None);
        assert_eq!(r.len(), 3);
        assert!(r[1].cells[1].text.contains("xhi"));
        assert!(r[1].cells[1].text.ends_with("p>"));
        assert_eq!(r[1].copy, "<p>");
        assert_eq!(r[2].copy, "</div>");
        let mut t = HashSet::new();
        t.insert("0".to_string());
        let folded = rows(&tree, &t, None);
        assert_eq!(folded.len(), 1);
        assert!(folded[0].cells[1].text.contains("p..."));
    }



}
