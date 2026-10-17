//! Notes in an Exchange mailbox, over Exchange Web Services (SOAP over HTTPS): the notes source that sits beside
//! [`dav_client`](crate::dav_client) for accounts whose server is Exchange and not CalDAV or CardDAV.
//!
//! A note is an item of class `IPM.StickyNote` in the mailbox's `notes` folder. This module builds the five requests a
//! notes sync needs (`FindItem`, `GetItem`, `CreateItem`, `UpdateItem`, `DeleteItem`), reads the answers, and walks the
//! folder: [`fetch_notes`] lists the items page by page and reads them in batches, [`create_note`], [`update_note`] and
//! [`delete_note`] write one note. Every call is blocking: run it on a background thread. The transport is a `send`
//! function (the same [`dav_client::Send`](crate::dav_client::Send) type, with [`session_send`](crate::dav_client::session_send)
//! as the real one), so the walk is tested with canned answers.
//!
//! Consent and privacy: the caller passes the `Authorization` value it got from the passwords store. The server URL
//! must be `https://`, so a password never travels in the clear; a plain `http://` URL is refused before anything is
//! sent. Nothing is sent to any server but the one the person typed for the account.
//!
//! Limits: at most [`MAX_NOTES`] notes are read in one walk and a body is cut at [`MAX_BODY_BYTES`]. Bodies are read
//! and written as plain text, so formatting in a note made elsewhere is flattened. Not here: finding the server URL
//! (autodiscover), OAuth sign-in, and the account kind in Settings; the Notes app passes the URL and login in.

use crate::dav_client::Send;
use crate::url::URLRequest;
use std::collections::BTreeMap;
use std::fmt;

/// Most notes one walk reads.
pub const MAX_NOTES: usize = 5_000;
/// Items asked for in one `GetItem` request.
pub const BATCH: usize = 50;
/// Items asked for in one `FindItem` page.
pub const PAGE: usize = 500;
/// A note body longer than this is cut (at a character boundary) when it is read or written.
pub const MAX_BODY_BYTES: usize = 1 << 20;
/// The item class of a note.
pub const NOTE_CLASS: &str = "IPM.StickyNote";

/// The address of an item: its identifier and the change key that says which version it is. The server refuses an
/// update or a delete that names an old change key only when the changes cannot be merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemId {
    pub id: String,
    pub change_key: String,
}

/// One note as the server keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub item: ItemId,
    pub subject: String,
    /// The plain-text body.
    pub body: String,
    /// `LastModifiedTime` as the server wrote it (an ISO 8601 UTC stamp such as `2026-10-17T09:30:00Z`).
    pub modified: String,
}

/// Why a call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExchangeError {
    /// The URL is not `https://`.
    NotSecure,
    /// A name, a subject or an identifier the call cannot carry (empty, or a control character).
    InvalidInput(&'static str),
    /// The transport failed before an answer came.
    Transport(String),
    /// The server refused the login (HTTP 401 or 403).
    Unauthorized,
    /// Any other HTTP status that is not 2xx.
    Status(u16),
    /// The server answered, and said no: its `MessageText`.
    Server(String),
    /// The answer is not what the request should get.
    Malformed(&'static str),
}

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExchangeError::NotSecure => write!(f, "The Exchange server address must start with https://"),
            ExchangeError::InvalidInput(what) => write!(f, "{what}"),
            ExchangeError::Transport(e) => write!(f, "{e}"),
            ExchangeError::Unauthorized => write!(f, "The Exchange server did not accept the login"),
            ExchangeError::Status(s) => write!(f, "The Exchange server answered {s}"),
            ExchangeError::Server(m) => write!(f, "{m}"),
            ExchangeError::Malformed(w) => write!(f, "The Exchange answer is not usable: {w}"),
        }
    }
}

impl std::error::Error for ExchangeError {}

// --- requests ----------------------------------------------------------------------------------------------------

