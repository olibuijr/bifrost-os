//! Out-of-process hosting. The unit runs in a worker process; the host process holds a `RemoteUnit`, which is an
//! `AudioUnit` itself, so the engine does not care where a unit lives. Each cycle crosses on the same objects the
//! server uses for client IO (`bifrost-audio-shm`): the input ring carries the unit's input planes plus two
//! control planes, the output ring carries its output planes, and an activation record per side does the
//! wake-up with the futex trigger.
//!
//! Control planes (f32 slots carrying u32 bit patterns, so they travel through the ring unchanged):
//! * MIDI plane: slot `k` holds a packed message for offset `k` (a collision moves it to the next free slot);
//! * parameter plane: `[count, sample_time_lo, sample_time_hi, (id, value, ramp)...]`.
//!
//! `Transport` abstracts the crossing: `ShmTransport` is the real one; `LoopbackTransport` runs the worker in
//! the same thread and is what the tests (and offline rendering of out-of-process units) use.

use crate::host::UnitHost;
use crate::midi::MidiEvent;
use crate::param::{ParamHandle, ParamSpec, ParameterTree};
use crate::unit::{AudioUnit, Block, RenderConfig, UnitError, UnitInfo};
use bifrost_audio_shm::{futex, Activation, Ring, STATUS_TRIGGERED};

/// Parameter-plane header slots before the first triple.
pub const PARAM_HEADER: usize = 3;

/// Packs `events` into `plane` (zeroed first). Returns how many did not fit.
pub fn pack_midi(events: &[MidiEvent], plane: &mut [f32]) -> usize {
    plane.iter_mut().for_each(|s| *s = 0.0);
    let mut dropped = 0;
    for e in events {
        let mut slot = e.offset as usize;
        while slot < plane.len() && plane[slot].to_bits() != 0 {
            slot += 1;
        }
        if slot < plane.len() {
            plane[slot] = f32::from_bits(e.pack());
        } else {
            dropped += 1;
        }
    }
    dropped
}

/// Unpacks a MIDI plane into `out` (sorted by offset by construction); returns the count. Excess is dropped.
pub fn unpack_midi(plane: &[f32], out: &mut [MidiEvent]) -> usize {
    let mut n = 0;
    for (k, s) in plane.iter().enumerate() {
        if let Some(e) = MidiEvent::unpack(s.to_bits(), k as u32) {
            if n == out.len() {
                break;
            }
            out[n] = e;
            n += 1;
        }
    }
    n
}

/// Most parameter changes one plane of `len` slots carries.
pub fn param_capacity(len: usize) -> usize {
    len.saturating_sub(PARAM_HEADER) / 3
}

/// Writes the header and the first `param_capacity` changes; returns how many were written.
pub fn pack_params(changes: &[(u32, f32, u32)], sample_time: f64, plane: &mut [f32]) -> usize {
    plane.iter_mut().for_each(|s| *s = 0.0);
    if plane.len() < PARAM_HEADER {
        return 0;
    }
    let n = changes.len().min(param_capacity(plane.len()));
    let st = sample_time.to_bits();
    plane[0] = f32::from_bits(n as u32);
    plane[1] = f32::from_bits(st as u32);
    plane[2] = f32::from_bits((st >> 32) as u32);
    for (k, &(id, v, ramp)) in changes.iter().take(n).enumerate() {
        let b = PARAM_HEADER + k * 3;
        plane[b] = f32::from_bits(id);
        plane[b + 1] = v;
        plane[b + 2] = f32::from_bits(ramp);
    }
    n
}

/// Reads the sample time and calls `f(id, value, ramp)` for each change.
pub fn unpack_params(plane: &[f32], mut f: impl FnMut(u32, f32, u32)) -> f64 {
    if plane.len() < PARAM_HEADER {
        return 0.0;
    }
    let n = (plane[0].to_bits() as usize).min(param_capacity(plane.len()));
    let st = plane[1].to_bits() as u64 | (plane[2].to_bits() as u64) << 32;
    for k in 0..n {
        let b = PARAM_HEADER + k * 3;
        f(plane[b].to_bits(), plane[b + 1], plane[b + 2].to_bits());
    }
    f64::from_bits(st)
}

