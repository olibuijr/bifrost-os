//! Audio render cycle: a render unit that fills a block of planar float samples from its render callbacks,
//! an I/O unit that starts and stops the pull of those blocks, and a component registry that finds render
//! units by a four-character description and creates them. This is the pure model behind a plug-in
//! host. No hardware is touched: a device thread owned by the shell calls `IoUnit::pull`. Times are sample
//! times in frames, counted from the start of the I/O unit.

use std::fmt;

/// Failures of the render model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// The unit is not initialized, or was uninitialized again.
    NotInitialized,
    /// The unit is already initialized.
    AlreadyInitialized,
    /// Output has not started.
    NotRunning,
    /// Output is already running.
    AlreadyRunning,
    /// A stream format is missing, out of range, or does not match the output it feeds.
    InvalidFormat,
    /// The bus does not exist on this unit.
    InvalidBus,
    /// A value is out of its allowed range, or a time is not finite.
    InvalidValue,
    /// More frames were asked for than the maximum frames per slice.
    TooManyFrames,
    /// The setting cannot change while the unit is initialized.
    PropertyLocked,
    /// No component is registered at that index.
    NoComponent,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            RenderError::NotInitialized => "the render unit is not initialized",
            RenderError::AlreadyInitialized => "the render unit is already initialized",
            RenderError::NotRunning => "output is not running",
            RenderError::AlreadyRunning => "output is already running",
            RenderError::InvalidFormat => "the stream format is missing or invalid",
            RenderError::InvalidBus => "the bus does not exist",
            RenderError::InvalidValue => "the value is out of range",
            RenderError::TooManyFrames => "more frames than the maximum per slice",
            RenderError::PropertyLocked => "the setting cannot change while initialized",
            RenderError::NoComponent => "no component is registered at that index",
        };
        f.write_str(text)
    }
}

/// The lowest accepted sample rate, in hertz.
pub const MIN_SAMPLE_RATE: f64 = 8_000.0;
/// The highest accepted sample rate, in hertz.
pub const MAX_SAMPLE_RATE: f64 = 768_000.0;
/// The most channels one bus carries.
pub const MAX_CHANNELS: u32 = 32;
/// The most buses one render unit has.
pub const MAX_BUSES: u32 = 64;
/// The frames per slice a unit accepts until a larger value is set.
pub const DEFAULT_MAX_FRAMES: usize = 4_096;
/// The largest frames per slice that can be set.
pub const MAX_FRAMES: usize = 65_536;

/// The layout of one bus: sample rate and channel count. Samples are 32-bit float, planar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamFormat {
    pub sample_rate: f64,
    pub channels: u32,
}

impl StreamFormat {
    /// A checked format.
    pub fn new(sample_rate: f64, channels: u32) -> Result<Self, RenderError> {
        let format = StreamFormat { sample_rate, channels };
        format.validate()?;
        Ok(format)
    }

    /// Refuses a rate outside the accepted range, a non-finite rate, or a channel count outside 1 to 32.
    pub fn validate(&self) -> Result<(), RenderError> {
        let rate_ok = self.sample_rate.is_finite()
            && (MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&self.sample_rate);
        if rate_ok && (1..=MAX_CHANNELS).contains(&self.channels) {
            Ok(())
        } else {
            Err(RenderError::InvalidFormat)
        }
    }
}

/// Which side of a render unit a property applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Settings for the whole unit (the bus argument is ignored).
    Global,
    /// The buses that feed the unit. A render callback sits on an input bus.
    Input,
    /// The buses the unit produces. Bus 0 is what `render` returns.
    Output,
}

/// Flags passed to render notifications.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderFlags {
    /// The notification runs before the inputs are pulled.
    pub pre_render: bool,
    /// The notification runs after the output is mixed.
    pub post_render: bool,
    /// Set on the post notification when every output sample is zero.
    pub output_is_silence: bool,
}

/// One block handed to a render callback. `channels` is planar: one vector per channel, `frames` long,
/// already zeroed. The callback writes into it.
pub struct RenderBlock<'a> {
    /// The input bus this callback is attached to.
    pub bus: u32,
    /// The number of frames in each channel.
    pub frames: usize,
    /// The sample time of the first frame.
    pub sample_time: f64,
    pub channels: &'a mut [Vec<f32>],
}

/// Supplies samples for one input bus of a render unit.
pub type RenderCallback = Box<dyn for<'a> FnMut(&mut RenderBlock<'a>) -> Result<(), RenderError>>;

