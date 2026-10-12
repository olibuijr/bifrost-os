//! Bluetooth: the radio is driven from the shell through the
//! kernel's Bluetooth management socket (BlueZ's "mgmt" API, `AF_BLUETOOTH`
//! `BTPROTO_HCI` on `HCI_CHANNEL_CONTROL`), the same interface `bluetoothd`
//! uses. Pure Rust over a handful of libc calls, no daemon.
//!
//! Why this and not the alternatives:
//! - Raw HCI sockets (`HCI_CHANNEL_RAW`) would mean writing the host stack's
//!   inquiry, name requests, SSP, link-key and LE bookkeeping by hand, and the
//!   kernel's own handling would race with it. The mgmt socket gives power,
//!   discovery, pairing, unpairing, disconnect and key loading as single
//!   commands with the kernel doing the HCI work.
//!
//! Layout: `Core` is the pure state machine (events in, commands out, unit
//! tested); `Bluetooth` runs it on its own thread against the real socket,
//! like `net::wifi::Wifi`. Link keys are kept in `bluetooth.ini` (mode 0600)
//! and loaded back with LOAD_LINK_KEYS because the kernel forgets bonds on
//! reboot. A connection is an L2CAP link held open to the device; carrying
//! audio over it (AVDTP/A2DP, SBC) is not part of this module.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

// ---- the mgmt wire format (linux/include/net/bluetooth/mgmt.h) ----

const OP_READ_INDEX_LIST: u16 = 0x0003;
const OP_READ_INFO: u16 = 0x0004;
const OP_SET_POWERED: u16 = 0x0005;
const OP_SET_CONNECTABLE: u16 = 0x0007;
const OP_SET_BONDABLE: u16 = 0x0009;
const OP_SET_SSP: u16 = 0x000B;
const OP_SET_LE: u16 = 0x000D;
const OP_SET_LOCAL_NAME: u16 = 0x000F;
const OP_LOAD_LINK_KEYS: u16 = 0x0012;
const OP_DISCONNECT: u16 = 0x0014;
const OP_GET_CONNECTIONS: u16 = 0x0015;
const OP_PIN_CODE_REPLY: u16 = 0x0016;
const OP_PIN_CODE_NEG_REPLY: u16 = 0x0017;
const OP_SET_IO_CAPABILITY: u16 = 0x0018;
const OP_PAIR_DEVICE: u16 = 0x0019;
const OP_UNPAIR_DEVICE: u16 = 0x001B;
const OP_USER_CONFIRM_REPLY: u16 = 0x001C;
const OP_USER_CONFIRM_NEG_REPLY: u16 = 0x001D;
const OP_START_DISCOVERY: u16 = 0x0023;
const OP_STOP_DISCOVERY: u16 = 0x0024;
const OP_CONFIRM_NAME: u16 = 0x0025;

const EV_CMD_COMPLETE: u16 = 0x0001;
const EV_CMD_STATUS: u16 = 0x0002;
const EV_INDEX_ADDED: u16 = 0x0004;
const EV_INDEX_REMOVED: u16 = 0x0005;
const EV_NEW_SETTINGS: u16 = 0x0006;
const EV_NEW_LINK_KEY: u16 = 0x0009;
const EV_DEVICE_CONNECTED: u16 = 0x000B;
const EV_DEVICE_DISCONNECTED: u16 = 0x000C;
const EV_CONNECT_FAILED: u16 = 0x000D;
const EV_PIN_CODE_REQUEST: u16 = 0x000E;
const EV_USER_CONFIRM_REQUEST: u16 = 0x000F;
const EV_AUTH_FAILED: u16 = 0x0011;
const EV_DEVICE_FOUND: u16 = 0x0012;
const EV_DISCOVERING: u16 = 0x0013;
const EV_DEVICE_UNPAIRED: u16 = 0x0016;

/// "no controller" index used by commands that are not per controller.
const INDEX_NONE: u16 = 0xFFFF;
const SETTING_POWERED: u32 = 1;
/// Discovery over BR/EDR and both LE address types.
const DISCOVER_ALL: u8 = 0x07;
/// Just works pairing: NoInputNoOutput, the user's tap is the consent.
const IO_NO_INPUT_NO_OUTPUT: u8 = 3;
const FOUND_CONFIRM_NAME: u32 = 0x04;
/// A scan stops by itself after this long (LE discovery never ends alone).
const SCAN_MS: u64 = 12_000;
/// A pairing that has not finished after this long is given up on.
const PAIR_MS: u64 = 40_000;
const LOCAL_NAME: &str = "BifrOSt";

const BREDR: u8 = 0;

/// One management packet: events from the kernel, commands to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    pub code: u16,
    pub index: u16,
    pub data: Vec<u8>,
}