/// One block as the host sends it.
pub struct Wire<'a, 'b> {
    pub frames: usize,
    pub inputs: &'a [&'b [f32]],
    pub midi: &'a [f32],
    pub params: &'a [f32],
}

/// The crossing to the worker: sends the block, waits for the answer, fills `outputs`.
pub trait Transport {
    fn run<'b>(&mut self, wire: &Wire<'_, 'b>, outputs: &mut [&'b mut [f32]]) -> Result<(), UnitError>;
}

// ------------------------------------------------------------------------------------------------- RemoteUnit

/// The host-side stand-in for a unit living in a worker.
pub struct RemoteUnit<T: Transport> {
    info: UnitInfo,
    tree: ParameterTree,
    seen: Vec<u32>,
    pending: Vec<Option<(f32, u32)>>,
    changes: Vec<(u32, f32, u32)>,
    midi_plane: Vec<f32>,
    param_plane: Vec<f32>,
    latency: u32,
    tail: u32,
    transport: T,
    /// Blocks that silenced the output because the worker failed or missed its deadline.
    pub failures: u32,
    ready: bool,
}

impl<T: Transport> RemoteUnit<T> {
    /// Mirrors `unit` (its facts and parameter specs); `transport` reaches the real one.
    pub fn mirror(unit: &dyn AudioUnit, transport: T) -> RemoteUnit<T> {
        let specs: Vec<ParamSpec> = unit.tree().specs().to_vec();
        let n = specs.len();
        RemoteUnit {
            info: unit.info().clone(),
            tree: ParameterTree::new(specs).expect("specs of a valid tree"),
            seen: vec![0; n],
            pending: vec![None; n],
            changes: Vec::with_capacity(n),
            midi_plane: Vec::new(),
            param_plane: Vec::new(),
            latency: unit.latency_frames(),
            tail: unit.tail_frames(),
            transport,
            failures: 0,
            ready: false,
        }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }
}

impl<T: Transport + Send> AudioUnit for RemoteUnit<T> {
    fn info(&self) -> &UnitInfo {
        &self.info
    }
    fn tree(&self) -> &ParameterTree {
        &self.tree
    }
    fn tree_mut(&mut self) -> &mut ParameterTree {
        &mut self.tree
    }
    fn initialize(&mut self, cfg: &RenderConfig) -> Result<(), UnitError> {
        crate::unit::check_config(&self.info, cfg)?;
        let n = cfg.max_frames as usize;
        self.midi_plane = vec![0.0; n];
        self.param_plane = vec![0.0; n];
        self.ready = true;
        Ok(())
    }
    fn uninitialize(&mut self) {
        self.ready = false;
    }
    fn reset(&mut self) {}
    fn latency_frames(&self) -> u32 {
        self.latency
    }
    fn tail_frames(&self) -> u32 {
        self.tail
    }
    fn render(&mut self, block: &mut Block<'_, '_>) {
        let n = block.frames;
        let silence = |block: &mut Block<'_, '_>| block.outputs.iter_mut().for_each(|o| o[..n].iter_mut().for_each(|s| *s = 0.0));
        if !self.ready || n > self.midi_plane.len() || n < PARAM_HEADER {
            silence(block);
            return;
        }
        let pending = &mut self.pending;
        self.tree.handle().poll_changes(&mut self.seen, |id, v, r| pending[id as usize] = Some((v, r)));
        self.changes.clear();
        for (id, p) in self.pending.iter().enumerate() {
            if let Some((v, r)) = p {
                self.changes.push((id as u32, *v, *r));
            }
        }
        let sent = pack_params(&self.changes, block.sample_time, &mut self.param_plane[..n]);
        for &(id, _, _) in self.changes.iter().take(sent) {
            self.pending[id as usize] = None;
        }
        pack_midi(block.midi, &mut self.midi_plane[..n]);
        let wire = Wire { frames: n, inputs: block.inputs, midi: &self.midi_plane[..n], params: &self.param_plane[..n] };
        if self.transport.run(&wire, block.outputs).is_err() {
            self.failures = self.failures.wrapping_add(1);
            silence(block);
        }
    }
}

// ----------------------------------------------------------------------------------------------------- Worker