/// Called around every render with the flags, the frame count and the sample time.
pub type RenderNotify = Box<dyn FnMut(RenderFlags, usize, f64)>;

fn planar(channels: usize, frames: usize) -> Vec<Vec<f32>> {
    vec![vec![0.0; frames]; channels]
}

/// A processing node with input and output buses. Its output is the sum of every input bus that has a
/// render callback; a bus carrying fewer channels than the output is repeated across the output channels.
pub struct RenderUnit {
    buses: u32,
    inputs: Vec<Option<StreamFormat>>,
    outputs: Vec<Option<StreamFormat>>,
    max_frames: usize,
    callbacks: Vec<Option<RenderCallback>>,
    notify: Vec<Option<RenderNotify>>,
    initialized: bool,
}

impl RenderUnit {
    /// A unit with `buses` input and output buses, not yet initialized.
    pub fn new(buses: u32) -> Result<Self, RenderError> {
        if !(1..=MAX_BUSES).contains(&buses) {
            return Err(RenderError::InvalidBus);
        }
        let n = buses as usize;
        Ok(RenderUnit {
            buses,
            inputs: vec![None; n],
            outputs: vec![None; n],
            max_frames: DEFAULT_MAX_FRAMES,
            callbacks: (0..n).map(|_| None).collect(),
            notify: Vec::new(),
            initialized: false,
        })
    }

    /// Refuses a change while the unit is initialized.
    fn check_unlocked(&self) -> Result<(), RenderError> {
        if self.initialized {
            Err(RenderError::PropertyLocked)
        } else {
            Ok(())
        }
    }

    /// The most frames one `render` call may produce.
    pub fn set_max_frames(&mut self, frames: usize) -> Result<(), RenderError> {
        self.check_unlocked()?;
        if !(1..=MAX_FRAMES).contains(&frames) {
            return Err(RenderError::InvalidValue);
        }
        self.max_frames = frames;
        Ok(())
    }

    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// Sets the format of one bus. The global scope has no format.
    pub fn set_stream_format(&mut self, scope: Scope, bus: u32, format: StreamFormat) -> Result<(), RenderError> {
        self.check_unlocked()?;
        format.validate()?;
        let slots = match scope {
            Scope::Input => &mut self.inputs,
            Scope::Output => &mut self.outputs,
            Scope::Global => return Err(RenderError::InvalidValue),
        };
        let slot = slots.get_mut(bus as usize).ok_or(RenderError::InvalidBus)?;
        *slot = Some(format);
        Ok(())
    }

    /// The format of one bus, or `None` when it was never set.
    pub fn stream_format(&self, scope: Scope, bus: u32) -> Option<StreamFormat> {
        match scope {
            Scope::Input => self.inputs.get(bus as usize).copied().flatten(),
            Scope::Output => self.outputs.get(bus as usize).copied().flatten(),
            Scope::Global => None,
        }
    }

    /// Attaches a callback to an input bus, or detaches it with `None`. Allowed while initialized.
    pub fn set_render_callback(&mut self, bus: u32, callback: Option<RenderCallback>) -> Result<(), RenderError> {
        let slot = self.callbacks.get_mut(bus as usize).ok_or(RenderError::InvalidBus)?;
        *slot = callback;
        Ok(())
    }

    /// Adds a render notification and returns its id for `remove_render_notify`.
    pub fn add_render_notify(&mut self, notify: RenderNotify) -> usize {
        self.notify.push(Some(notify));
        self.notify.len() - 1
    }

