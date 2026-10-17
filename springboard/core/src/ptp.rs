//! The Picture Transfer Protocol (ISO 15740, the open standard cameras and phones speak over USB): the container
//! codec, the datasets a camera describes itself and its files with, and a session that runs operations over a
//! transport. This is the camera-side transport the device browser and the camera model (`bifrost_ui::camera_device`)
//! wait for: it lists a camera's storage, reads and deletes files, takes a picture and reads the battery and clock.
//!
//! The codec and the datasets are pure and tested against a scripted camera. [`UsbTransport`] carries the same
//! containers over the bulk endpoints of a still-image-class USB interface through `usb_host`.
//!
//! Wire format: every message is a container, little endian: length (u32, header included), type (u16: 1 command,
//! 2 data, 3 response, 4 event), code (u16), transaction id (u32), then the payload. A command carries up to five u32
//! parameters; a data container carries the dataset or file bytes; a response carries a code and up to five
//! parameters. A string is a character count (u8, terminator included) and then UTF-16 code units.

use crate::usb_descriptors::{ConfigurationDescriptor, TransferType};
use crate::usb_host::UsbHostDevice;
use std::fmt;
use std::io;

/// Bytes in a container header.
pub const HEADER_LEN: usize = 12;
/// The largest container this session accepts (a camera's biggest still or short clip).
pub const MAX_CONTAINER: usize = 512 * 1024 * 1024;
/// Bytes asked of the transport per read.
const CHUNK: usize = 64 * 1024;

/// Operation codes.
pub mod op {
    pub const GET_DEVICE_INFO: u16 = 0x1001;
    pub const OPEN_SESSION: u16 = 0x1002;
    pub const CLOSE_SESSION: u16 = 0x1003;
    pub const GET_STORAGE_IDS: u16 = 0x1004;
    pub const GET_OBJECT_HANDLES: u16 = 0x1007;
    pub const GET_OBJECT_INFO: u16 = 0x1008;
    pub const GET_OBJECT: u16 = 0x1009;
    pub const GET_THUMB: u16 = 0x100A;
    pub const DELETE_OBJECT: u16 = 0x100B;
    pub const SEND_OBJECT_INFO: u16 = 0x100C;
    pub const SEND_OBJECT: u16 = 0x100D;
    pub const INITIATE_CAPTURE: u16 = 0x100E;
    pub const GET_DEVICE_PROP_VALUE: u16 = 0x1015;
    pub const SET_DEVICE_PROP_VALUE: u16 = 0x1016;
}

/// Response codes.
pub mod response {
    pub const OK: u16 = 0x2001;
    pub const GENERAL_ERROR: u16 = 0x2002;
    pub const SESSION_NOT_OPEN: u16 = 0x2003;
    pub const OPERATION_NOT_SUPPORTED: u16 = 0x2005;
    pub const INVALID_STORAGE_ID: u16 = 0x2008;
    pub const INVALID_OBJECT_HANDLE: u16 = 0x2009;
    pub const STORE_FULL: u16 = 0x200C;
    pub const OBJECT_WRITE_PROTECTED: u16 = 0x200D;
    pub const STORE_READ_ONLY: u16 = 0x200E;
    pub const ACCESS_DENIED: u16 = 0x200F;
    pub const STORE_NOT_AVAILABLE: u16 = 0x2013;
    pub const SESSION_ALREADY_OPEN: u16 = 0x201E;
}

/// Device property codes.
pub mod prop {
    pub const BATTERY_LEVEL: u16 = 0x5001;
    pub const DATE_TIME: u16 = 0x5011;
}

/// Object format codes that matter for sorting files into media kinds.
pub mod format {
    pub const UNDEFINED: u16 = 0x3000;
    pub const ASSOCIATION: u16 = 0x3001;
    pub const AIFF: u16 = 0x3007;
    pub const WAV: u16 = 0x3008;
    pub const MP3: u16 = 0x3009;
    pub const AVI: u16 = 0x300A;
    pub const MPEG: u16 = 0x300B;
    pub const ASF: u16 = 0x300C;
    pub const EXIF_JPEG: u16 = 0x3801;
    pub const TIFF_EP: u16 = 0x3802;
    pub const BMP: u16 = 0x3804;
    pub const GIF: u16 = 0x3807;
    pub const JFIF: u16 = 0x3808;
    pub const PNG: u16 = 0x380B;
    pub const TIFF: u16 = 0x380D;
    pub const JP2: u16 = 0x380F;
    pub const DNG: u16 = 0x3811;
    pub const HEIF: u16 = 0x3812;
    pub const MP4: u16 = 0xB982;
    pub const THREE_GP: u16 = 0xB984;
}

/// What failed.
#[derive(Debug)]
pub enum PtpError {
    Io(io::Error),
    /// The bytes do not form a valid container or dataset. Carries what was wrong.
    Malformed(&'static str),
    /// The camera answered with a response code other than OK.
    Response(u16),
    /// The reply belongs to another transaction.
    Mismatch,
}

impl fmt::Display for PtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PtpError::Io(e) => write!(f, "camera transport: {e}"),
            PtpError::Malformed(what) => write!(f, "camera sent a malformed {what}"),
            PtpError::Response(code) => write!(f, "camera refused: {} ({code:#06x})", response_name(*code)),
            PtpError::Mismatch => f.write_str("camera answered a different transaction"),
        }
    }
}

impl std::error::Error for PtpError {}

impl From<io::Error> for PtpError {
    fn from(e: io::Error) -> PtpError {
        PtpError::Io(e)
    }
}