/// The worker side: owns the real unit in a host and serves blocks.
pub struct Worker {
    host: UnitHost,
    params: ParamHandle,
    midi: Vec<MidiEvent>,
}

impl Worker {
    pub fn new(mut host: UnitHost, cfg: RenderConfig) -> Result<Worker, UnitError> {
        host.initialize(cfg)?;
        let params = host.unit().params();
        Ok(Worker { host, params, midi: vec![MidiEvent::default(); cfg.max_frames as usize] })
    }

    pub fn host(&self) -> &UnitHost {
        &self.host
    }

    /// Applies the block's parameter changes and MIDI, then renders. No allocation.
    pub fn serve<'b>(&mut self, wire: &Wire<'_, 'b>, outputs: &mut [&'b mut [f32]]) -> Result<(), UnitError> {
        let params = &self.params;
        let st = unpack_params(wire.params, |id, v, r| {
            let _ = params.set_ramped(id, v, r);
        });
        let n = unpack_midi(wire.midi, &mut self.midi);
        self.host.render(wire.inputs, outputs, wire.frames, st, &self.midi[..n])
    }
}

/// A transport that runs the worker in the calling thread.
pub struct LoopbackTransport {
    pub worker: Worker,
}

impl Transport for LoopbackTransport {
    fn run<'b>(&mut self, wire: &Wire<'_, 'b>, outputs: &mut [&'b mut [f32]]) -> Result<(), UnitError> {
        self.worker.serve(wire, outputs)
    }
}

// --------------------------------------------------------------------------------------------- shared memory

fn now_ns() -> u64 {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

/// The host side of the shared-memory crossing. `to_worker` holds `inputs + 2` planes (inputs, MIDI, parameters),
/// `from_worker` the output planes; `me` and `peer` are the two activation records.
pub struct ShmTransport<'a> {
    pub to_worker: Ring<'a>,
    pub from_worker: Ring<'a>,
    pub me: &'a Activation,
    pub peer: &'a Activation,
    pub timeout_ns: u64,
    pub rate: u32,
}

impl Transport for ShmTransport<'_> {
    fn run<'b>(&mut self, wire: &Wire<'_, 'b>, outputs: &mut [&'b mut [f32]]) -> Result<(), UnitError> {
        let n = wire.frames;
        let mut planes: [&[f32]; 34] = [&[]; 34];
        let k = wire.inputs.len();
        if k + 2 > planes.len() {
            return Err(UnitError::BadFormat);
        }
        planes[..k].copy_from_slice(wire.inputs);
        planes[k] = wire.midi;
        planes[k + 1] = &wire.params[..n];
        if self.to_worker.write(&planes[..k + 2], n as u32) != n as u32 {
            return Err(UnitError::Remote);
        }
        let seen = self.me.wake.load(std::sync::atomic::Ordering::Acquire);
        bifrost_audio_shm::reset(self.me, n as u32, self.rate);
        bifrost_audio_shm::finish(self.me, &[self.peer], now_ns());
        while self.me.status.load(std::sync::atomic::Ordering::Acquire) != STATUS_TRIGGERED {
            if !futex::wait(&self.me.wake, seen, self.timeout_ns) {
                return Err(UnitError::Remote);
            }
        }
        if self.from_worker.read(outputs, n as u32) != n as u32 {
            return Err(UnitError::Remote);
        }
        Ok(())
    }
}

/// One cycle of the worker side over shared memory: waits for the host's trigger, reads the planes, renders,
/// writes the outputs and triggers the host. `scratch` holds `inputs + 2` planes of `max_frames` samples;
/// `out` holds the output planes (all allocated once by the caller).
pub struct ShmWorker<'a> {
    pub worker: Worker,
    pub from_host: Ring<'a>,
    pub to_host: Ring<'a>,
    pub me: &'a Activation,
    pub peer: &'a Activation,
    pub timeout_ns: u64,
    pub rate: u32,
    pub scratch: Vec<Vec<f32>>,
    pub out: Vec<Vec<f32>>,
}

