//! Web Inspector depth (round 57): the Sources tab (view source with line numbers and syntax colours, search in
//! file), the Storage tab (localStorage, sessionStorage, document.cookie), the computed Styles of the selected
//! node, and the page scripts that edit attributes, text and storage entries. Everything here is pure (strings in,
//! rows or scripts out); `inspector.rs` owns the requests and the input.
use bifrost_app::kit::ui::{LABEL, LABEL_2, TINT};
use bifrost_core::json::{self, Json};

use crate::control::{js_str, Cell, Row};
use crate::elements::{self, Node, Tree};

/// Source bytes kept per tab; the rest is cut (a note row says so).
pub const MAX_SOURCE: usize = 400 * 1024;
/// Characters kept per source line.
const MAX_LINE: usize = 600;
/// Width of the line-number gutter, points.
const GUTTER_PT: f32 = 34.0;

type Runs = Vec<(char, String)>;

// ---- Sources: tokenizer ----

const JS_KEYWORDS: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "export", "extends", "false", "finally", "for",
    "from", "function", "if", "import", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try",
    "typeof", "undefined", "var", "void", "while", "yield",
];

fn push(out: &mut Runs, kind: char, text: &str) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some((k, t)) if *k == kind => t.push_str(text),
        _ => out.push((kind, text.to_string())),
    }
}

/// Index just past a quoted string starting at `i` (which holds the quote), honouring backslash escapes; always a
/// char boundary and at most `s.len()`.
fn string_end(s: &str, i: usize, multiline: bool) -> usize {
    let b = s.as_bytes();
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == q => {
                j += 1;
                break;
            }
            b'\n' if !multiline => break,
            _ => j += 1,
        }
    }
    let mut j = j.min(s.len());
    while !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

fn find_from(s: &str, from: usize, pat: &str) -> usize {
    if from >= s.len() {
        return s.len();
    }
    s[from..].find(pat).map_or(s.len(), |p| from + p)
}

/// Tokenizes script text: comments `p`, strings `v`, numbers `a`, keywords `t`, anything else `x`.
fn js_runs(s: &str, out: &mut Runs) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let end = if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let e = find_from(s, i, "\n");
            push(out, 'p', &s[i..e]);
            e
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let e = (find_from(s, i + 2, "*/") + 2).min(s.len());
            push(out, 'p', &s[i..e]);
            e
        } else if c == b'"' || c == b'\'' || c == b'`' {
            let e = string_end(s, i, c == b'`');
            push(out, 'v', &s[i..e]);
            e
        } else if c.is_ascii_digit() {
            let mut e = i;
            while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'.' || b[e] == b'_') {
                e += 1;
            }
            push(out, 'a', &s[i..e]);
            e
        } else if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let mut e = i;
            while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_' || b[e] == b'$') {
                e += 1;
            }
            let w = &s[i..e];
            push(out, if JS_KEYWORDS.contains(&w) { 't' } else { 'x' }, w);
            e
        } else {
            let e = i + s[i..].chars().next().map_or(1, char::len_utf8);
            push(out, 'x', &s[i..e]);
            e
        };
        i = end;
    }
}

/// Tokenizes stylesheet text: selectors `t`, property names `a`, strings and values `v`, comments and punctuation `p`.
fn css_runs(s: &str, out: &mut Runs) {
    let b = s.as_bytes();
    let (mut i, mut depth) = (0, 0i32);
    while i < b.len() {
        let c = b[i];
        let is_punct = |c: u8| matches!(c, b'{' | b'}' | b';' | b':' | b',' | b'(' | b')');
        let e = if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let e = (find_from(s, i + 2, "*/") + 2).min(s.len());
            push(out, 'p', &s[i..e]);
            e
        } else if c == b'"' || c == b'\'' {
            let e = string_end(s, i, false);
            push(out, 'v', &s[i..e]);
            e
        } else if is_punct(c) {
            match c {
                b'{' => depth += 1,
                b'}' => depth = (depth - 1).max(0),
                _ => {}
            }
            push(out, 'p', &s[i..i + 1]);
            i + 1
        } else if c.is_ascii_whitespace() {
            push(out, 'x', &s[i..i + 1]);
            i + 1
        } else {
            let mut e = i;
            while e < b.len() && !b[e].is_ascii_whitespace() && !is_punct(b[e]) && b[e] != b'"' && b[e] != b'\'' {
                e += 1;
            }
            while !s.is_char_boundary(e) {
                e += 1;
            }
            let w = &s[i..e];
            let after = s[e..].trim_start();
            let kind = if depth == 0 {
                't'
            } else if after.starts_with(':') && !w.starts_with('#') {
                'a'
            } else {
                'v'
            };
            push(out, kind, w);
            e
        };
        i = e;
    }
}

