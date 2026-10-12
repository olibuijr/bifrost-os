//! In-process hosting: validates every call against the initialized configuration, then runs the unit's render
//! block. `render` allocates nothing.

use crate::midi::MidiEvent;
use crate::unit::{check_config, AudioUnit, Block, RenderConfig, UnitError};

pub struct UnitHost {
    unit: Box<dyn AudioUnit>,
    cfg: Option<RenderConfig>,
    /// Sample time the next contiguous call is expected at, for `reset` on a discontinuity.
    expected: Option<f64>,
}

impl UnitHost {
    pub fn new(unit: Box<dyn AudioUnit>) -> UnitHost {
        UnitHost { unit, cfg: None, expected: None }
    }

    pub fn unit(&self) -> &dyn AudioUnit {
        &*self.unit
    }

    pub fn unit_mut(&mut self) -> &mut dyn AudioUnit {
        &mut *self.unit
    }

    pub fn config(&self) -> Option<RenderConfig> {
        self.cfg
    }

    pub fn is_initialized(&self) -> bool {
        self.cfg.is_some()
    }

    pub fn initialize(&mut self, cfg: RenderConfig) -> Result<(), UnitError> {
        if self.cfg.is_some() {
            return Err(UnitError::AlreadyInitialized);
        }
        check_config(self.unit.info(), &cfg)?;
        self.unit.initialize(&cfg)?;
        self.cfg = Some(cfg);
        self.expected = None;
        Ok(())
    }

    pub fn uninitialize(&mut self) -> Result<(), UnitError> {
        if self.cfg.take().is_none() {
            return Err(UnitError::NotInitialized);
        }
        self.unit.uninitialize();
        Ok(())
    }

    pub fn latency_frames(&self) -> u32 {
        self.unit.latency_frames()
    }

    pub fn tail_frames(&self) -> u32 {
        self.unit.tail_frames()
    }

    /// Renders `frames` frames. `inputs` must have the configured input channels, `outputs` the output
    /// channels, each slice at least `frames` long. A jump in `sample_time` resets the unit's state first.
    /// `midi` must be sorted by offset and offsets must be below `frames`; later events are dropped.
    pub fn render<'b>(
        &mut self,
        inputs: &[&'b [f32]],
        outputs: &mut [&'b mut [f32]],
        frames: usize,
        sample_time: f64,
        midi: &[MidiEvent],
    ) -> Result<(), UnitError> {
        let cfg = self.cfg.ok_or(UnitError::NotInitialized)?;
        if frames > cfg.max_frames as usize {
            return Err(UnitError::TooManyFrames);
        }
        if inputs.len() != cfg.input_channels as usize
            || outputs.len() != cfg.output_channels as usize
            || inputs.iter().any(|s| s.len() < frames)
            || outputs.iter().any(|s| s.len() < frames)
        {
            return Err(UnitError::BadFormat);
        }
        if let Some(e) = self.expected {
            if e != sample_time {
                self.unit.reset();
            }
        }
        self.expected = Some(sample_time + frames as f64);
        let n = midi.iter().take_while(|e| (e.offset as usize) < frames).count();
        let mut block = Block { frames, sample_time, inputs, outputs, midi: &midi[..n] };
        self.unit.render(&mut block);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::Gain;

    fn cfg() -> RenderConfig {
        RenderConfig { sample_rate: 48_000.0, max_frames: 8, input_channels: 1, output_channels: 1 }
    }

    #[test]
    fn render_before_initialize_fails() {
        let mut h = UnitHost::new(Box::new(Gain::new()));
        let i = [0.0f32; 4];
        let mut o = [0.0f32; 4];
        assert_eq!(h.render(&[&i], &mut [&mut o], 4, 0.0, &[]), Err(UnitError::NotInitialized));
    }

    #[test]
    fn initialize_twice_and_uninitialize_twice_fail() {
        let mut h = UnitHost::new(Box::new(Gain::new()));
        h.initialize(cfg()).unwrap();
        assert_eq!(h.initialize(cfg()), Err(UnitError::AlreadyInitialized));
        h.uninitialize().unwrap();
        assert_eq!(h.uninitialize(), Err(UnitError::NotInitialized));
    }

    #[test]
    fn shape_errors_are_reported() {
        let mut h = UnitHost::new(Box::new(Gain::new()));
        h.initialize(cfg()).unwrap();
        let i = [1.0f32; 16];
        let mut o = [0.0f32; 16];
        assert_eq!(h.render(&[&i], &mut [&mut o], 9, 0.0, &[]), Err(UnitError::TooManyFrames));
        assert_eq!(h.render(&[], &mut [&mut o], 4, 0.0, &[]), Err(UnitError::BadFormat));
        assert_eq!(h.render(&[&i[..2]], &mut [&mut o], 4, 0.0, &[]), Err(UnitError::BadFormat));
    }

    #[test]
    fn default_gain_passes_audio_through() {
        let mut h = UnitHost::new(Box::new(Gain::new()));
        h.initialize(cfg()).unwrap();
        let i = [0.5f32, -0.25, 1.0, 0.0];
        let mut o = [9.0f32; 4];
        h.render(&[&i], &mut [&mut o], 4, 0.0, &[]).unwrap();
        assert_eq!(o, i);
    }

    #[test]
    fn config_is_checked_against_the_unit() {
        let mut h = UnitHost::new(Box::new(Gain::new()));
        let mut c = cfg();
        c.output_channels = 2;
        assert_eq!(h.initialize(c), Err(UnitError::BadFormat));
    }
}
