//! The `pasteboard` service: named pasteboards that apps share, each holding typed items. An item carries several
//! representations of one copy (plain text, a link, a file URL, PNG, rich text), and a board has a change count that
//! moves on every write. A move is pushed to every running app as the `changed` event, so an app can watch a board
//! that another app writes. The `general` board is the shell's clipboard: its reads and writes go through
//! `shell::pasteboard`, so the plain clipboard and external clients see the same contents.
//!
//! Methods (JSON params, `board` defaults to `general`): `boards`, `unique`, `release {board}`, `write {board, items}`,
//! `clear {board}`, `change_count {board}`, `read {board}`, `types {board}`, `string {board, uti}`, `data {board, uti}`
//! (hex), `can_read {board, types}` and `best {board, types}` (the first preferred type the board holds, exact match, as
//! `availableType(from:)`; null when none). An item is `{"reps": [{"uti", "text"}]}`; data that is not text travels as
//! `"hex"`. Limits: 16 items a write, 16 representations an item, 16 named boards. Binary data is hex, not base64, so
//! the codec stays a few lines on each side. The general board keeps item grouping (`shell::pasteboard::set_items`, `snapshot_items`), so
//! the representations of one copy read back together; the wire to and from apps (`ToShell::PasteboardSet`,
//! `ToApp::Pasteboard`) is still flat, and general writes do not yet re-broadcast to other apps' mirrors.
use super::{Event, Shell, SystemService};
use crate::shell::{clipboard, pasteboard as shell_pasteboard, pasteboard_transfer as shell_pasteboard_transfer};
use bifrost_core::json::Json;
use bifrost_ui::uttype;
use std::collections::BTreeMap;

/// The shared clipboard board.
pub const GENERAL: &str = "general";
const MAX_ITEMS: usize = 16;
const MAX_REPS: usize = 16;
const MAX_BOARDS: usize = 16;

/// One representation of an item: a type id and its bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Representation {
    pub uti: String,
    pub data: Vec<u8>,
}

/// One item on a board: several representations of the same copy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub reps: Vec<Representation>,
}

impl Item {
    /// `availableType(from:)`: the first of `candidates` (in the order given) that the item holds.
    pub fn available_type<'a>(&self, candidates: &[&'a str]) -> Option<&'a str> {
        candidates.iter().copied().find(|c| self.reps.iter().any(|r| r.uti.as_str() == *c))
    }

    /// `data(forType:)` on this item.
    pub fn data(&self, uti: &str) -> Option<&[u8]> {
        self.reps.iter().find(|r| r.uti == uti).map(|r| r.data.as_slice())
    }
}

/// A board: its items, its change count, and the count last announced to apps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    pub change_count: u32,
    pub items: Vec<Item>,
    announced: u32,
}

impl Board {
    /// `clearContents` then `writeObjects`: replaces the items and counts as one change. Returns the new count.
    pub fn write(&mut self, items: Vec<Item>) -> u32 {
        self.items = items;
        self.change_count = self.change_count.wrapping_add(1);
        self.change_count
    }

    /// `types`: the distinct type ids, in order of first appearance.
    pub fn types(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for item in &self.items {
            for r in &item.reps {
                if !out.contains(&r.uti) {
                    out.push(r.uti.clone());
                }
            }
        }
        out
    }

    /// `string(forType:)`: the text of every item that holds the type, concatenated.
    pub fn string(&self, uti: &str) -> Option<String> {
        let mut out = String::new();
        let mut any = false;
        for item in &self.items {
            if let Some(d) = item.data(uti) {
                any = true;
                out.push_str(&String::from_utf8_lossy(d));
            }
        }
        if any {
            Some(out)
        } else {
            None
        }
    }

    /// `data(forType:)`: the data of the first item that holds the type.
    pub fn data(&self, uti: &str) -> Option<&[u8]> {
        self.items.iter().find_map(|item| item.data(uti))
    }

