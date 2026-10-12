//! Named pasteboards that apps share: typed items with several representations (text, a link, a file URL, PNG, rich
//! text), a change count per board, and a `changed` event when any app writes a board. This is the client for the
//! shell's `pasteboard` service (`springboard/src/shell/services/pasteboard_types.rs`), over `service::call`.
//!
//! ```ignore
//! use bifrost_app::pasteboard_types::{Board, Item, PNG, STRING};
//! let board = Board::named("notes-drafts");
//! let id = board.write(ctx, &[Item::default().with(STRING, b"hello").with(PNG, &png_bytes)]);
//! // in App::service_reply(ctx, request, result): pasteboard_types::decode_change_count(result)
//! // in App::service_event(ctx, service, event, body): pasteboard_types::decode_change(service, event, body)
//! ```
//! Every call is asynchronous: it returns the request id, and the answer arrives in `App::service_reply`. The
//! `decode_*` helpers turn the answer into typed values. Data that is not UTF-8 travels as hex.
//! Gap: the general board (`Board::general()`) keeps its items flat, one representation each, because the shell's
//! clipboard does not keep item grouping; `pasteboard::NativePasteboard` remains the synchronous path for it.

use crate::Ctx;
use bifrost_core::json::Json;
use springboard::shell::services::pasteboard_types::{from_hex, to_hex};

/// The shell service name.
pub const SERVICE: &str = "pasteboard";
/// The shared clipboard board.
pub const GENERAL: &str = "general";
pub const STRING: &str = "public.utf8-plain-text";
pub const URL: &str = "public.url";
pub const FILE_URL: &str = "public.file-url";
pub const HTML: &str = "public.html";
pub const RTF: &str = "public.rtf";
pub const PNG: &str = "public.png";

/// One representation: a type id and its bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Representation {
    pub uti: String,
    pub data: Vec<u8>,
}

/// One item on a board: several representations of the same copy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub representations: Vec<Representation>,
}

impl Item {
    /// Adds one representation and returns the item (builder style).
    pub fn with(mut self, uti: &str, data: &[u8]) -> Self {
        self.representations.push(Representation { uti: uti.to_string(), data: data.to_vec() });
        self
    }

    /// The data of the first representation of `uti`.
    pub fn data(&self, uti: &str) -> Option<&[u8]> {
        self.representations.iter().find(|r| r.uti == uti).map(|r| r.data.as_slice())
    }
}

/// A named pasteboard. Boards are shared by name between every running app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board(pub String);

impl Board {
    /// The shared clipboard board.
    pub fn general() -> Self {
        Board(GENERAL.to_string())
    }

    /// A named board; it is created by its first write.
    pub fn named(name: &str) -> Self {
        Board(name.to_string())
    }

    /// Replaces the items (one change). Reply: [`decode_change_count`].
    pub fn write(&self, ctx: &Ctx, items: &[Item]) -> u32 {
        self.send(ctx, "write", vec![("items", Json::Arr(items.iter().map(item_json).collect()))])
    }

    /// Empties the board (one change). Reply: [`decode_change_count`].
    pub fn clear(&self, ctx: &Ctx) -> u32 {
        self.send(ctx, "clear", Vec::new())
    }

    /// The change count. Reply: [`decode_change_count`].
    pub fn change_count(&self, ctx: &Ctx) -> u32 {
        self.send(ctx, "change_count", Vec::new())
    }

    /// The whole board. Reply: [`decode_read`].
    pub fn read(&self, ctx: &Ctx) -> u32 {
        self.send(ctx, "read", Vec::new())
    }

    /// The distinct type ids on the board. Reply: [`decode_types`].
    pub fn types(&self, ctx: &Ctx) -> u32 {
        self.send(ctx, "types", Vec::new())
    }

    /// The text of every item that holds `uti`, concatenated. Reply: [`decode_string`].
    pub fn string(&self, ctx: &Ctx, uti: &str) -> u32 {
        self.send(ctx, "string", vec![("uti", Json::Str(uti.to_string()))])
    }

    /// The data of the first item that holds `uti`. Reply: [`decode_data`].
    pub fn data(&self, ctx: &Ctx, uti: &str) -> u32 {
        self.send(ctx, "data", vec![("uti", Json::Str(uti.to_string()))])
    }

    /// Whether some item holds data of a type that conforms to one of `types`. Reply: [`decode_bool`].
    pub fn can_read(&self, ctx: &Ctx, types: &[&str]) -> u32 {
        self.send(ctx, "can_read", vec![("types", Json::Arr(types.iter().map(|t| Json::Str(t.to_string())).collect()))])
    }

