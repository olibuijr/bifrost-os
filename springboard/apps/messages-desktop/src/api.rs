//! The HTTP side of the Matrix client: one [`Api`] per signed-in account. Calls go through the shell's HTTPS client
//! (`springboard::deildu::http`), so certificates are checked against the same roots as the rest of the desktop. The
//! URLs and bodies come from [`crate::protocol`]. Every method blocks, so the window calls them from job threads
//! (see [`crate::sync`]).

use crate::attachment;
use crate::crypto::{self, Machine};
use crate::protocol::{self, Body, Ciphertext, Login, Sync, ToDevice};
use bifrost_core::json::{self, Json};
use springboard::deildu::http::{Client, Jar, Response, Transport};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The largest sync answer read. A first sync of a busy account is large.
const SYNC_LIMIT: usize = 32 * 1024 * 1024;
/// The largest answer of any other call.
const CALL_LIMIT: usize = 1024 * 1024;
/// The largest image the app uploads.
pub const UPLOAD_LIMIT: usize = 25 * 1024 * 1024;
/// The most messages kept waiting for a room key. The oldest are dropped first.
const MAX_PENDING: usize = 500;

/// The bodies that now open in one room, for the window: the room id, then (event id, body, the event it answers).
pub type Patch = (String, Vec<(String, Body, Option<String>)>);

/// The key work one sync leaves for the keys worker (see [`Api::run_key_work`]).
pub struct KeyWork {
    upload: bool,
    device_changed: Vec<String>,
    to_device: Vec<ToDevice>,
}

/// A signed-in session on one homeserver.
pub struct Api {
    http: Client,
    base: String,
    token: String,
    pub user_id: String,
    pub device_id: String,
    /// The end-to-end encryption state of this device (see [`crate::crypto`]).
    crypto: Mutex<Machine>,
    /// The rooms that have `m.room.encryption`, as the syncs reported them. A send to one of them is encrypted.
    encrypted_rooms: Mutex<HashSet<String>>,
    /// The joined members of each room, as the syncs reported them.
    room_members: Mutex<HashMap<String, Vec<String>>>,
    /// Encrypted messages that did not open yet, waiting for their room key.
    pending_ciphertexts: Mutex<Vec<(String, Ciphertext)>>,
}

impl Api {
    /// Signs in with a user name and password. Makes a new device on the account, so the token is per start.
    pub fn login(transport: Arc<dyn Transport>, base: &str, user: &str, password: &str, device_hint: Option<&str>) -> Result<Api, String> {
        let http = Client::new(transport, Jar::default()).require_https();
        let body = protocol::login_body(user, password, device_hint);
        let url = protocol::login_url(base);
        let answer = call(&http, "POST", &url, None, Some((body.as_bytes(), "application/json")), CALL_LIMIT)?;
        let Login { user_id, access_token, device_id } = protocol::parse_login(&answer)?;
        let crypto = Mutex::new(Machine::new(&user_id, &device_id));
        Ok(Api {
            http,
            base: base.to_string(),
            token: access_token,
            user_id,
            device_id,
            crypto,
            encrypted_rooms: Mutex::new(HashSet::new()),
            room_members: Mutex::new(HashMap::new()),
            pending_ciphertexts: Mutex::new(Vec::new()),
        })
    }

    /// Installs the crypto state of this login. `stored` is the device the keychain holds for the account (its id and
    /// pickle) and `key` its pickle key; the stored device is used only when the server gave back the same device id.
    /// Otherwise a new device is made. Returns true when the stored device was restored. The state is marked dirty, so
    /// the next [`Api::take_snapshot`] writes it.
    pub fn install_crypto(&self, key: Option<[u8; 32]>, stored: Option<(&str, &str)>, sessions: Option<&str>) -> Result<bool, String> {
        let key = match key {
            Some(key) => key,
            None => crypto::new_pickle_key()?,
        };
        let mut restored = false;
        let mut next = None;
        if let Some((device, pickled)) = stored {
            if device == self.device_id {
                if let Ok(mut m) = Machine::restore(&self.user_id, &self.device_id, &key, pickled) {
                    if let Some(blob) = sessions {
                        let _ = m.load_sessions(&key, blob);
                    }
                    next = Some(m);
                    restored = true;
                }
            }
        }
        let mut machine = next.unwrap_or_else(|| Machine::new(&self.user_id, &self.device_id));
        machine.set_pickle_key(key);
        *self.crypto.lock().map_err(|_| "the encryption state is unavailable".to_string())? = machine;
        Ok(restored)
    }