/// Escapes text for an XML element or attribute value.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 cannot carry most control characters, even escaped
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// `s` cut to at most [`MAX_BODY_BYTES`] bytes, at a character boundary.
fn cap(s: &str) -> &str {
    if s.len() <= MAX_BODY_BYTES {
        return s;
    }
    let mut end = MAX_BODY_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The SOAP envelope around one request element.
fn envelope(body: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" "#,
            r#"xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types" "#,
            r#"xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages">"#,
            r#"<soap:Header><t:RequestServerVersion Version="Exchange2013"/></soap:Header>"#,
            "<soap:Body>{}</soap:Body></soap:Envelope>"
        ),
        body
    )
}

fn item_id_xml(id: &ItemId) -> String {
    format!(r#"<t:ItemId Id="{}" ChangeKey="{}"/>"#, escape(&id.id), escape(&id.change_key))
}

/// `FindItem` for one page of the notes folder: the identifiers and change keys of the items from `offset` on.
pub fn find_notes_request(offset: usize) -> String {
    envelope(&format!(
        concat!(
            r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>"#,
            r#"<m:IndexedPageItemView MaxEntriesReturned="{}" Offset="{}" BasePoint="Beginning"/>"#,
            r#"<m:ParentFolderIds><t:DistinguishedFolderId Id="notes"/></m:ParentFolderIds></m:FindItem>"#
        ),
        PAGE, offset
    ))
}

/// `GetItem` for the notes in `ids`: subject, plain-text body and last modified time.
pub fn get_notes_request(ids: &[ItemId]) -> String {
    let list: String = ids.iter().map(item_id_xml).collect();
    envelope(&format!(
        concat!(
            r#"<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:BodyType>Text</t:BodyType><t:AdditionalProperties>"#,
            r#"<t:FieldURI FieldURI="item:Subject"/><t:FieldURI FieldURI="item:Body"/>"#,
            r#"<t:FieldURI FieldURI="item:LastModifiedTime"/></t:AdditionalProperties></m:ItemShape>"#,
            "<m:ItemIds>{}</m:ItemIds></m:GetItem>"
        ),
        list
    ))
}

/// `CreateItem` for a new note in the notes folder.
pub fn create_note_request(subject: &str, body: &str) -> String {
    envelope(&format!(
        concat!(
            r#"<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id="notes"/></m:SavedItemFolderId><m:Items>"#,
            r#"<t:Item><t:ItemClass>{}</t:ItemClass><t:Subject>{}</t:Subject><t:Body BodyType="Text">{}</t:Body></t:Item>"#,
            "</m:Items></m:CreateItem>"
        ),
        NOTE_CLASS,
        escape(subject),
        escape(cap(body))
    ))
}

/// `UpdateItem` that sets the subject and the body of the note `id`. A conflict with a newer change that cannot be
/// merged is the server's to refuse (`ConflictResolution` is `AutoResolve`).
pub fn update_note_request(id: &ItemId, subject: &str, body: &str) -> String {
    envelope(&format!(
        concat!(
            r#"<m:UpdateItem ConflictResolution="AutoResolve" MessageDisposition="SaveOnly"><m:ItemChanges>"#,
            "<t:ItemChange>{}<t:Updates>",
            r#"<t:SetItemField><t:FieldURI FieldURI="item:Subject"/><t:Item><t:Subject>{}</t:Subject></t:Item></t:SetItemField>"#,
            r#"<t:SetItemField><t:FieldURI FieldURI="item:Body"/><t:Item><t:Body BodyType="Text">{}</t:Body></t:Item></t:SetItemField>"#,
            "</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"
        ),
        item_id_xml(id),
        escape(subject),
        escape(cap(body))
    ))
}

/// `DeleteItem` for the note `id`: the note moves to the mailbox's Deleted Items, so a delete can be undone there.
pub fn delete_note_request(id: &ItemId) -> String {
    envelope(&format!(
        r#"<m:DeleteItem DeleteType="MoveToDeletedItems"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>"#,
        item_id_xml(id)
    ))
}

// --- answers -----------------------------------------------------------------------------------------------------

/// One element found in an answer: the text of its attributes and everything between its tags.
struct Element<'a> {
    attrs: &'a str,
    inner: &'a str,
}