    /// Type negotiation: the first of `prefs` (in order) that the board holds exactly, as `availableType(from:)`.
    /// Reply: [`decode_best`].
    pub fn best(&self, ctx: &Ctx, prefs: &[&str]) -> u32 {
        self.send(ctx, "best", vec![("types", Json::Arr(prefs.iter().map(|t| Json::Str(t.to_string())).collect()))])
    }

    /// Releases a named board (the general board cannot be released). Reply: [`decode_bool`].
    pub fn release(&self, ctx: &Ctx) -> u32 {
        self.send(ctx, "release", Vec::new())
    }

    fn send(&self, ctx: &Ctx, method: &str, extra: Vec<(&str, Json)>) -> u32 {
        let mut fields = vec![("board".to_string(), Json::Str(self.0.clone()))];
        fields.extend(extra.into_iter().map(|(k, v)| (k.to_string(), v)));
        crate::service::call(ctx, SERVICE, method, &Json::Obj(fields))
    }
}

/// The names of the named boards, the general board first. Reply: [`decode_names`].
pub fn boards(ctx: &Ctx) -> u32 {
    crate::service::call(ctx, SERVICE, "boards", &Json::Null)
}

/// A new empty named board with a unique name. Reply: [`decode_board_name`].
pub fn unique(ctx: &Ctx) -> u32 {
    crate::service::call(ctx, SERVICE, "unique", &Json::Null)
}

/// A whole board as read by [`Board::read`].
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub board: String,
    pub change_count: u32,
    pub items: Vec<Item>,
}

/// A `changed` event: a board whose change count moved.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub board: String,
    pub change_count: u32,
}

/// The change count from a reply to [`Board::write`], [`Board::clear`] or [`Board::change_count`].
pub fn decode_change_count(r: &Result<Json, String>) -> Option<u32> {
    count_of(r.as_ref().ok()?)
}

/// The board from a reply to [`Board::read`].
pub fn decode_read(r: &Result<Json, String>) -> Result<Snapshot, String> {
    let v = r.as_ref().map_err(|e| e.clone())?;
    let board = v.str_field("board").ok_or("the reply has no board")?;
    let change_count = count_of(v).ok_or("the reply has no change count")?;
    let items = v.get("items").and_then(Json::as_array).ok_or("the reply has no items")?;
    let items = items.iter().map(item_from_json).collect::<Result<Vec<_>, String>>()?;
    Ok(Snapshot { board, change_count, items })
}

/// The type ids from a reply to [`Board::types`].
pub fn decode_types(r: &Result<Json, String>) -> Option<Vec<String>> {
    r.as_ref().ok()?.as_array()?.iter().map(|t| t.as_str().map(str::to_string)).collect()
}

/// The text from a reply to [`Board::string`]; `None` when no item holds the type.
pub fn decode_string(r: &Result<Json, String>) -> Option<String> {
    r.as_ref().ok()?.as_str().map(str::to_string)
}

/// The data from a reply to [`Board::data`]; `None` when no item holds the type.
pub fn decode_data(r: &Result<Json, String>) -> Option<Vec<u8>> {
    from_hex(r.as_ref().ok()?.as_str()?).ok()
}

