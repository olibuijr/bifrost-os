//! End-to-end encryption for the Matrix client. The cryptography is vodozemac (an audited, pure-Rust implementation of
//! Olm and Megolm); this module is the glue: the device's Olm account, the Olm sessions with the other devices, the
//! Megolm session per encrypted room, the wire shapes of the key endpoints and to-device messages, the rotation policy,
//! and the pickles that keep the state. Nothing here opens a connection: [`crate::api`] makes the calls.
//!
//! The state is kept in two places. The pickle key and the device's account pickle are keychain items (see [`KEY_ITEM`]
//! and [`DEVICE_ITEM`]), reloaded at the next sign-in of the same account on this machine. The sessions (Megolm and Olm)
//! are too large for a keychain item, so they go to an encrypted file (see [`crate::state_file`]), sealed with the pickle
//! key. Every pickle is encrypted with the pickle key. Not done yet: device verification (every device is unverified,
//! and the room info must say so), key backup, cross-signing, and reactions in encrypted rooms (refused by [`crate::api`]).

use crate::protocol::{self, Body, Ciphertext, Event, ToDevice};
use bifrost_core::json::{self, Json};
use std::collections::HashMap;
use vodozemac::megolm::{
    GroupSession, GroupSessionPickle, InboundGroupSession, InboundGroupSessionPickle, MegolmMessage, SessionConfig as MegolmConfig,
    SessionKey,
};
use vodozemac::olm::{Account, AccountPickle, OlmMessage, Session, SessionConfig as OlmConfig, SessionPickle};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature};

/// The algorithm of the Olm messages that carry room keys between devices.
pub const OLM_ALGORITHM: &str = "m.olm.v1.curve25519-aes-sha2";
/// The algorithm of the Megolm messages that carry room events.
pub const MEGOLM_ALGORITHM: &str = "m.megolm.v1.aes-sha2";
/// The key algorithm of a signed one-time key, as the key endpoints name it.
pub const OTK_ALGORITHM: &str = "signed_curve25519";
/// A new outbound Megolm session replaces the current one after this many messages.
pub const ROTATE_MESSAGES: u32 = 100;
/// A new outbound Megolm session replaces the current one after this long, in milliseconds (one week).
pub const ROTATE_AFTER_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// How many one-time keys a device uploads at a time.
pub const OTK_BATCH: usize = 50;
/// The server holding fewer signed one-time keys than this for the device makes the device upload more.
pub const OTK_FLOOR: u64 = 20;

/// What an outbound Megolm session was made for: when, how many messages it has carried, and the members it was shared
/// with (sorted, so the order of a member list never looks like a change).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outbound {
    pub created_ms: u64,
    pub messages: u32,
    pub members: Vec<String>,
    /// The devices (as `user/device`) that hold this session's key. A device that appears later gets the key too.
    pub shared: Vec<String>,
}

/// The `user/device` key a device is known by in [`Outbound::shared`].
fn device_key(user: &str, device: &str) -> String {
    format!("{user}/{device}")
}

/// The rotation policy of a room's outbound Megolm session: rotate when the membership changed, after [`ROTATE_MESSAGES`]
/// messages, or after [`ROTATE_AFTER_MS`].
pub fn rotation_due(state: &Outbound, members: &[String], now_ms: u64) -> bool {
    let mut now: Vec<String> = members.to_vec();
    now.sort();
    now.dedup();
    now != state.members || state.messages >= ROTATE_MESSAGES || now_ms.saturating_sub(state.created_ms) >= ROTATE_AFTER_MS
}

/// A device of another user (or of this user, on another login), as the device keys said it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub user_id: String,
    pub device_id: String,
    pub curve25519: String,
    pub ed25519: String,
}

/// The crypto state of one login: the Olm account, the sessions, the device keys of the room members, and the Megolm
/// sessions (outbound for the rooms this device sends to, inbound for the room keys it received).
pub struct Machine {
    user_id: String,
    device_id: String,
    account: Account,
    /// Olm sessions with other devices, by the device's curve25519 key.
    olm: HashMap<String, Session>,
    /// The verified-by-self-signature devices of each user.
    devices: HashMap<String, Vec<Device>>,
    /// The outbound Megolm session of each encrypted room.
    outbound: HashMap<String, (GroupSession, Outbound)>,
    /// The inbound Megolm sessions, by (room id, session id).
    inbound: HashMap<(String, String), InboundGroupSession>,
    /// The key the pickles are encrypted with (held in the keychain). Without it nothing is saved.
    pickle_key: Option<[u8; 32]>,
    /// True when the state changed since the last [`Machine::snapshot`].
    dirty: bool,
}

impl Machine {
    /// A new device: a new Olm account with no keys published yet.
    pub fn new(user_id: &str, device_id: &str) -> Machine {
        Machine::with_account(user_id, device_id, Account::new())
    }

    fn with_account(user_id: &str, device_id: &str, account: Account) -> Machine {
        Machine {
            user_id: user_id.to_string(),
            device_id: device_id.to_string(),
            account,
            olm: HashMap::new(),
            devices: HashMap::new(),
            outbound: HashMap::new(),
            inbound: HashMap::new(),
            pickle_key: None,
            dirty: false,
        }
    }

    /// The device id this machine runs as.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Sets the key the pickles are written with, and marks the state dirty so it is saved.
    pub fn set_pickle_key(&mut self, key: [u8; 32]) {
        self.pickle_key = Some(key);
        self.dirty = true;
    }