/// header (opcode, index, length, little endian) + payload.
pub fn encode(code: u16, index: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(6 + payload.len());
    v.extend_from_slice(&code.to_le_bytes());
    v.extend_from_slice(&index.to_le_bytes());
    v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// One whole packet as read from the socket; None when truncated.
pub fn decode(buf: &[u8]) -> Option<Packet> {
    if buf.len() < 6 {
        return None;
    }
    let len = u16::from_le_bytes([buf[4], buf[5]]) as usize;
    let data = buf.get(6..6 + len)?.to_vec();
    Some(Packet { code: u16::from_le_bytes([buf[0], buf[1]]), index: u16::from_le_bytes([buf[2], buf[3]]), data })
}

/// "AA:BB:CC:DD:EE:FF" to the 6 wire bytes (least significant first).
pub fn parse_addr(s: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut parts = s.split(':');
    for slot in out.iter_mut().rev() {
        let p = parts.next()?;
        if p.len() != 2 {
            return None;
        }
        *slot = u8::from_str_radix(p, 16).ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// Wire bytes to "AA:BB:CC:DD:EE:FF".
pub fn fmt_addr(wire: &[u8]) -> String {
    wire.iter().rev().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// `mgmt_addr_info`: the address bytes and the type, 7 bytes.
fn addr_info(addr: &str, kind: u8) -> Option<Vec<u8>> {
    let mut v = parse_addr(addr)?.to_vec();
    v.push(kind);
    Some(v)
}

/// The text for a mgmt status code.
pub fn status_text(code: u8) -> &'static str {
    match code {
        4 => "Connection Failed",
        5 => "Authentication Failed",
        8 => "Timed Out",
        9 => "Already Connected",
        10 => "Device Busy",
        11 => "Rejected",
        15 => "Bluetooth Is Off",
        18 => "Bluetooth Is Blocked",
        19 => "Already Paired",
        _ => "Bluetooth Error",
    }
}

// ---- advertising / extended inquiry data ----

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Eir {
    pub name: Option<String>,
    pub class: Option<u32>,
    /// lists an audio profile (A2DP sink/source, headset, hands-free)
    pub audio_uuid: bool,
}

const UUID_AUDIO: [u16; 6] = [0x110A, 0x110B, 0x110D, 0x1108, 0x111E, 0x1203];

/// Reads the name, class of device and audio profiles out of EIR/AD structures.
pub fn parse_eir(eir: &[u8]) -> Eir {
    let mut out = Eir::default();
    let mut short = None;
    let mut i = 0;
    while i < eir.len() {
        let len = eir[i] as usize;
        if len == 0 || i + 1 + len > eir.len() {
            break;
        }
        let (kind, data) = (eir[i + 1], &eir[i + 2..i + 1 + len]);
        match kind {
            0x09 => out.name = Some(String::from_utf8_lossy(data).into_owned()),
            0x08 => short = Some(String::from_utf8_lossy(data).into_owned()),
            0x0D if data.len() >= 3 => out.class = Some(u32::from(data[0]) | u32::from(data[1]) << 8 | u32::from(data[2]) << 16),
            0x02 | 0x03 => out.audio_uuid |= data.chunks_exact(2).any(|u| UUID_AUDIO.contains(&u16::from_le_bytes([u[0], u[1]]))),
            _ => {}
        }
        i += 1 + len;
    }
    out.name = out.name.or(short).filter(|n| !n.is_empty());
    out
}

/// Major device class 4 is audio/video (headphones, speakers, car kits).
pub fn class_is_audio(class: u32) -> bool {
    class >> 8 & 0x1F == 4
}

// ---- what the shell sees ----

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// "AA:BB:CC:DD:EE:FF"
    pub addr: String,
    /// mgmt address type: 0 BR/EDR, 1 LE public, 2 LE random
    pub kind: u8,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    /// headphones, speaker, car kit: something that can be the sound's route
    pub audio: bool,
    pub rssi: Option<i8>,
}

impl Device {
    fn new(addr: String, kind: u8) -> Device {
        Device { addr, kind, name: String::new(), paired: false, connected: false, audio: false, rssi: None }
    }

    /// The name, or the address of a device that never told us one.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.addr
        } else {
            &self.name
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// no controller (yet): the kernel has no hci device or the socket failed
    #[default]
    Unavailable,
    Off,
    On,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub state: State,
    pub scanning: bool,
    /// paired devices first, then the ones found, each group by name
    pub devices: Vec<Device>,
    /// the address being paired or connected right now
    pub busy: Option<String>,
    pub error: Option<String>,
}

impl Status {
    pub fn on(&self) -> bool {
        self.state == State::On
    }

    pub fn device(&self, addr: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.addr == addr)
    }

    /// Name of the connected audio device, if any: what Now Playing shows as the route.
    pub fn audio_sink(&self) -> Option<&str> {
        self.devices.iter().find(|d| d.connected && d.audio).map(Device::label)
    }

    /// The Settings row's value: "Connected" when something is, else "On"/"Off".
    pub fn summary(&self) -> &'static str {
        match self.state {
            State::On if self.devices.iter().any(|d| d.connected) => "Connected",
            State::On => "On",
            _ => "Off",
        }
    }
}

/// What the shell can ask for; addresses are "AA:BB:CC:DD:EE:FF".
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Enable(bool),
    Scan,
    Pair(String),
    Connect(String),
    Disconnect(String),
    /// unpair and drop the stored key
    Forget(String),
}

// ---- the bonds on disk ----

#[derive(Clone, Debug, PartialEq)]
pub struct LinkKey {
    pub addr: String,
    pub kind: u8,
    pub key_type: u8,
    pub key: [u8; 16],
    pub pin_len: u8,
    pub name: String,
    pub audio: bool,
}

/// bluetooth.ini: `power=1` and one `key=addr,kind,type,hex,pin,audio,name` per bond.
pub fn encode_store(power: bool, keys: &[LinkKey]) -> String {
    let mut s = format!("power={}\n", u8::from(power));
    for k in keys {
        let hex: String = k.key.iter().map(|b| format!("{b:02x}")).collect();
        s.push_str(&format!("key={},{},{},{},{},{},{}\n", k.addr, k.kind, k.key_type, hex, k.pin_len, u8::from(k.audio), k.name.replace('\n', " ")));
    }
    s
}

pub fn parse_store(text: &str) -> (bool, Vec<LinkKey>) {
    let mut power = false;
    let mut keys = Vec::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("power=") {
            power = v.trim() == "1";
        } else if let Some(v) = line.strip_prefix("key=") {
            let f: Vec<&str> = v.splitn(7, ',').collect();
            if f.len() < 7 || parse_addr(f[0]).is_none() || f[3].len() != 32 || !f[3].is_ascii() {
                continue;
            }
            let mut key = [0u8; 16];
            let ok = (0..16).all(|i| u8::from_str_radix(&f[3][2 * i..2 * i + 2], 16).map(|b| key[i] = b).is_ok());
            if let (true, Ok(kind), Ok(key_type), Ok(pin_len)) = (ok, f[1].parse(), f[2].parse(), f[4].parse()) {
                keys.push(LinkKey { addr: f[0].to_string(), kind, key_type, key, pin_len, audio: f[5] == "1", name: f[6].to_string() });
            }
        }
    }
    (power, keys)
}

// ---- the pure state machine ----

/// What `Core` asks its runner to do, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Mgmt(u16, Vec<u8>),
    /// open (and keep) a link to this device
    Link(String, u8),
    /// close the held link
    Unlink(String),
    /// clear the rfkill soft block
    Unblock,
    /// write bluetooth.ini
    Save,
}

#[derive(Debug, Default)]
pub struct Core {
    pub status: Status,
    keys: Vec<LinkKey>,
    /// the user wants the radio on (persisted)
    power: bool,
    scan_since: Option<u64>,
    busy_since: u64,
}

impl Core {
    /// From the stored bonds; devices start out paired and disconnected.
    pub fn new(power: bool, keys: Vec<LinkKey>) -> Core {
        let mut c = Core { power, keys, ..Core::default() };
        c.status.devices = c.keys.iter().map(|k| Device { name: k.name.clone(), paired: true, audio: k.audio, ..Device::new(k.addr.clone(), k.kind) }).collect();
        c.sort();
        c
    }

    pub fn power(&self) -> bool {
        self.power
    }

    pub fn store(&self) -> String {
        encode_store(self.power, &self.keys)
    }

    /// A controller exists (or went away).
    pub fn set_available(&mut self, available: bool, powered: bool) {
        self.status.state = match (available, powered) {
            (false, _) => State::Unavailable,
            (true, true) => State::On,
            (true, false) => State::Off,
        };
    }

    /// What to do once a controller shows up: restore the saved power state.
    pub fn startup(&mut self) -> Vec<Out> {
        if self.power {
            self.enable()
        } else {
            vec![]
        }
    }

    fn sort(&mut self) {
        self.status.devices.sort_by(|a, b| b.paired.cmp(&a.paired).then(a.label().to_lowercase().cmp(&b.label().to_lowercase())));
    }

    fn entry(&mut self, addr: &str, kind: u8) -> &mut Device {
        if let Some(i) = self.status.devices.iter().position(|d| d.addr == addr) {
            return &mut self.status.devices[i];
        }
        self.status.devices.push(Device::new(addr.to_string(), kind));
        self.status.devices.last_mut().expect("just pushed")
    }