/// Tokenizes one tag from `<` at `i`; returns the index after `>` and the lower-cased tag name.
fn tag_runs(s: &str, mut i: usize, out: &mut Runs) -> (usize, String) {
    let b = s.as_bytes();
    let start = i;
    i += 1;
    if b.get(i) == Some(&b'/') {
        i += 1;
    }
    if b.get(i) == Some(&b'!') {
        i += 1;
    }
    push(out, 'p', &s[start..i]);
    let ns = i;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
        i += 1;
    }
    while !s.is_char_boundary(i) {
        i += 1;
    }
    let name = s[ns..i].to_ascii_lowercase();
    push(out, 't', &s[ns..i]);
    while i < b.len() {
        let c = b[i];
        if c == b'>' {
            push(out, 'p', ">");
            i += 1;
            break;
        } else if c.is_ascii_whitespace() || c == b'/' || c == b'=' {
            push(out, 'p', &s[i..i + 1]);
            i += 1;
        } else if c == b'"' || c == b'\'' {
            let e = string_end(s, i, true);
            push(out, 'v', &s[i..e]);
            i = e;
        } else {
            let st = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'>' | b'=' | b'/' | b'"' | b'\'') {
                i += 1;
            }
            while !s.is_char_boundary(i) {
                i += 1;
            }
            // a bare word after "=" is a value, otherwise an attribute name
            let is_val = s[..st].trim_end().ends_with('=');
            push(out, if is_val { 'v' } else { 'a' }, &s[st..i]);
        }
    }
    (i, name)
}

/// The whole document as colour runs (kinds `t` tag, `a` attribute or number, `v` string, `p` punctuation and
/// comments, `x` text).
fn html_runs(s: &str) -> Runs {
    let mut out = Runs::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' && s[i..].starts_with("<!--") {
            let e = (find_from(s, i + 4, "-->") + 3).min(s.len());
            push(&mut out, 'p', &s[i..e]);
            i = e;
        } else if b[i] == b'<' && b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'/' || *c == b'!') {
            let closing = b.get(i + 1) == Some(&b'/');
            let (e, name) = tag_runs(s, i, &mut out);
            i = e;
            if !closing && (name == "script" || name == "style") {
                let lower = s[i..].to_ascii_lowercase();
                let stop = lower.find(&format!("</{name}")).map_or(s.len(), |p| i + p);
                if name == "script" {
                    js_runs(&s[i..stop], &mut out);
                } else {
                    css_runs(&s[i..stop], &mut out);
                }
                i = stop;
            }
        } else {
            // step past this character, not one byte: text can start with a multi-byte character
            let next = i + s[i..].chars().next().map_or(1, char::len_utf8);
            let e = s[next..].find('<').map_or(s.len(), |p| next + p);
            let mut e = e.min(s.len());
            while !s.is_char_boundary(e) {
                e += 1;
            }
            let mut st = i;
            while !s.is_char_boundary(st) {
                st += 1;
            }
            push(&mut out, 'x', &s[st..e]);
            i = e.max(i + 1);
        }
    }
    out
}

/// Splits runs at newlines into lines (tabs become two spaces, long lines are cut).
fn split_lines(runs: Runs) -> Vec<Runs> {
    let mut lines: Vec<Runs> = vec![Vec::new()];
    for (k, t) in runs {
        for (n, part) in t.split('\n').enumerate() {
            if n > 0 {
                lines.push(Vec::new());
            }
            let part = part.trim_end_matches('\r').replace('\t', "  ");
            if !part.is_empty() {
                lines.last_mut().expect("one line").push((k, part));
            }
        }
    }
    for l in &mut lines {
        let mut room = MAX_LINE;
        let mut keep = 0;
        for (_, t) in l.iter_mut() {
            let n = t.chars().count();
            keep += 1;
            if n >= room {
                *t = t.chars().take(room).collect();
                break;
            }
            room -= n;
        }
        l.truncate(keep);
    }
    lines
}

