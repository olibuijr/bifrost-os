//! The control-side engine: builds the graph (player, mixer, effect and output nodes, taps), renders it by hand or
//! offline, and runs it on a device through `bifrost-audio-hal`.
//!
//! Topology (attach, connect, detach, taps) can change until the graph is handed to a device IO procedure; after
//! that only the atomics-backed controls move (node gain and mute, edge gains, player transport, effect
//! parameters, MIDI), which is what the real-time thread can pick up safely.

use crate::graph::{Graph, Kind, Node, PlayerRt, MAX_CHANNELS};
use crate::rt::{EdgeHandle, MidiHandle, MidiQueue, NodeHandle, PlayerHandle, Tap, TapRing};
use bifrost_audio_hal::{Device, IoProcConfig, IoProcId};
use bifrost_audio_types::{AudioError, BufferList};
use bifrost_audio_units::{AudioUnit, ParamHandle, UnitError, UnitHost, UnitKind};
use std::sync::Arc;

pub type NodeId = usize;

/// The output node (always present).
pub const OUTPUT: NodeId = 0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineConfig {
    pub sample_rate: f64,
    pub channels: u32,
    /// The largest block one render call covers (device buffers larger than this render in chunks).
    pub max_frames: u32,
}

impl Default for EngineConfig {
    fn default() -> EngineConfig {
        EngineConfig { sample_rate: 48_000.0, channels: 2, max_frames: 512 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EngineError {
    /// Rate, channel count or block size out of range.
    Config,
    NoNode,
    /// The connection would make the graph loop.
    Cycle,
    /// Into a player or an instrument, out of the output, or onto itself.
    BadConnection,
    /// A player's rate differs from the engine's, or the unit cannot handle the engine's channels.
    FormatMismatch,
    /// The graph is running on a device; topology cannot change.
    Frozen,
    NotPrepared,
    Unit(UnitError),
    Device(AudioError),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Config => f.write_str("the engine configuration is out of range"),
            EngineError::NoNode => f.write_str("no such node"),
            EngineError::Cycle => f.write_str("the connection would make the graph loop"),
            EngineError::BadConnection => f.write_str("the connection is not allowed"),
            EngineError::FormatMismatch => f.write_str("the format does not match the engine's"),
            EngineError::Frozen => f.write_str("the graph is running on a device; topology is frozen"),
            EngineError::NotPrepared => f.write_str("the engine is not prepared"),
            EngineError::Unit(e) => write!(f, "unit: {e}"),
            EngineError::Device(e) => write!(f, "device: {e:?}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<UnitError> for EngineError {
    fn from(e: UnitError) -> EngineError {
        EngineError::Unit(e)
    }
}

impl From<AudioError> for EngineError {
    fn from(e: AudioError) -> EngineError {
        EngineError::Device(e)
    }
}

fn log(msg: &str) {
    eprintln!("is.olibuijr.audio/engine: {msg}");
}

struct DeviceRun {
    device: Device,
    id: IoProcId,
    running: bool,
}

pub struct Engine {
    cfg: EngineConfig,
    graph: Option<Graph>,
    run: Option<DeviceRun>,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Result<Engine, EngineError> {
        if !(8_000.0..=768_000.0).contains(&cfg.sample_rate)
            || cfg.channels == 0
            || cfg.channels as usize > MAX_CHANNELS
            || cfg.max_frames == 0
            || cfg.max_frames > 65_536
        {
            return Err(EngineError::Config);
        }
        Ok(Engine { cfg, graph: Some(Graph::new(cfg.sample_rate, cfg.channels as usize, cfg.max_frames as usize)), run: None })
    }

    pub fn config(&self) -> EngineConfig {
        self.cfg
    }

    fn graph_mut(&mut self) -> Result<&mut Graph, EngineError> {
        self.graph.as_mut().ok_or(EngineError::Frozen)
    }

    fn graph_ref(&self) -> Result<&Graph, EngineError> {
        self.graph.as_ref().ok_or(EngineError::Frozen)
    }

    fn live(g: &Graph, id: NodeId) -> Result<(), EngineError> {
        match g.nodes.get(id) {
            Some(n) if !n.is_free() => Ok(()),
            _ => Err(EngineError::NoNode),
        }
    }

    /// Nodes attached, the output included.
    pub fn attached_count(&self) -> usize {
        self.graph.as_ref().map_or(0, |g| g.nodes.iter().filter(|n| !n.is_free()).count())
    }

    fn add(&mut self, node: Node) -> Result<NodeId, EngineError> {
        let g = self.graph_mut()?;
        g.prepared = false;
        if let Some(i) = g.nodes.iter().position(|n| n.is_free()) {
            g.nodes[i] = node;
            Ok(i)
        } else {
            g.nodes.push(node);
            Ok(g.nodes.len() - 1)
        }
    }

    /// A player over interleaved samples at the engine's rate.
    pub fn attach_player(&mut self, sample_rate: f64, channels: u32, interleaved: &[f32]) -> Result<(NodeId, PlayerHandle), EngineError> {
        if sample_rate != self.cfg.sample_rate {
            return Err(EngineError::FormatMismatch);
        }
        if channels == 0 || channels as usize > MAX_CHANNELS || interleaved.len() % channels as usize != 0 {
            return Err(EngineError::Config);
        }
        let ch = channels as usize;
        let frames = interleaved.len() / ch;
        let samples: Vec<Vec<f32>> = (0..ch).map(|c| (0..frames).map(|i| interleaved[i * ch + c]).collect()).collect();
        let ctl = PlayerHandle::new(frames as u64);
        let id = self.add(Node::new(Kind::Player(PlayerRt { samples, frames, ctl: ctl.clone() }), ch))?;
        Ok((id, ctl))
    }

    pub fn attach_mixer(&mut self) -> Result<NodeId, EngineError> {
        let ch = self.cfg.channels as usize;
        self.add(Node::new(Kind::Mixer, ch))
    }

    /// An effect, instrument or generator node. Returns its parameter handle and, for units that take MIDI, a
    /// MIDI handle.
    pub fn attach_unit(&mut self, unit: Box<dyn AudioUnit>) -> Result<(NodeId, ParamHandle, Option<MidiHandle>), EngineError> {
        if unit.info().max_channels < self.cfg.channels {
            return Err(EngineError::FormatMismatch);
        }
        let params = unit.params();
        let instrument = unit.info().kind != UnitKind::Effect;
        let queue = unit.info().accepts_midi.then(|| Arc::new(MidiQueue::new()));
        let ch = self.cfg.channels as usize;
        let kind = Kind::Effect { host: UnitHost::new(unit), midi: queue.clone(), events: Vec::new(), instrument };
        let id = self.add(Node::new(kind, ch))?;
        Ok((id, params, queue.map(MidiHandle)))
    }

    /// Removes a node and every connection to or from it.
    pub fn detach(&mut self, id: NodeId) -> Result<(), EngineError> {
        if id == OUTPUT {
            return Err(EngineError::BadConnection);
        }
        let g = self.graph_mut()?;
        Self::live(g, id)?;
        for n in g.nodes.iter_mut() {
            n.inputs.retain(|e| e.from != id);
        }
        g.nodes[id] = Node::new(Kind::Free, 0);
        g.prepared = false;
        Ok(())
    }

    /// Feeds `from` into `to` at unity gain; the handle changes the connection's gain. Connecting twice returns
    /// the existing connection.
    pub fn connect(&mut self, from: NodeId, to: NodeId) -> Result<EdgeHandle, EngineError> {
        let g = self.graph_mut()?;
        Self::live(g, from)?;
        Self::live(g, to)?;
        let into_source = matches!(g.nodes[to].kind, Kind::Player(_))
            || matches!(&g.nodes[to].kind, Kind::Effect { instrument: true, .. });
        if from == to || from == OUTPUT || into_source {
            return Err(EngineError::BadConnection);
        }
        if let Some(h) = g.edge_handle(to, from) {
            return Ok(h);
        }
        if g.depends_on(from, to) {
            return Err(EngineError::Cycle);
        }
        let gain = Arc::new(std::sync::atomic::AtomicU32::new(1.0f32.to_bits()));
        g.nodes[to].inputs.push(crate::graph::Edge { from, gain: gain.clone() });
        g.prepared = false;
        Ok(EdgeHandle(gain))
    }

    pub fn disconnect(&mut self, from: NodeId, to: NodeId) -> Result<(), EngineError> {
        let g = self.graph_mut()?;
        Self::live(g, to)?;
        let before = g.nodes[to].inputs.len();
        g.nodes[to].inputs.retain(|e| e.from != from);
        if g.nodes[to].inputs.len() == before {
            return Err(EngineError::NoNode);
        }
        g.prepared = false;
        Ok(())
    }

    /// Gain and mute of a node's output.
    pub fn node(&self, id: NodeId) -> Result<NodeHandle, EngineError> {
        let g = self.graph_ref()?;
        Self::live(g, id)?;
        Ok(NodeHandle(g.nodes[id].ctl.clone()))
    }

    /// A tap on a node's output: a ring of `capacity_frames` frames the render thread fills and a normal thread
    /// drains.
    pub fn attach_tap(&mut self, id: NodeId, capacity_frames: usize) -> Result<Tap, EngineError> {
        if capacity_frames == 0 {
            return Err(EngineError::Config);
        }
        let g = self.graph_mut()?;
        Self::live(g, id)?;
        let ring = Arc::new(TapRing::new(g.nodes[id].channels, capacity_frames));
        g.nodes[id].taps.push(ring.clone());
        Ok(Tap(ring))
    }

    /// Orders the graph, allocates its buffers and initializes its units. Not real-time safe.
    pub fn prepare(&mut self) -> Result<(), EngineError> {
        let g = self.graph_mut()?;
        g.prepare()?;
        Ok(())
    }

    pub fn is_prepared(&self) -> bool {
        self.graph.as_ref().map_or(self.run.is_some(), |g| g.is_prepared())
    }

    /// Frames from a source to the output through the slowest chain of units.
    pub fn latency_frames(&self) -> Result<u32, EngineError> {
        let g = self.graph_ref()?;
        if !g.is_prepared() {
            return Err(EngineError::NotPrepared);
        }
        Ok(g.latency_frames())
    }

    /// Frames of sound left after all sources end.
    pub fn tail_frames(&self) -> Result<u32, EngineError> {
        let g = self.graph_ref()?;
        if !g.is_prepared() {
            return Err(EngineError::NotPrepared);
        }
        Ok(g.tail_frames())
    }

    /// Manual rendering of one block (`frames <= max_frames`), real-time safe once prepared.
    pub fn render_block(&mut self, frames: usize, out: &mut [&mut [f32]]) -> Result<(), EngineError> {
        let g = self.graph_mut()?;
        if !g.is_prepared() {
            return Err(EngineError::NotPrepared);
        }
        Ok(g.render_block(frames, out)?)
    }

    /// Offline rendering of `frames` frames, faster than real time, deterministic: the same graph and controls
    /// give bit-identical output on every run and for every block size.
    pub fn render_offline(&mut self, frames: usize) -> Result<Vec<Vec<f32>>, EngineError> {
        let g = self.graph_mut()?;
        if !g.is_prepared() {
            g.prepare()?;
        }
        let mut out = vec![vec![0.0f32; frames]; g.channels()];
        let mut slices: Vec<&mut [f32]> = out.iter_mut().map(|v| &mut v[..]).collect();
        g.render_planar(frames, &mut slices)?;
        Ok(out)
    }

    /// Runs the graph on `device` at `buffer_frames` per cycle (0 = the device's). The first call hands the graph
    /// to the IO procedure (topology freezes); later calls restart it.
    pub fn start_on_device(&mut self, device: &Device, buffer_frames: u32) -> Result<(), EngineError> {
        if let Some(run) = &mut self.run {
            run.device.start(run.id)?;
            run.running = true;
            return Ok(());
        }
        self.prepare()?;
        let mut graph = self.graph.take().ok_or(EngineError::Frozen)?;
        let channels = graph.channels() as u32;
        let cfg = IoProcConfig { input_channels: 0, output_channels: channels, buffer_frames };
        let proc: bifrost_audio_hal::IoProc = Box::new(move |_ctx, _now, _input, _in_ts, output, _out_ts| {
            if render_into(&mut graph, output).is_err() {
                output.clear();
            }
        });
        let id = match device.add_io_proc(cfg, proc) {
            Ok(id) => id,
            Err(e) => {
                log(&format!("add_io_proc failed: {e:?}"));
                return Err(e.into());
            }
        };
        device.start(id)?;
        log(&format!("started on device {:?} at {} frames", device.id, buffer_frames));
        self.run = Some(DeviceRun { device: device.clone(), id, running: true });
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), EngineError> {
        if let Some(run) = &mut self.run {
            if run.running {
                run.device.stop(run.id)?;
                run.running = false;
            }
        }
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.running)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(run) = self.run.take() {
            let _ = run.device.stop(run.id);
            let _ = run.device.remove_io_proc(run.id);
        }
    }
}

/// Renders into a device buffer list: one buffer per channel (non-interleaved) or one interleaved buffer.
pub(crate) fn render_into(graph: &mut Graph, out: &mut BufferList) -> Result<(), UnitError> {
    let ch = graph.channels();
    if out.buffers.len() == ch && out.buffers.iter().all(|b| b.channels == 1) {
        let frames = out.buffers.iter().map(|b| b.data.len()).min().unwrap_or(0);
        let mut tmp: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| -> &mut [f32] { &mut [] });
        for (t, b) in tmp.iter_mut().zip(out.buffers.iter_mut()) {
            *t = &mut b.data[..frames];
        }
        graph.render_planar(frames, &mut tmp[..ch])
    } else if out.buffers.len() == 1 && out.buffers[0].channels as usize == ch {
        let frames = out.buffers[0].data.len() / ch;
        graph.render_interleaved(frames, &mut out.buffers[0].data)
    } else {
        Err(UnitError::BadFormat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::Buffer;
    use bifrost_audio_units::builtin::{Biquad, Gain, SineSynth, BQ_LOW_PASS, BQ_TYPE, GAIN_GAIN};
    use bifrost_audio_units::MidiEvent;

    fn engine(channels: u32) -> Engine {
        Engine::new(EngineConfig { sample_rate: 48_000.0, channels, max_frames: 8 }).unwrap()
    }

    fn mono_player(e: &mut Engine, s: &[f32]) -> (NodeId, PlayerHandle) {
        let (id, h) = e.attach_player(48_000.0, 1, s).unwrap();
        h.play();
        (id, h)
    }

    #[test]
    fn config_is_validated() {
        let bad = |c: EngineConfig| Engine::new(c).is_err();
        assert!(bad(EngineConfig { sample_rate: 1.0, channels: 2, max_frames: 8 }));
        assert!(bad(EngineConfig { sample_rate: 48_000.0, channels: 0, max_frames: 8 }));
        assert!(bad(EngineConfig { sample_rate: 48_000.0, channels: 33, max_frames: 8 }));
        assert!(bad(EngineConfig { sample_rate: 48_000.0, channels: 2, max_frames: 0 }));
    }

    #[test]
    fn a_mono_player_feeds_both_output_channels() {
        let mut e = engine(2);
        let (p, _) = mono_player(&mut e, &[1.0, 2.0, 3.0, 4.0]);
        e.connect(p, OUTPUT).unwrap();
        let out = e.render_offline(4).unwrap();
        assert_eq!(out, vec![vec![1.0, 2.0, 3.0, 4.0], vec![1.0, 2.0, 3.0, 4.0]]);
    }

    #[test]
    fn a_stereo_player_keeps_its_channels() {
        let mut e = engine(2);
        let (p, h) = e.attach_player(48_000.0, 2, &[1.0, -1.0, 2.0, -2.0]).unwrap();
        h.play();
        e.connect(p, OUTPUT).unwrap();
        assert_eq!(e.render_offline(2).unwrap(), vec![vec![1.0, 2.0], vec![-1.0, -2.0]]);
    }

    #[test]
    fn mixer_sums_inputs_with_edge_gains() {
        let mut e = engine(1);
        let (a, _) = mono_player(&mut e, &[1.0, 1.0, 1.0, 1.0]);
        let (b, _) = mono_player(&mut e, &[0.5, 1.0, 1.5, 2.0]);
        let m = e.attach_mixer().unwrap();
        e.connect(a, m).unwrap();
        let eb = e.connect(b, m).unwrap();
        e.connect(m, OUTPUT).unwrap();
        eb.set_gain(2.0);
        assert_eq!(e.render_offline(4).unwrap()[0], vec![2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn node_gain_ramps_linearly_across_a_block() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0; 8]);
        e.connect(p, OUTPUT).unwrap();
        e.prepare().unwrap();
        e.node(OUTPUT).unwrap().set_gain(0.0);
        // 1.0 -> 0.0 across 4 frames: steps of 0.25
        assert_eq!(e.render_offline(4).unwrap()[0], vec![0.75, 0.5, 0.25, 0.0]);
        assert_eq!(e.render_offline(4).unwrap()[0], vec![0.0; 4]);
    }

    #[test]
    fn mute_silences_and_unmute_returns() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0; 16]);
        e.connect(p, OUTPUT).unwrap();
        let h = e.node(p).unwrap();
        h.set_mute(true);
        e.prepare().unwrap();
        h.set_mute(true);
        assert_eq!(e.render_offline(8).unwrap()[0], vec![0.0; 8]);
    }

    #[test]
    fn player_loops_and_stops_at_the_end() {
        let mut e = engine(1);
        let (p, h) = mono_player(&mut e, &[1.0, 2.0, 3.0]);
        e.connect(p, OUTPUT).unwrap();
        h.set_looping(true);
        assert_eq!(e.render_offline(7).unwrap()[0], vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0]);
        h.seek(0);
        h.set_looping(false);
        assert_eq!(e.render_offline(5).unwrap()[0], vec![1.0, 2.0, 3.0, 0.0, 0.0]);
        assert!(h.is_finished() && !h.is_playing());
        assert_eq!(h.position(), 3);
    }