    /// `canReadItem(withDataConformingToTypes:)`: some representation conforms to one of `ancestors`.
    pub fn can_read(&self, ancestors: &[&str]) -> bool {
        self.items.iter().any(|item| item.reps.iter().any(|r| ancestors.iter().any(|a| uttype::conforms(&r.uti, a))))
    }

    /// Type negotiation (`availableType(from:)` over the whole board): the first of `prefs`, in the order given, that some
    /// item holds exactly.
    pub fn best_type(&self, prefs: &[&str]) -> Option<String> {
        prefs.iter().find(|p| self.items.iter().any(|item| item.reps.iter().any(|r| r.uti == **p))).map(|p| p.to_string())
    }
}

/// The `pasteboard` service's state: the named boards and the last change counts announced.
#[derive(Default)]
pub struct Pasteboards {
    boards: BTreeMap<String, Board>,
    general_announced: u32,
    unique: u32,
}

impl SystemService for Pasteboards {
    fn name(&self) -> &'static str {
        "pasteboard"
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.handle(method, params)
    }

    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        self.changes()
    }
}

impl Pasteboards {
    /// Answers one method. Pure over the boards, except that writes to `general` reach the shell's clipboard.
    pub fn handle(&mut self, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "boards" => Ok(Json::Arr(std::iter::once(GENERAL.to_string()).chain(self.boards.keys().cloned()).map(Json::Str).collect())),
            "unique" => {
                self.room()?;
                self.unique += 1;
                let name = format!("unique-{}", self.unique);
                self.boards.insert(name.clone(), Board::default());
                Ok(obj(vec![("board", Json::Str(name))]))
            }
            "release" => {
                let name = board_name(params)?;
                if name == GENERAL {
                    return Err("the general pasteboard cannot be released".into());
                }
                self.boards.remove(&name);
                Ok(Json::Bool(true))
            }
            "write" => {
                let name = board_name(params)?;
                let items = parse_items(params.get("items"))?;
                // the byte count of the payload feeds the paste progress panel (`pasteboard_transfer`)
                let total: u64 = items.iter().flat_map(|i| i.reps.iter()).map(|r| r.data.len() as u64).sum();
                shell_pasteboard_transfer::report_begin(total, bifrost_proto::sys::now_ms());
                let n = self.write(&name, items);
                shell_pasteboard_transfer::report_done();
                let n = n?;
                Ok(obj(vec![("change_count", Json::Num(n as f64))]))
            }
            "clear" => {
                let name = board_name(params)?;
                let n = self.write(&name, Vec::new())?;
                Ok(obj(vec![("change_count", Json::Num(n as f64))]))
            }
            "change_count" => {
                let name = board_name(params)?;
                Ok(Json::Num(self.count(&name) as f64))
            }
            "read" => {
                let name = board_name(params)?;
                let b = self.view(&name);
                Ok(board_json(&name, &b))
            }
            "types" => {
                let name = board_name(params)?;
                let b = self.view(&name);
                Ok(Json::Arr(b.types().into_iter().map(Json::Str).collect()))
            }
            "string" => {
                let name = board_name(params)?;
                let uti = uti_param(params)?;
                let b = self.view(&name);
                Ok(b.string(&uti).map_or(Json::Null, Json::Str))
            }
            "data" => {
                let name = board_name(params)?;
                let uti = uti_param(params)?;
                let b = self.view(&name);
                Ok(b.data(&uti).map_or(Json::Null, |d| Json::Str(to_hex(d))))
            }
            "can_read" => {
                let name = board_name(params)?;
                let types: Vec<String> = params.get("types").and_then(Json::as_array).unwrap_or(&[]).iter().filter_map(|t| t.as_str().map(str::to_string)).collect();
                let refs: Vec<&str> = types.iter().map(String::as_str).collect();
                let b = self.view(&name);
                Ok(Json::Bool(b.can_read(&refs)))
            }
            "best" => {
                let name = board_name(params)?;
                let types: Vec<String> = params.get("types").and_then(Json::as_array).unwrap_or(&[]).iter().filter_map(|t| t.as_str().map(str::to_string)).collect();
                let refs: Vec<&str> = types.iter().map(String::as_str).collect();
                let b = self.view(&name);
                Ok(b.best_type(&refs).map_or(Json::Null, Json::Str))
            }
            _ => Err(format!("pasteboard has no method {method}")),
        }
    }

    /// The change events since the last call: one per board whose count moved, and one for the general board.
    pub fn changes(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        let g = shell_pasteboard::snapshot().0;
        if g != self.general_announced {
            self.general_announced = g;
            out.push(changed(GENERAL, g));
        }
        for (name, b) in self.boards.iter_mut() {
            if b.change_count != b.announced {
                b.announced = b.change_count;
                out.push(changed(name, b.change_count));
            }
        }
        out
    }

    fn room(&self) -> Result<(), String> {
        if self.boards.len() >= MAX_BOARDS {
            Err(format!("at most {MAX_BOARDS} named pasteboards"))
        } else {
            Ok(())
        }
    }

    fn count(&self, name: &str) -> u32 {
        if name == GENERAL {
            shell_pasteboard::snapshot().0
        } else {
            self.boards.get(name).map_or(0, |b| b.change_count)
        }
    }

    /// A copy of the board for reading. The general board is built from the shell's clipboard state.
    fn view(&self, name: &str) -> Board {
        if name == GENERAL {
            let (count, grouped) = shell_pasteboard::snapshot_items();
            let items = grouped
                .into_iter()
                .map(|reps| Item { reps: reps.into_iter().map(|(uti, data)| Representation { uti, data }).collect() })
                .collect();
            Board { change_count: count, items, announced: 0 }
        } else {
            self.boards.get(name).cloned().unwrap_or_default()
        }
    }

    fn write(&mut self, name: &str, items: Vec<Item>) -> Result<u32, String> {
        if name == GENERAL {
            let grouped: Vec<Vec<(String, Vec<u8>)>> =
                items.iter().map(|it| it.reps.iter().map(|r| (r.uti.clone(), r.data.clone())).collect()).collect();
            let flat: Vec<(String, Vec<u8>)> = grouped.iter().flatten().cloned().collect();
            if let Some(text) = shell_pasteboard::set_items(grouped) {
                clipboard::set_text(&text);
            }
            shell_pasteboard::offer_wayland(&flat);
            return Ok(shell_pasteboard::snapshot().0);
        }
        if !self.boards.contains_key(name) {
            self.room()?;
        }
        Ok(self.boards.entry(name.to_string()).or_default().write(items))
    }
}

