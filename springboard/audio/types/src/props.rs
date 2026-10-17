//! The property table: one `const` per selector with its value kind, the scopes it exists in and whether it can be
//! set. Neutral names only (the mapping to the reference names is in tasks/audio/02-object-model.md).
//!
//! Owner: task 02. The selectors below are the contract; task 02 adds the full table (`PROPERTIES`) and lookups.

use crate::address::{Class, FourCC, Scope};
use crate::error::AudioError;
use crate::value::PropertyValue;

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

/// The application processes the server tracks (System).
pub const PROCESSES: FourCC = FourCC::new(b"prc#");
/// Process: the application id the client said in its Hello, its pid, and whether it runs output or input streams.
pub const APP_ID: FourCC = FourCC::new(b"appi");
pub const PROCESS_PID: FourCC = FourCC::new(b"ppid");
pub const RUNNING_OUTPUT: FourCC = FourCC::new(b"pout");
pub const RUNNING_INPUT: FourCC = FourCC::new(b"pinp");
/// Process: the audio session the application declared. The category is one of `ambient`, `solo-ambient`, `playback`,
/// `record` and `play-and-record` (empty: no session); `session-active` starts it; `session-mixes` and `session-ducks`
/// are its options (mix with others, duck others).
pub const SESSION_CATEGORY: FourCC = FourCC::new(b"scat");
pub const SESSION_ACTIVE: FourCC = FourCC::new(b"sact");
pub const SESSION_MIXES: FourCC = FourCC::new(b"smix");
pub const SESSION_DUCKS: FourCC = FourCC::new(b"sdck");
/// Device, per scope: the newest level and spectrum measurement of what the device plays (Output) or captures (Input),
/// in the text form of `bifrost_audio_engine::analysis::Measurement::encode`; empty until the device has run.
pub const LEVEL: FourCC = FourCC::new(b"levl");
/// Device, per scope: the processing stages that run for the device (one `kind id` per line) and the frames of
/// latency they add.
pub const DSP_PLAN: FourCC = FourCC::new(b"dsp ");
pub const DSP_LATENCY: FourCC = FourCC::new(b"dspl");

pub const VIRTUAL_FORMAT: FourCC = FourCC::new(b"sfmt");
pub const PHYSICAL_FORMAT: FourCC = FourCC::new(b"pft ");

/// The value type a property carries (the `PropertyValue` variant it must be set with).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Bool,
    U32,
    I32,
    F32,
    F64,
    String,
    Object,
    Objects,
    RangeU32,
    Rates,
    Format,
    Formats,
    ChannelPair,
    Pid,
    Bytes,
}

impl ValueKind {
    /// The kind of a value.
    pub fn of(v: &PropertyValue) -> ValueKind {
        match v {
            PropertyValue::Bool(_) => ValueKind::Bool,
            PropertyValue::U32(_) => ValueKind::U32,
            PropertyValue::I32(_) => ValueKind::I32,
            PropertyValue::F32(_) => ValueKind::F32,
            PropertyValue::F64(_) => ValueKind::F64,
            PropertyValue::String(_) => ValueKind::String,
            PropertyValue::Object(_) => ValueKind::Object,
            PropertyValue::Objects(_) => ValueKind::Objects,
            PropertyValue::RangeU32(..) => ValueKind::RangeU32,
            PropertyValue::Rates(_) => ValueKind::Rates,
            PropertyValue::Format(_) => ValueKind::Format,
            PropertyValue::Formats(_) => ValueKind::Formats,
            PropertyValue::ChannelPair(..) => ValueKind::ChannelPair,
            PropertyValue::Pid(_) => ValueKind::Pid,
            PropertyValue::Bytes(_) => ValueKind::Bytes,
        }
    }
}

/// Scope bits for `PropDef::scopes`.
pub const S_GLOBAL: u8 = 1;
pub const S_INPUT: u8 = 2;
pub const S_OUTPUT: u8 = 4;
pub const S_PLAY_THROUGH: u8 = 8;
/// Input and output.
pub const S_IO: u8 = S_INPUT | S_OUTPUT;