/// A tokenized document: one run list per line plus the plain text of each line (for search and copy).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    pub lines: Vec<Runs>,
    pub plain: Vec<String>,
    pub cut: bool,
}

pub fn tokenize(src: &str) -> Source {
    let cut = src.len() > MAX_SOURCE;
    let mut end = src.len().min(MAX_SOURCE);
    while !src.is_char_boundary(end) {
        end -= 1;
    }
    let lines = split_lines(html_runs(&src[..end]));
    let plain = lines.iter().map(|l| l.iter().map(|(_, t)| t.as_str()).collect()).collect();
    Source { lines, plain, cut }
}

fn note(text: String) -> Row {
    Row { cells: vec![Cell { x: 10.0, w: 4000.0, text, color: LABEL_2.now() }], copy: String::new(), toggle: None, select: None }
}

/// Rows of the Sources tab: a right-aligned line number and the coloured line. A non-empty `query` keeps the lines
/// that contain it (case-insensitive) and paints them in the tint colour; the first row says how many matched.
pub fn source_rows(src: &Source, query: &str) -> Vec<Row> {
    let q = query.trim().to_lowercase();
    let mut rows = Vec::new();
    let mut hits = 0;
    for (i, line) in src.lines.iter().enumerate() {
        let hit = !q.is_empty() && src.plain[i].to_lowercase().contains(&q);
        if !q.is_empty() && !hit {
            continue;
        }
        hits += usize::from(hit);
        rows.push(Row {
            cells: vec![
                Cell { x: 4.0, w: -GUTTER_PT, text: (i + 1).to_string(), color: LABEL_2.now() },
                Cell { x: 4.0 + GUTTER_PT + 8.0, w: 4000.0, text: elements::encode(line.clone()), color: if hit { TINT } else { LABEL.now() } },
            ],
            copy: src.plain[i].clone(),
            toggle: None,
            select: None,
        });
    }
    if !q.is_empty() {
        let head = if hits == 0 { "No matches".to_string() } else { format!("{hits} matching line{}", if hits == 1 { "" } else { "s" }) };
        rows.insert(0, note(head));
    }
    if src.cut {
        rows.push(note(format!("... source cut at {} KB", MAX_SOURCE / 1024)));
    }
    rows
}

// ---- Storage ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Local,
    Session,
    Cookie,
}

impl Store {
    pub fn name(self) -> &'static str {
        match self {
            Store::Local => "local",
            Store::Session => "session",
            Store::Cookie => "cookie",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Store::Local => "Local Storage",
            Store::Session => "Session Storage",
            Store::Cookie => "Cookies",
        }
    }
    fn parse(s: &str) -> Option<Store> {
        match s {
            "local" => Some(Store::Local),
            "session" => Some(Store::Session),
            "cookie" => Some(Store::Cookie),
            _ => None,
        }
    }
}

/// One page storage entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub store: Store,
    pub key: String,
    pub value: String,
}

/// Reads the storage of the page as `{local:[[k,v]..],session:[..],cookie:[..]}` (a store the page may not read is
/// empty). HttpOnly cookies are invisible to `document.cookie`.
pub const JS_STORAGE: &str = "(function(){function st(n){try{return window[n]}catch(e){return null}}\
    function lst(n){var s=st(n),r=[];if(s)for(var i=0;i<s.length&&i<500;i++){var k=s.key(i);r.push([k,String(s.getItem(k)).slice(0,4000)])}return r}\
    var c=[];try{c=document.cookie?document.cookie.split('; ').map(function(x){var i=x.indexOf('=');return i<0?[x,'']:[x.slice(0,i),x.slice(i+1)]}):[]}catch(e){}\
    return JSON.stringify({local:lst('localStorage'),session:lst('sessionStorage'),cookie:c})})()";

pub fn parse_storage(raw: &str) -> Result<Vec<Entry>, String> {
    let v = json::parse(raw)?;
    let mut out = Vec::new();
    for store in [Store::Local, Store::Session, Store::Cookie] {
        if let Some(Json::Arr(items)) = v.get(store.name()) {
            for it in items {
                if let Json::Arr(kv) = it {
                    let s = |i: usize| kv.get(i).and_then(Json::as_str).unwrap_or("").to_string();
                    out.push(Entry { store, key: s(0), value: s(1) });
                }
            }
        }
    }
    Ok(out)
}

/// The id a Storage row carries in `Row::select`.
pub fn entry_id(e: &Entry) -> String {
    format!("{}:{}", e.store.name(), e.key)
}

