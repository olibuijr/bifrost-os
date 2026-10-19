//! Smart cards for apps: the PC/SC API (readers, card state, connections, APDU exchange).
//!
//! PC/SC is the standard card-reader interface, and `pcsc-lite` (the `pcscd` daemon and `libpcsclite`) is its Linux
//! implementation; the installer enables `pcscd.socket`. This module loads `libpcsclite.so.1` when it is first needed and
//! calls the same functions the reference API names: establish a context, list readers, read reader states, connect to a
//! card, transmit command APDUs, disconnect. The shell links nothing: a machine without the library gets
//! `PcscError::LibraryMissing`. The parts that need no reader are pure and tested: the error codes, the reader state flags and
//! their mapping onto `smartcard_slot::SlotState`, the multi-string reader list, and the APDU helpers.
//!
//! Card data never leaves the machine through this module. A sandboxed app sees the daemon's socket only when its sandbox
//! binds it.

use bifrost_ui::smartcard_slot::{SlotEvent, SlotManager, SlotState};
use springboard::media::dl::Lib;
use std::ffi::{c_char, c_long, c_ulong, c_void, CString};
use std::fmt;
use std::ptr;

/// Longest answer to reset.
pub const MAX_ATR: usize = 33;
/// Room for the longest extended-length response and its status word.
const MAX_RESPONSE: usize = 65_538;

// --- errors ------------------------------------------------------------------------------------------------------

/// Why a PC/SC call failed. The variants follow the standard `SCARD_*` result codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcscError {
    InternalError,
    Cancelled,
    InvalidHandle,
    InvalidParameter,
    InsufficientBuffer,
    UnknownReader,
    Timeout,
    SharingViolation,
    NoSmartcard,
    UnknownCard,
    ProtocolMismatch,
    NotReady,
    CommError,
    ReaderUnavailable,
    NoService,
    ServiceStopped,
    NoReadersAvailable,
    UnsupportedCard,
    UnresponsiveCard,
    UnpoweredCard,
    ResetCard,
    RemovedCard,
    /// The library is not installed.
    LibraryMissing,
    /// A code this module does not name.
    Other(u32),
}

impl PcscError {
    /// The error for a result code (the low 32 bits). Zero is success and is not an error.
    pub fn from_code(code: u32) -> Option<PcscError> {
        Some(match code {
            0 => return None,
            0x8010_0001 => PcscError::InternalError,
            0x8010_0002 | 0x8010_0012 => PcscError::Cancelled,
            0x8010_0003 => PcscError::InvalidHandle,
            0x8010_0004 | 0x8010_0011 => PcscError::InvalidParameter,
            0x8010_0008 => PcscError::InsufficientBuffer,
            0x8010_0009 => PcscError::UnknownReader,
            0x8010_000A => PcscError::Timeout,
            0x8010_000B => PcscError::SharingViolation,
            0x8010_000C => PcscError::NoSmartcard,
            0x8010_000D => PcscError::UnknownCard,
            0x8010_000F => PcscError::ProtocolMismatch,
            0x8010_0010 => PcscError::NotReady,
            0x8010_0013 => PcscError::CommError,
            0x8010_0017 => PcscError::ReaderUnavailable,
            0x8010_001D => PcscError::NoService,
            0x8010_001E => PcscError::ServiceStopped,
            0x8010_002E => PcscError::NoReadersAvailable,
            0x8010_0065 => PcscError::UnsupportedCard,
            0x8010_0066 => PcscError::UnresponsiveCard,
            0x8010_0067 => PcscError::UnpoweredCard,
            0x8010_0068 => PcscError::ResetCard,
            0x8010_0069 => PcscError::RemovedCard,
            other => PcscError::Other(other),
        })
    }
}

