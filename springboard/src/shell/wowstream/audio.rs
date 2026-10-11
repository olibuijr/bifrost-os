//! Audio for the WoW stream: Opus packets (48 kHz stereo, 10 ms each) arrive from the client
//! core; a [`Sink`] turns them into sound.
//!
//! TODO: no pure-Rust Opus decoder is available offline (the crate registry holds none, and the
//! rule is no C library), so [`open`] returns a [`Noop`] sink and the stream is silent. To finish
//! it, implement `Sink` with a decoder that fills `wowstream_client::jitter::Jitter` (target
//! 40 ms, trims to 40 ms past 80 ms) and a writer thread that pulls 10 ms from the jitter buffer
//! into `crate::media::speaker::SpeakerOut::new(device, 48_000, 2)` (device `/dev/snd/pcmC0D3p`,
//! the low-latency PCM the Music app uses); return it from `open`.

/// Receives every Opus packet of the session, in order, on the router thread (keep it quick).
pub trait Sink: Send {
    fn packet(&mut self, opus: &[u8], pts_us: u64);
}

/// Drops the audio.
pub struct Noop;

impl Sink for Noop {
    fn packet(&mut self, _opus: &[u8], _pts_us: u64) {}
}

/// The sink for a new session.
pub fn open() -> Box<dyn Sink> {
    Box::new(Noop)
}
