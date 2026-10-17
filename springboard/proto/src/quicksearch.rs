//! Wire types for Core Search content indexing (`CSSearchableIndex`) and the continue-activity message
//! (`NativeUserActivity` of type `CSSearchableItemActionType`). Tags `0x1c0` to `0x1cf` in both directions:
//! `ToShell::SearchIndex` 0x1c0, `ToShell::SearchDelete` 0x1c1, `ToApp::ContinueActivity` 0x1c2.
//!
//! Every string is cut to a byte cap on encode (on a character boundary) and refused on decode when longer, and a
//! message carries at most `MAX_ITEMS` items, so a packet stays below `MAX_PACKET`.

use super::{str_, u32_, DecodeError, Reader, ToApp, ToShell};

pub const MAX_ITEMS: usize = 8;
pub const MAX_ID: usize = 128;
pub const MAX_DOMAIN: usize = 64;
pub const MAX_TITLE: usize = 160;
pub const MAX_DESC: usize = 320;
pub const MAX_KEYWORDS: usize = 8;
pub const MAX_KEYWORD: usize = 32;
pub const MAX_PATH: usize = 256;
pub const MAX_VALUES: usize = 32;

/// `CSSearchableItemActionType`: the activity type of a result the user opened from Search.
pub const ACTION_TYPE: &str = "bifrost.quicksearch.item";

/// `CSSearchableItem` with its `CSSearchableItemAttributeSet` flattened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchableItem {
    /// `uniqueIdentifier`: unique within the app
    pub unique_id: String,
    /// `domainIdentifier`: groups items so they can be deleted together; may be empty
    pub domain: String,
    pub title: String,
    /// `contentDescription`
    pub description: String,
    pub keywords: Vec<String>,
    /// `thumbnailURL` as a path inside the app's data dir
    pub thumbnail: String,
    /// `contentURL`
    pub content_url: String,
}

/// `ToShell::SearchDelete::kind`.
pub const DELETE_IDS: u8 = 0;
pub const DELETE_DOMAINS: u8 = 1;
pub const DELETE_ALL: u8 = 2;

fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut n = max;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

fn put(b: &mut Vec<u8>, s: &str, max: usize) {
    str_(b, cut(s, max));
}

