//! The render-side graph. `render_block` visits the nodes in dependency order and fills pre-allocated buffers;
//! it never allocates, locks or makes a syscall. Control (gain, mute, play, seek, effect parameters, MIDI, edge
//! gains) reaches it through the atomics of `rt`.

use crate::rt::{EdgeHandle, MidiQueue, NodeControl, PlayerControl, PlayerHandle, TapRing};
use bifrost_audio_units::{MidiEvent, RenderConfig, UnitError, UnitHost, UnitKind};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// The most channels a graph carries (bounded so the render path can use stack arrays).
pub const MAX_CHANNELS: usize = 32;
/// The most MIDI messages one block hands a unit.
pub const MIDI_PER_BLOCK: usize = 64;

pub(crate) struct PlayerRt {
    pub samples: Vec<Vec<f32>>,
    pub frames: usize,
    pub ctl: PlayerHandle,
}

pub(crate) enum Kind {
    /// A detached slot (ids stay stable).
    Free,
    Output,
    Mixer,
    Player(PlayerRt),
    Effect { host: UnitHost, midi: Option<Arc<MidiQueue>>, events: Vec<MidiEvent>, instrument: bool },
    /// Temporarily moved out while it renders.
    Taken,
}

pub(crate) struct Edge {
    pub from: usize,
    pub gain: Arc<std::sync::atomic::AtomicU32>,
}

pub(crate) struct Node {
    pub kind: Kind,
    pub channels: usize,
    pub ctl: Arc<NodeControl>,
    pub inputs: Vec<Edge>,
    pub out: Vec<Vec<f32>>,
    pub scratch: Vec<Vec<f32>>,
    pub prev_gain: f32,
    pub taps: Vec<Arc<TapRing>>,
}

impl Node {
    pub fn new(kind: Kind, channels: usize) -> Node {
        Node { kind, channels, ctl: Arc::new(NodeControl::new()), inputs: Vec::new(), out: Vec::new(), scratch: Vec::new(), prev_gain: 1.0, taps: Vec::new() }
    }
    pub fn is_free(&self) -> bool {
        matches!(self.kind, Kind::Free)
    }
}

pub struct Graph {
    pub(crate) nodes: Vec<Node>,
    pub(crate) order: Vec<usize>,
    pub(crate) rate: f64,
    pub(crate) channels: usize,
    pub(crate) max_frames: usize,
    pub(crate) prepared: bool,
    /// Frames rendered since the last `reset_time`, the sample time units see.
    pub(crate) sample_time: u64,
    /// Scratch for interleaved device buffers.
    pub(crate) interleave: Vec<Vec<f32>>,
}

/// Adds `src` times `g` into `dst`: equal channel counts pair up, a mono source feeds every destination channel,
/// otherwise channel `c` feeds channel `c` and extra source channels are dropped.
fn mix(dst: &mut [Vec<f32>], src: &[Vec<f32>], g: f32, n: usize) {
    if src.len() == 1 && dst.len() > 1 {
        for d in dst.iter_mut() {
            for (o, i) in d[..n].iter_mut().zip(src[0][..n].iter()) {
                *o += *i * g;
            }
        }
    } else {
        for (d, s) in dst.iter_mut().zip(src.iter()) {
            for (o, i) in d[..n].iter_mut().zip(s[..n].iter()) {
                *o += *i * g;
            }
        }
    }
}

fn zero(planes: &mut [Vec<f32>], n: usize) {
    for p in planes.iter_mut() {
        p[..n].iter_mut().for_each(|s| *s = 0.0);
    }
}