impl ShmWorker<'_> {
    /// Serves one cycle. Returns `Ok(false)` on a wake-up timeout (no block arrived).
    pub fn cycle(&mut self) -> Result<bool, UnitError> {
        use std::sync::atomic::Ordering::Acquire;
        while self.me.status.load(Acquire) != STATUS_TRIGGERED {
            let seen = self.me.wake.load(Acquire);
            if self.me.status.load(Acquire) == STATUS_TRIGGERED {
                break;
            }
            if !futex::wait(&self.me.wake, seen, self.timeout_ns) {
                return Ok(false);
            }
        }
        let n = self.from_host.readable() as usize;
        let planes = self.scratch.len();
        let nin = planes - 2;
        let nout = self.out.len();
        if planes > 34 || nout > 32 || self.scratch.iter().any(|p| p.len() < n) || self.out.iter().any(|p| p.len() < n) {
            return Err(UnitError::BadFormat);
        }
        {
            let mut dst: [&mut [f32]; 34] = std::array::from_fn(|_| -> &mut [f32] { &mut [] });
            for (d, p) in dst.iter_mut().zip(self.scratch.iter_mut()) {
                *d = &mut p[..n];
            }
            self.from_host.read(&mut dst[..planes], n as u32);
        }
        let mut ins: [&[f32]; 32] = [&[]; 32];
        for (d, p) in ins.iter_mut().zip(self.scratch[..nin].iter()) {
            *d = &p[..n];
        }
        let mut outs: [&mut [f32]; 32] = std::array::from_fn(|_| -> &mut [f32] { &mut [] });
        for (d, p) in outs.iter_mut().zip(self.out.iter_mut()) {
            *d = &mut p[..n];
        }
        let wire = Wire { frames: n, inputs: &ins[..nin], midi: &self.scratch[nin][..n], params: &self.scratch[nin + 1][..n] };
        let r = self.worker.serve(&wire, &mut outs[..nout]);
        let mut outs_ro: [&[f32]; 32] = [&[]; 32];
        for (d, p) in outs_ro.iter_mut().zip(self.out.iter()) {
            *d = &p[..n];
        }
        self.to_host.write(&outs_ro[..nout], n as u32);
        bifrost_audio_shm::reset(self.me, n as u32, self.rate);
        bifrost_audio_shm::finish(self.me, &[self.peer], now_ns());
        r.map(|_| true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::{Gain, SineSynth, GAIN_GAIN};

    #[test]
    fn midi_plane_round_trips_and_collisions_move_later() {
        let ev = [MidiEvent::note_on(2, 0, 60, 100), MidiEvent::note_on(2, 0, 64, 100), MidiEvent::note_off(5, 0, 60)];
        let mut plane = [1.0f32; 8];
        assert_eq!(pack_midi(&ev, &mut plane), 0);
        let mut out = [MidiEvent::default(); 8];
        let n = unpack_midi(&plane, &mut out);
        assert_eq!(n, 3);
        assert_eq!((out[0].offset, out[0].data1), (2, 60));
        assert_eq!((out[1].offset, out[1].data1), (3, 64));
        assert_eq!((out[2].offset, out[2].is_note_off()), (5, true));
    }

    #[test]
    fn midi_plane_drops_what_does_not_fit() {
        let ev = [MidiEvent::note_on(3, 0, 60, 1), MidiEvent::note_on(3, 0, 61, 1)];
        let mut plane = [0.0f32; 4];
        assert_eq!(pack_midi(&ev, &mut plane), 1);
        let ev3 = [MidiEvent::note_on(3, 0, 60, 1), MidiEvent::note_on(3, 0, 61, 1), MidiEvent::note_on(3, 0, 62, 1)];
        assert_eq!(pack_midi(&ev3, &mut plane), 2);
    }

    #[test]
    fn param_plane_round_trips_with_sample_time() {
        let mut plane = [0.0f32; 9];
        assert_eq!(param_capacity(9), 2);
        let sent = pack_params(&[(1, 0.5, 8), (2, -1.0, 0), (3, 9.0, 1)], 123_456_789.0, &mut plane);
        assert_eq!(sent, 2);
        let mut got = vec![];
        let st = unpack_params(&plane, |i, v, r| got.push((i, v, r)));
        assert_eq!(st, 123_456_789.0);
        assert_eq!(got, vec![(1, 0.5, 8), (2, -1.0, 0)]);
    }

    #[test]
    fn hostile_param_count_is_clamped() {
        let mut plane = [0.0f32; 6];
        plane[0] = f32::from_bits(u32::MAX);
        let mut n = 0;
        unpack_params(&plane, |_, _, _| n += 1);
        assert_eq!(n, 1);
    }

    fn pair(unit: Box<dyn AudioUnit>, inp: u32, out: u32, frames: u32) -> (UnitHost, RemoteUnit<LoopbackTransport>) {
        let cfg = RenderConfig { sample_rate: 48_000.0, max_frames: frames, input_channels: inp, output_channels: out };
        let worker_unit = rebuild(&*unit);
        let mut local = UnitHost::new(unit);
        local.initialize(cfg).unwrap();
        let worker = Worker::new(UnitHost::new(worker_unit), cfg).unwrap();
        let mut remote = RemoteUnit::mirror(local.unit(), LoopbackTransport { worker });
        remote.initialize(&cfg).unwrap();
        (local, remote)
    }

    fn rebuild(u: &dyn AudioUnit) -> Box<dyn AudioUnit> {
        if u.info().name == "Gain" {
            Box::new(Gain::new())
        } else {
            Box::new(SineSynth::new())
        }
    }

    #[test]
    fn remote_gain_matches_local_gain_including_ramps() {
        let (mut local, mut remote) = pair(Box::new(Gain::new()), 1, 1, 8);
        let lh = local.unit().params();
        let rh = remote.params();
        lh.set_ramped(GAIN_GAIN, 0.0, 4).unwrap();
        rh.set_ramped(GAIN_GAIN, 0.0, 4).unwrap();
        let i = [1.0f32; 8];
        let (mut a, mut b) = ([0.0f32; 8], [0.0f32; 8]);
        local.render(&[&i], &mut [&mut a], 8, 0.0, &[]).unwrap();
        remote.render(&mut Block { frames: 8, sample_time: 0.0, inputs: &[&i], outputs: &mut [&mut b], midi: &[] });
        assert_eq!(a, b);
        assert_eq!(&a[..5], &[1.0, 0.75, 0.5, 0.25, 0.0]);
        assert_eq!(remote.failures, 0);
    }

    #[test]
    fn remote_synth_matches_local_synth_with_midi() {
        let (mut local, mut remote) = pair(Box::new(SineSynth::new()), 0, 1, 32);
        let ev = [MidiEvent::note_on(4, 0, 69, 127), MidiEvent::note_off(20, 0, 69)];
        let (mut a, mut b) = ([0.0f32; 32], [0.0f32; 32]);
        local.render(&[], &mut [&mut a], 32, 0.0, &ev).unwrap();
        remote.render(&mut Block { frames: 32, sample_time: 0.0, inputs: &[], outputs: &mut [&mut b], midi: &ev });
        assert_eq!(a, b);
        assert!(b[5] > 0.0 && b[21..].iter().all(|s| *s == 0.0));
    }

    struct Dead;
    impl Transport for Dead {
        fn run<'b>(&mut self, _: &Wire<'_, 'b>, _: &mut [&'b mut [f32]]) -> Result<(), UnitError> {
            Err(UnitError::Remote)
        }
    }

    #[test]
    fn a_dead_worker_gives_silence_and_counts_failures() {
        let g = Gain::new();
        let mut r = RemoteUnit::mirror(&g, Dead);
        r.initialize(&RenderConfig { sample_rate: 48_000.0, max_frames: 4, input_channels: 1, output_channels: 1 }).unwrap();
        let i = [1.0f32; 4];
        let mut o = [7.0f32; 4];
        r.render(&mut Block { frames: 4, sample_time: 0.0, inputs: &[&i], outputs: &mut [&mut o], midi: &[] });
        assert_eq!(o, [0.0; 4]);
        assert_eq!(r.failures, 1);
    }

    #[test]
    fn latency_and_tail_are_mirrored() {
        let b = crate::builtin::Biquad::new();
        let r = RemoteUnit::mirror(&b, Dead);
        assert_eq!(r.latency_frames(), 0);
        assert_eq!(r.tail_frames(), b.tail_frames());
    }
}