impl<'a> Element<'a> {
    /// The value of attribute `name`, with its entities decoded.
    fn attr(&self, name: &str) -> Option<String> {
        let key = format!(" {name}=\"");
        let start = self.attrs.find(&key)? + key.len();
        let len = self.attrs[start..].find('"')?;
        Some(unescape(&self.attrs[start..start + len]))
    }

    /// The decoded text of the first child element named `name`.
    fn child_text(&self, name: &str) -> Option<String> {
        elements(self.inner, |n| n == name).into_iter().next().map(|e| unescape(e.inner))
    }
}

/// The name of an element without its namespace prefix.
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Every top-level element whose local name `want` accepts, in document order. An element inside a match is not listed
/// again on its own; look inside it with [`Element::child_text`] or another call.
fn elements<'a>(xml: &'a str, want: impl Fn(&str) -> bool) -> Vec<Element<'a>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(lt) = xml[pos..].find('<') {
        let tag_start = pos + lt + 1;
        let Some(gt) = xml[tag_start..].find('>') else { break };
        let tag = &xml[tag_start..tag_start + gt];
        let after = tag_start + gt + 1;
        // comments, declarations, processing instructions, CDATA and closing tags
        if tag.starts_with(['!', '?', '/']) {
            pos = if let Some(cdata) = xml[tag_start - 1..].strip_prefix("<![CDATA[") {
                let skipped = cdata.find("]]>").map_or(xml.len(), |e| tag_start - 1 + 9 + e + 3);
                skipped.max(after)
            } else {
                after
            };
            continue;
        }
        let name_end = tag.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(tag.len());
        let qualified = &tag[..name_end];
        if !want(local(qualified)) {
            pos = after;
            continue;
        }
        let attrs = &tag[name_end..];
        if tag.ends_with('/') {
            out.push(Element { attrs, inner: "" });
            pos = after;
            continue;
        }
        // find the matching close tag, counting nested elements of the same name
        let (open, close) = (format!("<{qualified}"), format!("</{qualified}>"));
        let mut depth = 1;
        let mut scan = after;
        let mut end = None;
        while depth > 0 {
            let next_close = xml[scan..].find(&close).map(|i| scan + i);
            let next_open = xml[scan..].find(&open).map(|i| scan + i).filter(|&i| {
                let rest = &xml[i + open.len()..];
                matches!(rest.chars().next(), Some(c) if c.is_whitespace() || c == '>' || c == '/')
            });
            match (next_open, next_close) {
                (Some(o), Some(c)) if o < c => {
                    let self_closing = xml[o..].find('>').is_some_and(|g| xml[o..o + g].ends_with('/'));
                    if !self_closing {
                        depth += 1;
                    }
                    scan = o + open.len();
                }
                (_, Some(c)) => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(c);
                    }
                    scan = c + close.len();
                }
                _ => break,
            }
        }
        match end {
            Some(c) => {
                out.push(Element { attrs, inner: &xml[after..c] });
                pos = c + close.len();
            }
            None => break,
        }
    }
    out
}

/// Decodes the five predefined XML entities and numeric character references. Unknown ones stay as written.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after.find(';').filter(|&semi| semi <= 10).and_then(|semi| {
            let ent = &after[..semi];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => ent
                    .strip_prefix('#')
                    .and_then(|n| match n.strip_prefix('x').or_else(|| n.strip_prefix('X')) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => n.parse::<u32>().ok(),
                    })
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, semi))
        });
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The response messages of an answer (`FindItemResponseMessage` and its kin), each checked: a message whose
/// `ResponseClass` is `Error` is returned as `Err` with the server's own words.
fn messages(xml: &str) -> Vec<Result<Element<'_>, ExchangeError>> {
    elements(xml, |n| n.ends_with("ResponseMessage"))
        .into_iter()
        .map(|e| match e.attr("ResponseClass").as_deref() {
            Some("Error") => Err(ExchangeError::Server(e.child_text("MessageText").unwrap_or_else(|| "The Exchange server reported an error".into()))),
            _ => Ok(e),
        })
        .collect()
}