impl fmt::Display for PcscError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            PcscError::InternalError => "internal error in the card service",
            PcscError::Cancelled => "the operation was cancelled",
            PcscError::InvalidHandle => "the card or context handle is not valid",
            PcscError::InvalidParameter => "a parameter is not valid",
            PcscError::InsufficientBuffer => "the buffer is too small",
            PcscError::UnknownReader => "no such reader",
            PcscError::Timeout => "the operation timed out",
            PcscError::SharingViolation => "another application is using the card",
            PcscError::NoSmartcard => "there is no card in the reader",
            PcscError::UnknownCard => "the card is not recognised",
            PcscError::ProtocolMismatch => "the card does not speak the requested protocol",
            PcscError::NotReady => "the reader is not ready",
            PcscError::CommError => "communication with the reader failed",
            PcscError::ReaderUnavailable => "the reader is not available",
            PcscError::NoService => "the card service is not running",
            PcscError::ServiceStopped => "the card service stopped",
            PcscError::NoReadersAvailable => "no card readers are connected",
            PcscError::UnsupportedCard => "the card is not supported",
            PcscError::UnresponsiveCard => "the card does not answer",
            PcscError::UnpoweredCard => "the card has no power",
            PcscError::ResetCard => "the card was reset",
            PcscError::RemovedCard => "the card was removed",
            PcscError::LibraryMissing => "the card library is not installed",
            PcscError::Other(code) => return write!(f, "card service error {code:#010x}"),
        };
        f.write_str(text)
    }
}

impl std::error::Error for PcscError {}

fn check(rc: c_long) -> Result<(), PcscError> {
    match PcscError::from_code(rc as u32) {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

// --- constants and pure helpers -----------------------------------------------------------------------------------

/// Reader state flags (`SCARD_STATE_*`).
pub mod state {
    pub const IGNORE: u32 = 0x0001;
    pub const CHANGED: u32 = 0x0002;
    pub const UNKNOWN: u32 = 0x0004;
    pub const UNAVAILABLE: u32 = 0x0008;
    pub const EMPTY: u32 = 0x0010;
    pub const PRESENT: u32 = 0x0020;
    pub const ATR_MATCH: u32 = 0x0040;
    pub const EXCLUSIVE: u32 = 0x0080;
    pub const IN_USE: u32 = 0x0100;
    pub const MUTE: u32 = 0x0200;
    pub const UNPOWERED: u32 = 0x0400;
}

/// How the card is shared with other applications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareMode {
    Exclusive,
    Shared,
}

impl ShareMode {
    fn code(self) -> c_ulong {
        match self {
            ShareMode::Exclusive => 1,
            ShareMode::Shared => 2,
        }
    }
}

/// A transmission protocol of the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    T0,
    T1,
}

impl Protocol {
    fn code(self) -> c_ulong {
        match self {
            Protocol::T0 => 1,
            Protocol::T1 => 2,
        }
    }

    fn from_code(code: c_ulong) -> Option<Protocol> {
        match code {
            1 => Some(Protocol::T0),
            2 => Some(Protocol::T1),
            _ => None,
        }
    }
}

/// What to do with the card when disconnecting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Leave,
    Reset,
    Unpower,
    Eject,
}

impl Disposition {
    fn code(self) -> c_ulong {
        match self {
            Disposition::Leave => 0,
            Disposition::Reset => 1,
            Disposition::Unpower => 2,
            Disposition::Eject => 3,
        }
    }
}

/// The reader names in a multi-string: names separated by NUL and ended by an empty one.
pub fn parse_multi_string(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|&b| b == 0)
        .take_while(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// A reader and what is in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderStatus {
    pub name: String,
    /// The event state flags (see [`state`]).
    pub flags: u32,
    /// The answer to reset of the card, empty when there is none.
    pub atr: Vec<u8>,
}

impl ReaderStatus {
    /// The slot state these flags describe. A reader the service does not know or cannot reach is `Missing`; a card
    /// that is present but silent or unpowered is `MuteCard`; a present card with an answer to reset is `ValidCard`; a
    /// present card whose answer has not arrived yet is `Probing`.
    pub fn slot_state(&self) -> SlotState {
        let f = self.flags;
        if f & (state::UNKNOWN | state::UNAVAILABLE | state::IGNORE) != 0 {
            SlotState::Missing
        } else if f & state::PRESENT != 0 {
            if f & (state::MUTE | state::UNPOWERED) != 0 {
                SlotState::MuteCard
            } else if self.atr.is_empty() {
                SlotState::Probing
            } else {
                SlotState::ValidCard
            }
        } else {
            SlotState::Empty
        }
    }

    /// Another application holds the card exclusively or is using it.
    pub fn in_use(&self) -> bool {
        self.flags & (state::EXCLUSIVE | state::IN_USE) != 0
    }
}