    /// Removes a notification. Removing the same id twice is an error.
    pub fn remove_render_notify(&mut self, id: usize) -> Result<(), RenderError> {
        let slot = self.notify.get_mut(id).ok_or(RenderError::InvalidValue)?;
        if slot.take().is_none() {
            return Err(RenderError::InvalidValue);
        }
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Checks the setup and marks the unit initialized. The output bus 0 needs a format. Every bus with a
    /// callback needs an input format at the same sample rate as the output.
    pub fn initialize(&mut self) -> Result<(), RenderError> {
        if self.initialized {
            return Err(RenderError::AlreadyInitialized);
        }
        let output = self.outputs[0].ok_or(RenderError::InvalidFormat)?;
        for bus in 0..self.buses as usize {
            if self.callbacks[bus].is_none() {
                continue;
            }
            let input = self.inputs[bus].ok_or(RenderError::InvalidFormat)?;
            if input.sample_rate != output.sample_rate {
                return Err(RenderError::InvalidFormat);
            }
        }
        self.initialized = true;
        Ok(())
    }

    pub fn uninitialize(&mut self) -> Result<(), RenderError> {
        if !self.initialized {
            return Err(RenderError::NotInitialized);
        }
        self.initialized = false;
        Ok(())
    }

    fn notify_all(&mut self, flags: RenderFlags, frames: usize, sample_time: f64) {
        for notify in self.notify.iter_mut().flatten() {
            notify(flags, frames, sample_time);
        }
    }

    /// Produces `frames` frames of output at `sample_time`. Pre notifications run first, then every input
    /// callback fills its block, the blocks are mixed into the output, and post notifications run.
    pub fn render(&mut self, frames: usize, sample_time: f64) -> Result<Vec<Vec<f32>>, RenderError> {
        if !self.initialized {
            return Err(RenderError::NotInitialized);
        }
        if frames == 0 || !sample_time.is_finite() || sample_time < 0.0 {
            return Err(RenderError::InvalidValue);
        }
        if frames > self.max_frames {
            return Err(RenderError::TooManyFrames);
        }
        let output = self.outputs[0].ok_or(RenderError::InvalidFormat)?;
        self.notify_all(RenderFlags { pre_render: true, ..RenderFlags::default() }, frames, sample_time);
        let mut out = planar(output.channels as usize, frames);
        for bus in 0..self.buses as usize {
            let Some(callback) = self.callbacks[bus].as_mut() else { continue };
            let input = self.inputs[bus].ok_or(RenderError::InvalidFormat)?;
            let mut block = planar(input.channels as usize, frames);
            callback(&mut RenderBlock { bus: bus as u32, frames, sample_time, channels: &mut block })?;
            for (c, dst) in out.iter_mut().enumerate() {
                let src = &block[c % block.len()];
                for (d, s) in dst.iter_mut().zip(src.iter()) {
                    *d += *s;
                }
            }
        }
        let silent = out.iter().all(|c| c.iter().all(|s| *s == 0.0));
        self.notify_all(
            RenderFlags { post_render: true, output_is_silence: silent, ..RenderFlags::default() },
            frames,
            sample_time,
        );
        Ok(out)
    }
}

/// Starts and stops the pull of a render unit. The pull position counts frames from the last start.
pub struct IoUnit {
    unit: RenderUnit,
    running: bool,
    position: u64,
}

impl IoUnit {
    pub fn new(unit: RenderUnit) -> Self {
        IoUnit { unit, running: false, position: 0 }
    }

    pub fn unit(&self) -> &RenderUnit {
        &self.unit
    }

    pub fn unit_mut(&mut self) -> &mut RenderUnit {
        &mut self.unit
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Starts the pull. The unit must be initialized. The position restarts at 0.
    pub fn start(&mut self) -> Result<(), RenderError> {
        if !self.unit.is_initialized() {
            return Err(RenderError::NotInitialized);
        }
        if self.running {
            return Err(RenderError::AlreadyRunning);
        }
        self.running = true;
        self.position = 0;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), RenderError> {
        if !self.running {
            return Err(RenderError::NotRunning);
        }
        self.running = false;
        Ok(())
    }

    /// Frames pulled since the last start.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Pulls one block at the current position. Fails while stopped.
    pub fn pull(&mut self, frames: usize) -> Result<Vec<Vec<f32>>, RenderError> {
        if !self.running {
            return Err(RenderError::NotRunning);
        }
        let out = self.unit.render(frames, self.position as f64)?;
        self.position += frames as u64;
        Ok(out)
    }

    /// Stops the pull if it runs, then uninitializes the unit.
    pub fn uninitialize(&mut self) -> Result<(), RenderError> {
        self.running = false;
        self.unit.uninitialize()
    }
}

/// Four-character code that matches any value when it is empty or all zero bytes.
const WILDCARD: [u8; 4] = [0; 4];

/// Parses a four-character ASCII code. The empty string is the wildcard.
pub fn four_cc(text: &str) -> Result<[u8; 4], RenderError> {
    if text.is_empty() {
        return Ok(WILDCARD);
    }
    let b = text.as_bytes();
    if b.len() != 4 || !b.iter().all(|c| c.is_ascii_graphic()) {
        return Err(RenderError::InvalidValue);
    }
    Ok([b[0], b[1], b[2], b[3]])
}

/// What a registered component is: its kind (for example an effect or a generator), its sub-kind and its
/// manufacturer, each a four-character code. A query field that is a wildcard matches any value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentDescription {
    pub kind: [u8; 4],
    pub sub_kind: [u8; 4],
    pub manufacturer: [u8; 4],
}

impl ComponentDescription {
    /// Builds a description from three four-character codes. An empty code is a wildcard.
    pub fn new(kind: &str, sub_kind: &str, manufacturer: &str) -> Result<Self, RenderError> {
        Ok(ComponentDescription {
            kind: four_cc(kind)?,
            sub_kind: four_cc(sub_kind)?,
            manufacturer: four_cc(manufacturer)?,
        })
    }