    #[test]
    fn player_seek_pause_and_resume() {
        let mut e = engine(1);
        let (p, h) = mono_player(&mut e, &[1.0, 2.0, 3.0, 4.0]);
        e.connect(p, OUTPUT).unwrap();
        h.seek(2);
        assert_eq!(e.render_offline(1).unwrap()[0], vec![3.0]);
        h.pause();
        assert_eq!(e.render_offline(2).unwrap()[0], vec![0.0, 0.0]);
        h.play();
        assert_eq!(e.render_offline(2).unwrap()[0], vec![4.0, 0.0]);
    }

    #[test]
    fn effect_nodes_process_audio_and_take_parameters() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0, 2.0, 3.0, 4.0]);
        let (fx, params, midi) = e.attach_unit(Box::new(Gain::new())).unwrap();
        assert!(midi.is_none());
        e.connect(p, fx).unwrap();
        e.connect(fx, OUTPUT).unwrap();
        params.set_ramped(GAIN_GAIN, 0.5, 0).unwrap();
        assert_eq!(e.render_offline(4).unwrap()[0], vec![0.5, 1.0, 1.5, 2.0]);
    }

    #[test]
    fn instrument_nodes_take_midi() {
        let mut e = engine(1);
        let (s, _, midi) = e.attach_unit(Box::new(SineSynth::new())).unwrap();
        let midi = midi.unwrap();
        e.connect(s, OUTPUT).unwrap();
        e.prepare().unwrap();
        assert_eq!(e.render_offline(8).unwrap()[0], vec![0.0; 8]);
        assert!(midi.send(MidiEvent::note_on(0, 0, 69, 127)));
        let out = e.render_offline(8).unwrap();
        assert_eq!(out[0][0], 0.0);
        assert!(out[0][1] > 0.0);
    }

    #[test]
    fn connections_are_validated() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0]);
        let m1 = e.attach_mixer().unwrap();
        let m2 = e.attach_mixer().unwrap();
        e.connect(m1, m2).unwrap();
        assert_eq!(e.connect(m2, m1).unwrap_err(), EngineError::Cycle);
        assert_eq!(e.connect(m1, m1).unwrap_err(), EngineError::BadConnection);
        assert_eq!(e.connect(m1, p).unwrap_err(), EngineError::BadConnection);
        assert_eq!(e.connect(OUTPUT, m1).unwrap_err(), EngineError::BadConnection);
        assert_eq!(e.connect(9, m1).unwrap_err(), EngineError::NoNode);
        let (syn, _, _) = e.attach_unit(Box::new(SineSynth::new())).unwrap();
        assert_eq!(e.connect(m1, syn).unwrap_err(), EngineError::BadConnection);
    }

    #[test]
    fn connecting_twice_returns_the_same_connection() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0; 4]);
        let a = e.connect(p, OUTPUT).unwrap();
        let b = e.connect(p, OUTPUT).unwrap();
        a.set_gain(0.5);
        assert_eq!(b.gain(), 0.5);
        assert_eq!(e.render_offline(2).unwrap()[0], vec![0.5, 0.5]);
    }

    #[test]
    fn player_rate_and_shape_are_checked() {
        let mut e = engine(2);
        assert_eq!(e.attach_player(44_100.0, 1, &[0.0]).unwrap_err(), EngineError::FormatMismatch);
        assert_eq!(e.attach_player(48_000.0, 2, &[0.0; 3]).unwrap_err(), EngineError::Config);
        assert_eq!(e.attach_player(48_000.0, 0, &[]).unwrap_err(), EngineError::Config);
    }

    #[test]
    fn detach_removes_connections_and_reuses_the_slot() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0; 4]);
        e.connect(p, OUTPUT).unwrap();
        assert_eq!(e.attached_count(), 2);
        e.detach(p).unwrap();
        assert_eq!(e.attached_count(), 1);
        assert_eq!(e.render_offline(2).unwrap()[0], vec![0.0, 0.0]);
        assert_eq!(e.detach(p).unwrap_err(), EngineError::NoNode);
        assert_eq!(e.detach(OUTPUT).unwrap_err(), EngineError::BadConnection);
        let (q, _) = mono_player(&mut e, &[2.0]);
        assert_eq!(q, p);
    }

    #[test]
    fn disconnect_stops_the_flow() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0; 4]);
        e.connect(p, OUTPUT).unwrap();
        e.disconnect(p, OUTPUT).unwrap();
        assert_eq!(e.disconnect(p, OUTPUT).unwrap_err(), EngineError::NoNode);
        assert_eq!(e.render_offline(2).unwrap()[0], vec![0.0, 0.0]);
    }

    #[test]
    fn taps_see_a_nodes_output() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0, 2.0, 3.0, 4.0]);
        let (fx, params, _) = e.attach_unit(Box::new(Gain::new())).unwrap();
        e.connect(p, fx).unwrap();
        e.connect(fx, OUTPUT).unwrap();
        params.set_ramped(GAIN_GAIN, 2.0, 0).unwrap();
        let pre = e.attach_tap(p, 16).unwrap();
        let post = e.attach_tap(fx, 16).unwrap();
        e.render_offline(4).unwrap();
        let (mut a, mut b) = (vec![vec![]], vec![vec![]]);
        assert_eq!(pre.read(&mut a, 16), 4);
        assert_eq!(post.read(&mut b, 16), 4);
        assert_eq!(a[0], vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(b[0], vec![2.0, 4.0, 6.0, 8.0]);
        assert_eq!(e.attach_tap(p, 0).unwrap_err(), EngineError::Config);
    }

    #[test]
    fn manual_rendering_needs_prepare_and_checks_sizes() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0; 16]);
        e.connect(p, OUTPUT).unwrap();
        let mut o = [0.0f32; 16];
        assert_eq!(e.render_block(4, &mut [&mut o[..4]]).unwrap_err(), EngineError::NotPrepared);
        e.prepare().unwrap();
        assert_eq!(e.render_block(9, &mut [&mut o[..9]]).unwrap_err(), EngineError::Unit(UnitError::TooManyFrames));
        assert_eq!(e.render_block(4, &mut [&mut o[..2]]).unwrap_err(), EngineError::Unit(UnitError::BadFormat));
        e.render_block(4, &mut [&mut o[..4]]).unwrap();
        assert_eq!(&o[..4], &[1.0; 4]);
    }

    #[test]
    fn latency_and_tail_come_from_the_units() {
        let mut e = engine(1);
        let (p, _) = mono_player(&mut e, &[1.0]);
        let (fx, _, _) = e.attach_unit(Box::new(Biquad::new())).unwrap();
        e.connect(p, fx).unwrap();
        e.connect(fx, OUTPUT).unwrap();
        assert_eq!(e.latency_frames().unwrap_err(), EngineError::NotPrepared);
        e.prepare().unwrap();
        assert_eq!(e.latency_frames().unwrap(), 0);
        assert_eq!(e.tail_frames().unwrap(), 2_400); // 50 ms at 48 kHz
    }

    /// Player, low-pass EQ, mixer, output: the task's reference graph.
    fn reference_graph(max_frames: u32) -> Engine {
        let mut e = Engine::new(EngineConfig { sample_rate: 48_000.0, channels: 2, max_frames }).unwrap();
        let tone: Vec<f32> = (0..1000).flat_map(|i| [((i * 37) % 101) as f32 / 101.0 - 0.5, ((i * 11) % 53) as f32 / 53.0 - 0.5]).collect();
        let (p, h) = e.attach_player(48_000.0, 2, &tone).unwrap();
        h.set_looping(true);
        h.play();
        let (eq, params, _) = e.attach_unit(Box::new(Biquad::new())).unwrap();
        params.set_ramped(BQ_TYPE, BQ_LOW_PASS, 0).unwrap();
        let m = e.attach_mixer().unwrap();
        e.connect(p, eq).unwrap();
        e.connect(eq, m).unwrap();
        e.connect(m, OUTPUT).unwrap();
        e
    }

    #[test]
    fn offline_rendering_is_bit_identical_across_runs() {
        let a = reference_graph(256).render_offline(5_000).unwrap();
        let b = reference_graph(256).render_offline(5_000).unwrap();
        let bits = |v: &Vec<Vec<f32>>| v.iter().flatten().map(|s| s.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&a), bits(&b));
        assert!(a[0].iter().any(|s| *s != 0.0));
    }

    #[test]
    fn offline_rendering_does_not_depend_on_the_block_size() {
        let a = reference_graph(64).render_offline(3_000).unwrap();
        let b = reference_graph(1_000).render_offline(3_000).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn rendering_in_pieces_equals_rendering_at_once() {
        let whole = reference_graph(128).render_offline(1_000).unwrap();
        let mut e = reference_graph(128);
        let mut parts = vec![vec![], vec![]];
        for n in [1usize, 99, 300, 600] {
            let p = e.render_offline(n).unwrap();
            for c in 0..2 {
                parts[c].extend_from_slice(&p[c]);
            }
        }
        assert_eq!(whole, parts);
    }

    #[test]
    fn device_buffer_lists_are_filled_planar_and_interleaved() {
        let mut e = engine(2);
        let (p, h) = e.attach_player(48_000.0, 2, &[1.0, -1.0, 2.0, -2.0, 3.0, -3.0]).unwrap();
        h.play();
        e.connect(p, OUTPUT).unwrap();
        e.prepare().unwrap();
        let g = e.graph.as_mut().unwrap();
        let mut planar = BufferList { buffers: vec![Buffer { channels: 1, data: vec![9.0; 3] }, Buffer { channels: 1, data: vec![9.0; 3] }] };
        render_into(g, &mut planar).unwrap();
        assert_eq!(planar.buffers[0].data, vec![1.0, 2.0, 3.0]);
        assert_eq!(planar.buffers[1].data, vec![-1.0, -2.0, -3.0]);
        h.seek(0);
        h.play(); // the first pass ran off the end and stopped the player
        let mut inter = BufferList { buffers: vec![Buffer { channels: 2, data: vec![9.0; 6] }] };
        render_into(g, &mut inter).unwrap();
        assert_eq!(inter.buffers[0].data, vec![1.0, -1.0, 2.0, -2.0, 3.0, -3.0]);
        let mut wrong = BufferList { buffers: vec![Buffer { channels: 3, data: vec![0.0; 6] }] };
        assert_eq!(render_into(g, &mut wrong).unwrap_err(), UnitError::BadFormat);
    }

    #[test]
    fn large_device_buffers_render_in_chunks() {
        let mut e = engine(1); // max_frames 8
        let (p, h) = e.attach_player(48_000.0, 1, &[1.0, 2.0, 3.0]).unwrap();
        h.set_looping(true);
        h.play();
        e.connect(p, OUTPUT).unwrap();
        let out = e.render_offline(20).unwrap();
        let want: Vec<f32> = (0..20).map(|i| [1.0, 2.0, 3.0][i % 3]).collect();
        assert_eq!(out[0], want);
    }
}