    fn mgmt(&self, op: u16, addr: &str, tail: &[u8]) -> Option<Out> {
        let kind = self.status.device(addr).map_or(BREDR, |d| d.kind);
        let mut p = addr_info(addr, kind)?;
        p.extend_from_slice(tail);
        Some(Out::Mgmt(op, p))
    }

    fn enable(&mut self) -> Vec<Out> {
        let mut keys = vec![0u8];
        keys.extend_from_slice(&(self.keys.len() as u16).to_le_bytes());
        for k in &self.keys {
            if let Some(a) = addr_info(&k.addr, k.kind) {
                keys.extend_from_slice(&a);
                keys.push(k.key_type);
                keys.extend_from_slice(&k.key);
                keys.push(k.pin_len);
            }
        }
        let mut name = LOCAL_NAME.as_bytes().to_vec();
        name.resize(249 + 11, 0);
        self.status.state = State::On;
        vec![
            Out::Unblock,
            Out::Mgmt(OP_LOAD_LINK_KEYS, keys),
            Out::Mgmt(OP_SET_BONDABLE, vec![1]),
            Out::Mgmt(OP_SET_SSP, vec![1]),
            Out::Mgmt(OP_SET_LE, vec![1]),
            Out::Mgmt(OP_SET_CONNECTABLE, vec![1]),
            Out::Mgmt(OP_SET_IO_CAPABILITY, vec![IO_NO_INPUT_NO_OUTPUT]),
            Out::Mgmt(OP_SET_LOCAL_NAME, name),
            Out::Mgmt(OP_SET_POWERED, vec![1]),
            Out::Save,
        ]
    }

    pub fn command(&mut self, cmd: Cmd, now: u64) -> Vec<Out> {
        self.status.error = None;
        if self.status.state == State::Unavailable {
            // remember the wish; the runner restores it when a controller appears
            if let Cmd::Enable(on) = cmd {
                self.power = on;
                return vec![Out::Save];
            }
            return vec![];
        }
        match cmd {
            Cmd::Enable(true) => {
                self.power = true;
                self.enable()
            }
            Cmd::Enable(false) => {
                self.power = false;
                let mut out: Vec<Out> = self.status.devices.iter().filter(|d| d.connected).map(|d| Out::Unlink(d.addr.clone())).collect();
                if self.status.scanning {
                    out.push(Out::Mgmt(OP_STOP_DISCOVERY, vec![DISCOVER_ALL]));
                }
                out.push(Out::Mgmt(OP_SET_POWERED, vec![0]));
                out.push(Out::Save);
                self.status.state = State::Off;
                self.status.scanning = false;
                self.status.busy = None;
                self.scan_since = None;
                self.status.devices.retain(|d| d.paired);
                self.status.devices.iter_mut().for_each(|d| d.connected = false);
                out
            }
            Cmd::Scan if self.status.on() && !self.status.scanning => {
                self.status.devices.retain(|d| d.paired || d.connected);
                self.status.scanning = true;
                self.scan_since = Some(now);
                vec![Out::Mgmt(OP_START_DISCOVERY, vec![DISCOVER_ALL])]
            }
            Cmd::Pair(addr) if self.status.on() && self.status.busy.is_none() => {
                let mut out = self.stop_scan();
                out.extend(self.mgmt(OP_PAIR_DEVICE, &addr, &[IO_NO_INPUT_NO_OUTPUT]));
                self.status.busy = Some(addr);
                self.busy_since = now;
                out
            }
            Cmd::Connect(addr) if self.status.on() && self.status.busy.is_none() => match self.status.device(&addr) {
                Some(d) if d.paired && !d.connected => {
                    let out = vec![Out::Link(addr.clone(), d.kind)];
                    self.status.busy = Some(addr);
                    self.busy_since = now;
                    out
                }
                _ => vec![],
            },
            Cmd::Disconnect(addr) => {
                let mut out = vec![Out::Unlink(addr.clone())];
                out.extend(self.mgmt(OP_DISCONNECT, &addr, &[]));
                out
            }
            Cmd::Forget(addr) => {
                let mut out = vec![Out::Unlink(addr.clone())];
                out.extend(self.mgmt(OP_UNPAIR_DEVICE, &addr, &[1]));
                self.forget(&addr);
                out.push(Out::Save);
                out
            }
            _ => vec![],
        }
    }

    fn forget(&mut self, addr: &str) {
        self.keys.retain(|k| k.addr != addr);
        self.status.devices.retain(|d| d.addr != addr);
        if self.status.busy.as_deref() == Some(addr) {
            self.status.busy = None;
        }
    }

    fn stop_scan(&mut self) -> Vec<Out> {
        self.scan_since = None;
        if std::mem::take(&mut self.status.scanning) {
            vec![Out::Mgmt(OP_STOP_DISCOVERY, vec![DISCOVER_ALL])]
        } else {
            vec![]
        }
    }

    /// Time passing: ends a scan and a pairing that took too long.
    pub fn tick(&mut self, now: u64) -> Vec<Out> {
        let mut out = vec![];
        if self.scan_since.is_some_and(|t| now.saturating_sub(t) >= SCAN_MS) {
            out = self.stop_scan();
        }
        if self.status.busy.is_some() && now.saturating_sub(self.busy_since) >= PAIR_MS {
            self.status.busy = None;
            self.status.error = Some("Timed Out".into());
        }
        out
    }

    /// Milliseconds until `tick` has something to do, or None when it has nothing pending (the
    /// runner then sleeps until a packet or a command).
    pub fn next_tick_ms(&self, now: u64) -> Option<u64> {
        let scan = self.scan_since.map(|t| (t + SCAN_MS).saturating_sub(now));
        let pair = self.status.busy.is_some().then(|| (self.busy_since + PAIR_MS).saturating_sub(now));
        scan.into_iter().chain(pair).min()
    }

    /// The link thread finished: the device is connected, or it was not reachable.
    pub fn link_result(&mut self, addr: &str, ok: bool) {
        if self.status.busy.as_deref() == Some(addr) {
            self.status.busy = None;
        }
        if ok {
            self.entry(addr, BREDR).connected = true;
        } else {
            self.status.error = Some("Could Not Connect".into());
        }
    }

    /// A link attempt was cancelled (disconnect or forget) before it finished.
    pub fn link_abandoned(&mut self, addr: &str) {
        if self.status.busy.as_deref() == Some(addr) {
            self.status.busy = None;
        }
    }

    fn settings(&mut self, s: u32) {
        if self.status.state != State::Unavailable {
            self.status.state = if s & SETTING_POWERED != 0 { State::On } else { State::Off };
            if !self.status.on() {
                self.status.scanning = false;
            }
        }
    }

    fn fill(&mut self, addr: &str, kind: u8, eir: &[u8]) {
        let e = parse_eir(eir);
        let d = self.entry(addr, kind);
        if let Some(n) = e.name {
            d.name = n;
        }
        d.audio |= e.audio_uuid || e.class.is_some_and(class_is_audio);
    }