/// A yes or no from a reply to [`Board::can_read`] or [`Board::release`].
pub fn decode_bool(r: &Result<Json, String>) -> Option<bool> {
    match r.as_ref().ok()? {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}

/// The negotiated type from a reply to [`Board::best`]; `None` when the board holds none of the preferences.
pub fn decode_best(r: &Result<Json, String>) -> Option<String> {
    r.as_ref().ok()?.as_str().map(str::to_string)
}

/// The board names from a reply to [`boards`].
pub fn decode_names(r: &Result<Json, String>) -> Option<Vec<String>> {
    decode_types(r)
}

/// The new board name from a reply to [`unique`].
pub fn decode_board_name(r: &Result<Json, String>) -> Option<String> {
    r.as_ref().ok()?.str_field("board")
}

/// A `changed` event from the shell, or `None` for any other service or event.
pub fn decode_change(service: &str, event: &str, body: &Json) -> Option<Change> {
    if service != SERVICE || event != "changed" {
        return None;
    }
    Some(Change { board: body.str_field("board")?, change_count: count_of(body)? })
}

fn count_of(v: &Json) -> Option<u32> {
    match v {
        Json::Num(_) => v.number().map(|n| n as u32),
        _ => v.get("change_count").and_then(Json::number).map(|n| n as u32),
    }
}

fn item_json(item: &Item) -> Json {
    let reps = item.representations.iter().map(rep_json).collect();
    Json::Obj(vec![("reps".to_string(), Json::Arr(reps))])
}

fn rep_json(r: &Representation) -> Json {
    let body = match std::str::from_utf8(&r.data) {
        Ok(t) => ("text", Json::Str(t.to_string())),
        Err(_) => ("hex", Json::Str(to_hex(&r.data))),
    };
    Json::Obj(vec![("uti".to_string(), Json::Str(r.uti.clone())), (body.0.to_string(), body.1)])
}

fn item_from_json(v: &Json) -> Result<Item, String> {
    let reps = v.get("reps").and_then(Json::as_array).ok_or("an item has no reps")?;
    let representations = reps.iter().map(|r| -> Result<Representation, String> {
        let uti = r.str_field("uti").ok_or("a representation has no uti")?;
        let data = match (r.str_field("text"), r.str_field("hex")) {
            (Some(t), _) => t.into_bytes(),
            (None, Some(h)) => from_hex(&h)?,
            (None, None) => return Err("a representation has no data".to_string()),
        };
        Ok(Representation { uti, data })
    });
    Ok(Item { representations: representations.collect::<Result<Vec<_>, String>>()? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip_through_the_wire_shape() {
        let item = Item::default().with(STRING, b"hi").with(PNG, &[0x89, 0x50, 0xff]);
        let reply = Json::Obj(vec![
            ("board".to_string(), Json::Str("b".into())),
            ("change_count".to_string(), Json::Num(4.0)),
            ("items".to_string(), Json::Arr(vec![item_json(&item)])),
        ]);
        let snap = decode_read(&Ok(reply)).unwrap();
        assert_eq!(snap, Snapshot { board: "b".into(), change_count: 4, items: vec![item.clone()] });
        assert_eq!(snap.items[0].data(PNG), Some(&[0x89u8, 0x50, 0xff][..]));
    }

    #[test]
    fn binary_data_is_hex_and_text_is_text_on_the_wire() {
        assert_eq!(rep_json(&Representation { uti: PNG.into(), data: vec![0x89, 0xff] }).str_field("hex").as_deref(), Some("89ff"));
        assert_eq!(rep_json(&Representation { uti: STRING.into(), data: b"x".to_vec() }).str_field("text").as_deref(), Some("x"));
    }

    #[test]
    fn a_representation_without_data_is_refused() {
        let bad = Json::Obj(vec![("reps".to_string(), Json::Arr(vec![Json::Obj(vec![("uti".to_string(), Json::Str(PNG.into()))])]))]);
        assert!(item_from_json(&bad).is_err());
    }

    #[test]
    fn replies_decode_to_their_types() {
        assert_eq!(decode_change_count(&Ok(Json::Obj(vec![("change_count".to_string(), Json::Num(7.0))]))), Some(7));
        assert_eq!(decode_string(&Ok(Json::Str("x".into()))), Some("x".into()));
        assert_eq!(decode_string(&Ok(Json::Null)), None);
        assert_eq!(decode_data(&Ok(Json::Str("89ff".into()))), Some(vec![0x89, 0xff]));
        assert_eq!(decode_data(&Ok(Json::Str("8".into()))), None);
        assert_eq!(decode_bool(&Ok(Json::Bool(true))), Some(true));
        assert_eq!(decode_types(&Ok(Json::Arr(vec![Json::Str("public.html".into())]))), Some(vec!["public.html".to_string()]));
        assert_eq!(decode_board_name(&Ok(Json::Obj(vec![("board".to_string(), Json::Str("unique-1".into()))]))), Some("unique-1".into()));
        assert_eq!(decode_change_count(&Err("no service".into())), None);
    }

    #[test]
    fn a_changed_event_decodes_only_for_this_service() {
        let body = Json::Obj(vec![("board".to_string(), Json::Str("b".into())), ("change_count".to_string(), Json::Num(2.0))]);
        assert_eq!(decode_change(SERVICE, "changed", &body), Some(Change { board: "b".into(), change_count: 2 }));
        assert_eq!(decode_change("dock", "changed", &body), None);
        assert_eq!(decode_change(SERVICE, "other", &body), None);
    }

    #[test]
    fn the_general_board_and_type_ids_match_the_shell() {
        assert_eq!(Board::general().0, GENERAL);
        assert_eq!(springboard::shell::pasteboard::mime_for(PNG), "image/png");
        assert_eq!(springboard::shell::pasteboard::mime_for(STRING), "text/plain;charset=utf-8");
    }
}