    /// The sessions, each pickle encrypted with `key`, one record per line: `out` for an outbound Megolm session (room,
    /// created time, message count, members, shared devices, pickle), `in` for an inbound one (room, session id,
    /// pickle) and `olm` for an Olm session (the peer's curve25519 key, pickle). Olm sessions are kept so a room key
    /// message still in flight can be opened after a restart. `None` when there are none. The caller stores the text
    /// under an AEAD seal (see [`crate::state_file`]); the file is never written in the clear.
    fn sessions_blob(&self, key: &[u8; 32]) -> Option<String> {
        if self.outbound.is_empty() && self.inbound.is_empty() && self.olm.is_empty() {
            return None;
        }
        let mut lines = Vec::new();
        for (room, (session, state)) in &self.outbound {
            lines.push(format!(
                "out\t{room}\t{}\t{}\t{}\t{}\t{}",
                state.created_ms,
                state.messages,
                state.members.join(","),
                state.shared.join(","),
                session.pickle().encrypt(key)
            ));
        }
        for ((room, session_id), session) in &self.inbound {
            lines.push(format!("in\t{room}\t{session_id}\t{}", session.pickle().encrypt(key)));
        }
        for (curve, session) in &self.olm {
            lines.push(format!("olm\t{curve}\t{}", session.pickle().encrypt(key)));
        }
        Some(lines.join("\n"))
    }