/// Feeds reader states to a slot manager, adding the slots it has not seen. Returns the insert and remove events.
pub fn apply_to_slots(manager: &mut SlotManager, statuses: &[ReaderStatus]) -> Vec<(String, SlotEvent)> {
    let mut events = Vec::new();
    for status in statuses {
        if manager.slot(&status.name).is_none() && manager.add_slot(&status.name).is_err() {
            continue;
        }
        let atr = (status.slot_state() == SlotState::ValidCard).then(|| status.atr.clone());
        if let Ok(Some(event)) = manager.report(&status.name, status.slot_state(), atr) {
            events.push((status.name.clone(), event));
        }
    }
    events
}

/// The command APDU that selects an application by its identifier (`SELECT` by name, first or only occurrence).
pub fn select_aid(aid: &[u8]) -> Option<Vec<u8>> {
    if aid.is_empty() || aid.len() > 16 {
        return None;
    }
    let mut apdu = vec![0x00, 0xA4, 0x04, 0x00, aid.len() as u8];
    apdu.extend_from_slice(aid);
    Some(apdu)
}

/// A response APDU: the data and the two status bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub data: Vec<u8>,
    pub sw1: u8,
    pub sw2: u8,
}

impl Response {
    /// Splits a response into data and status word. At least the two status bytes must be there.
    pub fn parse(bytes: &[u8]) -> Option<Response> {
        let (data, sw) = bytes.split_at(bytes.len().checked_sub(2)?);
        Some(Response { data: data.to_vec(), sw1: sw[0], sw2: sw[1] })
    }

    /// The status word as one number (`0x9000` is success).
    pub fn status(&self) -> u16 {
        u16::from(self.sw1) << 8 | u16::from(self.sw2)
    }

    pub fn is_success(&self) -> bool {
        self.status() == 0x9000
    }

    /// More data is waiting: `61 xx` says `xx` bytes can be read with GET RESPONSE.
    pub fn bytes_available(&self) -> Option<u8> {
        (self.sw1 == 0x61).then_some(self.sw2)
    }
}

// --- the library -------------------------------------------------------------------------------------------------

#[repr(C)]
struct IoRequest {
    protocol: c_ulong,
    pci_length: c_ulong,
}

#[repr(C)]
struct ReaderState {
    reader: *const c_char,
    user_data: *mut c_void,
    current: c_ulong,
    event: c_ulong,
    atr_len: c_ulong,
    atr: [u8; MAX_ATR],
}

type EstablishFn = unsafe extern "C" fn(c_ulong, *const c_void, *const c_void, *mut c_long) -> c_long;
type ReleaseFn = unsafe extern "C" fn(c_long) -> c_long;
type ListReadersFn = unsafe extern "C" fn(c_long, *const c_char, *mut c_char, *mut c_ulong) -> c_long;
type StatusChangeFn = unsafe extern "C" fn(c_long, c_ulong, *mut ReaderState, c_ulong) -> c_long;
type ConnectFn = unsafe extern "C" fn(c_long, *const c_char, c_ulong, c_ulong, *mut c_long, *mut c_ulong) -> c_long;
type DisconnectFn = unsafe extern "C" fn(c_long, c_ulong) -> c_long;
type TransmitFn =
    unsafe extern "C" fn(c_long, *const IoRequest, *const u8, c_ulong, *mut IoRequest, *mut u8, *mut c_ulong) -> c_long;

#[derive(Clone, Copy)]
struct Api {
    establish: EstablishFn,
    release: ReleaseFn,
    list_readers: ListReadersFn,
    status_change: StatusChangeFn,
    connect: ConnectFn,
    disconnect: DisconnectFn,
    transmit: TransmitFn,
}