    /// One kernel event; what to send back.
    pub fn event(&mut self, p: &Packet) -> Vec<Out> {
        let d = &p.data;
        let addr = || d.get(..6).map(fmt_addr);
        let kind = d.get(6).copied().unwrap_or(BREDR);
        match p.code {
            EV_CMD_COMPLETE | EV_CMD_STATUS if d.len() >= 3 => {
                let (op, status) = (u16::from_le_bytes([d[0], d[1]]), d[2]);
                self.reply(op, status, &d[3..]);
                vec![]
            }
            EV_INDEX_REMOVED => {
                self.status.state = State::Unavailable;
                self.status.scanning = false;
                self.status.devices.iter_mut().for_each(|d| d.connected = false);
                vec![]
            }
            EV_INDEX_ADDED => {
                self.set_available(true, false);
                self.startup()
            }
            EV_NEW_SETTINGS if d.len() >= 4 => {
                self.settings(u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
                vec![]
            }
            EV_DEVICE_FOUND if d.len() >= 14 => {
                let (Some(a), rssi, flags) = (addr(), d[7] as i8, u32::from_le_bytes([d[8], d[9], d[10], d[11]])) else { return vec![] };
                let eir_len = u16::from_le_bytes([d[12], d[13]]) as usize;
                let eir = d.get(14..14 + eir_len).unwrap_or(&[]);
                self.fill(&a, kind, eir);
                self.entry(&a, kind).rssi = Some(rssi);
                let nameless = self.status.device(&a).is_some_and(|x| x.name.is_empty());
                self.sort();
                // ask the controller to look the name up
                if flags & FOUND_CONFIRM_NAME != 0 && nameless {
                    return self.mgmt(OP_CONFIRM_NAME, &a, &[0]).into_iter().collect();
                }
                vec![]
            }
            EV_DISCOVERING if d.len() >= 2 => {
                self.status.scanning = d[1] != 0;
                vec![]
            }
            EV_DEVICE_CONNECTED if d.len() >= 13 => {
                let Some(a) = addr() else { return vec![] };
                let eir_len = u16::from_le_bytes([d[11], d[12]]) as usize;
                self.fill(&a, kind, d.get(13..13 + eir_len).unwrap_or(&[]));
                self.entry(&a, kind).connected = true;
                self.sort();
                vec![]
            }
            EV_DEVICE_DISCONNECTED => {
                if let Some(a) = addr() {
                    self.entry(&a, kind).connected = false;
                    return vec![Out::Unlink(a)];
                }
                vec![]
            }
            EV_CONNECT_FAILED | EV_AUTH_FAILED if d.len() >= 7 => {
                if let Some(a) = addr() {
                    if self.status.busy.as_deref() == Some(&a) {
                        self.status.busy = None;
                    }
                }
                self.status.error = Some(if p.code == EV_AUTH_FAILED { "Pairing Failed" } else { "Could Not Connect" }.into());
                vec![]
            }
            // pairing we started: accept; anyone else asking is refused
            EV_USER_CONFIRM_REQUEST | EV_PIN_CODE_REQUEST => {
                let Some(a) = addr() else { return vec![] };
                let ours = self.status.busy.as_deref() == Some(&a);
                let confirm = p.code == EV_USER_CONFIRM_REQUEST;
                let (op, tail): (u16, Vec<u8>) = match (confirm, ours) {
                    (true, true) => (OP_USER_CONFIRM_REPLY, vec![]),
                    (true, false) => (OP_USER_CONFIRM_NEG_REPLY, vec![]),
                    (false, true) => (OP_PIN_CODE_REPLY, [&[4u8][..], b"0000", &[0; 12]].concat()),
                    (false, false) => (OP_PIN_CODE_NEG_REPLY, vec![]),
                };
                self.mgmt(op, &a, &tail).into_iter().collect()
            }
            EV_NEW_LINK_KEY if d.len() >= 26 => {
                let Some(a) = d.get(1..7).map(fmt_addr) else { return vec![] };
                let mut key = [0u8; 16];
                key.copy_from_slice(&d[9..25]);
                let (name, audio) = self.status.device(&a).map_or((String::new(), false), |x| (x.name.clone(), x.audio));
                let entry = LinkKey { addr: a.clone(), kind: d[7], key_type: d[8], key, pin_len: d[25], name, audio };
                self.keys.retain(|k| k.addr != a);
                if d[0] != 0 {
                    self.keys.push(entry);
                }
                let dev = self.entry(&a, d[7]);
                dev.paired = true;
                self.sort();
                vec![Out::Save]
            }
            EV_DEVICE_UNPAIRED => {
                if let Some(a) = addr() {
                    self.forget(&a);
                }
                vec![Out::Save]
            }
            _ => vec![],
        }
    }

    /// The answer to one of our commands.
    fn reply(&mut self, op: u16, status: u8, data: &[u8]) {
        if status != 0 {
            match op {
                OP_PAIR_DEVICE | OP_SET_POWERED => {
                    self.status.busy = None;
                    self.status.error = Some(status_text(status).into());
                    if op == OP_SET_POWERED && self.status.state == State::On {
                        self.status.state = State::Off;
                    }
                }
                _ => {}
            }
            return;
        }
        match op {
            OP_SET_POWERED | OP_SET_CONNECTABLE | OP_SET_SSP | OP_SET_LE | OP_SET_BONDABLE if data.len() >= 4 => self.settings(u32::from_le_bytes([data[0], data[1], data[2], data[3]])),
            OP_PAIR_DEVICE if data.len() >= 7 => {
                let a = fmt_addr(&data[..6]);
                self.entry(&a, data[6]).paired = true;
                self.status.busy = None;
            }
            OP_GET_CONNECTIONS if data.len() >= 2 => {
                for c in data[2..].chunks_exact(7) {
                    self.entry(&fmt_addr(&c[..6]), c[6]).connected = true;
                }
            }
            _ => {}
        }
    }

    /// The address to connect to once a pairing succeeded (a paired, unconnected device).
    pub fn after_pair(&self, addr: &str) -> bool {
        self.status.device(addr).is_some_and(|d| d.paired && !d.connected)
    }
}

// ---- the thread that runs it ----

/// The Bluetooth thread; the shell keeps one, like `Wifi`.
pub struct Bluetooth {
    pub status: Arc<Mutex<Status>>,
    tx: Sender<Cmd>,
    /// Pokes the thread out of its wait so a command is not left to the poll timeout.
    wake: Wake,
}

impl Bluetooth {
    /// Starts the thread; `dir` holds bluetooth.ini.
    pub fn start(dir: &str) -> Bluetooth {
        let status = Arc::new(Mutex::new(Status::default()));
        let (tx, rx) = std::sync::mpsc::channel();
        let (wake, sleeper) = wake_pair();
        let (st, d, w) = (status.clone(), dir.to_string(), wake.clone());
        crate::bare::sys::spawn_named("bluetooth", move || run(&d, st, rx, sleeper, w));
        Bluetooth { status, tx, wake }
    }

    pub fn send(&self, c: Cmd) {
        if self.tx.send(c).is_ok() {
            self.wake.poke();
        }
    }