pub fn parse_entry_id(id: &str) -> Option<(Store, String)> {
    let (s, k) = id.split_once(':')?;
    Some((Store::parse(s)?, k.to_string()))
}

/// Rows of the Storage tab: a header per store, then `key  value` rows; `query` filters by key or value. The row of
/// `selected` (an `entry_id`) is tinted.
pub fn storage_rows(entries: &[Entry], query: &str, selected: Option<&str>, width: f32) -> Vec<Row> {
    let q = query.trim().to_lowercase();
    let mut rows = Vec::new();
    for store in [Store::Local, Store::Session, Store::Cookie] {
        let mine: Vec<&Entry> = entries.iter().filter(|e| e.store == store && (q.is_empty() || e.key.to_lowercase().contains(&q) || e.value.to_lowercase().contains(&q))).collect();
        rows.push(Row { cells: vec![Cell { x: 10.0, w: width - 20.0, text: format!("{} ({})", store.title(), mine.len()), color: LABEL.now() }], copy: String::new(), toggle: None, select: None });
        for e in mine {
            let id = entry_id(e);
            let on = selected == Some(id.as_str());
            let key_w = (width * 0.3).clamp(80.0, 240.0);
            rows.push(Row {
                cells: vec![
                    Cell { x: 22.0, w: key_w - 12.0, text: e.key.clone(), color: if on { TINT } else { LABEL.now() } },
                    Cell { x: 22.0 + key_w, w: (width - key_w - 32.0).max(40.0), text: e.value.replace('\n', " "), color: if on { TINT } else { LABEL_2.now() } },
                ],
                copy: e.value.clone(),
                toggle: None,
                select: Some(id),
            });
        }
    }
    rows
}

/// Sets (or, with `value` None, removes) an entry. Returns `"ok"` or an error string.
pub fn js_storage_edit(store: Store, key: &str, value: Option<&str>) -> String {
    format!(
        "(function(kind,key,val,del){{try{{if(kind==='cookie'){{document.cookie=del?key+'=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/':key+'='+val}}\
         else{{var s=window[kind==='local'?'localStorage':'sessionStorage'];if(del)s.removeItem(key);else s.setItem(key,val)}}return 'ok'}}catch(e){{return String(e)}}}})({},{},{},{})",
        js_str(store.name()),
        js_str(key),
        js_str(value.unwrap_or("")),
        value.is_none()
    )
}

// ---- Elements: edits and computed style ----

const JS_NODE: &str = "var e=document.documentElement,a=String(p).split('.').slice(1);for(var i=0;i<a.length&&e;i++)e=e.children[+a[i]];if(!e)return 'the node is gone';";

/// Sets attribute `name` of the node at `path` to `value`. Returns `"ok"` or an error string.
pub fn js_set_attr(path: &str, name: &str, value: &str) -> String {
    format!("(function(p,n,v){{{JS_NODE}try{{e.setAttribute(n,v);return 'ok'}}catch(x){{return String(x)}}}})({},{},{})", js_str(path), js_str(name), js_str(value))
}

/// Replaces the text of the node at `path` (refused for a node with element children).
pub fn js_set_text(path: &str, value: &str) -> String {
    format!("(function(p,v){{{JS_NODE}if(e.children.length)return 'the element has child elements';e.textContent=v;return 'ok'}})({},{})", js_str(path), js_str(value))
}

/// The computed style of the node at `path` as `[[property, value]..]` in a fixed short order.
pub fn js_styles_of(path: &str) -> String {
    format!(
        "(function(p){{{JS_NODE}var c=getComputedStyle(e),o=[];function g(n){{return c.getPropertyValue(n)}}function q(a){{return a.map(g).join(' ')}}\
         ['display','position','box-sizing','width','height'].forEach(function(n){{o.push([n,g(n)])}});\
         o.push(['margin',q(['margin-top','margin-right','margin-bottom','margin-left'])]);o.push(['border',q(['border-top-width','border-right-width','border-bottom-width','border-left-width'])]);\
         o.push(['padding',q(['padding-top','padding-right','padding-bottom','padding-left'])]);\
         ['color','background-color','font-family','font-size','font-weight','line-height','opacity','overflow'].forEach(function(n){{o.push([n,g(n)])}});\
         return JSON.stringify(o)}})({})",
        js_str(path)
    )
}