impl Api {
    fn load() -> Result<Api, PcscError> {
        let lib = Lib::open(&["libpcsclite.so.1", "libpcsclite.so"]).map_err(|_| PcscError::LibraryMissing)?;
        // SAFETY: each type is the `extern "C"` prototype of the function of that name in pcsclite.h on Linux, where
        // LONG is `long` and DWORD is `unsigned long`.
        unsafe {
            let sym = |e: std::io::Error| {
                let _ = e;
                PcscError::LibraryMissing
            };
            Ok(Api {
                establish: lib.sym("SCardEstablishContext").map_err(sym)?,
                release: lib.sym("SCardReleaseContext").map_err(sym)?,
                list_readers: lib.sym("SCardListReaders").map_err(sym)?,
                status_change: lib.sym("SCardGetStatusChange").map_err(sym)?,
                connect: lib.sym("SCardConnect").map_err(sym)?,
                disconnect: lib.sym("SCardDisconnect").map_err(sym)?,
                transmit: lib.sym("SCardTransmit").map_err(sym)?,
            })
        }
    }
}

/// A connection to the card service (`SCARDCONTEXT`). Dropping it releases the context.
pub struct Context {
    api: Api,
    handle: c_long,
}

impl Context {
    /// Connects to the card service in the system scope.
    pub fn establish() -> Result<Context, PcscError> {
        let api = Api::load()?;
        let mut handle: c_long = 0;
        // SAFETY: `handle` is a live out-parameter and the reserved arguments are null, as the API requires.
        check(unsafe { (api.establish)(2, ptr::null(), ptr::null(), &mut handle) })?;
        Ok(Context { api, handle })
    }