fn item_id_of(e: &Element<'_>) -> Option<ItemId> {
    let id = elements(e.inner, |n| n == "ItemId").into_iter().next()?;
    Some(ItemId { id: id.attr("Id")?, change_key: id.attr("ChangeKey").unwrap_or_default() })
}

/// The items one `FindItem` page lists, and whether it was the last page. A server fault in the page is an error.
pub fn parse_find(xml: &str) -> Result<(Vec<ItemId>, bool), ExchangeError> {
    let mut ids = Vec::new();
    let mut last = true;
    let msgs = messages(xml);
    if msgs.is_empty() {
        return Err(ExchangeError::Malformed("no FindItem response"));
    }
    for m in msgs {
        let m = m?;
        for folder in elements(m.inner, |n| n == "RootFolder") {
            if folder.attr("IncludesLastItemInRange").as_deref() == Some("false") {
                last = false;
            }
        }
        ids.extend(elements(m.inner, |n| n == "ItemId").iter().filter_map(|e| Some(ItemId { id: e.attr("Id")?, change_key: e.attr("ChangeKey").unwrap_or_default() })));
    }
    Ok((ids, last))
}

/// The notes a `GetItem` answer holds. A message that failed for one item (for example it was deleted since the list
/// was read) is skipped, and the others are kept; a fault that names no item fails the call.
pub fn parse_notes(xml: &str) -> Result<Vec<Note>, ExchangeError> {
    let msgs = messages(xml);
    if msgs.is_empty() {
        return Err(ExchangeError::Malformed("no GetItem response"));
    }
    let mut out = Vec::new();
    let mut faults = Vec::new();
    for m in msgs {
        match m {
            Ok(m) => {
                for item in elements(m.inner, |n| n == "Item" || n == "Message" || n == "Post") {
                    let Some(id) = item_id_of(&item) else { continue };
                    out.push(Note {
                        item: id,
                        subject: item.child_text("Subject").unwrap_or_default(),
                        body: item.child_text("Body").map(|b| cap(&b).to_string()).unwrap_or_default(),
                        modified: item.child_text("LastModifiedTime").unwrap_or_default(),
                    });
                }
            }
            Err(e) => faults.push(e),
        }
    }
    match (out.is_empty(), faults.into_iter().next()) {
        (true, Some(fault)) => Err(fault),
        _ => Ok(out),
    }
}

/// The identifier a `CreateItem` or `UpdateItem` answer names (the new change key after an update).
pub fn parse_written(xml: &str) -> Result<ItemId, ExchangeError> {
    for m in messages(xml) {
        if let Some(id) = item_id_of(&m?) {
            return Ok(id);
        }
    }
    Err(ExchangeError::Malformed("no item in the answer"))
}

/// Checks that a `DeleteItem` answer reports success.
pub fn parse_deleted(xml: &str) -> Result<(), ExchangeError> {
    let msgs = messages(xml);
    if msgs.is_empty() {
        return Err(ExchangeError::Malformed("no DeleteItem response"));
    }
    msgs.into_iter().try_for_each(|m| m.map(|_| ()))
}

// --- the walk ----------------------------------------------------------------------------------------------------

fn check_url(url: &str) -> Result<(), ExchangeError> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err(ExchangeError::NotSecure)
    }
}

fn check_id(id: &ItemId) -> Result<(), ExchangeError> {
    let clean = |s: &str| !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control);
    if clean(&id.id) && (id.change_key.is_empty() || clean(&id.change_key)) {
        Ok(())
    } else {
        Err(ExchangeError::InvalidInput("The note identifier is empty or holds a control character"))
    }
}

fn check_subject(subject: &str) -> Result<(), ExchangeError> {
    if subject.chars().any(|c| c == '\n' || c == '\r') {
        Err(ExchangeError::InvalidInput("A note subject is one line"))
    } else {
        Ok(())
    }
}