/// The bit of a scope.
pub const fn scope_bit(s: Scope) -> u8 {
    match s {
        Scope::Global => S_GLOBAL,
        Scope::Input => S_INPUT,
        Scope::Output => S_OUTPUT,
        Scope::PlayThrough => S_PLAY_THROUGH,
    }
}

/// One row of the property table.
#[derive(Debug, Clone, Copy)]
pub struct PropDef {
    pub selector: FourCC,
    pub name: &'static str,
    pub kind: ValueKind,
    /// Scope bits the property exists in.
    pub scopes: u8,
    /// Classes that carry it.
    pub classes: &'static [Class],
    pub settable: bool,
}

use Class::*;

const fn p(selector: FourCC, name: &'static str, kind: ValueKind, scopes: u8, classes: &'static [Class], settable: bool) -> PropDef {
    PropDef { selector, name, kind, scopes, classes, settable }
}

/// Every property of the object model.
pub static PROPERTIES: &[PropDef] = &[
    p(DEVICES, "devices", ValueKind::Objects, S_GLOBAL, &[System], false),
    p(DEFAULT_OUTPUT, "default-output", ValueKind::Object, S_GLOBAL, &[System], true),
    p(DEFAULT_INPUT, "default-input", ValueKind::Object, S_GLOBAL, &[System], true),
    p(DEFAULT_SOUND_EFFECTS, "default-sound-effects", ValueKind::Object, S_GLOBAL, &[System], true),
    p(TAPS, "taps", ValueKind::Objects, S_GLOBAL, &[System], false),
    p(NAME, "name", ValueKind::String, S_GLOBAL | S_IO, &[System, Plugin, Device, Stream, Volume, Mute, DataSource, Selector, Box, Clock, Tap, AggregateDevice], false),
    p(MANUFACTURER, "manufacturer", ValueKind::String, S_GLOBAL, &[Plugin, Device, Box, AggregateDevice], false),
    p(DEVICE_UID, "device-uid", ValueKind::String, S_GLOBAL, &[Device, AggregateDevice, Tap], false),
    p(MODEL_UID, "model-uid", ValueKind::String, S_GLOBAL, &[Device, AggregateDevice], false),
    p(TRANSPORT, "transport", ValueKind::U32, S_GLOBAL, &[Device, Box, AggregateDevice], false),
    p(IS_ALIVE, "is-alive", ValueKind::Bool, S_GLOBAL, &[Device, AggregateDevice], false),
    p(IS_RUNNING, "is-running", ValueKind::Bool, S_GLOBAL, &[Device, AggregateDevice], false),
    p(NOMINAL_RATE, "nominal-rate", ValueKind::F64, S_GLOBAL, &[Device, AggregateDevice], true),
    p(AVAILABLE_RATES, "available-rates", ValueKind::Rates, S_GLOBAL, &[Device, AggregateDevice], false),
    p(STREAMS, "streams", ValueKind::Objects, S_IO, &[Device, AggregateDevice], false),
    p(BUFFER_FRAMES, "buffer-frames", ValueKind::U32, S_GLOBAL, &[Device, AggregateDevice], true),
    p(BUFFER_FRAMES_RANGE, "buffer-frames-range", ValueKind::RangeU32, S_GLOBAL, &[Device, AggregateDevice], false),
    p(LATENCY, "latency", ValueKind::U32, S_IO, &[Device, Stream, AggregateDevice], false),
    p(SAFETY_OFFSET, "safety-offset", ValueKind::U32, S_IO, &[Device, Stream, AggregateDevice], false),
    p(ZERO_TS_PERIOD, "zero-timestamp-period", ValueKind::U32, S_GLOBAL, &[Device, AggregateDevice], false),
    p(CLOCK_DOMAIN, "clock-domain", ValueKind::U32, S_GLOBAL, &[Device, Clock, AggregateDevice], false),
    p(HOG_PID, "hog-pid", ValueKind::Pid, S_GLOBAL, &[Device, AggregateDevice], true),
    p(STEREO_CHANNELS, "stereo-channels", ValueKind::ChannelPair, S_IO, &[Device, AggregateDevice], true),
    p(CHANNEL_LAYOUT, "channel-layout", ValueKind::Bytes, S_IO, &[Device, AggregateDevice], true),
    p(JACK_CONNECTED, "jack-connected", ValueKind::Bool, S_IO, &[Device], false),
    p(DATA_SOURCE, "data-source", ValueKind::U32, S_IO, &[Device, DataSource, Selector], true),
    p(IO_WORKGROUP, "io-workgroup", ValueKind::Bytes, S_GLOBAL, &[Device, AggregateDevice], false),
    p(OVERLOAD, "overload", ValueKind::Bool, S_GLOBAL, &[Device, AggregateDevice], false),
    p(VOLUME_SCALAR, "volume", ValueKind::F32, S_IO, &[Volume], true),
    p(MUTE, "mute", ValueKind::Bool, S_IO, &[Mute], true),
    p(PROCESSES, "processes", ValueKind::Objects, S_GLOBAL, &[System], false),
    p(APP_ID, "app-id", ValueKind::String, S_GLOBAL, &[Process], false),
    p(PROCESS_PID, "process-pid", ValueKind::Pid, S_GLOBAL, &[Process], false),
    p(RUNNING_OUTPUT, "running-output", ValueKind::Bool, S_GLOBAL, &[Process], false),
    p(RUNNING_INPUT, "running-input", ValueKind::Bool, S_GLOBAL, &[Process], false),
    p(SESSION_CATEGORY, "session-category", ValueKind::String, S_GLOBAL, &[Process], true),
    p(SESSION_ACTIVE, "session-active", ValueKind::Bool, S_GLOBAL, &[Process], true),
    p(SESSION_MIXES, "session-mixes", ValueKind::Bool, S_GLOBAL, &[Process], true),
    p(SESSION_DUCKS, "session-ducks", ValueKind::Bool, S_GLOBAL, &[Process], true),
    p(LEVEL, "level", ValueKind::String, S_IO, &[Device], false),
    p(DSP_PLAN, "dsp-plan", ValueKind::String, S_IO, &[Device], false),
    p(DSP_LATENCY, "dsp-latency", ValueKind::U32, S_IO, &[Device], false),
    p(VIRTUAL_FORMAT, "virtual-format", ValueKind::Format, S_GLOBAL, &[Stream], true),
    p(PHYSICAL_FORMAT, "physical-format", ValueKind::Format, S_GLOBAL, &[Stream], true),
];

