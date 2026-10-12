//! The property table: one `const` per selector with its value kind, the scopes it exists in and whether it can be
//! set. Neutral names only (the mapping to the reference names is in tasks/audio/02-object-model.md).
//!
//! Owner: task 02. The selectors below are the contract; task 02 adds the full table (`PROPERTIES`) and lookups.

use crate::address::FourCC;

pub const DEVICES: FourCC = FourCC::new(b"dev#");
pub const DEFAULT_OUTPUT: FourCC = FourCC::new(b"dOut");
pub const DEFAULT_INPUT: FourCC = FourCC::new(b"dIn ");
pub const DEFAULT_SOUND_EFFECTS: FourCC = FourCC::new(b"sOut");
pub const TAPS: FourCC = FourCC::new(b"tps#");

pub const NAME: FourCC = FourCC::new(b"lnam");
pub const MANUFACTURER: FourCC = FourCC::new(b"lmak");
pub const DEVICE_UID: FourCC = FourCC::new(b"uid ");
pub const MODEL_UID: FourCC = FourCC::new(b"muid");
pub const TRANSPORT: FourCC = FourCC::new(b"tran");
pub const IS_ALIVE: FourCC = FourCC::new(b"livn");
pub const IS_RUNNING: FourCC = FourCC::new(b"goin");
pub const NOMINAL_RATE: FourCC = FourCC::new(b"nsrt");
pub const AVAILABLE_RATES: FourCC = FourCC::new(b"nsr#");
pub const STREAMS: FourCC = FourCC::new(b"stm#");
pub const BUFFER_FRAMES: FourCC = FourCC::new(b"fsiz");
pub const BUFFER_FRAMES_RANGE: FourCC = FourCC::new(b"fsz#");
pub const LATENCY: FourCC = FourCC::new(b"ltnc");
pub const SAFETY_OFFSET: FourCC = FourCC::new(b"saft");
pub const ZERO_TS_PERIOD: FourCC = FourCC::new(b"ring");
pub const CLOCK_DOMAIN: FourCC = FourCC::new(b"clkd");
pub const HOG_PID: FourCC = FourCC::new(b"oink");
pub const STEREO_CHANNELS: FourCC = FourCC::new(b"dch2");
pub const CHANNEL_LAYOUT: FourCC = FourCC::new(b"srnd");
pub const JACK_CONNECTED: FourCC = FourCC::new(b"jack");
pub const DATA_SOURCE: FourCC = FourCC::new(b"ssrc");
pub const IO_WORKGROUP: FourCC = FourCC::new(b"oswg");
pub const OVERLOAD: FourCC = FourCC::new(b"olod");
pub const VOLUME_SCALAR: FourCC = FourCC::new(b"volm");
pub const MUTE: FourCC = FourCC::new(b"mute");

pub const VIRTUAL_FORMAT: FourCC = FourCC::new(b"sfmt");
pub const PHYSICAL_FORMAT: FourCC = FourCC::new(b"pft ");
