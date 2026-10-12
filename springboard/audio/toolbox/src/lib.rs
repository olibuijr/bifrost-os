//! `bifrost-audio-toolbox`: services one level above the client library (docs/audio-architecture.md, "Crates").
//! Module owners: `converter`, `resample`, `file`, `queue`, `system_sound` (task 13); `varispeed` (task 11, the
//! adaptive resampler that aggregates and drift compensation use). The `varispeed` API below is shared by tasks 11
//! and 13, so it is fixed here.

pub mod converter;
pub mod file;
pub mod queue;
pub mod resample;
pub mod system_sound;

/// The adaptive resampler for drift compensation: cheap, low latency (a short polyphase or cubic kernel), its ratio
/// changed every cycle from a device DLL's rate without clicks.
pub mod varispeed {
    /// Resamples one channel group at a slowly varying ratio.
    pub struct Varispeed {
        pub channels: u32,
    }

    impl Varispeed {
        pub fn new(channels: u32) -> Varispeed {
            Varispeed { channels }
        }

        /// Sets the ratio (output rate / input rate), e.g. 1.0002 for a sub-device 200 ppm slow against the clock
        /// source. Applied smoothly over the next call.
        pub fn set_ratio(&mut self, ratio: f64) {
            let _ = ratio;
            todo!("audio-11")
        }

        /// Input frames needed to produce `out_frames` frames at the current ratio and phase.
        pub fn input_needed(&self, out_frames: u32) -> u32 {
            let _ = out_frames;
            todo!("audio-11")
        }

        /// Consumes input (per-channel slices) and fills `out_frames` frames of output; returns input frames used.
        pub fn process(&mut self, input: &[&[f32]], output: &mut [&mut [f32]], out_frames: u32) -> u32 {
            let _ = (input, output, out_frames);
            todo!("audio-11")
        }

        /// The processing delay in output frames (for latency reporting).
        pub fn latency(&self) -> f64 {
            todo!("audio-11")
        }
    }
}