    /// The keychain items to write, when the crypto state changed since the last call (see [`Machine::snapshot`]).
    pub fn take_snapshot(&self) -> Option<crypto::Snapshot> {
        self.crypto.lock().ok()?.snapshot()
    }

    /// Looks at a sync answer on the sync thread, before the window sees it: the rooms' encryption state and members are
    /// noted, and the encrypted messages that the keys already held open are opened. The network work (device queries,
    /// room keys, key uploads) is returned as a [`KeyWork`] for the keys worker, so a slow homeserver never holds the
    /// sync. Messages that do not open yet show as undecryptable, and are opened later (see `run_key_work`).
    pub fn sync_crypto(&self, sync: &mut Sync) -> Option<KeyWork> {
        if let Ok(mut rooms) = self.encrypted_rooms.lock() {
            for room in sync.rooms.iter().filter(|r| r.encrypted) {
                rooms.insert(room.id.clone());
            }
        }
        // Who is joined to each room, as far as the syncs say (the send path asks the server for the exact list).
        if let Ok(mut joined) = self.room_members.lock() {
            for room in &sync.rooms {
                let members = joined.entry(room.id.clone()).or_default();
                for m in &room.members {
                    if m.membership == "join" {
                        if !members.contains(&m.user_id) {
                            members.push(m.user_id.clone());
                        }
                    } else {
                        members.retain(|u| *u != m.user_id);
                    }
                }
            }
        }
        // Open what the keys already held open. The rest waits for the key work (see `run_key_work`).
        let mut failed: Vec<(String, Ciphertext)> = Vec::new();
        for room in sync.rooms.iter_mut().filter(|r| !r.ciphertexts.is_empty()) {
            if let Ok(mut m) = self.crypto.lock() {
                m.decrypt_events(&room.id, &mut room.events, &room.ciphertexts);
            }
            for ct in &room.ciphertexts {
                if room.events.iter().any(|e| e.id == ct.id && e.body == Body::Undecryptable) {
                    failed.push((room.id.clone(), ct.clone()));
                }
            }
        }
        if let Ok(mut pending) = self.pending_ciphertexts.lock() {
            pending.extend(failed);
            let excess = pending.len().saturating_sub(MAX_PENDING);
            pending.drain(..excess);
        }
        let to_device: Vec<ToDevice> = sync
            .to_device
            .iter()
            .filter(|e| e.kind == "m.room.encrypted" || e.kind == "m.room_key_request")
            .cloned()
            .collect();
        let upload = sync.otk_count.map_or(true, |n| n < crypto::OTK_FLOOR);
        let has_pending = self.pending_ciphertexts.lock().map(|p| !p.is_empty()).unwrap_or(false);
        if !upload && to_device.is_empty() && sync.device_changed.is_empty() && !has_pending {
            return None;
        }
        Some(KeyWork { upload, device_changed: sync.device_changed.clone(), to_device })
    }

    /// The key work of one sync, on the keys worker (not the sync thread): uploads keys when the server runs low,
    /// queries the devices of the users that changed or sent room keys and requests, takes in the room keys, answers the
    /// key requests of this user's devices, shares room keys with devices that appeared, and opens the messages that were
    /// waiting for a key. Returns the bodies that now open, per room, for the window.
    pub fn run_key_work(&self, work: KeyWork) -> Vec<Patch> {
        if work.upload {
            let _ = self.upload_keys();
        }
        let mut users: Vec<String> = work.device_changed.clone();
        users.extend(work.to_device.iter().filter(|e| e.kind == "m.room.encrypted" || e.kind == "m.room_key_request").map(|e| e.sender.clone()));
        users.sort();
        users.dedup();
        if !users.is_empty() {
            let _ = self.query_devices(&users);
        }
        if work.to_device.iter().any(|e| e.kind == "m.room.encrypted") {
            if let Ok(mut m) = self.crypto.lock() {
                m.receive(&work.to_device);
            }
        }
        self.answer_key_requests(&work.to_device);
        for (room, members) in self.rooms_with_new_devices(&work.device_changed) {
            let _ = self.share_with_new_devices(&room, &members);
        }
        // Retry the messages still waiting. Those that open now are patched in the window.
        let waiting: Vec<(String, Ciphertext)> = self.pending_ciphertexts.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
        let mut still = Vec::new();
        let mut patches: Vec<Patch> = Vec::new();
        for (room, ct) in waiting {
            match self.with_crypto(|m| Ok(m.open_room_event(&room, &ct))).ok().flatten() {
                Some((body, reply)) => match patches.iter_mut().find(|(r, _)| *r == room) {
                    Some((_, events)) => events.push((ct.id.clone(), body, reply)),
                    None => patches.push((room.clone(), vec![(ct.id.clone(), body, reply)])),
                },
                None => still.push((room, ct)),
            }
        }
        if let Ok(mut pending) = self.pending_ciphertexts.lock() {
            pending.extend(still);
        }
        patches
    }