    pub fn snapshot(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

#[cfg(target_os = "linux")]
use driver::run;

#[cfg(not(target_os = "linux"))]
fn run(_dir: &str, _status: Arc<Mutex<Status>>, rx: Receiver<Cmd>, _sleeper: Sleeper, _wake: Wake) {
    while rx.recv().is_ok() {}
}

/// The sending half of the wake channel: one byte makes the thread's poll return.
#[derive(Clone)]
struct Wake(Arc<std::os::unix::net::UnixStream>);

/// The thread's half: pollable, drained after a wake.
struct Sleeper(std::os::unix::net::UnixStream);

fn wake_pair() -> (Wake, Sleeper) {
    use std::os::unix::net::UnixStream;
    let (a, b) = UnixStream::pair().expect("socketpair");
    let _ = a.set_nonblocking(true);
    let _ = b.set_nonblocking(true);
    (Wake(Arc::new(a)), Sleeper(b))
}

impl Wake {
    fn poke(&self) {
        use std::io::Write;
        // a full buffer already means a wake is pending
        let _ = (&*self.0).write(&[1]);
    }
}

impl Sleeper {
    fn drain(&self) {
        use std::io::Read;
        let mut b = [0u8; 64];
        while matches!((&self.0).read(&mut b), Ok(n) if n > 0) {}
    }
}

#[cfg(target_os = "linux")]
mod driver {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::c_int;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::time::{Duration, Instant};

    const AF_BLUETOOTH: c_int = 31;
    const SOCK_RAW: c_int = 3;
    const SOCK_SEQPACKET: c_int = 5;
    const SOCK_CLOEXEC: c_int = 0o2000000;
    const BTPROTO_L2CAP: c_int = 0;
    const BTPROTO_HCI: c_int = 1;
    const HCI_CHANNEL_CONTROL: u16 = 3;
    const SOL_SOCKET: c_int = 1;
    const SO_SNDTIMEO: c_int = 21;
    /// How long a connect may block before the link thread gives up.
    const CONNECT_SECS: std::ffi::c_long = 10;

    /// Whether the host's own `bluetoothd` is running; two managers must not share one controller.
    fn host_bluez_running() -> bool {
        std::fs::read_dir("/proc").is_ok_and(|d| d.flatten().any(|e| std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "bluetoothd")))
    }

    #[repr(C)]
    struct SockaddrHci {
        family: u16,
        dev: u16,
        channel: u16,
    }

    #[repr(C)]
    struct SockaddrL2 {
        family: u16,
        psm: u16,
        bdaddr: [u8; 6],
        cid: u16,
        bdaddr_type: u8,
    }

    use crate::bare::sys::{self, bind, check, close, connect, read, setsockopt, socket, write};

    /// The management socket.
    struct Mgmt(c_int);

    impl Mgmt {
        fn open() -> std::io::Result<Mgmt> {
            // SAFETY: plain syscalls with a valid, correctly sized sockaddr.
            let fd = check(unsafe { socket(AF_BLUETOOTH, SOCK_RAW | SOCK_CLOEXEC, BTPROTO_HCI) })?;
            let a = SockaddrHci { family: AF_BLUETOOTH as u16, dev: INDEX_NONE, channel: HCI_CHANNEL_CONTROL };
            if let Err(e) = check(unsafe { bind(fd, &a as *const _ as *const u8, std::mem::size_of::<SockaddrHci>() as u32) }) {
                unsafe { close(fd) };
                return Err(e);
            }
            Ok(Mgmt(fd))
        }

        fn send(&self, op: u16, index: u16, payload: &[u8]) -> std::io::Result<()> {
            let b = encode(op, index, payload);
            // SAFETY: `b` is valid for its length.
            check(unsafe { write(self.0, b.as_ptr() as *const _, b.len()) as c_int }).map(|_| ())
        }

        /// The next packet within `ms`, None on timeout.
        fn recv(&self, ms: i32) -> std::io::Result<Option<Packet>> {
            if !sys::wait(&[self.0], ms).first().copied().unwrap_or(false) {
                return Ok(None);
            }
            let mut buf = vec![0u8; 4096];
            // SAFETY: `buf` is valid for its length.
            let n = check(unsafe { read(self.0, buf.as_mut_ptr() as *mut _, buf.len()) as c_int })? as usize;
            Ok(decode(&buf[..n]))
        }

        /// Sends a command and waits for its reply; other events wait in `later`.
        fn call(&self, op: u16, index: u16, payload: &[u8], later: &mut Vec<Packet>) -> std::io::Result<Vec<u8>> {
            self.send(op, index, payload)?;
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                let Some(p) = self.recv(200)? else { continue };
                if p.code == EV_CMD_COMPLETE && p.index == index && p.data.len() >= 3 && u16::from_le_bytes([p.data[0], p.data[1]]) == op {
                    return if p.data[2] == 0 { Ok(p.data[3..].to_vec()) } else { Err(std::io::Error::other(status_text(p.data[2]))) };
                }
                later.push(p);
            }
            Err(std::io::ErrorKind::TimedOut.into())
        }
    }

    impl Drop for Mgmt {
        fn drop(&mut self) {
            // SAFETY: the fd is ours.
            unsafe { close(self.0) };
        }
    }

    /// Clears the rfkill soft block of every Bluetooth radio.
    fn unblock() {
        let Ok(dir) = std::fs::read_dir("/sys/class/rfkill") else { return };
        for e in dir.flatten() {
            if std::fs::read_to_string(e.path().join("type")).is_ok_and(|t| t.trim() == "bluetooth") {
                let _ = std::fs::write(e.path().join("soft"), "0");
            }
        }
    }

    /// Opens an L2CAP link to the device and returns the held socket; the link lives as long as the fd.
    fn link(addr: &str, kind: u8) -> std::io::Result<OwnedFd> {
        let wire = parse_addr(addr).ok_or_else(|| std::io::Error::other("bad address"))?;
        // BR/EDR: the SDP channel, which every device has; LE: the ATT fixed channel
        let (psm, cid) = if kind == BREDR { (1u16, 0u16) } else { (0, 4) };
        // SAFETY: plain syscalls with a valid sockaddr.
        let fd = check(unsafe { socket(AF_BLUETOOTH, SOCK_SEQPACKET | SOCK_CLOEXEC, BTPROTO_L2CAP) })?;
        // SAFETY: `fd` is a fresh socket nobody else owns; it closes on every path from here.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // a blocking connect honours the send timeout, so an absent device cannot hang the thread
        let tv = sys::timeval { tv_sec: CONNECT_SECS, tv_usec: 0 };
        check(unsafe { setsockopt(fd.as_raw_fd(), SOL_SOCKET, SO_SNDTIMEO, &tv as *const _ as *const _, std::mem::size_of::<sys::timeval>() as u32) })?;
        let a = SockaddrL2 { family: AF_BLUETOOTH as u16, psm: psm.to_le(), bdaddr: wire, cid: cid.to_le(), bdaddr_type: kind };
        check(unsafe { connect(fd.as_raw_fd(), &a as *const _ as *const u8, std::mem::size_of::<SockaddrL2>() as u32) })?;
        Ok(fd)
    }

    /// The L2CAP links: held sockets and attempts still being made. A held
    /// socket closes when dropped, and a result nobody wants any more (the
    /// device was unlinked meanwhile) is closed instead of resurrecting it.
    #[derive(Default)]
    struct Links {
        held: HashMap<String, OwnedFd>,
        pending: std::collections::HashSet<String>,
    }