/// The standard name of a response code.
pub fn response_name(code: u16) -> &'static str {
    match code {
        response::OK => "OK",
        response::GENERAL_ERROR => "general error",
        response::SESSION_NOT_OPEN => "session not open",
        response::OPERATION_NOT_SUPPORTED => "operation not supported",
        response::INVALID_STORAGE_ID => "invalid storage",
        response::INVALID_OBJECT_HANDLE => "invalid object",
        response::STORE_FULL => "storage full",
        response::OBJECT_WRITE_PROTECTED => "object write protected",
        response::STORE_READ_ONLY => "storage read only",
        response::ACCESS_DENIED => "access denied",
        response::STORE_NOT_AVAILABLE => "storage not available",
        response::SESSION_ALREADY_OPEN => "session already open",
        _ => "unknown response",
    }
}

/// Whether a response code means the file or card is locked against changes.
pub fn is_write_protection(code: u16) -> bool {
    matches!(code, response::OBJECT_WRITE_PROTECTED | response::STORE_READ_ONLY | response::ACCESS_DENIED)
}

/// The type field of a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerType {
    Command,
    Data,
    Response,
    Event,
}

impl ContainerType {
    fn code(self) -> u16 {
        match self {
            ContainerType::Command => 1,
            ContainerType::Data => 2,
            ContainerType::Response => 3,
            ContainerType::Event => 4,
        }
    }

    fn from_code(code: u16) -> Option<ContainerType> {
        match code {
            1 => Some(ContainerType::Command),
            2 => Some(ContainerType::Data),
            3 => Some(ContainerType::Response),
            4 => Some(ContainerType::Event),
            _ => None,
        }
    }
}

/// One message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub kind: ContainerType,
    /// The operation (command, data), response or event code.
    pub code: u16,
    pub transaction: u32,
    pub payload: Vec<u8>,
}

impl Container {
    /// The container in wire order.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&((HEADER_LEN + self.payload.len()) as u32).to_le_bytes());
        out.extend_from_slice(&self.kind.code().to_le_bytes());
        out.extend_from_slice(&self.code.to_le_bytes());
        out.extend_from_slice(&self.transaction.to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// The length a container declares in its first four bytes.
    pub fn declared_length(bytes: &[u8]) -> Option<usize> {
        let head: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
        Some(u32::from_le_bytes(head) as usize)
    }

    /// Reads one whole container. The bytes must be exactly as long as the header declares.
    pub fn decode(bytes: &[u8]) -> Result<Container, PtpError> {
        if bytes.len() < HEADER_LEN {
            return Err(PtpError::Malformed("container header"));
        }
        let declared = Container::declared_length(bytes).unwrap_or(0);
        if declared != bytes.len() {
            return Err(PtpError::Malformed("container length"));
        }
        let kind = ContainerType::from_code(u16::from_le_bytes([bytes[4], bytes[5]])).ok_or(PtpError::Malformed("container type"))?;
        Ok(Container {
            kind,
            code: u16::from_le_bytes([bytes[6], bytes[7]]),
            transaction: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            payload: bytes[HEADER_LEN..].to_vec(),
        })
    }

    /// The payload as little-endian u32 parameters (a command's or response's parameters).
    pub fn params(&self) -> Vec<u32> {
        self.payload.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }
}

/// A command container for `code` with up to five parameters.
pub fn command(code: u16, transaction: u32, params: &[u32]) -> Container {
    let payload = params.iter().take(5).flat_map(|p| p.to_le_bytes()).collect();
    Container { kind: ContainerType::Command, code, transaction, payload }
}

/// A data container, as the host sends it with an operation that carries a dataset.
pub fn data(code: u16, transaction: u32, payload: &[u8]) -> Container {
    Container { kind: ContainerType::Data, code, transaction, payload: payload.to_vec() }
}

// --- datasets ----------------------------------------------------------------------------------------------------

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, at: 0 }
    }

    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], PtpError> {
        let end = self.at.checked_add(n).ok_or(PtpError::Malformed(what))?;
        let slice = self.bytes.get(self.at..end).ok_or(PtpError::Malformed(what))?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, PtpError> {
        Ok(self.take(1, "dataset")?[0])
    }

    fn u16(&mut self) -> Result<u16, PtpError> {
        let b = self.take(2, "dataset")?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, PtpError> {
        let b = self.take(4, "dataset")?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A PTP string: a count of UTF-16 units including the terminator, then the units. Zero is the empty string.
    fn string(&mut self) -> Result<String, PtpError> {
        let count = usize::from(self.u8()?);
        if count == 0 {
            return Ok(String::new());
        }
        let raw = self.take(count * 2, "string")?;
        let mut units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        if units.last() == Some(&0) {
            units.pop();
        }
        Ok(String::from_utf16_lossy(&units))
    }

    fn u16_array(&mut self) -> Result<Vec<u16>, PtpError> {
        let count = self.u32()? as usize;
        if count > self.bytes.len() {
            return Err(PtpError::Malformed("array"));
        }
        (0..count).map(|_| self.u16()).collect()
    }
}

/// A PTP string for the wire. Strings longer than 254 units are cut.
pub fn encode_string(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().take(254).collect();
    if units.is_empty() {
        return vec![0];
    }
    let mut out = vec![(units.len() + 1) as u8];
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// What a camera says about itself (the DeviceInfo dataset).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    pub standard_version: u16,
    pub vendor_extension_id: u32,
    pub functional_mode: u16,
    pub operations: Vec<u16>,
    pub events: Vec<u16>,
    pub properties: Vec<u16>,
    pub capture_formats: Vec<u16>,
    pub image_formats: Vec<u16>,
    pub manufacturer: String,
    pub model: String,
    pub device_version: String,
    pub serial_number: String,
}

impl DeviceInfo {
    pub fn parse(bytes: &[u8]) -> Result<DeviceInfo, PtpError> {
        let mut r = Reader::new(bytes);
        let standard_version = r.u16()?;
        let vendor_extension_id = r.u32()?;
        let _vendor_extension_version = r.u16()?;
        let _vendor_extension_desc = r.string()?;
        let functional_mode = r.u16()?;
        Ok(DeviceInfo {
            standard_version,
            vendor_extension_id,
            functional_mode,
            operations: r.u16_array()?,
            events: r.u16_array()?,
            properties: r.u16_array()?,
            capture_formats: r.u16_array()?,
            image_formats: r.u16_array()?,
            manufacturer: r.string()?,
            model: r.string()?,
            device_version: r.string()?,
            serial_number: r.string()?,
        })
    }

    pub fn supports_operation(&self, code: u16) -> bool {
        self.operations.contains(&code)
    }

    pub fn supports_property(&self, code: u16) -> bool {
        self.properties.contains(&code)
    }
}

/// What a camera says about one object (the ObjectInfo dataset).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectInfo {
    pub storage_id: u32,
    pub format: u16,
    pub protection: u16,
    pub size: u32,
    pub image_width: u32,
    pub image_height: u32,
    /// The handle of the folder holding it; 0 is the top level.
    pub parent: u32,
    pub association_type: u16,
    pub filename: String,
    pub capture_date: String,
}

impl ObjectInfo {
    pub fn parse(bytes: &[u8]) -> Result<ObjectInfo, PtpError> {
        let mut r = Reader::new(bytes);
        let storage_id = r.u32()?;
        let format = r.u16()?;
        let protection = r.u16()?;
        let size = r.u32()?;
        let _thumb_format = r.u16()?;
        let _thumb_size = r.u32()?;
        let _thumb_width = r.u32()?;
        let _thumb_height = r.u32()?;
        let image_width = r.u32()?;
        let image_height = r.u32()?;
        let _bit_depth = r.u32()?;
        let parent = r.u32()?;
        let association_type = r.u16()?;
        let _association_desc = r.u32()?;
        let _sequence = r.u32()?;
        let filename = r.string()?;
        let capture_date = r.string()?;
        Ok(ObjectInfo { storage_id, format, protection, size, image_width, image_height, parent, association_type, filename, capture_date })
    }

    /// A folder (an association other than a generic one still lists as a folder).
    pub fn is_folder(&self) -> bool {
        self.format == format::ASSOCIATION
    }

    /// The write-protection status field: 1 is read only.
    pub fn is_protected(&self) -> bool {
        self.protection == 1
    }
}

/// The kind of media a file holds, from its format code and, when the code says nothing, its file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Raw,
    Movie,
    Audio,
}

