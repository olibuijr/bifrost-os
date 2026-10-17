//! The wire format between Files (the host) and a file provider extension (see [`crate::file_provider`] for the items
//! and `bifrost_app::file_provider` for the domains). The transport is the extension's control socket: one JSON-RPC
//! request line, one reply line, as every app control method. Pure encoding and decoding, shared by the host and by
//! `bifrost_app::extension_service`, which answers.
//!
//! Methods, both addressed to the provider that serves one domain:
//! - [`ENUMERATE`] `{container}`: the items directly inside that container (an item id, `root` for the top). Result
//!   `{items: [item]}` with `item = {id, parent, name, type?, size?, mtime?, placeholder, caps}`. A folder has the
//!   type [`FOLDER_TYPE`]; `placeholder` is true while its bytes are not on this machine.
//! - [`FETCH`] `{id, dest}`: the provider writes the item's bytes to the file `dest` (the host chooses it) and answers
//!   `{path}`. This is how a placeholder is downloaded; it needs the `READING` capability.
//!
//! A provider reports a failure as an error object whose `message` is a provider error code name
//! (`bifrost_app::file_provider::ErrorCode::as_str`, for example `noSuchItem`).

use crate::file_provider::{ItemCapabilities, ItemIdentifier, LocalContent, ProvidedItem};
use bifrost_core::json::Json;

/// List a container.
pub const ENUMERATE: &str = "provider.enumerate";
/// Download an item's bytes to a file.
pub const FETCH: &str = "provider.fetch";
/// The `type` of a folder item.
pub const FOLDER_TYPE: &str = "public.folder";

/// The request line (with its newline) for `method` with `params`.
pub fn request_line(method: &str, params: Json) -> String {
    let request = Json::Obj(vec![
        ("jsonrpc".into(), Json::Str("2.0".into())),
        ("id".into(), Json::Num(1.0)),
        ("method".into(), Json::Str(method.into())),
        ("params".into(), params),
    ]);
    format!("{}\n", request.encode())
}

pub fn enumerate_params(container: &ItemIdentifier) -> Json {
    Json::Obj(vec![("container".into(), Json::Str(container.as_str().into()))])
}

pub fn container_from(params: &Json) -> Option<ItemIdentifier> {
    ItemIdentifier::new(params.get("container")?.as_str()?)
}

pub fn fetch_params(item: &ItemIdentifier, dest: &str) -> Json {
    Json::Obj(vec![("id".into(), Json::Str(item.as_str().into())), ("dest".into(), Json::Str(dest.into()))])
}

/// The item and the destination file of a fetch request.
pub fn fetch_from(params: &Json) -> Option<(ItemIdentifier, String)> {
    let dest = params.get("dest")?.as_str()?;
    if dest.is_empty() {
        return None;
    }
    Some((ItemIdentifier::new(params.get("id")?.as_str()?)?, dest.to_string()))
}

/// True for a folder item.
pub fn is_folder(item: &ProvidedItem) -> bool {
    item.content_type.as_deref() == Some(FOLDER_TYPE)
}

/// The item marked as a folder.
pub fn as_folder(mut item: ProvidedItem) -> ProvidedItem {
    item.content_type = Some(FOLDER_TYPE.into());
    item
}

pub fn item_json(item: &ProvidedItem) -> Json {
    let mut fields = vec![
        ("id".to_string(), Json::Str(item.identifier.as_str().into())),
        ("parent".to_string(), Json::Str(item.parent.as_str().into())),
        ("name".to_string(), Json::Str(item.filename().into())),
    ];
    if let Some(t) = &item.content_type {
        fields.push(("type".into(), Json::Str(t.clone())));
    }
    if let Some(n) = item.document_size {
        fields.push(("size".into(), Json::Num(n as f64)));
    }
    if let Some(n) = item.content_modification_date {
        fields.push(("mtime".into(), Json::Num(n as f64)));
    }
    fields.push(("placeholder".into(), Json::Bool(item.local_content() == LocalContent::Placeholder)));
    fields.push(("caps".into(), Json::Num(f64::from(item.capabilities().0))));
    Json::Obj(fields)
}

/// One item from its JSON. None when the id, the parent or the filename is missing or refused.
pub fn item_from(json: &Json) -> Option<ProvidedItem> {
    let id = ItemIdentifier::new(json.get("id")?.as_str()?)?;
    let parent = ItemIdentifier::new(json.get("parent")?.as_str()?)?;
    let mut item = ProvidedItem::new(id, parent, json.get("name")?.as_str()?).ok()?;
    item.content_type = json.get("type").and_then(Json::as_str).map(str::to_string);
    item.document_size = json.get("size").and_then(Json::as_u64);
    item.content_modification_date = json.get("mtime").and_then(Json::as_u64);
    let caps = json.get("caps").and_then(Json::as_u64).unwrap_or(0) as u32 & ItemCapabilities::ALL.0;
    item = item.with_capabilities(ItemCapabilities(caps));
    if matches!(json.get("placeholder"), Some(Json::Bool(true))) {
        item = item.as_placeholder();
    }
    Some(item)
}