    /// The devices of `users` as (user, device id), for the room info. Every one is unverified (see `Machine::devices_of`).
    pub fn device_list(&self, users: &[String]) -> Vec<(String, String)> {
        self.with_crypto(|m| Ok(users.iter().flat_map(|u| m.devices_of(u)).map(|d| (d.user_id, d.device_id)).collect::<Vec<_>>()))
            .unwrap_or_default()
    }

    /// Whether `room` is end to end encrypted, as the syncs said.
    pub fn is_encrypted(&self, room: &str) -> bool {
        self.encrypted_rooms.lock().map(|r| r.contains(room)).unwrap_or(false)
    }

    /// The encrypted rooms that have a member whose device list just changed, with their members.
    fn rooms_with_new_devices(&self, changed: &[String]) -> Vec<(String, Vec<String>)> {
        if changed.is_empty() {
            return Vec::new();
        }
        let (Ok(rooms), Ok(joined)) = (self.encrypted_rooms.lock(), self.room_members.lock()) else { return Vec::new() };
        rooms
            .iter()
            .filter_map(|room| joined.get(room).map(|members| (room.clone(), members.clone())))
            .filter(|(_, members)| members.iter().any(|m| changed.contains(m)))
            .collect()
    }

    /// Sends the room key of `room` to the devices of `members` that do not hold it yet (devices that appeared since the
    /// key was shared; see [`Machine::pending_shares`]). Claims one-time keys for the ones without an Olm session first.
    fn share_with_new_devices(&self, room: &str, members: &[String]) -> Result<(), String> {
        if self.with_crypto(|m| Ok(m.pending_shares(room, members)))?.is_empty() {
            return Ok(());
        }
        let missing = self.with_crypto(|m| Ok(m.devices_without_session(members)))?;
        self.claim_sessions(&missing)?;
        let now = now_ms();
        let (batch, _) = self.with_crypto(|m| Ok(m.prepare_room(room, members, now)))?;
        if batch.is_empty() {
            return Ok(());
        }
        self.send_room_keys(batch)
    }

    /// Answers the room key requests of this user's other devices (see [`Machine::key_requests`]): a device without an
    /// Olm session gets one claimed first, then the key is sent to it.
    fn answer_key_requests(&self, events: &[crate::protocol::ToDevice]) {
        let requests = match self.with_crypto(|m| Ok(m.key_requests(events))) {
            Ok(requests) => requests,
            Err(_) => return,
        };
        for (room, user, device) in requests {
            let needed = vec![(user.clone(), device.clone())];
            if self.with_crypto(|m| Ok(m.needs_session(&user, &device))).unwrap_or(false) {
                let _ = self.claim_sessions(&needed);
            }
            let answer = self.with_crypto(|m| Ok(m.answer_key_request(&room, &user, &device)));
            if let Ok(Some(message)) = answer {
                let _ = self.send_room_keys(vec![message]);
            }
        }
    }

    /// Uploads this device's keys and a batch of one-time keys (`POST /keys/upload`). Returns the server's count of
    /// signed one-time keys afterwards.
    pub fn upload_keys(&self) -> Result<u64, String> {
        let body = match self.crypto.lock() {
            Ok(mut m) => m.upload_body(crypto::OTK_BATCH),
            Err(_) => return Err("the encryption state is unavailable".into()),
        };
        let url = protocol::keys_upload_url(&self.base);
        let text = body.encode();
        let answer = call(&self.http, "POST", &url, Some(self.token.as_str()), Some((text.as_bytes(), "application/json")), CALL_LIMIT)?;
        if let Ok(mut m) = self.crypto.lock() {
            m.mark_published();
        }
        Ok(answer.get("one_time_key_counts").and_then(|c| c.get("signed_curve25519")).and_then(|v| v.as_u64()).unwrap_or(0))
    }