fn get(r: &mut Reader, max: usize) -> Result<String, DecodeError> {
    let s = r.str()?;
    if s.len() > max {
        return Err(DecodeError::BadValue);
    }
    Ok(s)
}

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    match m {
        ToShell::QuickSearchIndex(items) => {
            u32_(b, 0x1c0);
            let n = items.len().min(MAX_ITEMS);
            u32_(b, n as u32);
            for it in &items[..n] {
                put(b, &it.unique_id, MAX_ID);
                put(b, &it.domain, MAX_DOMAIN);
                put(b, &it.title, MAX_TITLE);
                put(b, &it.description, MAX_DESC);
                let k = it.keywords.len().min(MAX_KEYWORDS);
                u32_(b, k as u32);
                for w in &it.keywords[..k] {
                    put(b, w, MAX_KEYWORD);
                }
                put(b, &it.thumbnail, MAX_PATH);
                put(b, &it.content_url, MAX_PATH);
            }
        }
        ToShell::QuickSearchDelete { kind, values } => {
            u32_(b, 0x1c1);
            b.push(*kind);
            let n = values.len().min(MAX_VALUES);
            u32_(b, n as u32);
            for v in &values[..n] {
                put(b, v, MAX_ID);
            }
        }
        _ => {}
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    Ok(match t {
        0x1c0 => {
            let n = r.u32()? as usize;
            if n > MAX_ITEMS {
                return Err(DecodeError::BadValue);
            }
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                let unique_id = get(r, MAX_ID)?;
                let domain = get(r, MAX_DOMAIN)?;
                let title = get(r, MAX_TITLE)?;
                let description = get(r, MAX_DESC)?;
                let k = r.u32()? as usize;
                if k > MAX_KEYWORDS {
                    return Err(DecodeError::BadValue);
                }
                let keywords = (0..k).map(|_| get(r, MAX_KEYWORD)).collect::<Result<_, _>>()?;
                items.push(SearchableItem { unique_id, domain, title, description, keywords, thumbnail: get(r, MAX_PATH)?, content_url: get(r, MAX_PATH)? });
            }
            ToShell::QuickSearchIndex(items)
        }
        0x1c1 => {
            let kind = r.u8()?;
            if kind > DELETE_ALL {
                return Err(DecodeError::BadValue);
            }
            let n = r.u32()? as usize;
            if n > MAX_VALUES {
                return Err(DecodeError::BadValue);
            }
            ToShell::QuickSearchDelete { kind, values: (0..n).map(|_| get(r, MAX_ID)).collect::<Result<_, _>>()? }
        }
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

pub(crate) fn encode_app(m: &ToApp, b: &mut Vec<u8>) {
    match m {
        ToApp::ContinueActivity { activity_type, unique_id, domain } => {
            u32_(b, 0x1c2);
            put(b, activity_type, MAX_ID);
            put(b, unique_id, MAX_ID);
            put(b, domain, MAX_DOMAIN);
        }
        ToApp::ReindexAll => u32_(b, 0x1c3),
        ToApp::ReindexIdentifiers { identifiers } => {
            u32_(b, 0x1c4);
            u32_(b, identifiers.len() as u32);
            for id in identifiers {
                put(b, id, MAX_ID);
            }
        }
        _ => {}
    }
}

pub(crate) fn decode_app(t: u32, r: &mut Reader) -> Result<ToApp, DecodeError> {
    match t {
        0x1c2 => Ok(ToApp::ContinueActivity { activity_type: get(r, MAX_ID)?, unique_id: get(r, MAX_ID)?, domain: get(r, MAX_DOMAIN)? }),
        0x1c3 => Ok(ToApp::ReindexAll),
        0x1c4 => {
            let n = r.u32()? as usize;
            if n > MAX_ITEMS {
                return Err(DecodeError::BadValue);
            }
            Ok(ToApp::ReindexIdentifiers { identifiers: (0..n).map(|_| get(r, MAX_ID)).collect::<Result<_, _>>()? })
        }
        t => Err(DecodeError::UnknownTag(t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reindex_requests_round_trip() {
        for m in [ToApp::ReindexAll, ToApp::ReindexIdentifiers { identifiers: vec!["id1".into(), "id2".into()] }, ToApp::ReindexIdentifiers { identifiers: vec![] }] {
            assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()));
        }
    }

    fn item(n: usize) -> SearchableItem {
        SearchableItem { unique_id: format!("id{n}"), domain: "notes".into(), title: "Milk".into(), description: "Buy oat milk".into(), keywords: vec!["shop".into(), "ü".into()], thumbnail: "t.png".into(), content_url: "file:///a".into() }
    }

    #[test]
    fn messages_round_trip() {
        let m = ToShell::QuickSearchIndex(vec![item(1), item(2)]);
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        let d = ToShell::QuickSearchDelete { kind: DELETE_DOMAINS, values: vec!["notes".into()] };
        assert_eq!(ToShell::decode(&d.encode()), Ok(d));
        let c = ToApp::ContinueActivity { activity_type: ACTION_TYPE.into(), unique_id: "id1".into(), domain: "notes".into() };
        assert_eq!(ToApp::decode(&c.encode()), Ok(c));
    }

    #[test]
    fn full_packets_stay_under_the_limit_and_long_text_is_cut() {
        let big = SearchableItem { unique_id: "é".repeat(500), domain: "é".repeat(500), title: "é".repeat(500), description: "é".repeat(900), keywords: vec!["é".repeat(90); 20], thumbnail: "é".repeat(500), content_url: "é".repeat(500) };
        let m = ToShell::QuickSearchIndex(vec![big; 20]);
        let bytes = m.encode();
        assert!(bytes.len() <= crate::MAX_PACKET);
        let ToShell::QuickSearchIndex(items) = ToShell::decode(&bytes).unwrap() else { panic!() };
        assert_eq!(items.len(), MAX_ITEMS);
        assert!(items[0].unique_id.len() <= MAX_ID && items[0].keywords.len() == MAX_KEYWORDS);
    }

    #[test]
    fn bad_values_are_refused() {
        let mut b = Vec::new();
        u32_(&mut b, 0x1c1);
        b.push(9);
        u32_(&mut b, 0);
        assert_eq!(ToShell::decode(&b), Err(DecodeError::BadValue));
    }
}