impl Graph {
    pub(crate) fn new(rate: f64, channels: usize, max_frames: usize) -> Graph {
        Graph {
            nodes: vec![Node::new(Kind::Output, channels)],
            order: Vec::new(),
            rate,
            channels,
            max_frames,
            prepared: false,
            sample_time: 0,
            interleave: Vec::new(),
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    pub fn is_prepared(&self) -> bool {
        self.prepared
    }

    /// True when `to` is reachable from `from` following input edges backwards, i.e. `from` already depends on `to`.
    pub(crate) fn depends_on(&self, from: usize, to: usize) -> bool {
        let mut stack = vec![from];
        let mut seen = vec![false; self.nodes.len()];
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if std::mem::replace(&mut seen[n], true) {
                continue;
            }
            stack.extend(self.nodes[n].inputs.iter().map(|e| e.from));
        }
        false
    }

    /// Computes the order, allocates buffers and initializes the units. Allocation happens here, not in render.
    pub(crate) fn prepare(&mut self) -> Result<(), UnitError> {
        let order = self.deterministic_order();
        let active = self.nodes.iter().filter(|n| !n.is_free()).count();
        if order.len() != active {
            return Err(UnitError::BadFormat);
        }
        let (rate, mf) = (self.rate, self.max_frames);
        for node in self.nodes.iter_mut() {
            if node.is_free() {
                node.out = Vec::new();
                node.scratch = Vec::new();
                continue;
            }
            node.out = vec![vec![0.0; mf]; node.channels];
            node.scratch = if matches!(node.kind, Kind::Mixer | Kind::Output | Kind::Player(_)) { Vec::new() } else { vec![vec![0.0; mf]; node.channels] };
            node.prev_gain = node.ctl.target();
            if let Kind::Effect { host, instrument, events, .. } = &mut node.kind {
                if !host.is_initialized() {
                    let ch = node.channels as u32;
                    *instrument = host.unit().info().kind != UnitKind::Effect;
                    let cfg = RenderConfig { sample_rate: rate, max_frames: mf as u32, input_channels: if *instrument { 0 } else { ch }, output_channels: ch };
                    host.initialize(cfg)?;
                }
                if events.len() != MIDI_PER_BLOCK {
                    *events = vec![MidiEvent::default(); MIDI_PER_BLOCK];
                }
            }
        }
        self.order = order;
        self.interleave = vec![vec![0.0; mf]; self.channels];
        self.prepared = true;
        Ok(())
    }

    fn deterministic_order(&self) -> Vec<usize> {
        let n = self.nodes.len();
        let mut indeg: Vec<usize> = (0..n).map(|i| if self.nodes[i].is_free() { 0 } else { self.nodes[i].inputs.len() }).collect();
        let mut done = vec![false; n];
        let mut order = Vec::with_capacity(n);
        loop {
            let next = (0..n).find(|&i| !done[i] && !self.nodes[i].is_free() && indeg[i] == 0);
            let Some(i) = next else { break };
            done[i] = true;
            order.push(i);
            for j in 0..n {
                if !self.nodes[j].is_free() {
                    indeg[j] -= self.nodes[j].inputs.iter().filter(|e| e.from == i).count();
                }
            }
        }
        order
    }

    pub(crate) fn edge_handle(&self, to: usize, from: usize) -> Option<EdgeHandle> {
        self.nodes[to].inputs.iter().find(|e| e.from == from).map(|e| EdgeHandle(e.gain.clone()))
    }

    /// Longest-path latency in frames from any source to the output.
    pub fn latency_frames(&self) -> u32 {
        let mut lat = vec![0u32; self.nodes.len()];
        for &i in &self.order {
            let own = match &self.nodes[i].kind {
                Kind::Effect { host, .. } => host.latency_frames(),
                _ => 0,
            };
            lat[i] = own + self.nodes[i].inputs.iter().map(|e| lat[e.from]).max().unwrap_or(0);
        }
        lat[0]
    }

    /// Frames of sound that remain after every source ends: the longest chain of effect tails.
    pub fn tail_frames(&self) -> u32 {
        let mut tail = vec![0u32; self.nodes.len()];
        for &i in &self.order {
            let own = match &self.nodes[i].kind {
                Kind::Effect { host, .. } => host.tail_frames(),
                _ => 0,
            };
            tail[i] = own + self.nodes[i].inputs.iter().map(|e| tail[e.from]).max().unwrap_or(0);
        }
        tail[0]
    }

    /// Renders one block of `n <= max_frames` frames into `out` (one slice per channel). Real-time safe.
    pub fn render_block(&mut self, n: usize, out: &mut [&mut [f32]]) -> Result<(), UnitError> {
        if !self.prepared {
            return Err(UnitError::NotInitialized);
        }
        if n > self.max_frames {
            return Err(UnitError::TooManyFrames);
        }
        if out.len() != self.channels || out.iter().any(|s| s.len() < n) {
            return Err(UnitError::BadFormat);
        }
        for k in 0..self.order.len() {
            let idx = self.order[k];
            self.process(idx, n);
        }
        for (o, p) in out.iter_mut().zip(self.nodes[0].out.iter()) {
            o[..n].copy_from_slice(&p[..n]);
        }
        self.sample_time += n as u64;
        Ok(())
    }

    /// Renders any number of frames in chunks of `max_frames`.
    pub fn render_planar(&mut self, frames: usize, out: &mut [&mut [f32]]) -> Result<(), UnitError> {
        let ch = self.channels;
        if out.len() != ch || out.iter().any(|s| s.len() < frames) {
            return Err(UnitError::BadFormat);
        }
        let mut off = 0;
        while off < frames {
            let n = self.max_frames.min(frames - off);
            let mut tmp: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| -> &mut [f32] { &mut [] });
            for (t, o) in tmp.iter_mut().zip(out.iter_mut()) {
                *t = &mut o[off..off + n];
            }
            self.render_block(n, &mut tmp[..ch])?;
            off += n;
        }
        Ok(())
    }