/// Sorts a file into a media kind. Vendor RAW formats arrive as "undefined" or vendor codes, so the extension decides.
pub fn media_kind(format_code: u16, filename: &str) -> MediaKind {
    let ext = filename.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    const RAW: [&str; 14] = ["cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "orf", "rw2", "raf", "pef", "srw", "dng"];
    if RAW.contains(&ext.as_str()) || format_code == format::DNG {
        return MediaKind::Raw;
    }
    match format_code {
        format::AIFF | format::WAV | format::MP3 => MediaKind::Audio,
        format::AVI | format::MPEG | format::ASF | format::MP4 | format::THREE_GP => MediaKind::Movie,
        format::EXIF_JPEG | format::TIFF_EP | format::BMP | format::GIF | format::JFIF | format::PNG | format::TIFF
        | format::JP2 | format::HEIF => MediaKind::Image,
        _ => match ext.as_str() {
            "mov" | "mp4" | "m4v" | "avi" | "mts" | "m2ts" | "mpg" | "mpeg" | "3gp" => MediaKind::Movie,
            "wav" | "mp3" | "m4a" | "aif" | "aiff" => MediaKind::Audio,
            _ => MediaKind::Image,
        },
    }
}

// --- dates -------------------------------------------------------------------------------------------------------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Seconds since 1970-01-01 of a PTP date string `YYYYMMDDThhmmss` (a fraction or zone suffix is ignored). The camera's
/// clock has no zone, so the result is its local time read as if it were UTC; only differences between two such values
/// mean anything.
pub fn parse_date(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 15 || b[8] != b'T' || !b[..8].iter().chain(&b[9..15]).all(u8::is_ascii_digit) {
        return None;
    }
    let num = |from: usize, to: usize| text[from..to].parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    let (h, mi, s) = (num(9, 11)?, num(11, 13)?, num(13, 15)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s)
}

/// The PTP date string of a number of seconds since 1970-01-01.
pub fn format_date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

// --- session -----------------------------------------------------------------------------------------------------

/// Whatever carries containers to the camera and back.
pub trait Transport {
    /// Sends bytes to the camera. A container may be sent in one call.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Reads what the camera has sent, at most `max` bytes. An empty answer is a zero-length packet.
    fn read(&mut self, max: usize) -> io::Result<Vec<u8>>;
}

/// What an operation returned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub params: Vec<u32>,
    /// The data phase, empty when the operation has none.
    pub data: Vec<u8>,
}

/// A PTP session on a transport.
pub struct Session<T: Transport> {
    transport: T,
    transaction: u32,
}

