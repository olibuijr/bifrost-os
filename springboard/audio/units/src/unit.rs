//! The component trait. A unit renders planar f32 blocks on the RT thread, owns a parameter tree, takes MIDI and
//! reports latency and tail time. Effects have as many outputs as inputs; generators and instruments have no
//! inputs.

use crate::midi::MidiEvent;
use crate::param::{ParamHandle, ParameterTree};
use std::fmt;

/// What a unit does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitKind {
    /// Audio in, audio out, same channel count.
    Effect,
    /// No audio in, audio out, driven by MIDI.
    Instrument,
    /// No audio in, audio out, no MIDI.
    Generator,
}

/// Static facts about a unit.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitInfo {
    pub name: String,
    pub kind: UnitKind,
    pub accepts_midi: bool,
    /// The most channels the unit processes.
    pub max_channels: u32,
}

/// What the host tells a unit when it initializes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderConfig {
    pub sample_rate: f64,
    /// The most frames one `render` call covers.
    pub max_frames: u32,
    /// Channels the unit gets in and gives out (0 inputs for instruments and generators).
    pub input_channels: u32,
    pub output_channels: u32,
}

/// Failures of the unit model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitError {
    NotInitialized,
    AlreadyInitialized,
    /// Rate, channel count or frames out of range, or not supported by the unit.
    BadFormat,
    /// More frames than the configured maximum.
    TooManyFrames,
    BadParameter,
    BadValue,
    ReadOnly,
    NoPreset,
    NoComponent,
    /// A four-character code is not exactly four ASCII characters.
    BadCode,
    /// The out-of-process unit did not answer in time, or its transport failed.
    Remote,
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UnitError::NotInitialized => "the unit is not initialized",
            UnitError::AlreadyInitialized => "the unit is already initialized",
            UnitError::BadFormat => "the format is out of range or unsupported",
            UnitError::TooManyFrames => "more frames than the maximum per block",
            UnitError::BadParameter => "no such parameter",
            UnitError::BadValue => "the value is out of range",
            UnitError::ReadOnly => "the parameter is read-only",
            UnitError::NoPreset => "no such preset",
            UnitError::NoComponent => "no such component",
            UnitError::BadCode => "a code must be four ASCII characters",
            UnitError::Remote => "the out-of-process unit failed",
        })
    }
}

impl std::error::Error for UnitError {}

/// One render call. `inputs` and `outputs` hold one slice per channel, each `frames` long (checked by the host).
/// `midi` is sorted by offset.
pub struct Block<'a, 'b> {
    pub frames: usize,
    /// Sample time of the first frame of the block.
    pub sample_time: f64,
    pub inputs: &'a [&'b [f32]],
    pub outputs: &'a mut [&'b mut [f32]],
    pub midi: &'a [MidiEvent],
}

/// A processing component. `render` runs on the real-time thread: no allocation, no locks, no syscalls.
pub trait AudioUnit: Send {
    fn info(&self) -> &UnitInfo;
    fn tree(&self) -> &ParameterTree;
    fn tree_mut(&mut self) -> &mut ParameterTree;

    /// Allocates everything `render` needs for `cfg`. May be called again after `uninitialize`.
    fn initialize(&mut self, cfg: &RenderConfig) -> Result<(), UnitError>;
    fn uninitialize(&mut self) {}
    /// Clears internal state (delay lines, filter memory, voices); parameters keep their values.
    fn reset(&mut self);

    /// Frames the output lags the input.
    fn latency_frames(&self) -> u32 {
        0
    }

    /// Frames the unit keeps sounding after its input ends (reverb tails, release stages).
    fn tail_frames(&self) -> u32 {
        0
    }

    /// Fills `block.outputs`. The unit calls `tree_mut().sync()` itself at the start.
    fn render(&mut self, block: &mut Block<'_, '_>);

    /// The control-side parameter handle.
    fn params(&self) -> ParamHandle {
        self.tree().handle()
    }
}

/// Checks a configuration against a unit's facts.
pub fn check_config(info: &UnitInfo, cfg: &RenderConfig) -> Result<(), UnitError> {
    if !(cfg.sample_rate.is_finite() && (8_000.0..=768_000.0).contains(&cfg.sample_rate)) {
        return Err(UnitError::BadFormat);
    }
    if cfg.max_frames == 0 || cfg.max_frames > 65_536 || cfg.output_channels == 0 || cfg.output_channels > info.max_channels {
        return Err(UnitError::BadFormat);
    }
    match info.kind {
        UnitKind::Effect if cfg.input_channels != cfg.output_channels => Err(UnitError::BadFormat),
        UnitKind::Instrument | UnitKind::Generator if cfg.input_channels != 0 => Err(UnitError::BadFormat),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx() -> UnitInfo {
        UnitInfo { name: "t".into(), kind: UnitKind::Effect, accepts_midi: false, max_channels: 2 }
    }

    fn cfg(inp: u32, out: u32) -> RenderConfig {
        RenderConfig { sample_rate: 48_000.0, max_frames: 128, input_channels: inp, output_channels: out }
    }

    #[test]
    fn effects_need_matching_channels() {
        assert_eq!(check_config(&fx(), &cfg(2, 2)), Ok(()));
        assert_eq!(check_config(&fx(), &cfg(1, 2)), Err(UnitError::BadFormat));
        assert_eq!(check_config(&fx(), &cfg(3, 3)), Err(UnitError::BadFormat));
    }

    #[test]
    fn instruments_take_no_inputs() {
        let mut i = fx();
        i.kind = UnitKind::Instrument;
        assert_eq!(check_config(&i, &cfg(0, 2)), Ok(()));
        assert_eq!(check_config(&i, &cfg(2, 2)), Err(UnitError::BadFormat));
    }

    #[test]
    fn rate_and_frames_are_range_checked() {
        let mut c = cfg(2, 2);
        c.sample_rate = 7_999.0;
        assert_eq!(check_config(&fx(), &c), Err(UnitError::BadFormat));
        c.sample_rate = f64::NAN;
        assert_eq!(check_config(&fx(), &c), Err(UnitError::BadFormat));
        c = cfg(2, 2);
        c.max_frames = 0;
        assert_eq!(check_config(&fx(), &c), Err(UnitError::BadFormat));
        c.max_frames = 65_537;
        assert_eq!(check_config(&fx(), &c), Err(UnitError::BadFormat));
    }

    #[test]
    fn errors_display() {
        assert_eq!(UnitError::ReadOnly.to_string(), "the parameter is read-only");
    }
}