fn changed(board: &str, count: u32) -> Event {
    Event { app: None, event: "changed".into(), body: obj(vec![("board", Json::Str(board.to_string())), ("change_count", Json::Num(count as f64))]) }
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn board_name(params: &Json) -> Result<String, String> {
    let name = params.get("board").and_then(Json::as_str).unwrap_or(GENERAL);
    if name.is_empty() || name.len() > 64 || name.chars().any(|c| c.is_control()) {
        return Err("bad pasteboard name".into());
    }
    Ok(name.to_string())
}

fn uti_param(params: &Json) -> Result<String, String> {
    params.str_field("uti").filter(|u| !u.is_empty()).ok_or_else(|| "uti is required".to_string())
}

fn parse_items(v: Option<&Json>) -> Result<Vec<Item>, String> {
    let arr = v.and_then(Json::as_array).ok_or("items must be an array")?;
    if arr.len() > MAX_ITEMS {
        return Err(format!("at most {MAX_ITEMS} items"));
    }
    arr.iter().map(parse_item).collect()
}

fn parse_item(v: &Json) -> Result<Item, String> {
    let reps = v.get("reps").and_then(Json::as_array).ok_or("an item needs reps")?;
    if reps.is_empty() || reps.len() > MAX_REPS {
        return Err(format!("an item holds 1 to {MAX_REPS} representations"));
    }
    let reps = reps.iter().map(parse_rep).collect::<Result<Vec<_>, String>>()?;
    Ok(Item { reps })
}

fn parse_rep(v: &Json) -> Result<Representation, String> {
    let uti = uti_param(v)?;
    let data = match (v.get("text").and_then(Json::as_str), v.get("hex").and_then(Json::as_str)) {
        (Some(t), _) => t.as_bytes().to_vec(),
        (None, Some(h)) => from_hex(h)?,
        (None, None) => return Err("a representation needs text or hex".into()),
    };
    Ok(Representation { uti, data })
}

fn board_json(name: &str, b: &Board) -> Json {
    let items = b.items.iter().map(|it| obj(vec![("reps", Json::Arr(it.reps.iter().map(rep_json).collect()))])).collect();
    obj(vec![("board", Json::Str(name.to_string())), ("change_count", Json::Num(b.change_count as f64)), ("items", Json::Arr(items))])
}

/// A representation on the wire: `text` when its MIME type is text and the bytes are UTF-8, else `hex`.
fn rep_json(r: &Representation) -> Json {
    let mime = shell_pasteboard::mime_for(&r.uti);
    let mut fields = vec![("uti", Json::Str(r.uti.clone())), ("mime", Json::Str(mime.to_string()))];
    match std::str::from_utf8(&r.data) {
        Ok(t) if mime.starts_with("text/") => fields.push(("text", Json::Str(t.to_string()))),
        _ => fields.push(("hex", Json::Str(to_hex(&r.data)))),
    }
    obj(fields)
}

/// Lowercase hex of `d`.
pub fn to_hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes of a hex string (either case). An odd length or a non-hex digit is an error.
pub fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return Err("hex data has an odd length or a non-hex digit".into());
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "bad hex data".to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(uti: &str, data: &[u8]) -> Representation {
        Representation { uti: uti.into(), data: data.to_vec() }
    }

    fn text_item(s: &str) -> Item {
        Item { reps: vec![rep("public.utf8-plain-text", s.as_bytes())] }
    }

    #[test]
    fn the_general_board_reads_back_the_items_with_their_representations_together() {
        let _turn = shell_pasteboard::TURN.lock().unwrap_or_else(|e| e.into_inner());
        let both = |text: &str, url: &str| {
            vec![("public.utf8-plain-text".to_string(), text.as_bytes().to_vec()), ("public.url".to_string(), url.as_bytes().to_vec())]
        };
        shell_pasteboard::set_items(vec![both("Ada", "https://ada.is"), vec![("public.png".to_string(), vec![1, 2])]]);
        let view = Pasteboards::default().view(GENERAL);
        assert_eq!(view.items.len(), 2);
        assert_eq!(view.items[0].reps.len(), 2);
        assert_eq!(view.items[0].available_type(&["public.url"]), Some("public.url"));
        assert_eq!(view.items[1].data("public.png"), Some(&[1u8, 2][..]));
        assert_eq!(view.types(), vec!["public.utf8-plain-text".to_string(), "public.url".to_string(), "public.png".to_string()]);
    }

    #[test]
    fn a_write_is_one_change_and_replaces_the_items() {
        let mut b = Board::default();
        assert_eq!(b.write(vec![text_item("a")]), 1);
        assert_eq!(b.write(vec![text_item("b"), text_item("c")]), 2);
        assert_eq!(b.items.len(), 2);
        assert_eq!(b.write(Vec::new()), 3);
        assert!(b.items.is_empty());
    }

    #[test]
    fn types_are_distinct_in_order_of_first_appearance() {
        let mut b = Board::default();
        b.write(vec![Item { reps: vec![rep("public.html", b"<i>x</i>"), rep("public.utf8-plain-text", b"x")] }, Item { reps: vec![rep("public.html", b"<b/>")] }]);
        assert_eq!(b.types(), vec!["public.html".to_string(), "public.utf8-plain-text".to_string()]);
    }

    #[test]
    fn string_concatenates_items_and_data_takes_the_first() {
        let mut b = Board::default();
        b.write(vec![text_item("ab"), Item { reps: vec![rep("public.png", &[1, 2])] }, text_item("cd")]);
        assert_eq!(b.string("public.utf8-plain-text").as_deref(), Some("abcd"));
        assert_eq!(b.string("public.rtf"), None);
        assert_eq!(b.data("public.utf8-plain-text"), Some(&b"ab"[..]));
        assert_eq!(b.data("public.png"), Some(&[1u8, 2][..]));
    }

    #[test]
    fn available_type_takes_the_first_candidate_the_item_holds() {
        let it = Item { reps: vec![rep("public.html", b"x"), rep("public.png", &[0])] };
        assert_eq!(it.available_type(&["public.png", "public.html"]), Some("public.png"));
        assert_eq!(it.available_type(&["public.rtf"]), None);
    }

    #[test]
    fn can_read_follows_type_conformance() {
        let mut b = Board::default();
        b.write(vec![text_item("x")]);
        assert!(b.can_read(&["public.text"]));
        assert!(!b.can_read(&["public.url"]));
        assert!(!Board::default().can_read(&["public.data"]));
    }

    #[test]
    fn best_type_takes_the_first_preference_the_board_holds_exactly() {
        let mut b = Board::default();
        b.write(vec![Item { reps: vec![rep("public.html", b"<i>x</i>"), rep("public.png", &[0])] }]);
        assert_eq!(b.best_type(&["public.rtf", "public.png", "public.html"]).as_deref(), Some("public.png"));
        assert_eq!(b.best_type(&["public.html", "public.png"]).as_deref(), Some("public.html"));
        // exact match only: a parent type is not offered for a child
        assert_eq!(b.best_type(&["public.image"]), None);
        assert_eq!(Board::default().best_type(&["public.html"]), None);
    }

    #[test]
    fn best_method_answers_the_negotiated_type_or_null() {
        let mut p = Pasteboards::default();
        let body = bifrost_core::json::parse(r#"{"board":"t-best","items":[{"reps":[{"uti":"public.html","text":"<i>x</i>"},{"uti":"public.png","hex":"89ff"}]}]}"#).unwrap();
        p.handle("write", &body).unwrap();
        let q = bifrost_core::json::parse(r#"{"board":"t-best","types":["public.rtf","public.png","public.html"]}"#).unwrap();
        assert_eq!(p.handle("best", &q), Ok(Json::Str("public.png".into())));
        let none = bifrost_core::json::parse(r#"{"board":"t-best","types":["public.rtf"]}"#).unwrap();
        assert_eq!(p.handle("best", &none), Ok(Json::Null));
    }

    #[test]
    fn hex_round_trips_and_rejects_bad_input() {
        assert_eq!(to_hex(&[0, 255, 16]), "00ff10");
        assert_eq!(from_hex("00FF10"), Ok(vec![0, 255, 16]));
        assert!(from_hex("abc").is_err());
        assert!(from_hex("zz").is_err());
    }

    #[test]
    fn writes_parse_text_and_hex_and_refuse_bad_shapes() {
        let ok = bifrost_core::json::parse(r#"{"items":[{"reps":[{"uti":"public.utf8-plain-text","text":"hi"},{"uti":"public.png","hex":"89ff"}]}]}"#).unwrap();
        let items = parse_items(ok.get("items")).unwrap();
        assert_eq!(items[0].reps[0].data, b"hi".to_vec());
        assert_eq!(items[0].reps[1].data, vec![0x89, 0xff]);
        let no_items = bifrost_core::json::parse(r#"{"items":"x"}"#).unwrap();
        assert!(parse_items(no_items.get("items")).is_err());
        let empty_reps = bifrost_core::json::parse(r#"{"reps":[]}"#).unwrap();
        assert!(parse_item(&empty_reps).is_err());
        let no_data = bifrost_core::json::parse(r#"{"uti":"public.html"}"#).unwrap();
        assert!(parse_rep(&no_data).is_err());
        let many: Vec<String> = (0..MAX_ITEMS + 1).map(|_| r#"{"reps":[{"uti":"public.html","text":"x"}]}"#.to_string()).collect();
        let too_many = bifrost_core::json::parse(&format!("[{}]", many.join(","))).unwrap();
        assert!(parse_items(Some(&too_many)).is_err());
    }

    #[test]
    fn named_boards_round_trip_through_the_methods() {
        let mut p = Pasteboards::default();
        let body = bifrost_core::json::parse(r#"{"board":"t-round","items":[{"reps":[{"uti":"public.utf8-plain-text","text":"hi"},{"uti":"public.png","hex":"89ff"}]}]}"#).unwrap();
        assert_eq!(p.handle("write", &body).unwrap().get("change_count"), Some(&Json::Num(1.0)));
        let q = bifrost_core::json::parse(r#"{"board":"t-round"}"#).unwrap();
        assert_eq!(p.handle("change_count", &q), Ok(Json::Num(1.0)));
        assert_eq!(p.handle("string", &bifrost_core::json::parse(r#"{"board":"t-round","uti":"public.utf8-plain-text"}"#).unwrap()), Ok(Json::Str("hi".into())));
        assert_eq!(p.handle("data", &bifrost_core::json::parse(r#"{"board":"t-round","uti":"public.png"}"#).unwrap()), Ok(Json::Str("89ff".into())));
        let read = p.handle("read", &q).unwrap();
        let rep = &read.get("items").and_then(Json::as_array).unwrap()[0].get("reps").and_then(Json::as_array).unwrap()[1];
        assert_eq!(rep.str_field("mime").as_deref(), Some("image/png"));
        assert_eq!(rep.str_field("hex").as_deref(), Some("89ff"));
        assert_eq!(p.handle("clear", &q).unwrap().get("change_count"), Some(&Json::Num(2.0)));
        assert_eq!(p.handle("string", &bifrost_core::json::parse(r#"{"board":"t-round","uti":"public.utf8-plain-text"}"#).unwrap()), Ok(Json::Null));
        assert!(p.handle("nope", &q).is_err());
    }

    #[test]
    fn unique_boards_and_release_respect_the_limit() {
        let mut p = Pasteboards::default();
        let name = p.handle("unique", &Json::Null).unwrap().str_field("board").unwrap();
        assert!(name.starts_with("unique-"));
        let rel = bifrost_core::json::parse(&format!(r#"{{"board":"{name}"}}"#)).unwrap();
        assert_eq!(p.handle("release", &rel), Ok(Json::Bool(true)));
        assert!(p.handle("release", &bifrost_core::json::parse(r#"{"board":"general"}"#).unwrap()).is_err());
        for i in 0..MAX_BOARDS {
            p.handle("unique", &Json::Null).unwrap_or_else(|e| panic!("board {i}: {e}"));
        }
        assert!(p.handle("unique", &Json::Null).is_err());
    }

    #[test]
    fn a_moved_count_is_announced_once_per_board() {
        let mut p = Pasteboards::default();
        let body = bifrost_core::json::parse(r#"{"board":"t-events","items":[]}"#).unwrap();
        p.handle("write", &body).unwrap();
        fn mine(evs: &[Event]) -> usize {
            evs.iter().filter(|e| e.body.str_field("board").as_deref() == Some("t-events")).count()
        }
        assert_eq!(mine(&p.changes()), 1);
        assert_eq!(mine(&p.changes()), 0);
        p.handle("clear", &body).unwrap();
        let evs = p.changes();
        assert_eq!(mine(&evs), 1);
        let e = evs.iter().find(|e| e.body.str_field("board").as_deref() == Some("t-events")).unwrap();
        assert_eq!(e.event, "changed");
        assert_eq!(e.body.get("change_count"), Some(&Json::Num(2.0)));
    }
}