/// The result of [`ENUMERATE`].
pub fn items_result(items: &[ProvidedItem]) -> Json {
    Json::Obj(vec![("items".into(), Json::Arr(items.iter().map(item_json).collect()))])
}

/// The items of an [`ENUMERATE`] result. An item that cannot be read is left out; a result with no `items` list is
/// refused.
pub fn items_from(result: &Json) -> Option<Vec<ProvidedItem>> {
    Some(result.get("items")?.as_array()?.iter().filter_map(item_from).collect())
}

/// The result of [`FETCH`].
pub fn fetch_result(path: &str) -> Json {
    Json::Obj(vec![("path".into(), Json::Str(path.into()))])
}

pub fn path_from(result: &Json) -> Option<String> {
    result.get("path")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::parse;

    fn id(s: &str) -> ItemIdentifier {
        ItemIdentifier::new(s).unwrap()
    }

    fn doc() -> ProvidedItem {
        let mut item = ProvidedItem::new(id("d1"), ItemIdentifier::root_container(), "Plan.txt")
            .unwrap()
            .with_document_size(42)
            .with_capabilities(ItemCapabilities::READING.union(ItemCapabilities::EVICTING))
            .as_placeholder();
        item.content_type = Some("public.plain-text".into());
        item.content_modification_date = Some(1_700_000_000);
        item
    }

    #[test]
    fn an_item_survives_the_wire() {
        let text = item_json(&doc()).encode();
        let back = item_from(&parse(&text).unwrap()).unwrap();
        assert_eq!(back, doc());
        assert_eq!(back.local_content(), LocalContent::Placeholder);
        assert!(back.can(ItemCapabilities::READING));
        assert!(!back.can(ItemCapabilities::WRITING));
    }

    #[test]
    fn a_folder_is_a_typed_item_and_a_downloaded_item_is_not_a_placeholder() {
        let folder = as_folder(ProvidedItem::new(id("f1"), ItemIdentifier::root_container(), "Reports").unwrap());
        let back = item_from(&parse(&item_json(&folder).encode()).unwrap()).unwrap();
        assert!(is_folder(&back));
        assert_eq!(back.local_content(), LocalContent::Downloaded);
        assert!(!is_folder(&doc()));
    }

    #[test]
    fn the_enumerate_result_drops_unreadable_items_and_refuses_a_missing_list() {
        let json = parse("{\"items\":[{\"id\":\"a\",\"parent\":\"root\",\"name\":\"ok\"},{\"id\":\"\",\"parent\":\"root\",\"name\":\"no id\"},{\"id\":\"b\",\"parent\":\"root\",\"name\":\"a/b\"}]}").unwrap();
        let items = items_from(&json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].filename(), "ok");
        assert!(items_from(&parse("{}").unwrap()).is_none());
        let full = items_from(&items_result(&[doc()])).unwrap();
        assert_eq!(full, vec![doc()]);
    }

    #[test]
    fn capabilities_from_a_provider_are_masked_to_the_known_flags() {
        let json = parse("{\"id\":\"a\",\"parent\":\"root\",\"name\":\"x\",\"caps\":4294967295}").unwrap();
        assert_eq!(item_from(&json).unwrap().capabilities(), ItemCapabilities::ALL);
    }

    #[test]
    fn requests_carry_their_params() {
        let line = request_line(ENUMERATE, enumerate_params(&id("f1")));
        assert!(line.ends_with('\n'));
        let req = parse(line.trim()).unwrap();
        assert_eq!(req.get("method").and_then(Json::as_str), Some(ENUMERATE));
        assert_eq!(container_from(req.get("params").unwrap()), Some(id("f1")));

        let fetch = fetch_params(&id("d1"), "/tmp/x/Plan.txt");
        assert_eq!(fetch_from(&fetch), Some((id("d1"), "/tmp/x/Plan.txt".to_string())));
        assert!(fetch_from(&fetch_params(&id("d1"), "")).is_none());
        assert!(container_from(&Json::Obj(vec![])).is_none());
        assert_eq!(path_from(&fetch_result("/tmp/x")), Some("/tmp/x".to_string()));
    }
}