    impl Links {
        fn begin(&mut self, addr: &str) {
            self.pending.insert(addr.to_string());
        }

        /// Drops the link and any attempt in flight; true when there was one.
        fn cancel(&mut self, addr: &str) -> bool {
            let had = self.pending.remove(addr);
            self.held.remove(addr).is_some() || had
        }

        /// Takes a finished attempt; false when it was cancelled meanwhile
        /// (the socket is closed).
        fn finish(&mut self, addr: &str, fd: Option<OwnedFd>) -> bool {
            if !self.pending.remove(addr) {
                return false;
            }
            if let Some(fd) = fd {
                self.held.insert(addr.to_string(), fd);
            }
            true
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn fd() -> OwnedFd {
            std::os::unix::net::UnixStream::pair().unwrap().0.into()
        }

        #[test]
        fn a_finished_attempt_is_held_until_cancelled() {
            let mut l = Links::default();
            l.begin("A");
            assert!(l.finish("A", Some(fd())));
            assert!(l.held.contains_key("A"));
            assert!(l.cancel("A"));
            assert!(!l.cancel("A"));
        }

        #[test]
        fn a_cancelled_attempt_is_not_resurrected() {
            let mut l = Links::default();
            l.begin("A");
            assert!(l.cancel("A"));
            assert!(!l.finish("A", Some(fd())));
            assert!(l.held.is_empty());
        }

        #[test]
        fn a_second_link_replaces_and_closes_the_first() {
            let mut l = Links::default();
            l.begin("A");
            l.finish("A", Some(fd()));
            l.begin("A");
            l.finish("A", Some(fd()));
            assert_eq!(l.held.len(), 1);
        }
    }

    /// Waits up to `ms` for the management socket or the wake socket; true
    /// when the management socket has data.
    fn wait(mgmt: c_int, wake: c_int, ms: c_int) -> bool {
        // a negative fd (no management socket) is ignored by poll
        sys::wait(&[mgmt, wake], ms).first().copied().unwrap_or(false)
    }

    /// Milliseconds since `t0` (a `sys::now_ms` reading).
    fn since(t0: f64) -> u64 {
        (sys::now_ms() - t0) as u64
    }

    struct Runner {
        mgmt: Option<Mgmt>,
        index: u16,
        core: Core,
        links: Links,
        dir: String,
        shared: Arc<Mutex<Status>>,
        later: Vec<Packet>,
        done_tx: Sender<(String, std::io::Result<OwnedFd>)>,
        wake: Wake,
    }

    impl Runner {
        fn publish(&self) {
            if let Ok(mut s) = self.shared.lock() {
                *s = self.core.status.clone();
            }
        }

        /// Finds the controller.
        fn attach(&mut self) {
            let Some(m) = self.mgmt.as_ref() else { return };
            let first = |s: &mut Vec<Packet>| m.call(OP_READ_INDEX_LIST, INDEX_NONE, &[], s).ok().and_then(|d| d.get(2..4).map(|i| u16::from_le_bytes([i[0], i[1]])));
            let index = first(&mut self.later);
            // a running bluetoothd owns the controller
            if host_bluez_running() {
                self.core.set_available(false, false);
                return;
            }
            let Some(i) = index else {
                self.core.set_available(false, false);
                return;
            };
            self.index = i;
            let info = m.call(OP_READ_INFO, i, &[], &mut self.later).unwrap_or_default();
            let powered = info.get(13..17).is_some_and(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) & SETTING_POWERED != 0);
            self.core.set_available(true, powered);
            let out = self.core.startup();
            self.apply(out);
        }

        fn apply(&mut self, out: Vec<Out>) {
            for o in out {
                match o {
                    Out::Mgmt(op, payload) => {
                        if let Some(Err(e)) = self.mgmt.as_ref().map(|m| m.send(op, self.index, &payload)) {
                            eprintln!("springboard: bluetooth: command {op:#x}: {e}");
                        }
                    }
                    Out::Unblock => unblock(),
                    Out::Save => self.save(),
                    Out::Unlink(a) => {
                        if self.links.cancel(&a) {
                            self.core.link_abandoned(&a);
                        }
                    }
                    Out::Link(a, kind) => {
                        self.links.begin(&a);
                        let (tx, wake) = (self.done_tx.clone(), self.wake.clone());
                        crate::bare::sys::spawn_named("bt-link", move || {
                            let r = link(&a, kind);
                            let _ = tx.send((a, r));
                            wake.poke();
                        });
                    }
                }
            }
        }

        fn save(&self) {
            use std::os::unix::fs::OpenOptionsExt;
            use std::io::Write;
            let path = format!("{}/bluetooth.ini", self.dir);
            let r = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path).and_then(|mut f| f.write_all(self.core.store().as_bytes()));
            if let Err(e) = r {
                eprintln!("springboard: bluetooth: {path}: {e}");
            }
        }
    }

