//! Audio engine graph: nodes that you attach, connect into a chain that ends at the engine's output node, and pull
//! through block by block. Players hold float samples, gain nodes scale their input, and mixer nodes sum several
//! inputs. Every connection names a format that must match both ends. The graph is checked for cycles when a
//! connection is made, and only nodes upstream of the output are rendered. This is the pure model behind a node
//! graph; no device is opened here and the owner drives `render` from its own real-time thread.

/// The sample rate and channel count of a stream. Samples are interleaved floats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Index of a node in an engine. Node 0 is the output node.
pub type NodeId = usize;

/// The output node of every engine.
pub const OUTPUT: NodeId = 0;

/// Failures of the graph model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// The format is outside 8 kHz to 192 kHz or has no channels or more than 8.
    InvalidFormat,
    /// A value is not finite, or the sample count does not fit the channel count.
    InvalidValue,
    /// No node is at that index, or it was detached.
    NoNode,
    /// The node cannot take that role: a player has no input, or the output cannot be detached.
    InvalidConnection,
    /// The input of a node that takes a single input is already connected.
    InputBusy,
    /// The connection would make a node feed itself.
    Cycle,
    /// The graph cannot change while the engine runs.
    Running,
    /// The engine is not running, so it cannot render.
    NotRunning,
    /// The engine has not been prepared since the last graph change.
    NotPrepared,
}

#[derive(Clone, Debug)]
enum Kind {
    /// Plays its samples once, then renders silence.
    Player { samples: Vec<f32>, cursor: usize },
    /// Scales its input by a gain.
    Gain(f32),
    /// Sums any number of inputs.
    Mixer,
    /// The output node. Sums its single input.
    Output,
}

#[derive(Clone, Debug)]
struct Node {
    kind: Kind,
    format: Format,
    inputs: Vec<NodeId>,
}

/// A graph of audio nodes with one output.
#[derive(Debug)]
pub struct AudioEngine {
    format: Format,
    nodes: Vec<Option<Node>>,
    order: Vec<NodeId>,
    prepared: bool,
    running: bool,
}

fn valid_format(format: Format) -> Result<(), EngineError> {
    if (8_000..=192_000).contains(&format.sample_rate) && (1..=8).contains(&format.channels) {
        Ok(())
    } else {
        Err(EngineError::InvalidFormat)
    }
}

impl AudioEngine {
    /// An engine whose output node and gain and mixer nodes use `format`.
    pub fn new(format: Format) -> Result<Self, EngineError> {
        valid_format(format)?;
        let output = Node { kind: Kind::Output, format, inputs: Vec::new() };
        Ok(Self { format, nodes: vec![Some(output)], order: Vec::new(), prepared: false, running: false })
    }