/// The table row of a selector.
pub fn lookup(selector: FourCC) -> Option<&'static PropDef> {
    PROPERTIES.iter().find(|d| d.selector == selector)
}

/// Whether `value` has the type the property wants.
pub fn check_type(def: &PropDef, value: &PropertyValue) -> Result<(), AudioError> {
    if ValueKind::of(value) == def.kind {
        Ok(())
    } else {
        Err(AudioError::BadValueType)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_are_unique_and_found() {
        for (i, a) in PROPERTIES.iter().enumerate() {
            for b in &PROPERTIES[i + 1..] {
                assert_ne!(a.selector, b.selector, "{}", a.name);
            }
            assert_eq!(lookup(a.selector).unwrap().name, a.name);
        }
        assert_eq!(PROPERTIES.len(), 44);
    }

    #[test]
    fn unknown_selector_is_none() {
        assert!(lookup(FourCC::new(b"zzzz")).is_none());
    }

    #[test]
    fn settable_flags_and_kinds() {
        assert!(lookup(NOMINAL_RATE).unwrap().settable);
        assert!(!lookup(IS_ALIVE).unwrap().settable);
        assert!(!lookup(DEVICES).unwrap().settable);
        assert_eq!(lookup(HOG_PID).unwrap().kind, ValueKind::Pid);
        assert_eq!(scope_bit(Scope::Output), 4);
        assert_eq!(FourCC::new(b"nsrt").as_u32(), 0x6e73_7274);
    }

    #[test]
    fn type_check() {
        let d = lookup(BUFFER_FRAMES).unwrap();
        assert_eq!(check_type(d, &PropertyValue::U32(64)), Ok(()));
        assert_eq!(check_type(d, &PropertyValue::F64(64.0)), Err(AudioError::BadValueType));
    }
}