impl<T: Transport> Session<T> {
    pub fn new(transport: T) -> Session<T> {
        Session { transport, transaction: 0 }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn next_transaction(&mut self, code: u16) -> u32 {
        // OpenSession is sent outside any session with transaction 0; the ones after it count from 1.
        if code == op::OPEN_SESSION {
            self.transaction = 0;
        } else {
            self.transaction = if self.transaction >= 0xFFFF_FFFE { 1 } else { self.transaction + 1 };
        }
        self.transaction
    }

    fn read_container(&mut self) -> Result<Container, PtpError> {
        let mut bytes = self.transport.read(CHUNK)?;
        for _ in 0..3 {
            if !bytes.is_empty() {
                break;
            }
            bytes = self.transport.read(CHUNK)?;
        }
        let want = Container::declared_length(&bytes).ok_or(PtpError::Malformed("container header"))?;
        if !(HEADER_LEN..=MAX_CONTAINER).contains(&want) {
            return Err(PtpError::Malformed("container length"));
        }
        while bytes.len() < want {
            let more = self.transport.read(CHUNK)?;
            if more.is_empty() {
                return Err(PtpError::Malformed("container (short)"));
            }
            bytes.extend_from_slice(&more);
        }
        bytes.truncate(want);
        Container::decode(&bytes)
    }

    /// Runs one operation: the command, the optional outgoing data, the optional incoming data, the response.
    pub fn transact(&mut self, code: u16, params: &[u32], outgoing: Option<&[u8]>) -> Result<Reply, PtpError> {
        let tx = self.next_transaction(code);
        self.transport.write(&command(code, tx, params).encode())?;
        if let Some(payload) = outgoing {
            self.transport.write(&data(code, tx, payload).encode())?;
        }
        let mut incoming = Vec::new();
        loop {
            let container = self.read_container()?;
            match container.kind {
                ContainerType::Event => continue,
                ContainerType::Command => return Err(PtpError::Malformed("reply")),
                ContainerType::Data => {
                    if container.transaction != tx {
                        return Err(PtpError::Mismatch);
                    }
                    incoming = container.payload;
                }
                ContainerType::Response => {
                    if container.transaction != tx {
                        return Err(PtpError::Mismatch);
                    }
                    if container.code != response::OK {
                        return Err(PtpError::Response(container.code));
                    }
                    return Ok(Reply { params: container.params(), data: incoming });
                }
            }
        }
    }

    /// Opens a session. A camera that still has one open from an earlier run counts as open.
    pub fn open_session(&mut self, id: u32) -> Result<(), PtpError> {
        match self.transact(op::OPEN_SESSION, &[id], None) {
            Ok(_) | Err(PtpError::Response(response::SESSION_ALREADY_OPEN)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn close_session(&mut self) -> Result<(), PtpError> {
        self.transact(op::CLOSE_SESSION, &[], None).map(|_| ())
    }

    pub fn device_info(&mut self) -> Result<DeviceInfo, PtpError> {
        DeviceInfo::parse(&self.transact(op::GET_DEVICE_INFO, &[], None)?.data)
    }

    fn handle_array(bytes: &[u8]) -> Result<Vec<u32>, PtpError> {
        let mut r = Reader::new(bytes);
        let count = r.u32()? as usize;
        if count > bytes.len() {
            return Err(PtpError::Malformed("array"));
        }
        (0..count).map(|_| r.u32()).collect()
    }

    pub fn storage_ids(&mut self) -> Result<Vec<u32>, PtpError> {
        Session::<T>::handle_array(&self.transact(op::GET_STORAGE_IDS, &[], None)?.data)
    }

    /// The handles of the objects in a storage. `format` 0 means any format; `parent` 0xFFFFFFFF means every object at
    /// any depth, 0 the top level only.
    pub fn object_handles(&mut self, storage: u32, format_code: u16, parent: u32) -> Result<Vec<u32>, PtpError> {
        let reply = self.transact(op::GET_OBJECT_HANDLES, &[storage, u32::from(format_code), parent], None)?;
        Session::<T>::handle_array(&reply.data)
    }

    pub fn object_info(&mut self, handle: u32) -> Result<ObjectInfo, PtpError> {
        ObjectInfo::parse(&self.transact(op::GET_OBJECT_INFO, &[handle], None)?.data)
    }

    /// The whole file.
    pub fn get_object(&mut self, handle: u32) -> Result<Vec<u8>, PtpError> {
        Ok(self.transact(op::GET_OBJECT, &[handle], None)?.data)
    }

    /// The camera's thumbnail of the file (JPEG on most cameras).
    pub fn get_thumb(&mut self, handle: u32) -> Result<Vec<u8>, PtpError> {
        Ok(self.transact(op::GET_THUMB, &[handle], None)?.data)
    }

    pub fn delete_object(&mut self, handle: u32) -> Result<(), PtpError> {
        self.transact(op::DELETE_OBJECT, &[handle, 0], None).map(|_| ())
    }

    /// Takes a picture on the camera. `storage` 0 lets the camera choose; `format_code` 0 means its default.
    pub fn initiate_capture(&mut self, storage: u32, format_code: u16) -> Result<(), PtpError> {
        self.transact(op::INITIATE_CAPTURE, &[storage, u32::from(format_code)], None).map(|_| ())
    }

    /// The raw value of a device property.
    pub fn device_prop(&mut self, property: u16) -> Result<Vec<u8>, PtpError> {
        Ok(self.transact(op::GET_DEVICE_PROP_VALUE, &[u32::from(property)], None)?.data)
    }

    pub fn set_device_prop(&mut self, property: u16, value: &[u8]) -> Result<(), PtpError> {
        self.transact(op::SET_DEVICE_PROP_VALUE, &[u32::from(property)], Some(value)).map(|_| ())
    }

    /// The battery level as a percentage, for a camera that reports it as 0 to 100.
    pub fn battery_level(&mut self) -> Result<u8, PtpError> {
        let value = self.device_prop(prop::BATTERY_LEVEL)?;
        value.first().copied().map(|p| p.min(100)).ok_or(PtpError::Malformed("battery level"))
    }

    /// The camera's clock in seconds (see [`parse_date`]).
    pub fn camera_time(&mut self) -> Result<i64, PtpError> {
        let value = self.device_prop(prop::DATE_TIME)?;
        let text = Reader::new(&value).string()?;
        parse_date(&text).ok_or(PtpError::Malformed("date"))
    }

    /// Sets the camera's clock from `seconds` (the host's local time, in the same terms as [`parse_date`]).
    pub fn set_camera_time(&mut self, seconds: i64) -> Result<(), PtpError> {
        self.set_device_prop(prop::DATE_TIME, &encode_string(&format_date(seconds)))
    }
}

// --- an in-memory camera -----------------------------------------------------------------------------------------

fn encode_u16_array(v: &[u16]) -> Vec<u8> {
    let mut out = (v.len() as u32).to_le_bytes().to_vec();
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// A DeviceInfo dataset as a camera sends it.
pub fn encode_device_info(info: &DeviceInfo) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&info.standard_version.to_le_bytes());
    b.extend_from_slice(&info.vendor_extension_id.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend(encode_string(""));
    b.extend_from_slice(&info.functional_mode.to_le_bytes());
    for list in [&info.operations, &info.events, &info.properties, &info.capture_formats, &info.image_formats] {
        b.extend(encode_u16_array(list));
    }
    for s in [&info.manufacturer, &info.model, &info.device_version, &info.serial_number] {
        b.extend(encode_string(s));
    }
    b
}

/// An ObjectInfo dataset as a camera sends it.
pub fn encode_object_info(o: &ObjectInfo) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&o.storage_id.to_le_bytes());
    b.extend_from_slice(&o.format.to_le_bytes());
    b.extend_from_slice(&o.protection.to_le_bytes());
    b.extend_from_slice(&o.size.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    for v in [0u32, 0, 0, o.image_width, o.image_height, 24] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&o.parent.to_le_bytes());
    b.extend_from_slice(&o.association_type.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend(encode_string(&o.filename));
    b.extend(encode_string(&o.capture_date));
    b.extend(encode_string(""));
    b.extend(encode_string(""));
    b
}

/// A camera held in memory. It answers the operations a session uses the way a real camera does (data containers are
/// cut into 512-byte packets like a bulk endpoint), so apps and tests can run a whole import without hardware.
pub struct MemoryCamera {
    info: DeviceInfo,
    queue: std::collections::VecDeque<Vec<u8>>,
    objects: Vec<(u32, ObjectInfo, Vec<u8>)>,
    deleted: Vec<u32>,
    clock: String,
    battery: u8,
    write_protected: bool,
    captures: u32,
    next_handle: u32,
}

impl MemoryCamera {
    /// An empty camera named `model` with one storage and the operations a still camera offers.
    pub fn new(model: &str) -> MemoryCamera {
        let info = DeviceInfo {
            standard_version: 100,
            operations: vec![
                op::GET_DEVICE_INFO,
                op::OPEN_SESSION,
                op::CLOSE_SESSION,
                op::GET_STORAGE_IDS,
                op::GET_OBJECT_HANDLES,
                op::GET_OBJECT_INFO,
                op::GET_OBJECT,
                op::GET_THUMB,
                op::DELETE_OBJECT,
                op::INITIATE_CAPTURE,
                op::GET_DEVICE_PROP_VALUE,
                op::SET_DEVICE_PROP_VALUE,
            ],
            properties: vec![prop::BATTERY_LEVEL, prop::DATE_TIME],
            capture_formats: vec![format::EXIF_JPEG],
            image_formats: vec![format::EXIF_JPEG],
            manufacturer: "Memory".into(),
            model: model.into(),
            device_version: "1.0".into(),
            serial_number: "0".into(),
            ..DeviceInfo::default()
        };
        MemoryCamera {
            info,
            queue: std::collections::VecDeque::new(),
            objects: Vec::new(),
            deleted: Vec::new(),
            clock: "20240131T101530".into(),
            battery: 77,
            write_protected: false,
            captures: 0,
            next_handle: 1,
        }
    }

    /// Adds a folder under `parent` (0 for the top level). Returns its handle.
    pub fn add_folder(&mut self, parent: u32, name: &str) -> u32 {
        let info = ObjectInfo {
            storage_id: 0x10001,
            format: format::ASSOCIATION,
            parent,
            filename: name.into(),
            association_type: 1,
            ..ObjectInfo::default()
        };
        self.push(info, Vec::new())
    }

    /// Adds a file under `parent`. Returns its handle.
    pub fn add_file(&mut self, parent: u32, name: &str, format_code: u16, bytes: Vec<u8>, date: &str) -> u32 {
        let info = ObjectInfo {
            storage_id: 0x10001,
            format: format_code,
            size: bytes.len() as u32,
            image_width: 640,
            image_height: 480,
            parent,
            filename: name.into(),
            capture_date: date.into(),
            ..ObjectInfo::default()
        };
        self.push(info, bytes)
    }

    fn push(&mut self, info: ObjectInfo, bytes: Vec<u8>) -> u32 {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.objects.push((handle, info, bytes));
        handle
    }

    /// Makes deletes fail with a write-protection response, like a card with its lock switch on.
    pub fn set_write_protected(&mut self, on: bool) {
        self.write_protected = on;
    }

    pub fn set_battery(&mut self, percent: u8) {
        self.battery = percent;
    }

    /// The handles deleted so far, in order.
    pub fn deleted(&self) -> &[u32] {
        &self.deleted
    }

    /// How many pictures were taken.
    pub fn captures(&self) -> u32 {
        self.captures
    }

    /// The clock as a PTP date string.
    pub fn clock(&self) -> &str {
        &self.clock
    }

    /// The handles that are still on the camera.
    pub fn handles(&self) -> Vec<u32> {
        self.objects.iter().map(|o| o.0).collect()
    }

    fn reply(&mut self, tx: u32, code: u16, payload: Option<Vec<u8>>, op_code: u16) {
        if let Some(p) = payload {
            for chunk in data(op_code, tx, &p).encode().chunks(512) {
                self.queue.push_back(chunk.to_vec());
            }
        }
        self.queue.push_back(Container { kind: ContainerType::Response, code, transaction: tx, payload: Vec::new() }.encode());
    }
}

impl Transport for MemoryCamera {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let c = Container::decode(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        match c.kind {
            ContainerType::Command => {
                let p = c.params();
                let first = p.first().copied().unwrap_or(0);
                let tx = c.transaction;
                match c.code {
                    op::OPEN_SESSION | op::CLOSE_SESSION | op::SET_DEVICE_PROP_VALUE => self.reply(tx, response::OK, None, c.code),
                    op::GET_DEVICE_INFO => {
                        let d = encode_device_info(&self.info);
                        self.reply(tx, response::OK, Some(d), c.code);
                    }
                    op::GET_STORAGE_IDS => {
                        let mut d = 1u32.to_le_bytes().to_vec();
                        d.extend_from_slice(&0x10001u32.to_le_bytes());
                        self.reply(tx, response::OK, Some(d), c.code);
                    }
                    op::GET_OBJECT_HANDLES => {
                        let mut d = (self.objects.len() as u32).to_le_bytes().to_vec();
                        for o in &self.objects {
                            d.extend_from_slice(&o.0.to_le_bytes());
                        }
                        self.reply(tx, response::OK, Some(d), c.code);
                    }
                    op::GET_OBJECT_INFO => {
                        let found = self.objects.iter().find(|o| o.0 == first).map(|o| encode_object_info(&o.1));
                        match found {
                            Some(d) => self.reply(tx, response::OK, Some(d), c.code),
                            None => self.reply(tx, response::INVALID_OBJECT_HANDLE, None, c.code),
                        }
                    }
                    op::GET_OBJECT | op::GET_THUMB => {
                        let found = self.objects.iter().find(|o| o.0 == first).map(|o| o.2.clone());
                        match found {
                            Some(d) => self.reply(tx, response::OK, Some(d), c.code),
                            None => self.reply(tx, response::INVALID_OBJECT_HANDLE, None, c.code),
                        }
                    }
                    op::DELETE_OBJECT => {
                        if self.write_protected {
                            self.reply(tx, response::OBJECT_WRITE_PROTECTED, None, c.code);
                        } else if self.objects.iter().any(|o| o.0 == first) {
                            self.deleted.push(first);
                            self.objects.retain(|o| o.0 != first);
                            self.reply(tx, response::OK, None, c.code);
                        } else {
                            self.reply(tx, response::INVALID_OBJECT_HANDLE, None, c.code);
                        }
                    }
                    op::INITIATE_CAPTURE => {
                        self.captures += 1;
                        self.reply(tx, response::OK, None, c.code);
                    }
                    op::GET_DEVICE_PROP_VALUE => {
                        let value = if first == u32::from(prop::BATTERY_LEVEL) { vec![self.battery] } else { encode_string(&self.clock) };
                        self.reply(tx, response::OK, Some(value), c.code);
                    }
                    _ => self.reply(tx, response::OPERATION_NOT_SUPPORTED, None, c.code),
                }
            }
            ContainerType::Data => {
                // the only operation that sends data here sets the clock
                if let Ok(text) = Reader::new(&c.payload).string() {
                    self.clock = text;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn read(&mut self, _max: usize) -> io::Result<Vec<u8>> {
        Ok(self.queue.pop_front().unwrap_or_default())
    }
}

// --- USB ---------------------------------------------------------------------------------------------------------

/// Still-image class, subclass and protocol of a PTP interface.
pub const IMAGE_CLASS: (u8, u8, u8) = (6, 1, 1);

/// Containers over the bulk endpoints of a still-image USB interface.
pub struct UsbTransport {
    device: UsbHostDevice,
    bulk_in: u8,
    bulk_out: u8,
    packet_size: usize,
    timeout_ms: u32,
}

/// The interface number and the bulk IN and OUT endpoint addresses of the still-image interface of a configuration.
pub fn find_interface(config: &ConfigurationDescriptor) -> Option<(u8, u8, u8, u16)> {
    let iface = config
        .interfaces
        .iter()
        .find(|i| (i.class, i.subclass, i.protocol) == IMAGE_CLASS && i.alternate == 0)?;
    let bulk = |wanted_in: bool| {
        iface.endpoints.iter().find(|e| e.transfer_type() == TransferType::Bulk && e.is_in() == wanted_in)
    };
    let (bulk_in, bulk_out) = (bulk(true)?, bulk(false)?);
    Some((iface.number, bulk_in.address, bulk_out.address, bulk_out.max_packet_size & 0x7ff))
}

impl UsbTransport {
    /// Claims the camera's still-image interface and returns the transport. `config` is the configuration the device is
    /// in (from `UsbHostDevice::descriptors`).
    pub fn open(mut device: UsbHostDevice, config: &ConfigurationDescriptor) -> io::Result<UsbTransport> {
        let (interface, bulk_in, bulk_out, packet) = find_interface(config)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no still-image interface on this device"))?;
        device.claim_interface(interface)?;
        Ok(UsbTransport { device, bulk_in, bulk_out, packet_size: usize::from(packet.max(8)), timeout_ms: 5000 })
    }

    /// Changes how long a transfer waits for the camera.
    pub fn set_timeout_ms(&mut self, ms: u32) {
        self.timeout_ms = ms;
    }
}

impl Transport for UsbTransport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut buf = bytes.to_vec();
        let mut sent = 0;
        while sent < buf.len() {
            let n = self.device.transfer(self.bulk_out, &mut buf[sent..], self.timeout_ms)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "camera took no bytes"));
            }
            sent += n;
        }
        // a message that ends on a packet boundary is closed by a zero-length packet
        if !buf.is_empty() && buf.len() % self.packet_size == 0 {
            let mut none = [0u8; 0];
            self.device.transfer(self.bulk_out, &mut none, self.timeout_ms)?;
        }
        Ok(())
    }

    fn read(&mut self, max: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; max];
        let n = self.device.transfer(self.bulk_in, &mut buf, self.timeout_ms)?;
        buf.truncate(n);
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb_descriptors::{EndpointDescriptor, InterfaceDescriptor};
    use std::collections::VecDeque;

    fn camera_with_files() -> MemoryCamera {
        let mut cam = MemoryCamera::new("Shutter 1");
        let dcim = cam.add_folder(0, "DCIM");
        let photo: Vec<u8> = (0..1500u32).map(|i| (i % 251) as u8).collect();
        cam.add_file(dcim, "IMG_0001.JPG", format::EXIF_JPEG, photo, "20240131T101530");
        cam.add_file(dcim, "IMG_0002.CR2", format::UNDEFINED, vec![9; 9], "20240131T101600");
        cam
    }

    #[test]
    fn a_container_round_trips() {
        let c = command(op::GET_OBJECT, 7, &[0xdead_beef, 2]);
        let bytes = c.encode();
        assert_eq!(bytes.len(), HEADER_LEN + 8);
        assert_eq!(Container::declared_length(&bytes), Some(20));
        let back = Container::decode(&bytes).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.params(), vec![0xdead_beef, 2]);
    }

    #[test]
    fn a_container_of_the_wrong_length_or_type_is_refused() {
        let mut bytes = command(op::GET_DEVICE_INFO, 1, &[]).encode();
        bytes.push(0);
        assert!(Container::decode(&bytes).is_err());
        let mut bad = command(op::GET_DEVICE_INFO, 1, &[]).encode();
        bad[4] = 9;
        assert!(Container::decode(&bad).is_err());
        assert!(Container::decode(&[1, 2, 3]).is_err());
    }

    #[test]
    fn a_command_carries_at_most_five_parameters() {
        assert_eq!(command(op::GET_OBJECT, 1, &[1, 2, 3, 4, 5, 6, 7]).params().len(), 5);
    }

    #[test]
    fn strings_round_trip_including_the_empty_one() {
        for s in ["", "IMG_0001.JPG", "Skýrsla ár"] {
            let bytes = encode_string(s);
            assert_eq!(Reader::new(&bytes).string().unwrap(), s);
        }
        assert_eq!(encode_string(""), vec![0]);
        assert_eq!(encode_string("ab"), vec![3, b'a', 0, b'b', 0, 0, 0]);
    }

    #[test]
    fn the_device_info_dataset_parses() {
        let want = DeviceInfo {
            standard_version: 100,
            operations: vec![op::GET_DEVICE_INFO, op::INITIATE_CAPTURE],
            properties: vec![prop::DATE_TIME],
            manufacturer: "Acme".into(),
            model: "Shutter 1".into(),
            device_version: "1.0".into(),
            serial_number: "SN42".into(),
            ..DeviceInfo::default()
        };
        let bytes = encode_device_info(&want);
        let info = DeviceInfo::parse(&bytes).unwrap();
        assert_eq!(info, want);
        assert!(info.supports_operation(op::INITIATE_CAPTURE));
        assert!(!info.supports_operation(op::SEND_OBJECT));
        assert!(info.supports_property(prop::DATE_TIME));
        assert!(DeviceInfo::parse(&bytes[..10]).is_err());
    }

    #[test]
    fn the_object_info_dataset_parses() {
        let want = ObjectInfo {
            storage_id: 5,
            format: format::EXIF_JPEG,
            size: 1500,
            image_width: 640,
            image_height: 480,
            parent: 3,
            filename: "a.jpg".into(),
            capture_date: "20240131T101530".into(),
            ..ObjectInfo::default()
        };
        let info = ObjectInfo::parse(&encode_object_info(&want)).unwrap();
        assert_eq!(info, want);
        assert!(!info.is_folder());
        assert!(ObjectInfo::parse(&[0; 8]).is_err());
    }

    #[test]
    fn media_kinds_follow_the_format_and_then_the_extension() {
        assert_eq!(media_kind(format::EXIF_JPEG, "a.jpg"), MediaKind::Image);
        assert_eq!(media_kind(format::UNDEFINED, "IMG_1.CR2"), MediaKind::Raw);
        assert_eq!(media_kind(format::DNG, "x"), MediaKind::Raw);
        assert_eq!(media_kind(format::MP4, "x"), MediaKind::Movie);
        assert_eq!(media_kind(format::UNDEFINED, "clip.MOV"), MediaKind::Movie);
        assert_eq!(media_kind(format::WAV, "x"), MediaKind::Audio);
        assert_eq!(media_kind(format::UNDEFINED, "note.m4a"), MediaKind::Audio);
    }

    #[test]
    fn dates_convert_both_ways() {
        assert_eq!(parse_date("19700101T000000"), Some(0));
        assert_eq!(parse_date("20000301T000000"), Some(11_017 * 86_400));
        let t = parse_date("20240131T101530").unwrap();
        assert_eq!(format_date(t), "20240131T101530");
        assert_eq!(parse_date("20240131T101530.5+0100"), Some(t));
        assert_eq!(format_date(-1), "19691231T235959");
        assert_eq!(parse_date("2024-01-31"), None);
        assert_eq!(parse_date("20241331T000000"), None);
    }

    #[test]
    fn a_session_lists_the_storage_and_reads_object_info() {
        let mut s = Session::new(camera_with_files());
        s.open_session(1).unwrap();
        assert_eq!(s.device_info().unwrap().model, "Shutter 1");
        assert_eq!(s.storage_ids().unwrap(), vec![0x10001]);
        let handles = s.object_handles(0xFFFF_FFFF, 0, 0xFFFF_FFFF).unwrap();
        assert_eq!(handles, vec![1, 2, 3]);
        let info = s.object_info(2).unwrap();
        assert_eq!((info.filename.as_str(), info.parent), ("IMG_0001.JPG", 1));
        assert!(matches!(s.object_info(99), Err(PtpError::Response(response::INVALID_OBJECT_HANDLE))));
    }

    #[test]
    fn a_file_larger_than_one_packet_is_reassembled() {
        let mut s = Session::new(camera_with_files());
        s.open_session(1).unwrap();
        let bytes = s.get_object(2).unwrap();
        assert_eq!(bytes.len(), 1500);
        assert_eq!(bytes[250], 250);
        assert_eq!(bytes[251], 0);
    }

    #[test]
    fn transactions_count_from_one_after_open_session() {
        let mut s = Session::new(camera_with_files());
        s.open_session(1).unwrap();
        assert_eq!(s.transaction, 0);
        s.storage_ids().unwrap();
        s.storage_ids().unwrap();
        assert_eq!(s.transaction, 2);
    }

    #[test]
    fn a_delete_removes_the_object_and_a_protected_one_is_refused() {
        let mut s = Session::new(camera_with_files());
        s.open_session(1).unwrap();
        s.delete_object(3).unwrap();
        assert_eq!(s.transport().deleted(), &[3]);
        s.transport.set_write_protected(true);
        let err = s.delete_object(2).unwrap_err();
        match err {
            PtpError::Response(code) => assert!(is_write_protection(code)),
            other => panic!("expected a response error, got {other}"),
        }
    }

    #[test]
    fn capture_battery_and_clock_work() {
        let mut s = Session::new(camera_with_files());
        s.open_session(1).unwrap();
        s.initiate_capture(0, 0).unwrap();
        assert_eq!(s.transport().captures(), 1);
        assert_eq!(s.battery_level().unwrap(), 77);
        assert_eq!(s.camera_time().unwrap(), parse_date("20240131T101530").unwrap());
        s.set_camera_time(parse_date("20250101T000000").unwrap()).unwrap();
        assert_eq!(s.transport().clock(), "20250101T000000");
    }

    #[test]
    fn an_unsupported_operation_is_a_response_error() {
        let mut s = Session::new(camera_with_files());
        s.open_session(1).unwrap();
        let err = s.transact(0x9999, &[], None).unwrap_err();
        assert!(matches!(err, PtpError::Response(response::OPERATION_NOT_SUPPORTED)));
        assert_eq!(response_name(response::OPERATION_NOT_SUPPORTED), "operation not supported");
    }

    #[test]
    fn a_reply_for_another_transaction_is_refused() {
        struct Wrong;
        impl Transport for Wrong {
            fn write(&mut self, _: &[u8]) -> io::Result<()> {
                Ok(())
            }
            fn read(&mut self, _: usize) -> io::Result<Vec<u8>> {
                Ok(Container { kind: ContainerType::Response, code: response::OK, transaction: 99, payload: vec![] }.encode())
            }
        }
        let mut s = Session::new(Wrong);
        assert!(matches!(s.storage_ids(), Err(PtpError::Mismatch)));
    }

    #[test]
    fn events_between_the_phases_are_skipped() {
        struct Noisy(VecDeque<Vec<u8>>);
        impl Transport for Noisy {
            fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
                let c = Container::decode(bytes).unwrap();
                let event = Container { kind: ContainerType::Event, code: 0x4002, transaction: 0, payload: vec![] };
                let ok = Container { kind: ContainerType::Response, code: response::OK, transaction: c.transaction, payload: vec![] };
                self.0.push_back(event.encode());
                self.0.push_back(ok.encode());
                Ok(())
            }
            fn read(&mut self, _: usize) -> io::Result<Vec<u8>> {
                Ok(self.0.pop_front().unwrap_or_default())
            }
        }
        let mut s = Session::new(Noisy(VecDeque::new()));
        s.open_session(1).unwrap();
        s.close_session().unwrap();
    }

    #[test]
    fn the_still_image_interface_is_found_in_a_configuration() {
        let ep = |address: u8, attributes: u8| EndpointDescriptor { address, attributes, max_packet_size: 512, interval: 0 };
        let config = ConfigurationDescriptor {
            value: 1,
            attributes: 0x80,
            max_power_ma: 100,
            interfaces: vec![
                InterfaceDescriptor { number: 0, alternate: 0, class: 3, subclass: 0, protocol: 0, endpoints: vec![] },
                InterfaceDescriptor {
                    number: 2,
                    alternate: 0,
                    class: 6,
                    subclass: 1,
                    protocol: 1,
                    endpoints: vec![ep(0x81, 0x02), ep(0x02, 0x02), ep(0x83, 0x03)],
                },
            ],
        };
        assert_eq!(find_interface(&config), Some((2, 0x81, 0x02, 512)));
        let mut none = config.clone();
        none.interfaces.remove(1);
        assert_eq!(find_interface(&none), None);
    }
}