    /// The format of the engine.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Number of attached nodes, including the output.
    pub fn attached_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.is_some()).count()
    }

    fn ensure_stopped(&self) -> Result<(), EngineError> {
        if self.running {
            Err(EngineError::Running)
        } else {
            Ok(())
        }
    }

    fn add(&mut self, node: Node) -> NodeId {
        self.prepared = false;
        if let Some(slot) = self.nodes.iter().skip(1).position(|n| n.is_none()) {
            let id = slot + 1;
            self.nodes[id] = Some(node);
            return id;
        }
        self.nodes.push(Some(node));
        self.nodes.len() - 1
    }

    /// Attaches a player that renders `samples` (interleaved, a whole number of frames) in `format`.
    pub fn attach_player(&mut self, format: Format, samples: Vec<f32>) -> Result<NodeId, EngineError> {
        self.ensure_stopped()?;
        valid_format(format)?;
        if samples.iter().any(|s| !s.is_finite()) || samples.len() % format.channels as usize != 0 {
            return Err(EngineError::InvalidValue);
        }
        // a player keeps its own format; gain, mixer and output nodes use the engine format
        Ok(self.add(Node { kind: Kind::Player { samples, cursor: 0 }, format, inputs: Vec::new() }))
    }

    /// Attaches a gain node. The gain must be finite.
    pub fn attach_gain(&mut self, gain: f32) -> Result<NodeId, EngineError> {
        self.ensure_stopped()?;
        if !gain.is_finite() {
            return Err(EngineError::InvalidValue);
        }
        Ok(self.add(Node { kind: Kind::Gain(gain), format: self.format, inputs: Vec::new() }))
    }

    /// Attaches a mixer node that sums all of its inputs.
    pub fn attach_mixer(&mut self) -> Result<NodeId, EngineError> {
        self.ensure_stopped()?;
        Ok(self.add(Node { kind: Kind::Mixer, format: self.format, inputs: Vec::new() }))
    }

    /// Detaches a node and removes every connection that feeds it. The output node cannot be detached.
    pub fn detach(&mut self, id: NodeId) -> Result<(), EngineError> {
        self.ensure_stopped()?;
        if id == OUTPUT {
            return Err(EngineError::InvalidConnection);
        }
        match self.nodes.get_mut(id) {
            Some(slot) if slot.is_some() => *slot = None,
            _ => return Err(EngineError::NoNode),
        }
        for node in self.nodes.iter_mut().flatten() {
            node.inputs.retain(|&u| u != id);
        }
        self.prepared = false;
        Ok(())
    }

    fn node(&self, id: NodeId) -> Result<&Node, EngineError> {
        self.nodes.get(id).and_then(|n| n.as_ref()).ok_or(EngineError::NoNode)
    }

    /// True when `target` is `start` or lies upstream of it.
    fn feeds(&self, start: NodeId, target: NodeId) -> bool {
        let mut stack = vec![start];
        let mut seen = vec![false; self.nodes.len()];
        while let Some(id) = stack.pop() {
            if id == target {
                return true;
            }
            if seen[id] {
                continue;
            }
            seen[id] = true;
            if let Some(Some(node)) = self.nodes.get(id) {
                stack.extend(node.inputs.iter().copied());
            }
        }
        false
    }

    /// Connects the output of `from` to the input of `to`. Both nodes must use `format`. A player takes no input,
    /// and a gain node or the output takes one input. A connection that would loop back is refused.
    pub fn connect(&mut self, from: NodeId, to: NodeId, format: Format) -> Result<(), EngineError> {
        self.ensure_stopped()?;
        valid_format(format)?;
        let from_node = self.node(from)?;
        let to_node = self.node(to)?;
        if from_node.format != format || to_node.format != format {
            return Err(EngineError::InvalidFormat);
        }
        match to_node.kind {
            Kind::Player { .. } => return Err(EngineError::InvalidConnection),
            Kind::Gain(_) | Kind::Output if !to_node.inputs.is_empty() => {
                return Err(EngineError::InputBusy)
            }
            _ => {}
        }
        if self.feeds(from, to) {
            return Err(EngineError::Cycle);
        }
        if let Some(Some(node)) = self.nodes.get_mut(to) {
            node.inputs.push(from);
        }
        self.prepared = false;
        Ok(())
    }

    /// Removes every connection into the node `id`.
    pub fn disconnect_input(&mut self, id: NodeId) -> Result<(), EngineError> {
        self.ensure_stopped()?;
        match self.nodes.get_mut(id) {
            Some(Some(node)) => node.inputs.clear(),
            _ => return Err(EngineError::NoNode),
        }
        self.prepared = false;
        Ok(())
    }

    /// Computes the render order: every node upstream of the output, each after its inputs.
    pub fn prepare(&mut self) {
        let mut order = Vec::new();
        let mut done = vec![false; self.nodes.len()];
        self.visit(OUTPUT, &mut done, &mut order);
        self.order = order;
        self.prepared = true;
    }

    fn visit(&self, id: NodeId, done: &mut [bool], order: &mut Vec<NodeId>) {
        if done[id] {
            return;
        }
        done[id] = true;
        if let Some(Some(node)) = self.nodes.get(id) {
            for &u in &node.inputs {
                self.visit(u, done, order);
            }
            order.push(id);
        }
    }

    /// Starts the engine. It must be prepared since the last graph change.
    pub fn start(&mut self) -> Result<(), EngineError> {
        if !self.prepared {
            return Err(EngineError::NotPrepared);
        }
        self.running = true;
        Ok(())
    }

    /// Stops the engine. The graph can change again.
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// True while the engine runs.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Renders `frames` frames of the output as interleaved samples. Players advance their cursors.
    pub fn render(&mut self, frames: usize) -> Result<Vec<f32>, EngineError> {
        if !self.running {
            return Err(EngineError::NotRunning);
        }
        let channels = self.format.channels as usize;
        let n = frames * channels;
        let mut blocks: Vec<Vec<f32>> = vec![Vec::new(); self.nodes.len()];
        let order = self.order.clone();
        for id in order {
            let node = match self.nodes.get_mut(id) {
                Some(Some(node)) => node,
                _ => continue,
            };
            let inputs = node.inputs.clone();
            let mut sum = vec![0.0f32; n];
            for u in inputs {
                for (o, s) in sum.iter_mut().zip(blocks[u].iter()) {
                    *o += *s;
                }
            }
            let out = match &mut node.kind {
                Kind::Player { samples, cursor } => {
                    let mut out = vec![0.0f32; n];
                    let avail = samples.len().saturating_sub(*cursor).min(n);
                    out[..avail].copy_from_slice(&samples[*cursor..*cursor + avail]);
                    *cursor += avail;
                    out
                }
                Kind::Gain(g) => {
                    for s in sum.iter_mut() {
                        *s *= *g;
                    }
                    sum
                }
                Kind::Mixer | Kind::Output => sum,
            };
            blocks[id] = out;
        }
        Ok(std::mem::take(&mut blocks[OUTPUT]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEREO: Format = Format { sample_rate: 48_000, channels: 2 };

    fn engine() -> AudioEngine {
        AudioEngine::new(STEREO).unwrap()
    }

    #[test]
    fn player_feeds_output_and_then_renders_silence() {
        let mut e = engine();
        let p = e.attach_player(STEREO, vec![0.1, 0.2, 0.3, 0.4]).unwrap();
        e.connect(p, OUTPUT, STEREO).unwrap();
        e.prepare();
        e.start().unwrap();
        assert_eq!(e.render(2).unwrap(), vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(e.render(2).unwrap(), vec![0.0; 4]);
    }

    #[test]
    fn gain_scales_its_input() {
        let mut e = engine();
        let p = e.attach_player(STEREO, vec![1.0, 1.0]).unwrap();
        let g = e.attach_gain(0.5).unwrap();
        e.connect(p, g, STEREO).unwrap();
        e.connect(g, OUTPUT, STEREO).unwrap();
        e.prepare();
        e.start().unwrap();
        assert_eq!(e.render(1).unwrap(), vec![0.5, 0.5]);
    }

    #[test]
    fn mixer_sums_two_players() {
        let mut e = engine();
        let a = e.attach_player(STEREO, vec![0.25, 0.25]).unwrap();
        let b = e.attach_player(STEREO, vec![0.5, 0.5]).unwrap();
        let m = e.attach_mixer().unwrap();
        e.connect(a, m, STEREO).unwrap();
        e.connect(b, m, STEREO).unwrap();
        e.connect(m, OUTPUT, STEREO).unwrap();
        e.prepare();
        e.start().unwrap();
        assert_eq!(e.render(1).unwrap(), vec![0.75, 0.75]);
    }

    #[test]
    fn cycles_and_self_feeds_are_refused() {
        let mut e = engine();
        let m = e.attach_mixer().unwrap();
        let g = e.attach_gain(1.0).unwrap();
        e.connect(m, g, STEREO).unwrap();
        assert_eq!(e.connect(g, m, STEREO), Err(EngineError::Cycle));
        assert_eq!(e.connect(m, m, STEREO), Err(EngineError::Cycle));
    }

    #[test]
    fn format_must_match_both_ends() {
        let mut e = engine();
        let mono_44 = Format { sample_rate: 44_100, channels: 1 };
        let p = e.attach_player(mono_44, vec![0.1]).unwrap();
        assert_eq!(e.connect(p, OUTPUT, mono_44), Err(EngineError::InvalidFormat));
        let q = e.attach_player(STEREO, vec![0.1, 0.1]).unwrap();
        assert_eq!(e.connect(q, OUTPUT, mono_44), Err(EngineError::InvalidFormat));
        assert_eq!(e.attach_player(STEREO, vec![0.1, 0.1, 0.1]), Err(EngineError::InvalidValue));
    }

    #[test]
    fn unconnected_nodes_are_not_rendered_and_inputs_are_single() {
        let mut e = engine();
        let _lonely = e.attach_player(STEREO, vec![1.0, 1.0]).unwrap();
        e.prepare();
        e.start().unwrap();
        assert_eq!(e.render(1).unwrap(), vec![0.0, 0.0]);
        let p = e.attach_player(STEREO, vec![0.2, 0.2]).unwrap();
        let g = e.attach_gain(1.0).unwrap();
        e.connect(g, OUTPUT, STEREO).unwrap();
        assert_eq!(e.connect(p, OUTPUT, STEREO), Err(EngineError::InputBusy));
        e.connect(p, g, STEREO).unwrap();
        let q = e.attach_player(STEREO, vec![0.1, 0.1]).unwrap();
        assert_eq!(e.connect(q, g, STEREO), Err(EngineError::InputBusy));
        assert_eq!(e.connect(OUTPUT, p, STEREO), Err(EngineError::InvalidConnection));
    }

    #[test]
    fn engine_state_gates_start_and_render() {
        let mut e = engine();
        assert_eq!(e.start(), Err(EngineError::NotPrepared));
        assert_eq!(e.render(1), Err(EngineError::NotRunning));
        e.prepare();
        e.start().unwrap();
        assert_eq!(e.attach_mixer(), Err(EngineError::Running));
        e.stop();
        assert!(e.attach_mixer().is_ok());
        assert_eq!(e.start(), Err(EngineError::NotPrepared));
    }

    #[test]
    fn detach_removes_connections_and_output_stays() {
        let mut e = engine();
        let p = e.attach_player(STEREO, vec![0.3, 0.3]).unwrap();
        e.connect(p, OUTPUT, STEREO).unwrap();
        assert_eq!(e.detach(OUTPUT), Err(EngineError::InvalidConnection));
        e.detach(p).unwrap();
        assert_eq!(e.detach(p), Err(EngineError::NoNode));
        assert_eq!(e.attached_count(), 1);
        e.prepare();
        e.start().unwrap();
        assert_eq!(e.render(1).unwrap(), vec![0.0, 0.0]);
    }
}