pub fn parse_styles(raw: &str) -> Result<Vec<(String, String)>, String> {
    match json::parse(raw)? {
        Json::Arr(items) => Ok(items
            .iter()
            .filter_map(|it| match it {
                Json::Arr(kv) => Some((kv.first()?.as_str()?.to_string(), kv.get(1)?.as_str()?.to_string())),
                _ => None,
            })
            .collect()),
        _ => Err(raw.to_string()),
    }
}

/// The "Computed Style" rows shown under the attributes of the selected node.
pub fn style_rows(styles: &[(String, String)], width: f32) -> Vec<Row> {
    if styles.is_empty() {
        return Vec::new();
    }
    let cell = |text: String, color: u32| Cell { x: 10.0, w: width - 20.0, text, color };
    let mut rows = vec![Row { cells: vec![cell("Computed Style".into(), LABEL.now())], copy: String::new(), toggle: None, select: None }];
    for (k, v) in styles {
        rows.push(Row { cells: vec![cell(format!("   {k}: {v}"), LABEL_2.now())], copy: format!("{k}: {v}"), toggle: None, select: None });
    }
    rows
}

/// The node at element-index `path` (`0.1.3`) of a tree.
pub fn find_node<'a>(tree: &'a Tree, path: &str) -> Option<&'a Node> {
    fn go<'a>(n: &'a Node, path: &str) -> Option<&'a Node> {
        if n.path == path {
            return Some(n);
        }
        n.children.iter().find(|c| path.starts_with(&format!("{}.", c.path)) || c.path == path).and_then(|c| go(c, path))
    }
    go(tree.root.as_ref()?, path)
}

/// The id of an attribute row (`Row::toggle`): `@attr:name`.
pub fn attr_id(name: &str) -> String {
    format!("@attr:{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_html_script_and_style() {
        let s = tokenize("<!-- c -->\n<a href=\"x\">hi</a>\n<script>var n = 1; // t\n</script>\n<style>p { color: red }</style>");
        assert_eq!(s.lines[0][0].0, 'p');
        assert!(s.lines[1].iter().any(|(k, t)| *k == 'a' && t == "href"));
        assert!(s.lines[1].iter().any(|(k, t)| *k == 'v' && t == "\"x\""));
        assert!(s.lines[2].iter().any(|(k, t)| *k == 't' && t == "var"));
        assert!(s.lines[2].iter().any(|(k, t)| *k == 'a' && t == "1"));
        assert!(s.lines[4].iter().any(|(k, t)| *k == 'a' && t == "color"));
        assert_eq!(s.plain[1], "<a href=\"x\">hi</a>");
    }

    #[test]
    fn search_keeps_matching_lines_with_numbers() {
        let s = tokenize("<p>one</p>\n<p>two</p>\n<p>one more</p>");
        let rows = source_rows(&s, "ONE");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].cells[0].text, "1");
        assert_eq!(rows[2].cells[0].text, "3");
        assert!(source_rows(&s, "zzz")[0].cells[0].text.contains("No matches"));
        assert_eq!(source_rows(&s, "").len(), 3);
    }

    #[test]
    fn storage_parses_and_ids_round_trip() {
        let e = parse_storage(r#"{"local":[["a","1"]],"session":[],"cookie":[["c","x=y"]]}"#).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(parse_entry_id(&entry_id(&e[1])), Some((Store::Cookie, "c".to_string())));
        assert_eq!(storage_rows(&e, "", None, 600.0).len(), 5);
        assert_eq!(storage_rows(&e, "zz", None, 600.0).len(), 3);
    }

    #[test]
    fn styles_parse_and_find_node() {
        assert_eq!(parse_styles(r#"[["display","block"]]"#).unwrap(), vec![("display".to_string(), "block".to_string())]);
        let tree = elements::parse(r#"{"root":{"t":"html","p":"0","k":[{"t":"body","p":"0.0","k":[{"t":"p","p":"0.0.0","x":"hi"}]}]},"nodes":3}"#).unwrap();
        assert_eq!(find_node(&tree, "0.0.0").map(|n| n.text.as_str()), Some("hi"));
    }

    #[test]
    fn tokenizer_survives_odd_input() {
        for s in ["<", "<a", "<a href=", "<script>", "<!--", "é<é>é", "<style>a{", "<script>'abc", "\u{1F600}<p class='x"] {
            let _ = tokenize(s);
        }
    }
}