/// Posts one SOAP request and returns the answer text.
fn post(send: &mut Send<'_>, url: &str, auth: &str, body: String) -> Result<String, ExchangeError> {
    check_url(url)?;
    let mut req = URLRequest::post(url, body.into_bytes());
    req.set_header("Authorization", auth);
    req.set_header("Content-Type", "text/xml; charset=utf-8");
    let (status, text) = send(&req).map_err(ExchangeError::Transport)?;
    match status {
        401 | 403 => Err(ExchangeError::Unauthorized),
        // a SOAP fault comes as 500 with a body that names it; surface the body's words when it has them
        500 => match elements(&text, |n| n == "faultstring").into_iter().next() {
            Some(f) => Err(ExchangeError::Server(unescape(f.inner.trim()))),
            None => Err(ExchangeError::Status(500)),
        },
        200..=299 => Ok(text),
        other => Err(ExchangeError::Status(other)),
    }
}

/// Every note in the mailbox's notes folder, up to [`MAX_NOTES`]: the folder is listed page by page and the items are
/// read [`BATCH`] at a time.
pub fn fetch_notes(send: &mut Send<'_>, url: &str, auth: &str) -> Result<Vec<Note>, ExchangeError> {
    let mut ids: Vec<ItemId> = Vec::new();
    loop {
        let (page, last) = parse_find(&post(send, url, auth, find_notes_request(ids.len()))?)?;
        let got = page.len();
        ids.extend(page);
        if last || got == 0 || ids.len() >= MAX_NOTES {
            break;
        }
    }
    ids.truncate(MAX_NOTES);
    let mut notes = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(BATCH) {
        notes.extend(parse_notes(&post(send, url, auth, get_notes_request(chunk))?)?);
    }
    Ok(notes)
}

/// Makes a new note and returns its address.
pub fn create_note(send: &mut Send<'_>, url: &str, auth: &str, subject: &str, body: &str) -> Result<ItemId, ExchangeError> {
    check_subject(subject)?;
    parse_written(&post(send, url, auth, create_note_request(subject, body))?)
}

/// Replaces the subject and body of a note and returns its address with the new change key.
pub fn update_note(send: &mut Send<'_>, url: &str, auth: &str, id: &ItemId, subject: &str, body: &str) -> Result<ItemId, ExchangeError> {
    check_id(id)?;
    check_subject(subject)?;
    parse_written(&post(send, url, auth, update_note_request(id, subject, body))?)
}

/// Moves a note to Deleted Items.
pub fn delete_note(send: &mut Send<'_>, url: &str, auth: &str, id: &ItemId) -> Result<(), ExchangeError> {
    check_id(id)?;
    parse_deleted(&post(send, url, auth, delete_note_request(id))?)
}

/// What changed between two reads of the folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Notes in `now` that were not in `before`.
    pub added: Vec<Note>,
    /// Notes in both whose change key differs.
    pub changed: Vec<Note>,
    /// The identifiers of notes in `before` that `now` no longer has.
    pub removed: Vec<String>,
}