    pub fn run(dir: &str, shared: Arc<Mutex<Status>>, rx: Receiver<Cmd>, sleeper: Sleeper, wake: Wake) {
        let t0 = sys::now_ms();
        let (power, keys) = std::fs::read_to_string(format!("{dir}/bluetooth.ini")).map(|t| parse_store(&t)).unwrap_or_default();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let mut r = Runner { mgmt: Mgmt::open().map_err(|e| eprintln!("springboard: bluetooth: mgmt socket: {e}")).ok(), index: 0, core: Core::new(power, keys), links: Links::default(), dir: dir.to_string(), shared, later: vec![], done_tx, wake };
        r.attach();
        r.publish();
        loop {
            loop {
                match rx.try_recv() {
                    Ok(c) => {
                        let out = r.core.command(c, since(t0));
                        r.apply(out);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            while let Ok((addr, res)) = done_rx.try_recv() {
                let ok = res.is_ok();
                if r.links.finish(&addr, res.ok()) {
                    r.core.link_result(&addr, ok);
                }
            }
            let out = r.core.tick(since(t0));
            r.apply(out);
            let mut packets = std::mem::take(&mut r.later);
            // sleeps until a packet, a command or a finished link; the timeout is only the next `tick` deadline (none: forever)
            let mgmt_fd = r.mgmt.as_ref().map_or(-1, |m| m.0);
            let timeout = r.core.next_tick_ms(since(t0)).map_or(-1, |ms| ms.min(i32::MAX as u64 - 1) as i32 + 1);
            let readable = wait(mgmt_fd, sleeper.0.as_raw_fd(), timeout);
            sleeper.drain();
            if readable {
                match r.mgmt.as_ref().map(|m| m.recv(0)) {
                    Some(Ok(Some(p))) => packets.push(p),
                    Some(Ok(None)) | None => {}
                    Some(Err(e)) => {
                        eprintln!("springboard: bluetooth: socket: {e}");
                        r.mgmt = None;
                        r.core.set_available(false, false);
                    }
                }
            }
            for p in packets {
                if p.code == EV_INDEX_ADDED {
                    r.index = p.index;
                }
                let was_on = r.core.status.on();
                let out = r.core.event(&p);
                r.apply(out);
                // just powered: learn which devices are already connected
                if !was_on && r.core.status.on() {
                    r.apply(vec![Out::Mgmt(OP_GET_CONNECTIONS, vec![])]);
                }
                // a finished pairing continues with the connection
                if p.code == EV_CMD_COMPLETE && p.data.len() >= 9 && u16::from_le_bytes([p.data[0], p.data[1]]) == OP_PAIR_DEVICE && p.data[2] == 0 {
                    let a = fmt_addr(&p.data[3..9]);
                    if r.core.after_pair(&a) {
                        let out = r.core.command(Cmd::Connect(a), since(t0));
                        r.apply(out);
                    }
                }
            }
            r.publish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "AA:BB:CC:DD:EE:01";

    #[test]
    fn idle_core_has_no_tick_deadline() {
        let mut c = Core::new(false, vec![]);
        assert_eq!(c.next_tick_ms(5_000), None);
        c.scan_since = Some(1_000);
        assert_eq!(c.next_tick_ms(2_000), Some(SCAN_MS - 1_000));
        assert_eq!(c.next_tick_ms(1_000 + SCAN_MS + 5), Some(0));
    }

    fn found(addr: &str, name: &str, class: u32, flags: u32) -> Packet {
        let mut eir = vec![];
        if !name.is_empty() {
            eir.extend([name.len() as u8 + 1, 0x09]);
            eir.extend(name.as_bytes());
        }
        if class != 0 {
            eir.extend([4, 0x0D, class as u8, (class >> 8) as u8, (class >> 16) as u8]);
        }
        let mut d = parse_addr(addr).unwrap().to_vec();
        d.extend([BREDR, (-60i8) as u8]);
        d.extend(flags.to_le_bytes());
        d.extend((eir.len() as u16).to_le_bytes());
        d.extend(eir);
        Packet { code: EV_DEVICE_FOUND, index: 0, data: d }
    }

    fn on_core() -> Core {
        let mut c = Core::new(false, vec![]);
        c.set_available(true, false);
        c.command(Cmd::Enable(true), 0);
        c
    }

    fn cmd_complete(op: u16, status: u8, data: &[u8]) -> Packet {
        let mut d = op.to_le_bytes().to_vec();
        d.push(status);
        d.extend(data);
        Packet { code: EV_CMD_COMPLETE, index: 0, data: d }
    }

    #[test]
    fn packets_round_trip_and_truncated_ones_are_refused() {
        let b = encode(OP_SET_POWERED, 0, &[1]);
        assert_eq!(b, [5, 0, 0, 0, 1, 0, 1]);
        assert_eq!(decode(&b), Some(Packet { code: 5, index: 0, data: vec![1] }));
        assert_eq!(decode(&b[..6]), None);
        assert_eq!(decode(&[1, 2]), None);
    }

    #[test]
    fn addresses_are_reversed_on_the_wire() {
        assert_eq!(parse_addr("01:02:03:04:05:06"), Some([6, 5, 4, 3, 2, 1]));
        assert_eq!(fmt_addr(&[6, 5, 4, 3, 2, 1]), "01:02:03:04:05:06");
        assert_eq!(parse_addr("01:02:03:04:05"), None);
        assert_eq!(parse_addr("01:02:03:04:05:06:07"), None);
        assert_eq!(parse_addr("zz:02:03:04:05:06"), None);
    }

    #[test]
    fn eir_gives_name_class_and_audio_profiles() {
        let e = parse_eir(&[3, 0x08, b'A', b'B', 5, 0x09, b'B', b'u', b'd', b's', 5, 0x03, 0x0B, 0x11, 0x1E, 0x11]);
        assert_eq!(e.name.as_deref(), Some("Buds"), "the complete name wins over the short one");
        assert!(e.audio_uuid);
        assert_eq!(parse_eir(&[4, 0x0D, 0x14, 0x04, 0x24]).class, Some(0x240414));
        assert!(class_is_audio(0x240414) && !class_is_audio(0x000104));
        assert_eq!(parse_eir(&[9, 0x09, b'x']), Eir::default(), "a length past the end stops the parse");
    }

    #[test]
    fn the_store_round_trips_and_skips_bad_lines() {
        let k = LinkKey { addr: A.into(), kind: 0, key_type: 4, key: [0xAB; 16], pin_len: 0, name: "Buds, Pro".into(), audio: true };
        let text = encode_store(true, std::slice::from_ref(&k));
        assert_eq!(parse_store(&text), (true, vec![k]));
        let (p, keys) = parse_store("power=0\nkey=garbage\nkey=AA:BB:CC:DD:EE:01,0,4,short,0,1,x\n");
        assert!(!p && keys.is_empty());
    }

    #[test]
    fn enabling_loads_keys_configures_and_powers_on_last() {
        let k = LinkKey { addr: A.into(), kind: 0, key_type: 4, key: [1; 16], pin_len: 0, name: "Buds".into(), audio: true };
        let mut c = Core::new(true, vec![k]);
        c.set_available(true, false);
        let out = c.startup();
        assert_eq!(out[0], Out::Unblock);
        let Out::Mgmt(OP_LOAD_LINK_KEYS, keys) = &out[1] else { panic!("keys first: {out:?}") };
        assert_eq!(keys.len(), 3 + 25);
        assert_eq!(&keys[1..3], &[1, 0], "one key");
        assert_eq!(out[out.len() - 2], Out::Mgmt(OP_SET_POWERED, vec![1]));
        assert_eq!(out.last(), Some(&Out::Save));
        assert!(c.status.on() && c.power());
        assert!(c.status.device(A).unwrap().paired, "stored bonds list as paired");
    }

    #[test]
    fn disabling_drops_links_and_forgets_what_was_found() {
        let mut c = on_core();
        c.event(&found("AA:BB:CC:DD:EE:02", "Nearby", 0, 0));
        c.event(&Packet { code: EV_NEW_LINK_KEY, index: 0, data: [&[1u8][..], &parse_addr(A).unwrap(), &[0, 4], &[9; 16], &[0]].concat() });
        c.event(&Packet { code: EV_DEVICE_CONNECTED, index: 0, data: [&parse_addr(A).unwrap()[..], &[0], &0u32.to_le_bytes(), &0u16.to_le_bytes()].concat() });
        let out = c.command(Cmd::Enable(false), 5);
        assert!(out.contains(&Out::Unlink(A.into())));
        assert!(out.contains(&Out::Mgmt(OP_SET_POWERED, vec![0])));
        assert_eq!(c.status.state, State::Off);
        assert_eq!(c.status.devices.len(), 1, "only the paired device stays");
        assert!(!c.status.devices[0].connected);
        assert_eq!(c.status.summary(), "Off");
    }

    #[test]
    fn scanning_collects_devices_asks_for_names_and_stops_by_itself() {
        let mut c = on_core();
        let out = c.command(Cmd::Scan, 100);
        assert_eq!(out, vec![Out::Mgmt(OP_START_DISCOVERY, vec![DISCOVER_ALL])]);
        assert!(c.status.scanning);
        assert!(c.command(Cmd::Scan, 101).is_empty(), "already scanning");
        let ask = c.event(&found(A, "", 0x240414, FOUND_CONFIRM_NAME));
        assert_eq!(ask, vec![Out::Mgmt(OP_CONFIRM_NAME, [&parse_addr(A).unwrap()[..], &[0, 0]].concat())]);
        assert!(c.status.device(A).unwrap().audio);
        assert!(c.event(&found(A, "Buds", 0x240414, 0)).is_empty());
        assert_eq!(c.status.device(A).unwrap().name, "Buds");
        assert_eq!(c.status.device(A).unwrap().rssi, Some(-60));
        assert!(c.tick(100 + SCAN_MS - 1).is_empty());
        assert_eq!(c.tick(100 + SCAN_MS), vec![Out::Mgmt(OP_STOP_DISCOVERY, vec![DISCOVER_ALL])]);
        assert!(!c.status.scanning);
    }

    #[test]
    fn pairing_accepts_only_what_we_started() {
        let mut c = on_core();
        c.event(&found(A, "Buds", 0x240414, 0));
        let out = c.command(Cmd::Pair(A.into()), 0);
        assert_eq!(out, vec![Out::Mgmt(OP_PAIR_DEVICE, [&parse_addr(A).unwrap()[..], &[0, IO_NO_INPUT_NO_OUTPUT]].concat())]);
        assert_eq!(c.status.busy.as_deref(), Some(A));
        let ask = |addr: &str| Packet { code: EV_USER_CONFIRM_REQUEST, index: 0, data: [&parse_addr(addr).unwrap()[..], &[0, 0], &1234u32.to_le_bytes()].concat() };
        assert!(matches!(c.event(&ask(A))[0], Out::Mgmt(OP_USER_CONFIRM_REPLY, _)));
        assert!(matches!(c.event(&ask("11:22:33:44:55:66"))[0], Out::Mgmt(OP_USER_CONFIRM_NEG_REPLY, _)));
        // success: the reply carries the address, then the runner connects
        c.event(&cmd_complete(OP_PAIR_DEVICE, 0, &[&parse_addr(A).unwrap()[..], &[0]].concat()));
        assert!(c.status.busy.is_none() && c.status.device(A).unwrap().paired);
        assert!(c.after_pair(A));
        let out = c.command(Cmd::Connect(A.into()), 1);
        assert_eq!(out, vec![Out::Link(A.into(), 0)]);
        c.link_result(A, true);
        assert!(c.status.device(A).unwrap().connected && c.status.busy.is_none());
    }

    #[test]
    fn a_failed_or_slow_pairing_reports_and_frees_the_slot() {
        let mut c = on_core();
        c.event(&found(A, "Buds", 0, 0));
        c.command(Cmd::Pair(A.into()), 0);
        c.event(&cmd_complete(OP_PAIR_DEVICE, 5, &[]));
        assert!(c.status.busy.is_none());
        assert_eq!(c.status.error.as_deref(), Some("Authentication Failed"));
        c.command(Cmd::Pair(A.into()), 10);
        c.tick(10 + PAIR_MS);
        assert!(c.status.busy.is_none());
        assert_eq!(c.status.error.as_deref(), Some("Timed Out"));
    }

    #[test]
    fn a_new_link_key_is_kept_only_when_the_kernel_says_to_store_it() {
        let mut c = on_core();
        let key = |hint: u8| Packet { code: EV_NEW_LINK_KEY, index: 0, data: [&[hint][..], &parse_addr(A).unwrap(), &[0, 4], &[7; 16], &[0]].concat() };
        c.event(&key(0));
        assert!(!c.store().contains("key="));
        assert_eq!(c.event(&key(1)), vec![Out::Save]);
        assert!(c.store().contains(&format!("key={A},0,4,{}", "07".repeat(16))));
    }

    #[test]
    fn forgetting_unpairs_and_drops_the_bond_and_the_device() {
        let mut c = on_core();
        c.event(&Packet { code: EV_NEW_LINK_KEY, index: 0, data: [&[1u8][..], &parse_addr(A).unwrap(), &[0, 4], &[7; 16], &[0]].concat() });
        let out = c.command(Cmd::Forget(A.into()), 0);
        assert_eq!(out[0], Out::Unlink(A.into()));
        assert_eq!(out[1], Out::Mgmt(OP_UNPAIR_DEVICE, [&parse_addr(A).unwrap()[..], &[0, 1]].concat()));
        assert_eq!(out.last(), Some(&Out::Save));
        assert!(c.status.devices.is_empty() && !c.store().contains("key="));
    }

    #[test]
    fn connection_events_drive_the_connected_state_and_the_audio_sink() {
        let mut c = on_core();
        c.event(&found(A, "Buds", 0x240414, 0));
        c.event(&found("AA:BB:CC:DD:EE:02", "Phone", 0x5A020C, 0));
        assert_eq!(c.status.audio_sink(), None);
        let wire = parse_addr(A).unwrap();
        c.event(&Packet { code: EV_DEVICE_CONNECTED, index: 0, data: [&wire[..], &[0], &0u32.to_le_bytes(), &0u16.to_le_bytes()].concat() });
        assert_eq!(c.status.audio_sink(), Some("Buds"));
        assert_eq!(c.status.summary(), "Connected");
        let out = c.event(&Packet { code: EV_DEVICE_DISCONNECTED, index: 0, data: [&wire[..], &[0, 3]].concat() });
        assert_eq!(out, vec![Out::Unlink(A.into())]);
        assert_eq!(c.status.audio_sink(), None);
    }

    #[test]
    fn power_follows_settings_and_a_refused_power_on_shows_why() {
        let mut c = on_core();
        c.event(&Packet { code: EV_NEW_SETTINGS, index: 0, data: 0u32.to_le_bytes().to_vec() });
        assert_eq!(c.status.state, State::Off);
        c.event(&Packet { code: EV_NEW_SETTINGS, index: 0, data: 1u32.to_le_bytes().to_vec() });
        assert_eq!(c.status.state, State::On);
        let mut c = on_core();
        c.event(&cmd_complete(OP_SET_POWERED, 18, &[]));
        assert_eq!(c.status.state, State::Off);
        assert_eq!(c.status.error.as_deref(), Some("Bluetooth Is Blocked"));
    }

    #[test]
    fn without_a_controller_only_the_wish_is_remembered() {
        let mut c = Core::new(false, vec![]);
        assert_eq!(c.status.state, State::Unavailable);
        assert_eq!(c.command(Cmd::Enable(true), 0), vec![Out::Save]);
        assert!(c.power());
        assert!(c.command(Cmd::Scan, 0).is_empty());
        // the controller appears later: the wish is carried out
        let out = c.event(&Packet { code: EV_INDEX_ADDED, index: 0, data: vec![] });
        assert!(out.contains(&Out::Mgmt(OP_SET_POWERED, vec![1])));
    }

    #[test]
    fn get_connections_marks_devices_connected() {
        let mut c = on_core();
        let wire = parse_addr(A).unwrap();
        c.event(&cmd_complete(OP_GET_CONNECTIONS, 0, &[&1u16.to_le_bytes()[..], &wire, &[0]].concat()));
        assert!(c.status.device(A).unwrap().connected);
    }
}