    /// Renders into an interleaved buffer (`frames * channels` samples).
    pub fn render_interleaved(&mut self, frames: usize, out: &mut [f32]) -> Result<(), UnitError> {
        let ch = self.channels;
        if out.len() < frames * ch {
            return Err(UnitError::BadFormat);
        }
        let mut off = 0;
        while off < frames {
            let n = self.max_frames.min(frames - off);
            let mut bufs = std::mem::take(&mut self.interleave);
            {
                let mut tmp: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| -> &mut [f32] { &mut [] });
                for (t, b) in tmp.iter_mut().zip(bufs.iter_mut()) {
                    *t = &mut b[..n];
                }
                let r = self.render_block(n, &mut tmp[..ch]);
                if r.is_err() {
                    self.interleave = bufs;
                    return r;
                }
            }
            for i in 0..n {
                for (c, b) in bufs.iter().enumerate() {
                    out[(off + i) * ch + c] = b[i];
                }
            }
            self.interleave = bufs;
            off += n;
        }
        Ok(())
    }

    fn process(&mut self, idx: usize, n: usize) {
        let mut kind = std::mem::replace(&mut self.nodes[idx].kind, Kind::Taken);
        let mut out = std::mem::take(&mut self.nodes[idx].out);
        let mut scratch = std::mem::take(&mut self.nodes[idx].scratch);
        match &mut kind {
            Kind::Output | Kind::Mixer => {
                zero(&mut out, n);
                for e in &self.nodes[idx].inputs {
                    mix(&mut out, &self.nodes[e.from].out, f32::from_bits(e.gain.load(Ordering::Relaxed)), n);
                }
            }
            Kind::Player(p) => render_player(p, &mut out, n),
            Kind::Effect { host, midi, events, instrument } => {
                let ch = out.len();
                let ne = midi.as_ref().map_or(0, |q| q.drain(&mut events[..])) as usize;
                let st = self.sample_time as f64;
                if *instrument {
                    zero(&mut out, n);
                    let mut outs: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| -> &mut [f32] { &mut [] });
                    for (o, p) in outs.iter_mut().zip(out.iter_mut()) {
                        *o = &mut p[..n];
                    }
                    if host.render(&[], &mut outs[..ch], n, st, &events[..ne]).is_err() {
                        zero(&mut out, n);
                    }
                } else {
                    zero(&mut scratch, n);
                    for e in &self.nodes[idx].inputs {
                        mix(&mut scratch, &self.nodes[e.from].out, f32::from_bits(e.gain.load(Ordering::Relaxed)), n);
                    }
                    let mut ins: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                    for (i, p) in ins.iter_mut().zip(scratch.iter()) {
                        *i = &p[..n];
                    }
                    let mut outs: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| -> &mut [f32] { &mut [] });
                    for (o, p) in outs.iter_mut().zip(out.iter_mut()) {
                        *o = &mut p[..n];
                    }
                    if host.render(&ins[..ch], &mut outs[..ch], n, st, &events[..ne]).is_err() {
                        zero(&mut out, n);
                    }
                }
            }
            Kind::Free | Kind::Taken => {}
        }
        // Node gain: a linear ramp across the block from the previous target (identity when both are 1.0).
        let g1 = self.nodes[idx].ctl.target();
        let g0 = self.nodes[idx].prev_gain;
        if g0 != 1.0 || g1 != 1.0 {
            let step = (g1 - g0) / n as f32;
            for p in out.iter_mut() {
                for (i, s) in p[..n].iter_mut().enumerate() {
                    *s *= if g0 == g1 { g1 } else { g0 + step * (i + 1) as f32 };
                }
            }
        }
        self.nodes[idx].prev_gain = g1;
        for t in &self.nodes[idx].taps {
            t.write(&out, n);
        }
        self.nodes[idx].kind = kind;
        self.nodes[idx].out = out;
        self.nodes[idx].scratch = scratch;
    }
}

fn render_player(p: &mut PlayerRt, out: &mut [Vec<f32>], n: usize) {
    let c: &PlayerControl = &p.ctl.0;
    let mut pos = c.position.load(Ordering::Relaxed) as usize;
    if let Some(s) = p.ctl.take_seek() {
        pos = s as usize;
    }
    zero(out, n);
    let mut done = 0;
    if c.playing.load(Ordering::Relaxed) {
        let looping = c.looping.load(Ordering::Relaxed);
        while done < n && p.frames > 0 {
            if pos >= p.frames {
                if looping {
                    pos = 0;
                } else {
                    c.playing.store(false, Ordering::Relaxed);
                    c.finished.store(true, Ordering::Relaxed);
                    break;
                }
            }
            let k = (n - done).min(p.frames - pos);
            for (o, s) in out.iter_mut().zip(p.samples.iter()) {
                o[done..done + k].copy_from_slice(&s[pos..pos + k]);
            }
            pos += k;
            done += k;
        }
        if !looping && pos >= p.frames && p.frames > 0 && !c.finished.load(Ordering::Relaxed) {
            // Ran exactly to the end of the data: stop now so the position reads as the length.
            c.playing.store(false, Ordering::Relaxed);
            c.finished.store(true, Ordering::Relaxed);
        }
    }
    c.position.store(pos as u64, Ordering::Relaxed);
}