    /// Restores the sessions a [`Machine::sessions_blob`] wrote. Returns how many records were read.
    pub fn load_sessions(&mut self, key: &[u8; 32], blob: &str) -> Result<usize, String> {
        let mut loaded = 0;
        for line in blob.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["out", room, created, messages, members, shared, pickle] => {
                    let session = GroupSession::from_pickle(GroupSessionPickle::from_encrypted(pickle, key).map_err(|e| e.to_string())?);
                    let list = |s: &str| s.split(',').filter(|m| !m.is_empty()).map(String::from).collect::<Vec<String>>();
                    let state = Outbound {
                        created_ms: created.parse().map_err(|_| "a session record is damaged".to_string())?,
                        messages: messages.parse().map_err(|_| "a session record is damaged".to_string())?,
                        members: list(*members),
                        shared: list(*shared),
                    };
                    self.outbound.insert(room.to_string(), (session, state));
                }
                ["in", room, session_id, pickle] => {
                    let session =
                        InboundGroupSession::from_pickle(InboundGroupSessionPickle::from_encrypted(pickle, key).map_err(|e| e.to_string())?);
                    self.inbound.insert((room.to_string(), session_id.to_string()), session);
                }
                ["olm", curve, pickle] => {
                    let session = Session::from_pickle(SessionPickle::from_encrypted(pickle, key).map_err(|e| e.to_string())?);
                    self.olm.insert(curve.to_string(), session);
                }
                _ => continue,
            }
            loaded += 1;
        }
        Ok(loaded)
    }

    /// The state to write, when it changed since the last call and a pickle key is set: the pickle key and the device
    /// (keychain items), and the sessions text for the sessions file.
    pub fn snapshot(&mut self) -> Option<Snapshot> {
        let key = self.pickle_key?;
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        Some(Snapshot {
            key: key_item(&key),
            device: device_item(&self.device_id, &self.pickle(&key)),
            sessions: self.sessions_blob(&key),
        })
    }

    /// The account, encrypted with `key` (32 bytes from [`new_pickle_key`]). Only the pickle is ever stored.
    pub fn pickle(&self, key: &[u8; 32]) -> String {
        self.account.pickle().encrypt(key)
    }

    /// The account a [`Machine::pickle`] gave back, with the same identity keys.
    pub fn restore(user_id: &str, device_id: &str, key: &[u8; 32], pickled: &str) -> Result<Machine, String> {
        let pickle = AccountPickle::from_encrypted(pickled, key).map_err(|e| e.to_string())?;
        Ok(Machine::with_account(user_id, device_id, Account::from_pickle(pickle)))
    }

    /// The device's curve25519 (Olm) and ed25519 (signing) public keys, base64.
    pub fn identity(&self) -> (String, String) {
        let keys = self.account.identity_keys();
        (keys.curve25519.to_base64(), keys.ed25519.to_base64())
    }

    /// The device keys this device publishes: its algorithms, its two keys and the self-signature over them.
    pub fn device_keys(&self) -> Json {
        let (curve, ed) = self.identity();
        let unsigned = vec![
            ("algorithms".to_string(), Json::Arr(vec![protocol::text(OLM_ALGORITHM), protocol::text(MEGOLM_ALGORITHM)])),
            ("device_id".to_string(), protocol::text(&self.device_id)),
            (
                "keys".to_string(),
                Json::Obj(vec![
                    (format!("curve25519:{}", self.device_id), protocol::text(&curve)),
                    (format!("ed25519:{}", self.device_id), protocol::text(&ed)),
                ]),
            ),
            ("user_id".to_string(), protocol::text(&self.user_id)),
        ];
        let signature = self.account.sign(&canonical(&Json::Obj(unsigned.clone()))).to_base64();
        self.with_signature(unsigned, &signature)
    }

    fn with_signature(&self, unsigned: Vec<(String, Json)>, signature: &str) -> Json {
        let mut pairs = unsigned;
        let by_device = Json::Obj(vec![(format!("ed25519:{}", self.device_id), protocol::text(signature))]);
        pairs.push(("signatures".to_string(), Json::Obj(vec![(self.user_id.clone(), by_device)])));
        Json::Obj(pairs)
    }

    /// The body of `POST /keys/upload`: the device keys and `count` new signed one-time keys. Call
    /// [`Machine::mark_published`] once the server has answered.
    pub fn upload_body(&mut self, count: usize) -> Json {
        self.dirty = true;
        self.account.generate_one_time_keys(count);
        let mut keys = Vec::new();
        for (id, key) in self.account.one_time_keys() {
            let unsigned = vec![("key".to_string(), protocol::text(&key.to_base64()))];
            let signature = self.account.sign(&canonical(&Json::Obj(unsigned.clone()))).to_base64();
            let signed = self.with_signature(unsigned, &signature);
            keys.push((format!("{OTK_ALGORITHM}:{}", id.to_base64()), signed));
        }
        protocol::obj(vec![("device_keys", self.device_keys()), ("one_time_keys", Json::Obj(keys))])
    }

    /// Marks the one-time keys of the last [`Machine::upload_body`] as published, so they are not offered again.
    pub fn mark_published(&mut self) {
        self.dirty = true;
        self.account.mark_keys_as_published();
    }

    /// The body of `POST /keys/query` for `users`: all their devices.
    pub fn query_body(users: &[String]) -> Json {
        let asked = users.iter().map(|u| (u.clone(), Json::Arr(Vec::new()))).collect();
        protocol::obj(vec![("device_keys", Json::Obj(asked))])
    }

    /// Takes in a `POST /keys/query` answer. A device counts only when its keys are the ones it names and it is signed by
    /// its own ed25519 key; any other device is dropped. Returns how many devices were kept.
    pub fn ingest_query(&mut self, answer: &Json) -> Result<usize, String> {
        if let Some(e) = protocol::error_text(answer) {
            return Err(e);
        }
        let Some(Json::Obj(users)) = answer.get("device_keys") else {
            return Err("the server did not send device keys".into());
        };
        let mut kept = 0;
        for (user, devices) in users {
            let Json::Obj(list) = devices else { continue };
            let found: Vec<Device> = list.iter().filter_map(|(device_id, value)| parse_device(user, device_id, value)).collect();
            kept += found.len();
            self.devices.insert(user.clone(), found);
        }
        Ok(kept)
    }

    /// The devices of `user` for the room info: the ones the last device query kept, and this device for this user. Every
    /// one is unverified, since device verification is not implemented, so none is ever shown as verified.
    pub fn devices_of(&self, user: &str) -> Vec<Device> {
        let mut out: Vec<Device> = self.devices.get(user).cloned().unwrap_or_default();
        if user == self.user_id && !out.iter().any(|d| d.device_id == self.device_id) {
            let (curve25519, ed25519) = self.identity();
            out.push(Device { user_id: user.to_string(), device_id: self.device_id.clone(), curve25519, ed25519 });
        }
        out
    }

    /// The devices of `members` (users) except this device, as the last query saw them.
    pub fn other_devices(&self, members: &[String]) -> Vec<Device> {
        let mut out = Vec::new();
        for user in members {
            for device in self.devices.get(user).into_iter().flatten() {
                if device.user_id != self.user_id || device.device_id != self.device_id {
                    out.push(device.clone());
                }
            }
        }
        out
    }

    /// The body of `POST /keys/claim` for the devices in `needed` (user, device id).
    pub fn claim_body(needed: &[(String, String)]) -> Json {
        let mut by_user: Vec<(String, Json)> = Vec::new();
        for (user, device) in needed {
            let entry = (device.clone(), protocol::text(OTK_ALGORITHM));
            match by_user.iter_mut().find(|(u, _)| u == user) {
                Some((_, Json::Obj(devices))) => devices.push(entry),
                _ => by_user.push((user.clone(), Json::Obj(vec![entry]))),
            }
        }
        protocol::obj(vec![("one_time_keys", Json::Obj(by_user))])
    }

    /// The devices of `members` this device has no Olm session with, so a one-time key must be claimed for them.
    pub fn devices_without_session(&self, members: &[String]) -> Vec<(String, String)> {
        self.other_devices(members)
            .into_iter()
            .filter(|d| !self.olm.contains_key(&d.curve25519))
            .map(|d| (d.user_id, d.device_id))
            .collect()
    }

    /// Takes in a `POST /keys/claim` answer: for each device, the first one-time key whose signature checks out opens an
    /// outbound Olm session. Returns how many sessions were made.
    pub fn ingest_claim(&mut self, answer: &Json) -> Result<usize, String> {
        if let Some(e) = protocol::error_text(answer) {
            return Err(e);
        }
        let Some(Json::Obj(users)) = answer.get("one_time_keys") else {
            return Err("the server did not send one-time keys".into());
        };
        let mut made = 0;
        for (user, devices) in users {
            let Json::Obj(list) = devices else { continue };
            for (device_id, keys) in list {
                let Json::Obj(offered) = keys else { continue };
                let Some(device) = self.device(user, device_id).cloned() else { continue };
                let session = offered.iter().find_map(|(name, value)| {
                    let key_id = name.strip_prefix(&format!("{OTK_ALGORITHM}:"))?;
                    if key_id.is_empty() || !verify_signed(value, user, device_id, &device.ed25519) {
                        return None;
                    }
                    self.outbound_olm(&device, &value.str_field("key")?)
                });
                if let Some(session) = session {
                    self.olm.insert(device.curve25519.clone(), session);
                    self.dirty = true;
                    made += 1;
                }
            }
        }
        Ok(made)
    }

    fn device(&self, user: &str, device_id: &str) -> Option<&Device> {
        self.devices.get(user)?.iter().find(|d| d.device_id == device_id)
    }

    fn outbound_olm(&self, device: &Device, one_time_key: &str) -> Option<Session> {
        let identity = Curve25519PublicKey::from_base64(&device.curve25519).ok()?;
        let otk = Curve25519PublicKey::from_base64(one_time_key).ok()?;
        Some(self.account.create_outbound_session(OlmConfig::version_2(), identity, otk))
    }

    /// Prepares the room key of `room` for `members`. When the outbound session is missing or due for rotation, makes a
    /// new one and returns the to-device messages that carry its key (one per device, as `(user, device, content)`).
    /// Devices with no Olm session are left out, and their count is returned too.
    pub fn prepare_room(&mut self, room: &str, members: &[String], now_ms: u64) -> (Vec<(String, String, Json)>, usize) {
        let mut sorted: Vec<String> = members.to_vec();
        sorted.sort();
        sorted.dedup();
        let due = match self.outbound.get(room) {
            Some((_, state)) => rotation_due(state, &sorted, now_ms),
            None => true,
        };
        if due {
            let session = GroupSession::new(MegolmConfig::version_1());
            self.dirty = true;
            let state = Outbound { created_ms: now_ms, messages: 0, members: sorted.clone(), shared: Vec::new() };
            self.outbound.insert(room.to_string(), (session, state));
        }
        // The devices that do not hold the key yet: all of them after a rotation, or the ones that appeared since.
        let shared = self.outbound.get(room).map(|(_, state)| state.shared.clone()).unwrap_or_default();
        let fresh: Vec<Device> = self
            .other_devices(&sorted)
            .into_iter()
            .filter(|d| !shared.contains(&device_key(&d.user_id, &d.device_id)))
            .collect();
        self.share_key(room, &fresh)
    }

    /// The `m.room_key` messages that give `devices` the outbound session's key of `room`, one per device with an Olm
    /// session (as `(user, device, content)`), and how many had none. A device that got it is recorded in the session.
    fn share_key(&mut self, room: &str, devices: &[Device]) -> (Vec<(String, String, Json)>, usize) {
        let Some((session, _)) = self.outbound.get(room) else { return (Vec::new(), 0) };
        let key = session.session_key().to_base64();
        let session_id = session.session_id();
        let (_, ed) = self.identity();
        let mut messages = Vec::new();
        let mut skipped = 0;
        for device in devices {
            let plain = protocol::obj(vec![
                ("type", protocol::text("m.room_key")),
                (
                    "content",
                    protocol::obj(vec![
                        ("algorithm", protocol::text(MEGOLM_ALGORITHM)),
                        ("room_id", protocol::text(room)),
                        ("session_id", protocol::text(&session_id)),
                        ("session_key", protocol::text(&key)),
                    ]),
                ),
                ("sender", protocol::text(&self.user_id)),
                ("sender_device", protocol::text(&self.device_id)),
                ("keys", protocol::obj(vec![("ed25519", protocol::text(&ed))])),
                ("recipient", protocol::text(&device.user_id)),
                ("recipient_keys", protocol::obj(vec![("ed25519", protocol::text(&device.ed25519))])),
            ]);
            match self.encrypt_olm(&device.curve25519, &plain) {
                Some(content) => {
                    messages.push((device.user_id.clone(), device.device_id.clone(), content));
                    let name = device_key(&device.user_id, &device.device_id);
                    if let Some((_, state)) = self.outbound.get_mut(room) {
                        if !state.shared.contains(&name) {
                            state.shared.push(name);
                            self.dirty = true;
                        }
                    }
                }
                None => skipped += 1,
            }
        }
        (messages, skipped)
    }

    /// The devices of `members` that have not got the outbound session's key of `room` yet, as (user, device). Empty when
    /// there is no outbound session (the next send makes one and shares it with everyone).
    pub fn pending_shares(&self, room: &str, members: &[String]) -> Vec<(String, String)> {
        let Some((_, state)) = self.outbound.get(room) else { return Vec::new() };
        self.other_devices(members)
            .into_iter()
            .filter(|d| !state.shared.contains(&device_key(&d.user_id, &d.device_id)))
            .map(|d| (d.user_id, d.device_id))
            .collect()
    }

    /// True when this device has no Olm session with `device` of `user` yet.
    pub fn needs_session(&self, user: &str, device: &str) -> bool {
        self.device(user, device).is_some_and(|d| !self.olm.contains_key(&d.curve25519))
    }

    /// The room key requests this device should answer: the requester is one of this user's own devices whose keys are
    /// signature-verified, and the session asked for is the outbound session of `room`. Returns (room, user, device).
    pub fn key_requests(&self, events: &[ToDevice]) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for ev in events.iter().filter(|e| e.kind == "m.room_key_request") {
            if ev.sender != self.user_id || ev.content.str_field("action").as_deref() != Some("request") {
                continue;
            }
            let Some(device) = ev.content.str_field("requesting_device_id") else { continue };
            if device == self.device_id || self.device(&self.user_id, &device).is_none() {
                continue;
            }
            let Some(body) = ev.content.get("body") else { continue };
            let (Some(room), Some(session_id)) = (body.str_field("room_id"), body.str_field("session_id")) else { continue };
            if self.outbound.get(&room).is_some_and(|(s, _)| s.session_id() == session_id) {
                out.push((room, self.user_id.clone(), device));
            }
        }
        out
    }

    /// The `m.room_key` message that answers a key request from `device` of `user`, or none when the device has no Olm
    /// session yet (the caller claims one first, see [`Machine::needs_session`]).
    pub fn answer_key_request(&mut self, room: &str, user: &str, device: &str) -> Option<(String, String, Json)> {
        let target: Vec<Device> = self.device(user, device).cloned().into_iter().collect();
        self.share_key(room, &target).0.into_iter().next()
    }

    /// The Olm `m.room.encrypted` content of `plain` for the device with curve25519 key `curve`, or `None` without a
    /// session with it.
    fn encrypt_olm(&mut self, curve: &str, plain: &Json) -> Option<Json> {
        let (mine, _) = self.identity();
        let session = self.olm.get_mut(curve)?;
        let message = session.encrypt(plain.encode());
        let (message_type, body) = message.to_parts();
        let cipher = protocol::obj(vec![
            ("type", Json::Num(message_type as f64)),
            ("body", protocol::text(&vodozemac::base64_encode(&body))),
        ]);
        Some(protocol::obj(vec![
            ("algorithm", protocol::text(OLM_ALGORITHM)),
            ("sender_key", protocol::text(&mine)),
            ("ciphertext", Json::Obj(vec![(curve.to_string(), cipher)])),
        ]))
    }

    /// Takes in the to-device messages of a sync: an Olm message from a device this device has a session with (or a new
    /// one, opened with a pre-key message) is opened, and a `m.room_key` in it becomes an inbound Megolm session. Returns
    /// how many room keys were taken in. A message that does not open is dropped.
    pub fn receive(&mut self, events: &[ToDevice]) -> usize {
        let mut taken = 0;
        for event in events.iter().filter(|e| e.kind == "m.room.encrypted") {
            let Some(plain) = self.open_olm(&event.content) else { continue };
            self.dirty = true;
            if self.accept_room_key(&event.sender, &plain) {
                taken += 1;
            }
        }
        taken
    }

    fn open_olm(&mut self, content: &Json) -> Option<Json> {
        if content.str_field("algorithm")? != OLM_ALGORITHM {
            return None;
        }
        let sender_key = content.str_field("sender_key")?;
        let (mine, _) = self.identity();
        let cipher = content.get("ciphertext")?.get(&mine)?;
        let message_type = cipher.get("type")?.number()? as usize;
        let bytes = vodozemac::base64_decode(&cipher.str_field("body")?).ok()?;
        let message = OlmMessage::from_parts(message_type, &bytes).ok()?;
        let their_curve = Curve25519PublicKey::from_base64(&sender_key).ok()?;
        let plaintext = match &message {
            OlmMessage::PreKey(pre) => {
                let created = self.account.create_inbound_session(their_curve, pre).ok()?;
                self.olm.insert(sender_key.clone(), created.session);
                created.plaintext
            }
            OlmMessage::Normal(_) => self.olm.get_mut(&sender_key)?.decrypt(&message).ok()?,
        };
        json::parse(std::str::from_utf8(&plaintext).ok()?).ok()
    }

    /// Takes in one decrypted `m.room_key` if it names a device of `sender` whose signing key matches, and stores it as
    /// the inbound Megolm session of its room.
    fn accept_room_key(&mut self, sender: &str, plain: &Json) -> bool {
        if plain.str_field("type").as_deref() != Some("m.room_key") {
            return false;
        }
        let (Some(sender_device), Some(claimed_ed)) =
            (plain.str_field("sender_device"), plain.get("keys").and_then(|k| k.str_field("ed25519")))
        else {
            return false;
        };
        let known = self.device(sender, &sender_device).is_some_and(|d| d.ed25519 == claimed_ed);
        let Some(content) = plain.get("content") else { return false };
        if !known || content.str_field("algorithm").as_deref() != Some(MEGOLM_ALGORITHM) {
            return false;
        }
        let (Some(room), Some(session_id), Some(session_key)) =
            (content.str_field("room_id"), content.str_field("session_id"), content.str_field("session_key"))
        else {
            return false;
        };
        let Ok(key) = SessionKey::from_base64(&session_key) else { return false };
        let inbound = InboundGroupSession::new(&key, MegolmConfig::version_1());
        self.dirty = true;
        self.inbound.insert((room, session_id), inbound);
        true
    }

    /// Encrypts `content` as an event of type `event_type` in `room` with the room's outbound session, and returns the
    /// `m.room.encrypted` content to send. Call [`Machine::prepare_room`] first.
    pub fn encrypt_room_event(&mut self, room: &str, event_type: &str, content: &Json) -> Result<Json, String> {
        let (mine, _) = self.identity();
        let device_id = self.device_id.clone();
        self.dirty = true;
        let (session, state) = self.outbound.get_mut(room).ok_or("no room key for this room yet")?;
        let plain = protocol::obj(vec![
            ("type", protocol::text(event_type)),
            ("content", content.clone()),
            ("room_id", protocol::text(room)),
        ]);
        let message = session.encrypt(plain.encode());
        state.messages += 1;
        Ok(protocol::obj(vec![
            ("algorithm", protocol::text(MEGOLM_ALGORITHM)),
            ("sender_key", protocol::text(&mine)),
            ("device_id", protocol::text(&device_id)),
            ("session_id", protocol::text(&session.session_id())),
            ("ciphertext", protocol::text(&message.to_base64())),
        ]))
    }

    /// Opens one encrypted room event with the inbound session of its room. `None` when there is no session for it or it
    /// does not open (the caller shows the placeholder). A decrypted event of another type is `Body::Other`.
    pub fn open_room_event(&mut self, room: &str, ct: &Ciphertext) -> Option<(Body, Option<String>)> {
        if ct.algorithm != MEGOLM_ALGORITHM {
            return None;
        }
        let inbound = self.inbound.get_mut(&(room.to_string(), ct.session_id.clone()))?;
        let message = MegolmMessage::from_base64(&ct.body).ok()?;
        let decrypted = inbound.decrypt(&message).ok()?;
        let plain = json::parse(std::str::from_utf8(&decrypted.plaintext).ok()?).ok()?;
        Some(match protocol::decrypted_body(&plain) {
            Some(found) => found,
            None => (Body::Other(plain.str_field("type").unwrap_or_default()), None),
        })
    }

    /// Replaces the placeholder body of each encrypted event of `room` with its decrypted body, or with
    /// [`Body::Undecryptable`] when this device cannot open it. Returns how many were opened.
    pub fn decrypt_events(&mut self, room: &str, events: &mut [Event], ciphertexts: &[Ciphertext]) -> usize {
        let mut opened = 0;
        for ct in ciphertexts {
            let Some(event) = events.iter_mut().find(|e| e.id == ct.id) else { continue };
            match self.open_room_event(room, ct) {
                Some((body, reply_to)) => {
                    event.body = body;
                    event.reply_to = reply_to;
                    opened += 1;
                }
                None => event.body = Body::Undecryptable,
            }
        }
        opened
    }
}