    /// True when every field of `self` is a wildcard or equal to the matching field of `candidate`.
    pub fn matches(&self, candidate: &ComponentDescription) -> bool {
        let field = |q: [u8; 4], c: [u8; 4]| q == WILDCARD || q == c;
        field(self.kind, candidate.kind)
            && field(self.sub_kind, candidate.sub_kind)
            && field(self.manufacturer, candidate.manufacturer)
    }
}

/// Builds a fresh render unit for a registered component.
pub type ComponentFactory = fn() -> RenderUnit;

struct Entry {
    description: ComponentDescription,
    name: String,
    factory: ComponentFactory,
}

/// The components a host can find and create. Entries keep their registration order.
#[derive(Default)]
pub struct ComponentRegistry {
    entries: Vec<Entry>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, description: ComponentDescription, name: &str, factory: ComponentFactory) {
        self.entries.push(Entry { description, name: name.to_string(), factory });
    }

    /// The index of the first matching entry after `after` (or from the start when `None`).
    pub fn find_next(&self, query: &ComponentDescription, after: Option<usize>) -> Option<usize> {
        let start = after.map_or(0, |i| i + 1);
        (start..self.entries.len()).find(|&i| query.matches(&self.entries[i].description))
    }

    pub fn name(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(|e| e.name.as_str())
    }

    /// A new render unit built by the entry's factory.
    pub fn instantiate(&self, index: usize) -> Result<RenderUnit, RenderError> {
        let entry = self.entries.get(index).ok_or(RenderError::NoComponent)?;
        Ok((entry.factory)())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn stereo() -> StreamFormat {
        StreamFormat::new(48_000.0, 2).unwrap()
    }

    fn filled(value: f32) -> RenderCallback {
        Box::new(move |block: &mut RenderBlock<'_>| {
            for channel in block.channels.iter_mut() {
                for s in channel.iter_mut() {
                    *s = value;
                }
            }
            Ok(())
        })
    }

    fn configured(buses: u32) -> RenderUnit {
        let mut unit = RenderUnit::new(buses).unwrap();
        unit.set_stream_format(Scope::Output, 0, stereo()).unwrap();
        unit
    }