    /// Fetches and checks the device keys of `users` (`POST /keys/query`).
    pub fn query_devices(&self, users: &[String]) -> Result<usize, String> {
        let text = Machine::query_body(users).encode();
        let url = protocol::keys_query_url(&self.base);
        let answer = call(&self.http, "POST", &url, Some(self.token.as_str()), Some((text.as_bytes(), "application/json")), CALL_LIMIT)?;
        self.with_crypto(|m| m.ingest_query(&answer))
    }

    /// Claims one one-time key for each device in `needed` and opens an Olm session with it (`POST /keys/claim`).
    fn claim_sessions(&self, needed: &[(String, String)]) -> Result<usize, String> {
        if needed.is_empty() {
            return Ok(0);
        }
        let text = Machine::claim_body(needed).encode();
        let url = protocol::keys_claim_url(&self.base);
        let answer = call(&self.http, "POST", &url, Some(self.token.as_str()), Some((text.as_bytes(), "application/json")), CALL_LIMIT)?;
        self.with_crypto(|m| m.ingest_claim(&answer))
    }

    /// Runs `f` on the encryption state.
    fn with_crypto<T>(&self, f: impl FnOnce(&mut Machine) -> Result<T, String>) -> Result<T, String> {
        let mut m = self.crypto.lock().map_err(|_| "the encryption state is unavailable".to_string())?;
        f(&mut *m)
    }