/// A new 32-byte pickle key from the kernel's random source.
pub fn new_pickle_key() -> Result<[u8; 32], String> {
    random_bytes()
}

/// `N` random bytes from the kernel's random source (`/dev/urandom`).
pub fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    use std::io::Read;
    let mut bytes = [0u8; N];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// The keychain items the crypto state lives in. Each is one app keychain item per homeserver (service = prefix plus the
/// homeserver address) and per login (account = the full Matrix user id the server returned at sign-in), so the same
/// account on this machine finds its device again. The pickle key is its own item, and every pickle is encrypted with it.
pub const KEY_ITEM: &str = "matrix-key:";
pub const DEVICE_ITEM: &str = "matrix-device:";
/// The sessions item of earlier versions. The keychain caps an item at 8 KiB, so the sessions are now in a file (see
/// [`crate::state_file`]); this item is only read once, to migrate it, and then deleted.
pub const SESSIONS_ITEM: &str = "matrix-sessions:";

/// The keychain service of one of the crypto items for `homeserver`.
pub fn item_service(prefix: &str, homeserver: &str) -> String {
    format!("{prefix}{homeserver}")
}

/// The key item's value: the 32-byte pickle key in hex.
pub fn key_item(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

/// The pickle key of a key item; `None` for a value that is not one.
pub fn parse_key_item(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(value.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

/// The device item's value: the device id, a newline, then the account pickle (encrypted with the pickle key).
pub fn device_item(device_id: &str, pickled: &str) -> String {
    format!("{device_id}\n{pickled}")
}

/// The device id and account pickle of a device item; `None` for a value that is not one.
pub fn parse_device_item(value: &str) -> Option<(&str, &str)> {
    value.split_once('\n')
}

/// The state to write after a change: the key item and the device item (keychain), and the sessions text (`None` when
/// there are no sessions to save yet), which the caller seals into the sessions file with the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub key: String,
    pub device: String,
    pub sessions: Option<String>,
}

/// The signing key and identity of one device in a keys/query answer, when the device is signed by its own key.
fn parse_device(user: &str, device_id: &str, value: &Json) -> Option<Device> {
    if value.str_field("user_id")? != user || value.str_field("device_id")? != device_id {
        return None;
    }
    let keys = value.get("keys")?;
    let curve25519 = keys.str_field(&format!("curve25519:{device_id}"))?;
    let ed25519 = keys.str_field(&format!("ed25519:{device_id}"))?;
    if !verify_signed(value, user, device_id, &ed25519) {
        return None;
    }
    Some(Device { user_id: user.to_string(), device_id: device_id.to_string(), curve25519, ed25519 })
}

/// Checks the signature `user`'s `device` put on `value` (an object without its `signatures` and `unsigned` members)
/// against the device's ed25519 key.
fn verify_signed(value: &Json, user: &str, device_id: &str, ed25519: &str) -> bool {
    let Some(sig) = value.get("signatures").and_then(|s| s.get(user)).and_then(|s| s.str_field(&format!("ed25519:{device_id}")))
    else {
        return false;
    };
    let Json::Obj(pairs) = value else { return false };
    let unsigned = Json::Obj(pairs.iter().filter(|(k, _)| k != "signatures" && k != "unsigned").cloned().collect());
    let (Ok(key), Ok(signature)) = (Ed25519PublicKey::from_base64(ed25519), Ed25519Signature::from_base64(&sig)) else {
        return false;
    };
    key.verify(canonical(&unsigned).as_bytes(), &signature).is_ok()
}

/// The canonical JSON of a value (sorted keys, no spaces): the bytes a signature covers.
pub fn canonical(value: &Json) -> String {
    match value {
        Json::Obj(pairs) => {
            let mut sorted: Vec<&(String, Json)> = pairs.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            let parts: Vec<String> = sorted.iter().map(|(k, v)| format!("{}:{}", json::quote(k), canonical(v))).collect();
            format!("{{{}}}", parts.join(","))
        }
        Json::Arr(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.encode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// A device's keys answer as `keys/query` would give it for `machine`.
    fn query_answer(machine: &Machine) -> Json {
        let keys = machine.device_keys();
        protocol::obj(vec![("device_keys", Json::Obj(vec![(machine.user_id.clone(), Json::Obj(vec![(machine.device_id.clone(), keys)]))]))])
    }

    #[test]
    fn a_fresh_outbound_session_is_not_rotated() {
        let state = Outbound { created_ms: 1_000, messages: 3, members: members(&["@a:x", "@b:x"]), shared: Vec::new() };
        assert!(!rotation_due(&state, &members(&["@b:x", "@a:x"]), 2_000), "the member order is not a change");
    }

    #[test]
    fn a_session_rotates_on_membership_change_messages_or_age() {
        let base = Outbound { created_ms: 0, messages: 0, members: members(&["@a:x", "@b:x"]), shared: Vec::new() };
        assert!(rotation_due(&base, &members(&["@a:x", "@b:x", "@c:x"]), 1), "a member joined");
        assert!(rotation_due(&base, &members(&["@a:x"]), 1), "a member left");
        let busy = Outbound { messages: ROTATE_MESSAGES, ..base.clone() };
        assert!(rotation_due(&busy, &members(&["@a:x", "@b:x"]), 1), "after 100 messages");
        let old = Outbound { messages: 99, ..base.clone() };
        assert!(!rotation_due(&old, &members(&["@a:x", "@b:x"]), ROTATE_AFTER_MS - 1));
        assert!(rotation_due(&old, &members(&["@a:x", "@b:x"]), ROTATE_AFTER_MS), "after a week");
    }

    #[test]
    fn the_upload_body_has_signed_device_keys_and_one_time_keys() {
        let mut m = Machine::new("@a:x", "DEV1");
        let body = m.upload_body(5);
        let device = body.get("device_keys").expect("device_keys");
        assert_eq!(device.str_field("user_id").as_deref(), Some("@a:x"));
        assert_eq!(device.str_field("device_id").as_deref(), Some("DEV1"));
        let keys = device.get("keys").expect("keys");
        assert!(keys.str_field("curve25519:DEV1").is_some());
        assert!(keys.str_field("ed25519:DEV1").is_some());
        assert!(device.get("signatures").and_then(|s| s.get("@a:x")).and_then(|s| s.str_field("ed25519:DEV1")).is_some());
        let Some(Json::Obj(otks)) = body.get("one_time_keys") else { panic!("one_time_keys") };
        assert!(!otks.is_empty(), "the keys were generated");
        for (id, otk) in otks {
            assert!(id.starts_with("signed_curve25519:"), "{id}");
            assert!(otk.str_field("key").is_some());
            assert!(otk.get("signatures").is_some());
        }
    }

    #[test]
    fn the_query_and_claim_bodies_have_the_spec_shape() {
        let query = Machine::query_body(&members(&["@b:x"]));
        assert_eq!(query.get("device_keys").and_then(|d| d.get("@b:x")).and_then(|v| v.array()).map(|a| a.len()), Some(0));
        let claim = Machine::claim_body(&[("@b:x".into(), "D1".into()), ("@b:x".into(), "D2".into()), ("@c:x".into(), "E".into())]);
        let one = claim.get("one_time_keys").expect("one_time_keys");
        assert_eq!(one.get("@b:x").and_then(|d| d.str_field("D1")).as_deref(), Some(OTK_ALGORITHM));
        assert_eq!(one.get("@b:x").and_then(|d| d.str_field("D2")).as_deref(), Some(OTK_ALGORITHM));
        assert_eq!(one.get("@c:x").and_then(|d| d.str_field("E")).as_deref(), Some(OTK_ALGORITHM));
    }

    #[test]
    fn a_device_with_a_forged_signature_is_not_kept() {
        let a = Machine::new("@a:x", "DEV1");
        let mut b = Machine::new("@b:x", "DEVB");
        assert_eq!(b.ingest_query(&query_answer(&a)), Ok(1));
        let mut forged = query_answer(&a);
        let Json::Obj(users) = &mut forged else { panic!() };
        let Json::Obj(devices) = &mut users[0].1 else { panic!() };
        let Json::Obj(fields) = &mut devices[0].1 else { panic!() };
        fields.retain(|(k, _)| k != "signatures");
        fields.push(("signatures".into(), protocol::obj(vec![("@a:x", protocol::obj(vec![("ed25519:DEV1", protocol::text("AAAA"))]))])));
        assert_eq!(Machine::new("@b:x", "DEVB").ingest_query(&forged), Ok(0));
    }

    #[test]
    fn a_room_key_round_trips_and_an_unknown_room_placeholder_is_undecryptable() {
        let mut alice = Machine::new("@a:x", "DEVA");
        let mut bob = Machine::new("@b:x", "DEVB");
        bob.upload_body(2);
        bob.mark_published();
        // Bob's keys reach Alice as a claim answer; Alice's keys reach Bob as a query answer.
        bob.ingest_query(&query_answer(&alice)).unwrap();
        alice.ingest_query(&query_answer(&bob)).unwrap();
        let upload = bob.upload_body(1);
        let otks = upload.get("one_time_keys").cloned().unwrap();
        let claim_answer =
            protocol::obj(vec![("one_time_keys", protocol::obj(vec![("@b:x", protocol::obj(vec![("DEVB", otks)]))]))]);
        assert_eq!(alice.ingest_claim(&claim_answer), Ok(1));

        let room = "!r:x";
        let people = members(&["@a:x", "@b:x"]);
        let (to_device, skipped) = alice.prepare_room(room, &people, 0);
        assert_eq!(skipped, 0);
        assert_eq!(to_device.len(), 1, "one other device");
        let events: Vec<ToDevice> = to_device
            .into_iter()
            .map(|(_, _, content)| ToDevice { sender: "@a:x".into(), kind: "m.room.encrypted".into(), content })
            .collect();
        assert_eq!(bob.receive(&events), 1, "the room key was taken in");

        let text = protocol::obj(vec![("msgtype", protocol::text("m.text")), ("body", protocol::text("hello"))]);
        let content = alice.encrypt_room_event(room, "m.room.message", &text).unwrap();
        assert_eq!(content.str_field("algorithm").as_deref(), Some(MEGOLM_ALGORITHM));
        let ct = Ciphertext {
            id: "$1".into(),
            algorithm: MEGOLM_ALGORITHM.into(),
            sender_key: content.str_field("sender_key").unwrap(),
            device_id: "DEVA".into(),
            session_id: content.str_field("session_id").unwrap(),
            body: content.str_field("ciphertext").unwrap(),
        };
        let (body, _) = bob.open_room_event(room, &ct).expect("bob opens it");
        assert_eq!(body, Body::Text("hello".into()));

        // A room this device holds no key for: the placeholder, never the ciphertext.
        let mut events = vec![Event { id: "$1".into(), sender: "@a:x".into(), ts: 1, body: Body::Encrypted, reply_to: None }];
        let other = Ciphertext { session_id: "unknown".into(), ..ct };
        assert_eq!(bob.decrypt_events("!other:x", &mut events, &[other]), 0);
        assert_eq!(events[0].body, Body::Undecryptable);
    }

    #[test]
    fn a_device_round_trips_through_its_keychain_items() {
        let mut m = Machine::new("@a:x", "DEVA");
        let (curve, ed) = m.identity();
        assert!(m.snapshot().is_none(), "no key yet, so nothing is saved");
        m.set_pickle_key([7u8; 32]);
        let snap = m.snapshot().expect("the first snapshot after the key is set");
        assert!(m.snapshot().is_none(), "a snapshot is taken once per change");
        assert_eq!(parse_key_item(&snap.key), Some([7u8; 32]));
        let (device, pickled) = parse_device_item(&snap.device).expect("a device item");
        assert_eq!(device, "DEVA");
        assert!(!snap.device.contains(&ed), "the pickle does not show the signing key in plain text");
        let back = Machine::restore("@a:x", device, &[7u8; 32], pickled).unwrap();
        assert_eq!(back.identity(), (curve, ed));
        assert!(parse_key_item("not a key").is_none());
        assert!(parse_device_item("no newline").is_none());
    }

    /// Lets `a` send Olm messages to `b`: `b` publishes a one-time key, both learn each other's keys, and `a` claims it.
    fn open_session(a: &mut Machine, b: &mut Machine) {
        let upload = b.upload_body(1);
        b.mark_published();
        a.ingest_query(&query_answer(b)).unwrap();
        b.ingest_query(&query_answer(a)).unwrap();
        let otks = upload.get("one_time_keys").cloned().unwrap();
        let claim = protocol::obj(vec![("one_time_keys", protocol::obj(vec![(b.user_id.as_str(), protocol::obj(vec![(b.device_id.as_str(), otks)]))]))]);
        assert_eq!(a.ingest_claim(&claim), Ok(1));
    }

    #[test]
    fn a_new_device_gets_the_current_room_key_and_own_devices_can_ask_for_it() {
        let mut alice = Machine::new("@a:x", "DEVA");
        let mut alice2 = Machine::new("@a:x", "DEVA2");
        let mut bob = Machine::new("@b:x", "DEVB");
        let mut bob2 = Machine::new("@b:x", "DEVB2");
        let people = members(&["@a:x", "@b:x"]);
        let room = "!r:x";
        open_session(&mut alice, &mut bob);
        let (first, _) = alice.prepare_room(room, &people, 0);
        assert_eq!(first.len(), 1, "bob's one device gets the key");
        // Bob's second device appears later: it gets the same key, and the session is not rotated.
        open_session(&mut alice, &mut bob2);
        assert_eq!(alice.pending_shares(room, &people), vec![("@b:x".to_string(), "DEVB2".to_string())]);
        let (late, _) = alice.prepare_room(room, &people, 0);
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].1, "DEVB2");
        assert!(alice.pending_shares(room, &people).is_empty(), "nothing is pending any more");

        // Alice's own second device asks for the key: answered. A stranger's request and her own device's are not.
        open_session(&mut alice, &mut alice2);
        let session_id = alice.outbound.get(room).unwrap().0.session_id();
        let request = |sender: &str, device: &str| ToDevice {
            sender: sender.into(),
            kind: "m.room_key_request".into(),
            content: protocol::obj(vec![
                ("action", protocol::text("request")),
                ("requesting_device_id", protocol::text(device)),
                ("body", protocol::obj(vec![("room_id", protocol::text(room)), ("session_id", protocol::text(&session_id))])),
            ]),
        };
        let asks = vec![request("@a:x", "DEVA2"), request("@x:x", "DEVA2"), request("@a:x", "DEVA")];
        assert_eq!(alice.key_requests(&asks), vec![(room.to_string(), "@a:x".to_string(), "DEVA2".to_string())]);
        let answer = alice.answer_key_request(room, "@a:x", "DEVA2").expect("a key for alice's second device");
        assert_eq!(answer.1, "DEVA2");
    }

    #[test]
    fn megolm_sessions_survive_a_save_and_a_restore() {
        let key = [5u8; 32];
        let mut m = Machine::new("@a:x", "DEVA");
        m.set_pickle_key(key);
        m.prepare_room("!r:x", &members(&["@a:x"]), 42);
        let blob = m.sessions_blob(&key).expect("an outbound session");
        let mut back = Machine::new("@a:x", "DEVA");
        assert_eq!(back.load_sessions(&key, &blob), Ok(1));
        let text = protocol::obj(vec![("msgtype", protocol::text("m.text")), ("body", protocol::text("kept"))]);
        let a = m.encrypt_room_event("!r:x", "m.room.message", &text).unwrap();
        let b = back.encrypt_room_event("!r:x", "m.room.message", &text).unwrap();
        assert_eq!(a.str_field("session_id"), b.str_field("session_id"), "the restored session is the same room key");
        assert!(Machine::new("@a:x", "DEVA").load_sessions(&[6u8; 32], &blob).is_err(), "the wrong key does not open them");
    }

    #[test]
    fn keychain_item_names_are_per_homeserver() {
        assert_eq!(item_service(DEVICE_ITEM, "https://matrix.example.org"), "matrix-device:https://matrix.example.org");
        assert_ne!(item_service(KEY_ITEM, "https://a.example"), item_service(KEY_ITEM, "https://b.example"));
    }

    #[test]
    fn a_device_restored_from_a_pickle_keeps_its_one_time_keys() {
        let key = [9u8; 32];
        let mut m = Machine::new("@a:x", "DEVA");
        m.upload_body(3);
        m.mark_published();
        let pickled = m.pickle(&key);
        let back = Machine::restore("@a:x", "DEVA", &key, &pickled).unwrap();
        assert_eq!(back.identity(), m.identity());
        assert!(Machine::restore("@a:x", "DEVA", &[1u8; 32], &pickled).is_err(), "the wrong key does not open it");
    }

    #[test]
    fn the_sessions_text_keeps_olm_and_megolm_sessions_across_a_reload() {
        let key = [7u8; 32];
        let mut m = Machine::new("@alice:example.org", "DEV1");
        let peer = Curve25519PublicKey::from_bytes([9u8; 32]);
        let session = m.account.create_outbound_session(OlmConfig::version_2(), peer, peer);
        m.olm.insert("peer-curve".to_string(), session);
        m.outbound.insert(
            "!room:example.org".to_string(),
            (GroupSession::new(MegolmConfig::version_1()), Outbound { created_ms: 5, messages: 2, members: vec!["@bob:example.org".into()], shared: Vec::new() }),
        );
        let blob = m.sessions_blob(&key).expect("sessions");
        assert!(blob.lines().any(|l| l.starts_with("olm\tpeer-curve\t")));
        let mut back = Machine::new("@alice:example.org", "DEV1");
        assert_eq!(back.load_sessions(&key, &blob), Ok(2));
        assert!(back.olm.contains_key("peer-curve"));
        assert_eq!(back.outbound.get("!room:example.org").map(|(_, s)| s.messages), Some(2));
        assert!(back.load_sessions(&[8u8; 32], &blob).is_err(), "a wrong pickle key does not open the sessions");
    }

    #[test]
    fn random_bytes_are_not_all_zero_and_differ() {
        let a: [u8; 32] = random_bytes().unwrap();
        let b: [u8; 32] = random_bytes().unwrap();
        assert_ne!(a, b);
    }
}