    #[test]
    fn render_needs_initialize_and_an_output_format() {
        let mut unit = RenderUnit::new(1).unwrap();
        assert_eq!(unit.render(16, 0.0), Err(RenderError::NotInitialized));
        assert_eq!(unit.initialize(), Err(RenderError::InvalidFormat));
        unit.set_stream_format(Scope::Output, 0, stereo()).unwrap();
        unit.initialize().unwrap();
        let out = unit.render(16, 0.0).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| c.len() == 16 && c.iter().all(|s| *s == 0.0)));
    }

    #[test]
    fn output_is_the_sum_of_the_input_buses() {
        let mut unit = configured(2);
        unit.set_stream_format(Scope::Input, 0, stereo()).unwrap();
        unit.set_stream_format(Scope::Input, 1, stereo()).unwrap();
        unit.set_render_callback(0, Some(filled(0.25))).unwrap();
        unit.set_render_callback(1, Some(filled(0.5))).unwrap();
        unit.initialize().unwrap();
        let out = unit.render(8, 0.0).unwrap();
        assert!(out.iter().all(|c| c.iter().all(|s| (*s - 0.75).abs() < 1e-6)));
    }

    #[test]
    fn frame_limit_and_locks_hold_while_initialized() {
        let mut unit = configured(1);
        assert_eq!(unit.set_max_frames(0), Err(RenderError::InvalidValue));
        unit.set_max_frames(32).unwrap();
        unit.initialize().unwrap();
        assert_eq!(unit.render(33, 0.0), Err(RenderError::TooManyFrames));
        assert_eq!(unit.set_max_frames(64), Err(RenderError::PropertyLocked));
        assert_eq!(unit.set_stream_format(Scope::Output, 0, stereo()), Err(RenderError::PropertyLocked));
        unit.uninitialize().unwrap();
        unit.set_max_frames(64).unwrap();
        assert_eq!(unit.max_frames(), 64);
    }

    #[test]
    fn io_unit_starts_pulls_and_stops() {
        let mut unit = configured(1);
        assert_eq!(IoUnit::new(configured(1)).start(), Err(RenderError::NotInitialized));
        unit.initialize().unwrap();
        let mut io = IoUnit::new(unit);
        assert_eq!(io.pull(4), Err(RenderError::NotRunning));
        io.start().unwrap();
        assert_eq!(io.start(), Err(RenderError::AlreadyRunning));
        io.pull(4).unwrap();
        io.pull(4).unwrap();
        assert_eq!(io.position(), 8);
        io.stop().unwrap();
        assert_eq!(io.stop(), Err(RenderError::NotRunning));
        io.start().unwrap();
        assert_eq!(io.position(), 0);
    }

    #[test]
    fn callbacks_see_the_running_sample_time() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&seen);
        let mut unit = configured(1);
        unit.set_stream_format(Scope::Input, 0, stereo()).unwrap();
        let callback: RenderCallback = Box::new(move |block: &mut RenderBlock<'_>| {
            log.borrow_mut().push(block.sample_time);
            Ok(())
        });
        unit.set_render_callback(0, Some(callback)).unwrap();
        unit.initialize().unwrap();
        let mut io = IoUnit::new(unit);
        io.start().unwrap();
        io.pull(4).unwrap();
        io.pull(4).unwrap();
        assert_eq!(*seen.borrow(), vec![0.0, 4.0]);
    }

    #[test]
    fn notifications_bracket_each_render_with_silence_flag() {
        let flags = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&flags);
        let mut unit = configured(1);
        let id = unit.add_render_notify(Box::new(move |f: RenderFlags, frames: usize, _t: f64| {
            log.borrow_mut().push((f.pre_render, f.post_render, f.output_is_silence, frames));
        }));
        unit.initialize().unwrap();
        unit.render(8, 0.0).unwrap();
        assert_eq!(*flags.borrow(), vec![(true, false, false, 8), (false, true, true, 8)]);
        unit.remove_render_notify(id).unwrap();
        assert_eq!(unit.remove_render_notify(id), Err(RenderError::InvalidValue));
        unit.render(8, 8.0).unwrap();
        assert_eq!(flags.borrow().len(), 2);
    }

    #[test]
    fn stream_format_rejects_bad_rates_and_channel_counts() {
        assert_eq!(StreamFormat::new(0.0, 2), Err(RenderError::InvalidFormat));
        assert_eq!(StreamFormat::new(f64::NAN, 2), Err(RenderError::InvalidFormat));
        assert_eq!(StreamFormat::new(48_000.0, 0), Err(RenderError::InvalidFormat));
        assert_eq!(StreamFormat::new(48_000.0, 33), Err(RenderError::InvalidFormat));
        assert!(StreamFormat::new(48_000.0, 32).is_ok());
    }

    #[test]
    fn callback_bus_must_match_the_output_sample_rate() {
        let mut unit = configured(1);
        unit.set_stream_format(Scope::Input, 0, StreamFormat::new(44_100.0, 2).unwrap()).unwrap();
        unit.set_render_callback(0, Some(filled(1.0))).unwrap();
        assert_eq!(unit.initialize(), Err(RenderError::InvalidFormat));
    }

    #[test]
    fn registry_finds_by_wildcard_and_instantiates() {
        fn reverb() -> RenderUnit {
            configured(1)
        }
        fn delay() -> RenderUnit {
            configured(2)
        }
        assert_eq!(ComponentDescription::new("aufxx", "", ""), Err(RenderError::InvalidValue));
        let mut registry = ComponentRegistry::new();
        registry.register(ComponentDescription::new("aufx", "rvrb", "Bfst").unwrap(), "Reverb", reverb);
        registry.register(ComponentDescription::new("aufx", "dely", "Bfst").unwrap(), "Delay", delay);
        registry.register(ComponentDescription::new("aumu", "syn1", "Bfst").unwrap(), "Synth", reverb);
        let effects = ComponentDescription::new("aufx", "", "").unwrap();
        let first = registry.find_next(&effects, None);
        assert_eq!(first, Some(0));
        assert_eq!(registry.find_next(&effects, first), Some(1));
        assert_eq!(registry.find_next(&effects, Some(1)), None);
        assert_eq!(registry.name(1), Some("Delay"));
        let unit = registry.instantiate(1).unwrap();
        assert_eq!(unit.stream_format(Scope::Output, 0), Some(stereo()));
        assert_eq!(registry.instantiate(9).err(), Some(RenderError::NoComponent));
    }
}