    /// Sends `event_type` with `content` to the encrypted `room`: fetches the members' devices, claims one-time keys for
    /// the devices without a session, shares the room key when it is new or due for rotation, then sends the
    /// `m.room.encrypted` event.
    fn send_encrypted(&self, room: &str, event_type: &str, content: &Json) -> Result<(), String> {
        let members = self.joined_members(room)?;
        self.query_devices(&members)?;
        let missing = self.with_crypto(|m| Ok(m.devices_without_session(&members)))?;
        self.claim_sessions(&missing)?;
        let now = now_ms();
        let (batch, _skipped) = self.with_crypto(|m| Ok(m.prepare_room(room, &members, now)))?;
        if !batch.is_empty() {
            self.send_room_keys(batch)?;
        }
        let encrypted = self.with_crypto(|m| m.encrypt_room_event(room, event_type, content))?;
        let url = protocol::send_event_url(&self.base, room, "m.room.encrypted", &txn_id());
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((encrypted.encode().as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    /// The user ids joined to `room` (`GET /rooms/{id}/joined_members`).
    fn joined_members(&self, room: &str) -> Result<Vec<String>, String> {
        let url = protocol::joined_members_url(&self.base, room);
        let answer = call(&self.http, "GET", &url, Some(self.token.as_str()), None, CALL_LIMIT)?;
        protocol::parse_joined_members(&answer)
    }

    /// Sends the room key messages, one `m.room.encrypted` to each device, in one to-device request.
    fn send_room_keys(&self, batch: Vec<(String, String, Json)>) -> Result<(), String> {
        let mut users: Vec<(String, Vec<(String, Json)>)> = Vec::new();
        for (user, device, content) in batch {
            match users.iter_mut().find(|(u, _)| *u == user) {
                Some((_, devices)) => devices.push((device, content)),
                None => users.push((user, vec![(device, content)])),
            }
        }
        let messages = Json::Obj(users.into_iter().map(|(u, devices)| (u, Json::Obj(devices))).collect());
        let text = protocol::obj(vec![("messages", messages)]).encode();
        let url = protocol::send_to_device_url(&self.base, "m.room.encrypted", &txn_id());
        call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((text.as_bytes(), "application/json")), CALL_LIMIT).map(|_| ())
    }

    /// One sync: the changes after `since`, or the whole state when there is no token. Long-polls for up to
    /// [`protocol::SYNC_TIMEOUT_MS`].
    pub fn sync(&self, since: Option<&str>) -> Result<Sync, String> {
        let url = protocol::sync_url(&self.base, since);
        let answer = call(&self.http, "GET", &url, Some(self.token.as_str()), None, SYNC_LIMIT)?;
        protocol::parse_sync(&answer)
    }

    /// Sends a text message to `room`.
    pub fn send_text(&self, room: &str, text: &str) -> Result<(), String> {
        self.send_content(room, &protocol::text_content(text))
    }

    /// Uploads an image and sends it to `room` as an image message.
    pub fn send_image(&self, room: &str, name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        if self.is_encrypted(room) {
            return self.send_encrypted_media(room, true, name, mime, bytes);
        }
        let mxc = self.upload(name, mime, bytes)?;
        let info = protocol::ImageInfo { name, mxc: &mxc, mime, size: bytes.len() as u64 };
        self.send_content(room, &protocol::image_content(&info))
    }

    /// Uploads any file and sends it to `room` as a file message.
    pub fn send_file(&self, room: &str, name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        if self.is_encrypted(room) {
            return self.send_encrypted_media(room, false, name, mime, bytes);
        }
        let mxc = self.upload(name, mime, bytes)?;
        let info = protocol::ImageInfo { name, mxc: &mxc, mime, size: bytes.len() as u64 };
        self.send_content(room, &protocol::file_content(&info))
    }

    /// Sends a text message that answers the message `reply_to`.
    pub fn send_reply(&self, room: &str, text: &str, reply_to: &str) -> Result<(), String> {
        self.send_content(room, &protocol::reply_content(text, reply_to))
    }

    /// Replaces the words of the message `target` with `text`.
    pub fn send_edit(&self, room: &str, target: &str, text: &str) -> Result<(), String> {
        self.send_content(room, &protocol::edit_content(target, text))
    }

    /// Reacts to the message `target` with `key` (an emoji).
    pub fn send_reaction(&self, room: &str, target: &str, key: &str) -> Result<(), String> {
        if self.is_encrypted(room) {
            return Err("Reactions are not sent in encrypted rooms yet".into());
        }
        let url = protocol::send_event_url(&self.base, room, "m.reaction", &txn_id());
        let content = protocol::reaction_content(target, key);
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((content.as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    /// Removes (redacts) `event`: Undo Send for a message, or taking back a reaction.
    pub fn redact(&self, room: &str, event: &str) -> Result<(), String> {
        let url = protocol::redact_url(&self.base, room, event, &txn_id());
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some(("{}".as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    /// Makes a private conversation with `users` and returns its room id: a direct conversation for one person, a
    /// group for several. The signed-in user is left out of the invitations.
    pub fn create_conversation(&self, users: &[String]) -> Result<String, String> {
        let others: Vec<String> = users.iter().filter(|u| **u != self.user_id).cloned().collect();
        let body = match others.as_slice() {
            [] => return Err("Enter someone other than yourself".into()),
            [one] => protocol::create_direct_body(one),
            many => protocol::create_group_body(many),
        };
        let url = protocol::create_room_url(&self.base);
        let answer = call(&self.http, "POST", &url, Some(self.token.as_str()), Some((body.as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_room_id(&answer)
    }

    /// Leaves `room`, then forgets it so it does not come back in the list. A failed forget is not an error: the room
    /// is left either way.
    pub fn leave(&self, room: &str) -> Result<(), String> {
        let empty = Some(("{}".as_bytes(), "application/json"));
        call(&self.http, "POST", &protocol::leave_url(&self.base, room), Some(self.token.as_str()), empty, CALL_LIMIT)?;
        let _ = call(&self.http, "POST", &protocol::forget_url(&self.base, room), Some(self.token.as_str()), empty, CALL_LIMIT);
        Ok(())
    }

    /// Renames `room`.
    pub fn rename_room(&self, room: &str, name: &str) -> Result<(), String> {
        let url = protocol::room_name_url(&self.base, room);
        let body = protocol::room_name_body(name);
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((body.as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    /// Ends this device's session on the homeserver. The access token stops working.
    pub fn logout(&self) -> Result<(), String> {
        let empty = Some(("{}".as_bytes(), "application/json"));
        call(&self.http, "POST", &protocol::logout_url(&self.base), Some(self.token.as_str()), empty, CALL_LIMIT).map(|_| ())
    }

    /// Sets or clears the account's "marked as unread" flag for `room`, so the other devices show it too.
    pub fn set_marked_unread(&self, room: &str, unread: bool) -> Result<(), String> {
        let url = protocol::room_account_data_url(&self.base, &self.user_id, room, protocol::MARKED_UNREAD);
        let body = protocol::marked_unread_body(unread);
        call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((body.as_bytes(), "application/json")), CALL_LIMIT).map(|_| ())
    }

    /// Marks `event` as read in `room` (a public read receipt).
    pub fn read_receipt(&self, room: &str, event: &str) -> Result<(), String> {
        let url = protocol::receipt_url(&self.base, room, event);
        call(&self.http, "POST", &url, Some(self.token.as_str()), Some(("{}".as_bytes(), "application/json")), CALL_LIMIT)
            .map(|_| ())
    }

    /// Sends a picture or file to an encrypted room. The bytes are encrypted under a new key (see [`attachment`]); the
    /// server gets only the ciphertext, under a neutral name, and the message with its `file` object goes out as a Megolm
    /// event.
    fn send_encrypted_media(&self, room: &str, image: bool, name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        let sealed = attachment::encrypt(bytes)?;
        let url = self.upload("attachment", "application/octet-stream", &sealed.cipher)?;
        let file = attachment::file_object(&url, &sealed);
        let content = protocol::encrypted_media_content(image, name, mime, bytes.len() as u64, &file);
        self.send_content(room, &content)
    }

    fn send_content(&self, room: &str, content: &str) -> Result<(), String> {
        if self.is_encrypted(room) {
            let plain = json::parse(content)?;
            return self.send_encrypted(room, "m.room.message", &plain);
        }
        let url = protocol::send_url(&self.base, room, &txn_id());
        let answer = call(&self.http, "PUT", &url, Some(self.token.as_str()), Some((content.as_bytes(), "application/json")), CALL_LIMIT)?;
        protocol::parse_event_id(&answer).map(|_| ())
    }

    fn upload(&self, name: &str, mime: &str, bytes: &[u8]) -> Result<String, String> {
        let url = protocol::upload_url(&self.base, name);
        let answer = call(&self.http, "POST", &url, Some(self.token.as_str()), Some((bytes, mime)), CALL_LIMIT)?;
        protocol::parse_content_uri(&answer)
    }
}

/// One API call. `token` is sent as a bearer token; `body` is the bytes and their content type. A 2xx answer is parsed
/// as JSON; any other answer becomes its error text, or the status when it has none.
fn call(
    http: &Client,
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<(&[u8], &str)>,
    limit: usize,
) -> Result<Json, String> {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers: Vec<(&str, &str)> = Vec::new();
    if let Some(a) = &auth {
        headers.push(("Authorization", a.as_str()));
    }
    if let Some((_, content_type)) = body {
        headers.push(("Content-Type", content_type));
    }
    let bytes = body.map(|(b, _)| b);
    let response = http.request_with_headers(method, url, &headers, bytes, limit).map_err(|e| e.to_string())?;
    answer(&response)
}

fn answer(response: &Response) -> Result<Json, String> {
    let text = String::from_utf8_lossy(&response.body);
    let parsed = json::parse(&text);
    if (200..300).contains(&response.status) {
        return parsed.map_err(|_| format!("the server answered with text that is not JSON (HTTP {})", response.status));
    }
    match parsed {
        Ok(body) => Err(protocol::error_text(&body).unwrap_or_else(|| format!("the server answered HTTP {}", response.status))),
        Err(_) => Err(format!("the server answered HTTP {}", response.status)),
    }
}

/// The wall clock in milliseconds since the epoch.
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A transaction id unique to this run: the time and a counter.
fn txn_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("bifrost{ms}-{n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, body: &str) -> Response {
        Response { status, body: body.as_bytes().to_vec(), ..Default::default() }
    }

    #[test]
    fn a_success_answer_is_its_json() {
        let json = answer(&response(200, r#"{"event_id":"$x"}"#)).unwrap();
        assert_eq!(json.str_field("event_id").as_deref(), Some("$x"));
    }

    #[test]
    fn an_error_answer_says_what_the_server_said() {
        let err = answer(&response(403, r#"{"errcode":"M_FORBIDDEN","error":"no"}"#)).unwrap_err();
        assert_eq!(err, "M_FORBIDDEN: no");
        assert_eq!(answer(&response(502, "<html>bad gateway</html>")).unwrap_err(), "the server answered HTTP 502");
    }

    #[test]
    fn a_success_that_is_not_json_is_an_error() {
        assert!(answer(&response(200, "<html>")).unwrap_err().contains("not JSON"));
    }

    #[test]
    fn transaction_ids_do_not_repeat() {
        assert_ne!(txn_id(), txn_id());
    }
}