    /// The names of the connected readers. No readers is an empty list, not an error.
    pub fn readers(&self) -> Result<Vec<String>, PcscError> {
        let mut len: c_ulong = 0;
        // SAFETY: a null buffer asks for the needed length, which is written to `len`.
        let rc = unsafe { (self.api.list_readers)(self.handle, ptr::null(), ptr::null_mut(), &mut len) };
        if PcscError::from_code(rc as u32) == Some(PcscError::NoReadersAvailable) {
            return Ok(Vec::new());
        }
        check(rc)?;
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` holds `len` bytes, which is what the call may write.
        let rc = unsafe { (self.api.list_readers)(self.handle, ptr::null(), buf.as_mut_ptr() as *mut c_char, &mut len) };
        check(rc)?;
        buf.truncate(len as usize);
        Ok(parse_multi_string(&buf))
    }

    /// The state of each named reader now (it does not wait for a change).
    pub fn statuses(&self, readers: &[String]) -> Result<Vec<ReaderStatus>, PcscError> {
        if readers.is_empty() {
            return Ok(Vec::new());
        }
        let names: Vec<CString> =
            readers.iter().map(|r| CString::new(r.as_str()).map_err(|_| PcscError::InvalidParameter)).collect::<Result<_, _>>()?;
        let mut states: Vec<ReaderState> = names
            .iter()
            .map(|n| ReaderState {
                reader: n.as_ptr(),
                user_data: ptr::null_mut(),
                current: 0,
                event: 0,
                atr_len: 0,
                atr: [0; MAX_ATR],
            })
            .collect();
        // SAFETY: `states` holds `readers.len()` structures laid out as SCARD_READERSTATE, whose names stay alive in
        // `names` for the call. A zero timeout returns at once.
        check(unsafe { (self.api.status_change)(self.handle, 0, states.as_mut_ptr(), states.len() as c_ulong) })?;
        Ok(states
            .iter()
            .zip(readers)
            .map(|(s, name)| ReaderStatus {
                name: name.clone(),
                flags: (s.event as u32) & 0xffff,
                atr: s.atr[..(s.atr_len as usize).min(MAX_ATR)].to_vec(),
            })
            .collect())
    }

    /// The state of every connected reader.
    pub fn all_statuses(&self) -> Result<Vec<ReaderStatus>, PcscError> {
        let readers = self.readers()?;
        self.statuses(&readers)
    }

    /// Connects to the card in `reader`. The protocol the card agrees on is the first of `protocols` it supports.
    pub fn connect(&self, reader: &str, share: ShareMode, protocols: &[Protocol]) -> Result<Card, PcscError> {
        let name = CString::new(reader).map_err(|_| PcscError::InvalidParameter)?;
        let wanted = protocols.iter().fold(0, |acc, p| acc | p.code());
        let mut card: c_long = 0;
        let mut active: c_ulong = 0;
        // SAFETY: `name` is NUL terminated and `card` and `active` are live out-parameters.
        check(unsafe { (self.api.connect)(self.handle, name.as_ptr(), share.code(), wanted, &mut card, &mut active) })?;
        Ok(Card { api: self.api, handle: card, protocol: Protocol::from_code(active), connected: true })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by establish and is released once.
        let _ = unsafe { (self.api.release)(self.handle) };
    }
}

/// A connected card (`SCARDHANDLE`). Dropping it disconnects and leaves the card as it is.
pub struct Card {
    api: Api,
    handle: c_long,
    protocol: Option<Protocol>,
    connected: bool,
}

impl Card {
    /// The protocol in use, if it is T=0 or T=1.
    pub fn protocol(&self) -> Option<Protocol> {
        self.protocol
    }

    /// Sends a command APDU and returns the response bytes (data and status word).
    pub fn transmit(&self, apdu: &[u8]) -> Result<Vec<u8>, PcscError> {
        let protocol = self.protocol.ok_or(PcscError::ProtocolMismatch)?;
        let send_pci = IoRequest { protocol: protocol.code(), pci_length: std::mem::size_of::<IoRequest>() as c_ulong };
        let mut recv = vec![0u8; MAX_RESPONSE];
        let mut recv_len = recv.len() as c_ulong;
        // SAFETY: the request structure and buffers are live for the call, and `recv_len` is the size of `recv`.
        check(unsafe {
            (self.api.transmit)(
                self.handle,
                &send_pci,
                apdu.as_ptr(),
                apdu.len() as c_ulong,
                ptr::null_mut(),
                recv.as_mut_ptr(),
                &mut recv_len,
            )
        })?;
        recv.truncate(recv_len as usize);
        Ok(recv)
    }

    /// Sends a command APDU and splits the response.
    pub fn exchange(&self, apdu: &[u8]) -> Result<Response, PcscError> {
        Response::parse(&self.transmit(apdu)?).ok_or(PcscError::CommError)
    }

    /// Disconnects, doing `disposition` to the card.
    pub fn disconnect(mut self, disposition: Disposition) -> Result<(), PcscError> {
        self.connected = false;
        // SAFETY: the handle is connected and is disconnected once.
        check(unsafe { (self.api.disconnect)(self.handle, disposition.code()) })
    }
}

impl Drop for Card {
    fn drop(&mut self) {
        if self.connected {
            // SAFETY: the handle is still connected.
            let _ = unsafe { (self.api.disconnect)(self.handle, Disposition::Leave.code()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(flags: u32, atr: &[u8]) -> ReaderStatus {
        ReaderStatus { name: "Reader 0".into(), flags, atr: atr.to_vec() }
    }

    #[test]
    fn result_codes_map_to_errors() {
        assert_eq!(PcscError::from_code(0), None);
        assert_eq!(PcscError::from_code(0x8010_000C), Some(PcscError::NoSmartcard));
        assert_eq!(PcscError::from_code(0x8010_0009), Some(PcscError::UnknownReader));
        assert_eq!(PcscError::from_code(0x8010_001D), Some(PcscError::NoService));
        assert_eq!(PcscError::from_code(0x8010_002E), Some(PcscError::NoReadersAvailable));
        assert_eq!(PcscError::from_code(0x8010_0069), Some(PcscError::RemovedCard));
        assert_eq!(PcscError::from_code(0x1234), Some(PcscError::Other(0x1234)));
        assert_eq!(PcscError::NoSmartcard.to_string(), "there is no card in the reader");
        assert_eq!(PcscError::Other(0x1234).to_string(), "card service error 0x00001234");
    }

    #[test]
    fn the_result_is_read_from_the_low_32_bits_of_a_long() {
        assert_eq!(check(0), Ok(()));
        assert_eq!(check(0x8010_000C), Err(PcscError::NoSmartcard));
        assert_eq!(check((0x8010_000Cu32 as i32) as c_long), Err(PcscError::NoSmartcard));
    }

    #[test]
    fn a_multi_string_splits_into_reader_names() {
        assert_eq!(parse_multi_string(b"Reader A\0Reader B\0\0"), vec!["Reader A", "Reader B"]);
        assert_eq!(parse_multi_string(b"\0\0"), Vec::<String>::new());
        assert_eq!(parse_multi_string(b"Only\0"), vec!["Only"]);
        assert_eq!(parse_multi_string(b""), Vec::<String>::new());
    }

    #[test]
    fn reader_flags_map_to_slot_states() {
        assert_eq!(status(state::EMPTY, &[]).slot_state(), SlotState::Empty);
        assert_eq!(status(state::PRESENT, &[0x3b, 0x00]).slot_state(), SlotState::ValidCard);
        assert_eq!(status(state::PRESENT, &[]).slot_state(), SlotState::Probing);
        assert_eq!(status(state::PRESENT | state::MUTE, &[]).slot_state(), SlotState::MuteCard);
        assert_eq!(status(state::PRESENT | state::UNPOWERED, &[0x3b]).slot_state(), SlotState::MuteCard);
        assert_eq!(status(state::UNKNOWN, &[]).slot_state(), SlotState::Missing);
        assert_eq!(status(state::UNAVAILABLE | state::PRESENT, &[1]).slot_state(), SlotState::Missing);
        assert!(status(state::PRESENT | state::IN_USE, &[1]).in_use());
        assert!(!status(state::PRESENT, &[1]).in_use());
    }

    #[test]
    fn reader_states_drive_the_slot_manager() {
        let mut manager = SlotManager::new();
        let events = apply_to_slots(&mut manager, &[status(state::EMPTY, &[])]);
        assert!(events.is_empty());
        assert_eq!(manager.slot_names(), vec!["Reader 0"]);
        let events = apply_to_slots(&mut manager, &[status(state::PRESENT, &[0x3b, 0x8f])]);
        assert_eq!(events, vec![("Reader 0".to_string(), SlotEvent::CardInserted)]);
        assert_eq!(manager.slot("Reader 0").unwrap().atr(), Some(&[0x3b, 0x8f][..]));
        let events = apply_to_slots(&mut manager, &[status(state::EMPTY, &[])]);
        assert_eq!(events, vec![("Reader 0".to_string(), SlotEvent::CardRemoved)]);
    }

    #[test]
    fn select_builds_the_apdu_and_refuses_odd_identifiers() {
        assert_eq!(select_aid(&[0xA0, 0x00, 0x00, 0x00, 0x03]).unwrap(), vec![0x00, 0xA4, 0x04, 0x00, 5, 0xA0, 0, 0, 0, 3]);
        assert_eq!(select_aid(&[]), None);
        assert_eq!(select_aid(&[0; 17]), None);
    }

    #[test]
    fn a_response_splits_into_data_and_status() {
        let r = Response::parse(&[1, 2, 3, 0x90, 0x00]).unwrap();
        assert_eq!(r.data, vec![1, 2, 3]);
        assert!(r.is_success());
        assert_eq!(r.status(), 0x9000);
        let more = Response::parse(&[0x61, 0x20]).unwrap();
        assert!(more.data.is_empty() && !more.is_success());
        assert_eq!(more.bytes_available(), Some(0x20));
        assert_eq!(Response::parse(&[0x90]), None);
        assert_eq!(Response::parse(&[]), None);
        assert_eq!(Response::parse(&[0x6a, 0x82]).unwrap().bytes_available(), None);
    }

    #[test]
    fn protocol_and_share_codes_follow_the_header() {
        assert_eq!((Protocol::T0.code(), Protocol::T1.code()), (1, 2));
        assert_eq!(Protocol::from_code(2), Some(Protocol::T1));
        assert_eq!(Protocol::from_code(4), None);
        assert_eq!((ShareMode::Exclusive.code(), ShareMode::Shared.code()), (1, 2));
        assert_eq!(Disposition::Eject.code(), 3);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_structures_have_the_c_layout_on_64_bit() {
        assert_eq!(std::mem::size_of::<IoRequest>(), 16);
        // pointer, pointer, three unsigned longs (40 bytes), 33 bytes of answer to reset: 73, padded to 8
        assert_eq!(std::mem::size_of::<ReaderState>(), 80);
    }

    #[test]
    fn connecting_to_the_service_fails_cleanly_or_lists_readers() {
        // without the library or the daemon this is an error; with both it is a list. It must not panic either way.
        match Context::establish() {
            Err(e) => assert!(matches!(e, PcscError::LibraryMissing | PcscError::NoService | PcscError::ServiceStopped | PcscError::Other(_) | PcscError::CommError | PcscError::InternalError), "{e}"),
            Ok(context) => {
                let _ = context.all_statuses();
            }
        }
    }
}