/// Compares two reads by item identifier and change key, so an app imports only what moved since its last sync.
pub fn changes(before: &[Note], now: &[Note]) -> Changes {
    let old: BTreeMap<&str, &Note> = before.iter().map(|n| (n.item.id.as_str(), n)).collect();
    let mut out = Changes::default();
    for n in now {
        match old.get(n.item.id.as_str()) {
            None => out.added.push(n.clone()),
            Some(o) if o.item.change_key != n.item.change_key => out.changed.push(n.clone()),
            Some(_) => {}
        }
    }
    let kept: std::collections::BTreeSet<&str> = now.iter().map(|n| n.item.id.as_str()).collect();
    out.removed = before.iter().filter(|n| !kept.contains(n.item.id.as_str())).map(|n| n.item.id.clone()).collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://mail.example.com/EWS/Exchange.asmx";

    fn id(n: &str) -> ItemId {
        ItemId { id: format!("AAMk{n}"), change_key: format!("CQ{n}") }
    }

    fn find_answer(ids: &[&str], last: bool) -> String {
        let items: String = ids.iter().map(|n| format!(r#"<t:Item><t:ItemId Id="AAMk{n}" ChangeKey="CQ{n}"/></t:Item>"#)).collect();
        format!(
            r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body>
            <m:FindItemResponse><m:ResponseMessages><m:FindItemResponseMessage ResponseClass="Success">
            <m:ResponseCode>NoError</m:ResponseCode><m:RootFolder IndexedPagingOffset="{}" IncludesLastItemInRange="{last}">
            <t:Items>{items}</t:Items></m:RootFolder></m:FindItemResponseMessage></m:ResponseMessages></m:FindItemResponse></s:Body></s:Envelope>"#,
            ids.len()
        )
    }

    fn get_answer(notes: &[(&str, &str, &str)]) -> String {
        let msgs: String = notes
            .iter()
            .map(|(n, subject, body)| {
                format!(
                    r#"<m:GetItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:Items>
                    <t:Item><t:ItemId Id="AAMk{n}" ChangeKey="CQ{n}"/><t:Subject>{subject}</t:Subject>
                    <t:Body BodyType="Text">{body}</t:Body><t:LastModifiedTime>2026-10-17T09:30:00Z</t:LastModifiedTime></t:Item>
                    </m:Items></m:GetItemResponseMessage>"#
                )
            })
            .collect();
        format!(r#"<s:Envelope xmlns:s="x"><s:Body><m:GetItemResponse><m:ResponseMessages>{msgs}</m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>"#)
    }

    #[test]
    fn requests_name_the_notes_folder_and_escape_what_the_person_typed() {
        let find = find_notes_request(40);
        assert!(find.contains(r#"<t:DistinguishedFolderId Id="notes"/>"#));
        assert!(find.contains(r#"Offset="40""#) && find.contains(r#"MaxEntriesReturned="500""#));
        let create = create_note_request("Milk & <eggs>", "line 1\nline \"2\"");
        assert!(create.contains("<t:ItemClass>IPM.StickyNote</t:ItemClass>"));
        assert!(create.contains("<t:Subject>Milk &amp; &lt;eggs&gt;</t:Subject>"));
        assert!(create.contains("line 1\nline &quot;2&quot;"));
        let update = update_note_request(&id("1"), "s", "b");
        assert!(update.contains(r#"<t:ItemId Id="AAMk1" ChangeKey="CQ1"/>"#));
        assert!(update.contains(r#"FieldURI="item:Subject""#) && update.contains(r#"FieldURI="item:Body""#));
        assert!(delete_note_request(&id("1")).contains(r#"DeleteType="MoveToDeletedItems""#));
        assert!(get_notes_request(&[id("1"), id("2")]).matches("<t:ItemId ").count() == 2);
    }

    #[test]
    fn control_characters_cannot_break_the_xml() {
        let req = create_note_request("a\u{0}b\u{1b}", "x\u{7}y");
        assert!(req.contains("<t:Subject>ab</t:Subject>"));
        assert!(req.contains(">xy</t:Body>"));
    }

    #[test]
    fn a_find_page_lists_ids_and_says_whether_it_was_the_last() {
        let (ids, last) = parse_find(&find_answer(&["1", "2"], false)).unwrap();
        assert_eq!(ids, vec![id("1"), id("2")]);
        assert!(!last);
        assert!(parse_find(&find_answer(&["3"], true)).unwrap().1);
        assert_eq!(parse_find("<x/>").unwrap_err(), ExchangeError::Malformed("no FindItem response"));
    }

    #[test]
    fn a_get_answer_gives_subject_body_and_time_with_entities_decoded() {
        let notes = parse_notes(&get_answer(&[("1", "Milk &amp; eggs", "two &lt;dozen&gt;\nand &#x41;"), ("2", "Empty", "")])).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].item, id("1"));
        assert_eq!(notes[0].subject, "Milk & eggs");
        assert_eq!(notes[0].body, "two <dozen>\nand A");
        assert_eq!(notes[0].modified, "2026-10-17T09:30:00Z");
        assert_eq!(notes[1].body, "");
    }

    #[test]
    fn one_failed_item_is_skipped_but_a_wholly_failed_answer_is_an_error() {
        let bad = r#"<m:GetItemResponseMessage ResponseClass="Error"><m:MessageText>The specified object was not found in the store.</m:MessageText><m:ResponseCode>ErrorItemNotFound</m:ResponseCode></m:GetItemResponseMessage>"#;
        let good = get_answer(&[("1", "kept", "b")]);
        let mixed = good.replace("</m:ResponseMessages>", &format!("{bad}</m:ResponseMessages>"));
        assert_eq!(parse_notes(&mixed).unwrap().len(), 1);
        let only_bad = format!("<s:Body><m:GetItemResponse><m:ResponseMessages>{bad}</m:ResponseMessages></m:GetItemResponse></s:Body>");
        assert_eq!(parse_notes(&only_bad).unwrap_err(), ExchangeError::Server("The specified object was not found in the store.".into()));
    }

    #[test]
    fn a_write_answer_names_the_new_change_key() {
        let ok = r#"<m:UpdateItemResponse><m:ResponseMessages><m:UpdateItemResponseMessage ResponseClass="Success"><m:Items>
            <t:Item><t:ItemId Id="AAMk1" ChangeKey="CQnew"/></t:Item></m:Items></m:UpdateItemResponseMessage></m:ResponseMessages></m:UpdateItemResponse>"#;
        assert_eq!(parse_written(ok).unwrap(), ItemId { id: "AAMk1".into(), change_key: "CQnew".into() });
        let refused = r#"<m:UpdateItemResponseMessage ResponseClass="Error"><m:MessageText>Conflict</m:MessageText></m:UpdateItemResponseMessage>"#;
        assert_eq!(parse_written(refused).unwrap_err(), ExchangeError::Server("Conflict".into()));
        assert!(parse_deleted(r#"<m:DeleteItemResponseMessage ResponseClass="Success"/>"#).is_ok());
        assert!(parse_deleted("<x/>").is_err());
    }

    #[test]
    fn a_plain_http_server_is_refused_before_anything_is_sent() {
        let mut sent = 0;
        let mut send = |_: &URLRequest| {
            sent += 1;
            Ok((200, String::new()))
        };
        assert_eq!(fetch_notes(&mut send, "http://mail.example.com/EWS/Exchange.asmx", "Basic x").unwrap_err(), ExchangeError::NotSecure);
        assert_eq!(sent, 0);
    }

    #[test]
    fn the_walk_pages_the_folder_then_reads_the_items_in_batches() {
        let mut calls: Vec<String> = Vec::new();
        let mut send = |req: &URLRequest| {
            let body = String::from_utf8(req.body.clone().unwrap()).unwrap();
            assert_eq!(req.method, "POST");
            assert_eq!(req.url, URL);
            assert!(req.headers.iter().any(|(k, v)| k == "Authorization" && v == "Basic abc"));
            if body.contains("<m:FindItem ") {
                calls.push(if body.contains(r#"Offset="0""#) { "find0".into() } else { "find2".into() });
                Ok((200, if body.contains(r#"Offset="0""#) { find_answer(&["1", "2"], false) } else { find_answer(&["3"], true) }))
            } else {
                let n = body.matches("<t:ItemId ").count();
                calls.push(format!("get{n}"));
                let rows: Vec<(String, String, String)> = body
                    .split(r#"Id="AAMk"#)
                    .skip(1)
                    .map(|s| s.split('"').next().unwrap().to_string())
                    .map(|n| (n.clone(), format!("note {n}"), "body".to_string()))
                    .collect();
                let refs: Vec<(&str, &str, &str)> = rows.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())).collect();
                Ok((200, get_answer(&refs)))
            }
        };
        let notes = fetch_notes(&mut send, URL, "Basic abc").unwrap();
        assert_eq!(notes.iter().map(|n| n.subject.as_str()).collect::<Vec<_>>(), vec!["note 1", "note 2", "note 3"]);
        assert_eq!(calls, vec!["find0", "find2", "get3"]);
    }

    #[test]
    fn http_failures_are_told_apart() {
        let mut denied = |_: &URLRequest| Ok((401, String::new()));
        assert_eq!(fetch_notes(&mut denied, URL, "Basic x").unwrap_err(), ExchangeError::Unauthorized);
        let mut fault = |_: &URLRequest| Ok((500, "<s:Fault><faultstring>Bad request &amp; worse</faultstring></s:Fault>".to_string()));
        assert_eq!(fetch_notes(&mut fault, URL, "Basic x").unwrap_err(), ExchangeError::Server("Bad request & worse".into()));
        let mut down = |_: &URLRequest| Err("offline".to_string());
        assert_eq!(fetch_notes(&mut down, URL, "Basic x").unwrap_err(), ExchangeError::Transport("offline".into()));
        let mut gone = |_: &URLRequest| Ok((404, String::new()));
        assert_eq!(fetch_notes(&mut gone, URL, "Basic x").unwrap_err(), ExchangeError::Status(404));
    }

    #[test]
    fn write_calls_check_their_input_and_send_one_request() {
        let ok = r#"<m:CreateItemResponseMessage ResponseClass="Success"><t:Item><t:ItemId Id="AAMk9" ChangeKey="CQ9"/></t:Item></m:CreateItemResponseMessage>"#;
        let mut sent = Vec::new();
        let mut send = |req: &URLRequest| {
            sent.push(String::from_utf8(req.body.clone().unwrap()).unwrap());
            Ok((200, ok.to_string()))
        };
        assert_eq!(create_note(&mut send, URL, "Basic x", "Title", "Body").unwrap(), id("9"));
        assert!(matches!(create_note(&mut send, URL, "Basic x", "two\nlines", "b"), Err(ExchangeError::InvalidInput(_))));
        let blank = ItemId { id: String::new(), change_key: String::new() };
        assert!(matches!(update_note(&mut send, URL, "Basic x", &blank, "s", "b"), Err(ExchangeError::InvalidInput(_))));
        assert!(matches!(delete_note(&mut send, URL, "Basic x", &blank), Err(ExchangeError::InvalidInput(_))));
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("<m:CreateItem>"));
    }

    #[test]
    fn a_long_body_is_cut_at_a_character_boundary() {
        let long = "é".repeat(MAX_BODY_BYTES);
        let cut = cap(&long);
        assert!(cut.len() <= MAX_BODY_BYTES && cut.len() > MAX_BODY_BYTES - 2);
        assert!(long.is_char_boundary(cut.len()));
    }

    #[test]
    fn changes_say_what_was_added_changed_and_removed() {
        let note = |n: &str, key: &str| Note { item: ItemId { id: format!("AAMk{n}"), change_key: key.into() }, subject: n.into(), body: String::new(), modified: String::new() };
        let before = vec![note("1", "a"), note("2", "a"), note("3", "a")];
        let now = vec![note("1", "a"), note("2", "b"), note("4", "a")];
        let c = changes(&before, &now);
        assert_eq!(c.added.iter().map(|n| n.subject.as_str()).collect::<Vec<_>>(), vec!["4"]);
        assert_eq!(c.changed.iter().map(|n| n.subject.as_str()).collect::<Vec<_>>(), vec!["2"]);
        assert_eq!(c.removed, vec!["AAMk3".to_string()]);
        assert_eq!(changes(&now, &now), Changes::default());
    }

    #[test]
    fn nested_elements_of_the_same_name_are_matched_by_depth() {
        let xml = "<a:Items><a:Items><a:Items/>x</a:Items>y</a:Items><a:Items>z</a:Items>";
        let found = elements(xml, |n| n == "Items");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].inner, "<a:Items><a:Items/>x</a:Items>y");
        assert_eq!(found[1].inner, "z");
    }
}
