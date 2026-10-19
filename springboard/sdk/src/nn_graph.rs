//! Neural network graphs for apps: named nodes joined by edges, run over `Feature` maps on the CPU. A [`Graph`] is the
//! app-facing counterpart of a network graph of convolution, pooling, fully connected, activation, add and concat nodes.
//! Nodes are added in order and may only read nodes added before them, so a graph has no cycles and runs front to back.
//! It also has a binary model format ([`Graph::to_bytes`], [`Graph::from_bytes`]) whose reader treats its input as
//! hostile: every length is checked against the bytes left before anything is allocated.
//!
//! The layer maths is `bifrost_ui::cnn_layers`. Sequence helpers run a whole input sequence through an LSTM or GRU
//! ([`lstm_sequence`], [`gru_sequence`]) and a batch through a fully connected layer ([`matrix_inference`]).
//!
//! Gaps: no GPU path, no training, no batch normalisation node and no user-defined node. See docs/bifrost-ui.md "GPU kits".
use std::collections::HashSet;

use bifrost_ui::cnn_layers::{conv2d, dense, gru_step, lstm_step, pool2d, relu, Cell, Feature, LayerError, Pooling, RecurrentWeights};
use bifrost_ui::nn::{softmax, Activation, Padding, Window};

/// The most nodes a graph may hold.
pub const MAX_NODES: usize = 4096;
/// The most weights and biases, together, a stored graph may hold.
pub const MAX_FLOATS: usize = 1 << 24;
/// The longest node name, in bytes.
pub const MAX_NAME_BYTES: usize = 256;

const MAGIC: &[u8; 4] = b"BNNG";
const VERSION: u8 = 1;

/// A node of a graph, by position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(pub usize);

/// What a node does.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeOp {
    /// A graph input, named; `run` is given a feature map for it. Takes no inputs.
    Input,
    /// Two-dimensional convolution, weights laid out `[out][in][kernel_h][kernel_w]`. Takes one input.
    Conv2d { out_channels: usize, weights: Vec<f32>, bias: Vec<f32>, windows: [Window; 2] },
    /// Max or average pooling. Takes one input.
    Pool { kind: Pooling, windows: [Window; 2] },
    /// Fully connected layer over the flattened input, weights `[outputs][input length]`. Takes one input.
    Dense { outputs: usize, weights: Vec<f32>, bias: Vec<f32> },
    /// `max(x, 0)`. Takes one input.
    Relu,
    /// Softmax over all the values of the input. Takes one input.
    Softmax,
    /// Element-wise sum of two inputs of the same shape (a residual connection).
    Add,
    /// The inputs joined along the channel axis. Takes two or more inputs of the same height and width.
    Concat,
}

/// Why a graph call or a model file was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphError {
    /// A node name is empty or longer than `MAX_NAME_BYTES`.
    BadName,
    /// Two nodes have this name.
    DuplicateName(String),
    /// More than `MAX_NODES` nodes.
    TooManyNodes,
    /// An edge names a node that does not exist yet.
    UnknownNode(usize),
    /// A node has the wrong number of inputs.
    WrongInputCount { node: String, got: usize },
    /// The graph has no nodes, so it has no output.
    NoOutput,
    /// `run` was not given a feature map for this input.
    MissingInput(String),
    /// A node's inputs do not fit together.
    ShapeMismatch { node: String },
    /// A layer refused its input or its weights.
    Layer(LayerError),
    /// The model bytes are malformed. The text says how.
    Archive(&'static str),
}

impl core::fmt::Display for GraphError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GraphError::BadName => write!(f, "graph: a node name is empty or longer than {MAX_NAME_BYTES} bytes"),
            GraphError::DuplicateName(n) => write!(f, "graph: two nodes are named {n}"),
            GraphError::TooManyNodes => write!(f, "graph: more than {MAX_NODES} nodes"),
            GraphError::UnknownNode(i) => write!(f, "graph: node {i} does not exist yet"),
            GraphError::WrongInputCount { node, got } => write!(f, "graph: node {node} cannot take {got} inputs"),
            GraphError::NoOutput => write!(f, "graph: no nodes"),
            GraphError::MissingInput(n) => write!(f, "graph: no feature map was given for input {n}"),
            GraphError::ShapeMismatch { node } => write!(f, "graph: the inputs of node {node} do not fit together"),
            GraphError::Layer(e) => write!(f, "graph: layer error {e:?}"),
            GraphError::Archive(why) => write!(f, "graph: bad model data: {why}"),
        }
    }
}

impl std::error::Error for GraphError {}

impl From<LayerError> for GraphError {
    fn from(e: LayerError) -> Self {
        GraphError::Layer(e)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Node {
    name: String,
    op: NodeOp,
    inputs: Vec<usize>,
}

/// A directed acyclic graph of layers. The last node added is the output unless `set_output` names another.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    nodes: Vec<Node>,
    output: Option<usize>,
}

impl Graph {
    pub fn new() -> Self {
        Graph::default()
    }

    /// The number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The names of the nodes, in the order they were added.
    pub fn node_names(&self) -> Vec<&str> {
        self.nodes.iter().map(|n| n.name.as_str()).collect()
    }

    /// The names of the input nodes, which `run` needs a feature map for.
    pub fn input_names(&self) -> Vec<&str> {
        self.nodes.iter().filter(|n| n.op == NodeOp::Input).map(|n| n.name.as_str()).collect()
    }

    /// Adds an input node.
    pub fn add_input(&mut self, name: &str) -> Result<NodeId, GraphError> {
        self.add(name, NodeOp::Input, &[])
    }

    /// Adds a node reading `inputs`. The name must be new and every input must already be in the graph.
    pub fn add(&mut self, name: &str, op: NodeOp, inputs: &[NodeId]) -> Result<NodeId, GraphError> {
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(GraphError::BadName);
        }
        if self.nodes.len() >= MAX_NODES {
            return Err(GraphError::TooManyNodes);
        }
        if self.nodes.iter().any(|n| n.name == name) {
            return Err(GraphError::DuplicateName(name.to_string()));
        }
        if let Some(bad) = inputs.iter().find(|i| i.0 >= self.nodes.len()) {
            return Err(GraphError::UnknownNode(bad.0));
        }
        let ok = match op {
            NodeOp::Input => inputs.is_empty(),
            NodeOp::Add => inputs.len() == 2,
            NodeOp::Concat => inputs.len() >= 2,
            _ => inputs.len() == 1,
        };
        if !ok {
            return Err(GraphError::WrongInputCount { node: name.to_string(), got: inputs.len() });
        }
        self.nodes.push(Node { name: name.to_string(), op, inputs: inputs.iter().map(|i| i.0).collect() });
        Ok(NodeId(self.nodes.len() - 1))
    }

    /// Makes `id` the node `run` returns.
    pub fn set_output(&mut self, id: NodeId) -> Result<(), GraphError> {
        if id.0 >= self.nodes.len() {
            return Err(GraphError::UnknownNode(id.0));
        }
        self.output = Some(id.0);
        Ok(())
    }

    /// The node `run` returns: the one set with `set_output`, else the last added.
    pub fn output(&self) -> Option<NodeId> {
        self.output.or(self.nodes.len().checked_sub(1)).map(NodeId)
    }

    /// Runs the graph. `inputs` gives a feature map for each input node by name; extra entries are ignored.
    pub fn run(&self, inputs: &[(&str, &Feature)]) -> Result<Feature, GraphError> {
        let out = self.output().ok_or(GraphError::NoOutput)?;
        let mut values: Vec<Feature> = Vec::with_capacity(out.0 + 1);
        for node in &self.nodes[..=out.0] {
            let value = {
                let arg = |k: usize| &values[node.inputs[k]];
                match &node.op {
                    NodeOp::Input => {
                        let given = inputs
                            .iter()
                            .find(|(n, _)| *n == node.name)
                            .ok_or_else(|| GraphError::MissingInput(node.name.clone()))?;
                        given.1.clone()
                    }
                    NodeOp::Conv2d { out_channels, weights, bias, windows } => conv2d(arg(0), weights, bias, *out_channels, *windows)?,
                    NodeOp::Pool { kind, windows } => pool2d(arg(0), *kind, *windows)?,
                    NodeOp::Dense { outputs, weights, bias } => dense(arg(0), weights, bias, *outputs)?,
                    NodeOp::Relu => relu(arg(0)),
                    NodeOp::Softmax => {
                        let x = arg(0);
                        Feature::new(x.channels, x.height, x.width, softmax(&x.data))?
                    }
                    NodeOp::Add => {
                        let (a, b) = (arg(0), arg(1));
                        if a.dims() != b.dims() {
                            return Err(GraphError::ShapeMismatch { node: node.name.clone() });
                        }
                        let data = a.data.iter().zip(&b.data).map(|(x, y)| x + y).collect();
                        Feature::new(a.channels, a.height, a.width, data)?
                    }
                    NodeOp::Concat => {
                        let first = arg(0);
                        let mut channels = 0;
                        let mut data = Vec::new();
                        for k in 0..node.inputs.len() {
                            let x = arg(k);
                            if (x.height, x.width) != (first.height, first.width) {
                                return Err(GraphError::ShapeMismatch { node: node.name.clone() });
                            }
                            channels += x.channels;
                            data.extend_from_slice(&x.data);
                        }
                        Feature::new(channels, first.height, first.width, data)?
                    }
                }
            };
            values.push(value);
        }
        values.pop().ok_or(GraphError::NoOutput)
    }

    /// Writes the graph as model bytes: magic, version, node count, output, then each node with its edges and weights.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        put_u32(&mut out, self.nodes.len());
        put_u32(&mut out, self.output().map_or(0, |o| o.0));
        for node in &self.nodes {
            put_u16(&mut out, node.name.len());
            out.extend_from_slice(node.name.as_bytes());
            let tag = match node.op {
                NodeOp::Input => 0,
                NodeOp::Conv2d { .. } => 1,
                NodeOp::Pool { .. } => 2,
                NodeOp::Dense { .. } => 3,
                NodeOp::Relu => 4,
                NodeOp::Softmax => 5,
                NodeOp::Add => 6,
                NodeOp::Concat => 7,
            };
            out.push(tag);
            put_u32(&mut out, node.inputs.len());
            for &i in &node.inputs {
                put_u32(&mut out, i);
            }
            match &node.op {
                NodeOp::Conv2d { out_channels, weights, bias, windows } => {
                    put_u32(&mut out, *out_channels);
                    put_windows(&mut out, windows);
                    put_floats(&mut out, weights);
                    put_floats(&mut out, bias);
                }
                NodeOp::Pool { kind, windows } => {
                    out.push(if *kind == Pooling::Max { 0 } else { 1 });
                    put_windows(&mut out, windows);
                }
                NodeOp::Dense { outputs, weights, bias } => {
                    put_u32(&mut out, *outputs);
                    put_floats(&mut out, weights);
                    put_floats(&mut out, bias);
                }
                NodeOp::Input | NodeOp::Relu | NodeOp::Softmax | NodeOp::Add | NodeOp::Concat => {}
            }
        }
        out
    }

    /// Reads model bytes written by [`Graph::to_bytes`]. Truncated, oversized, forward-referencing or otherwise malformed
    /// data is refused with an error; nothing is allocated for a length the bytes cannot back.
    pub fn from_bytes(bytes: &[u8]) -> Result<Graph, GraphError> {
        let mut r = Reader { bytes, at: 0, floats: 0 };
        if r.take(4)? != MAGIC {
            return Err(GraphError::Archive("not a model file"));
        }
        if r.u8()? != VERSION {
            return Err(GraphError::Archive("unknown version"));
        }
        let count = r.u32()?;
        let output = r.u32()?;
        if count == 0 {
            return Err(GraphError::Archive("no nodes"));
        }
        if count > MAX_NODES {
            return Err(GraphError::TooManyNodes);
        }
        if output >= count {
            return Err(GraphError::Archive("the output node does not exist"));
        }
        let mut graph = Graph::new();
        let mut names = HashSet::new();
        for _ in 0..count {
            let name_len = r.u16()?;
            let name = std::str::from_utf8(r.take(name_len)?).map_err(|_| GraphError::Archive("a node name is not text"))?.to_string();
            if !names.insert(name.clone()) {
                return Err(GraphError::DuplicateName(name));
            }
            let tag = r.u8()?;
            let input_count = r.u32()?;
            if input_count > r.left() / 4 {
                return Err(GraphError::Archive("the data ends inside a node"));
            }
            let mut inputs = Vec::with_capacity(input_count);
            for _ in 0..input_count {
                inputs.push(NodeId(r.u32()?));
            }
            let op = match tag {
                0 => NodeOp::Input,
                1 => {
                    let out_channels = r.u32()?;
                    let windows = r.windows()?;
                    let weights = r.floats()?;
                    let bias = r.floats()?;
                    NodeOp::Conv2d { out_channels, weights, bias, windows }
                }
                2 => {
                    let kind = match r.u8()? {
                        0 => Pooling::Max,
                        1 => Pooling::Average,
                        _ => return Err(GraphError::Archive("unknown pooling kind")),
                    };
                    NodeOp::Pool { kind, windows: r.windows()? }
                }
                3 => {
                    let outputs = r.u32()?;
                    let weights = r.floats()?;
                    let bias = r.floats()?;
                    NodeOp::Dense { outputs, weights, bias }
                }
                4 => NodeOp::Relu,
                5 => NodeOp::Softmax,
                6 => NodeOp::Add,
                7 => NodeOp::Concat,
                _ => return Err(GraphError::Archive("unknown node kind")),
            };
            graph.add(&name, op, &inputs)?;
        }
        if r.left() != 0 {
            return Err(GraphError::Archive("bytes after the last node"));
        }
        // The last node is the output by default; only another choice is stored.
        if output != count - 1 {
            graph.output = Some(output);
        }
        Ok(graph)
    }
}

fn put_u16(out: &mut Vec<u8>, v: usize) {
    out.extend_from_slice(&(v as u16).to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: usize) {
    out.extend_from_slice(&(v as u32).to_le_bytes());
}

fn put_floats(out: &mut Vec<u8>, values: &[f32]) {
    put_u32(out, values.len());
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn put_windows(out: &mut Vec<u8>, windows: &[Window; 2]) {
    for w in windows {
        put_u32(out, w.kernel);
        put_u32(out, w.stride);
        put_u32(out, w.dilation);
        let (tag, before, after) = match w.padding {
            Padding::Same => (0u8, 0, 0),
            Padding::Valid => (1, 0, 0),
            Padding::Explicit { before, after } => (2, before, after),
        };
        out.push(tag);
        put_u32(out, before);
        put_u32(out, after);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    /// Weights and biases read so far.
    floats: usize,
}

impl<'a> Reader<'a> {
    fn left(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], GraphError> {
        if n > self.left() {
            return Err(GraphError::Archive("the data ends too early"));
        }
        let s = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, GraphError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<usize, GraphError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]) as usize)
    }

    fn u32(&mut self) -> Result<usize, GraphError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }

    fn floats(&mut self) -> Result<Vec<f32>, GraphError> {
        let n = self.u32()?;
        if n > self.left() / 4 {
            return Err(GraphError::Archive("a weight list is longer than the data"));
        }
        self.floats += n;
        if self.floats > MAX_FLOATS {
            return Err(GraphError::Archive("more weights than the limit"));
        }
        let raw = self.take(n * 4)?;
        Ok(raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }

    fn window(&mut self) -> Result<Window, GraphError> {
        let kernel = self.u32()?;
        let stride = self.u32()?;
        let dilation = self.u32()?;
        let tag = self.u8()?;
        let before = self.u32()?;
        let after = self.u32()?;
        let padding = match tag {
            0 => Padding::Same,
            1 => Padding::Valid,
            2 => Padding::Explicit { before, after },
            _ => return Err(GraphError::Archive("unknown padding")),
        };
        Ok(Window { kernel, stride, dilation, padding })
    }

    fn windows(&mut self) -> Result<[Window; 2], GraphError> {
        let a = self.window()?;
        let b = self.window()?;
        Ok([a, b])
    }
}

/// The result of running a sequence through a recurrent layer.
#[derive(Clone, Debug, PartialEq)]
pub struct SequenceResult {
    /// The hidden state after each step, in order.
    pub outputs: Vec<Vec<f32>>,
    /// The hidden state after the last step (the initial state for an empty sequence).
    pub hidden: Vec<f32>,
    /// The cell state after the last step. Empty for a GRU.
    pub cell: Vec<f32>,
}

/// Runs `inputs` (one vector per step) through an LSTM. The state starts at zero unless `state` gives `(hidden, cell)`.
pub fn lstm_sequence(w: &RecurrentWeights, inputs: &[Vec<f32>], state: Option<(&[f32], &[f32])>) -> Result<SequenceResult, LayerError> {
    if w.cell != Cell::Lstm {
        return Err(LayerError::WrongCell);
    }
    let (mut h, mut c) = match state {
        Some((h, c)) => (h.to_vec(), c.to_vec()),
        None => (vec![0.0; w.hidden], vec![0.0; w.hidden]),
    };
    let mut outputs = Vec::with_capacity(inputs.len());
    for x in inputs {
        let (nh, nc) = lstm_step(w, x, &h, &c)?;
        h = nh;
        c = nc;
        outputs.push(h.clone());
    }
    Ok(SequenceResult { outputs, hidden: h, cell: c })
}

/// Runs `inputs` through a GRU. The state starts at zero unless `h0` is given.
pub fn gru_sequence(w: &RecurrentWeights, inputs: &[Vec<f32>], h0: Option<&[f32]>) -> Result<SequenceResult, LayerError> {
    if w.cell != Cell::Gru {
        return Err(LayerError::WrongCell);
    }
    let mut h = h0.map_or_else(|| vec![0.0; w.hidden], |s| s.to_vec());
    let mut outputs = Vec::with_capacity(inputs.len());
    for x in inputs {
        h = gru_step(w, x, &h)?;
        outputs.push(h.clone());
    }
    Ok(SequenceResult { outputs, hidden: h, cell: Vec::new() })
}

/// A batch through a fully connected layer: `input` is `batch` rows of `input_len` values, `weights` is
/// `[outputs][input_len]`, and the result is `batch` rows of `outputs` values with `activation` applied.
pub fn matrix_inference(
    input: &[f32],
    batch: usize,
    input_len: usize,
    weights: &[f32],
    bias: &[f32],
    outputs: usize,
    activation: Activation,
) -> Result<Vec<f32>, LayerError> {
    let want_input = batch.checked_mul(input_len).ok_or(LayerError::Length { expected: usize::MAX, got: input.len() })?;
    let want_weights = outputs.checked_mul(input_len).ok_or(LayerError::Length { expected: usize::MAX, got: weights.len() })?;
    for (expected, got) in [(want_input, input.len()), (want_weights, weights.len()), (outputs, bias.len())] {
        if expected != got {
            return Err(LayerError::Length { expected, got });
        }
    }
    let mut out = Vec::with_capacity(batch * outputs);
    for b in 0..batch {
        let row = &input[b * input_len..(b + 1) * input_len];
        for o in 0..outputs {
            let w = &weights[o * input_len..(o + 1) * input_len];
            out.push(activation.eval(bias[o] + w.iter().zip(row).map(|(a, b)| a * b).sum::<f32>()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: Window = Window { kernel: 1, stride: 1, dilation: 1, padding: Padding::Valid };

    fn feature(c: usize, h: usize, w: usize, data: Vec<f32>) -> Feature {
        Feature::new(c, h, w, data).unwrap()
    }

    fn scaled_relu_graph() -> Graph {
        let mut g = Graph::new();
        let x = g.add_input("x").unwrap();
        let c = g.add("scale", NodeOp::Conv2d { out_channels: 1, weights: vec![2.0], bias: vec![-1.0], windows: [ONE, ONE] }, &[x]).unwrap();
        let r = g.add("relu", NodeOp::Relu, &[c]).unwrap();
        g.add("residual", NodeOp::Add, &[x, r]).unwrap();
        g
    }

    #[test]
    fn a_residual_graph_computes_front_to_back() {
        let data = vec![-1.0, 0.0, 0.5, 1.0, 1.5, 2.0, -2.0, 3.0, 0.25];
        let x = feature(1, 3, 3, data.clone());
        let g = scaled_relu_graph();
        assert_eq!(g.input_names(), vec!["x"]);
        assert_eq!(g.node_names(), vec!["x", "scale", "relu", "residual"]);
        let out = g.run(&[("x", &x)]).unwrap();
        let expected: Vec<f32> = data.iter().map(|v| v + (2.0 * v - 1.0f32).max(0.0)).collect();
        assert_eq!(out.data, expected);
        assert_eq!(out.dims(), [1, 3, 3]);
    }

    #[test]
    fn the_output_can_be_an_inner_node() {
        let mut g = scaled_relu_graph();
        g.set_output(NodeId(2)).unwrap();
        let x = feature(1, 1, 3, vec![-1.0, 1.0, 2.0]);
        assert_eq!(g.run(&[("x", &x)]).unwrap().data, vec![0.0, 1.0, 3.0]);
        assert_eq!(g.set_output(NodeId(9)), Err(GraphError::UnknownNode(9)));
    }

    #[test]
    fn concat_joins_channels_and_softmax_sums_to_one() {
        let mut g = Graph::new();
        let a = g.add_input("a").unwrap();
        let b = g.add_input("b").unwrap();
        let c = g.add("both", NodeOp::Concat, &[a, b]).unwrap();
        g.add("probs", NodeOp::Softmax, &[c]).unwrap();
        let fa = feature(1, 1, 2, vec![1.0, 2.0]);
        let fb = feature(2, 1, 2, vec![3.0, 4.0, 5.0, 6.0]);
        let out = g.run(&[("a", &fa), ("b", &fb)]).unwrap();
        assert_eq!(out.dims(), [3, 1, 2]);
        assert!((out.data.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(out.data.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn bad_graphs_and_bad_runs_are_refused() {
        let mut g = Graph::new();
        assert_eq!(g.run(&[]), Err(GraphError::NoOutput));
        assert_eq!(g.add_input(""), Err(GraphError::BadName));
        let x = g.add_input("x").unwrap();
        assert_eq!(g.add_input("x"), Err(GraphError::DuplicateName("x".into())));
        assert_eq!(g.add("r", NodeOp::Relu, &[NodeId(5)]), Err(GraphError::UnknownNode(5)));
        assert_eq!(g.add("r", NodeOp::Relu, &[]), Err(GraphError::WrongInputCount { node: "r".into(), got: 0 }));
        assert_eq!(g.add("s", NodeOp::Add, &[x]), Err(GraphError::WrongInputCount { node: "s".into(), got: 1 }));
        assert_eq!(g.add("i", NodeOp::Input, &[x]), Err(GraphError::WrongInputCount { node: "i".into(), got: 1 }));
        assert_eq!(g.run(&[]), Err(GraphError::MissingInput("x".into())));
        let y = g.add_input("y").unwrap();
        g.add("sum", NodeOp::Add, &[x, y]).unwrap();
        let a = feature(1, 1, 2, vec![1.0, 2.0]);
        let b = feature(1, 1, 3, vec![1.0, 2.0, 3.0]);
        assert_eq!(g.run(&[("x", &a), ("y", &b)]), Err(GraphError::ShapeMismatch { node: "sum".into() }));
    }

    #[test]
    fn a_layer_error_comes_back_as_a_graph_error() {
        let mut g = Graph::new();
        let x = g.add_input("x").unwrap();
        g.add("fc", NodeOp::Dense { outputs: 2, weights: vec![0.0; 3], bias: vec![0.0; 2] }, &[x]).unwrap();
        let f = feature(1, 1, 2, vec![1.0, 2.0]);
        assert!(matches!(g.run(&[("x", &f)]), Err(GraphError::Layer(LayerError::Length { .. }))));
    }

    fn classifier() -> Graph {
        let same = Window { kernel: 3, stride: 1, dilation: 1, padding: Padding::Same };
        let pool = Window { kernel: 2, stride: 2, dilation: 1, padding: Padding::Valid };
        let fill = |n: usize| -> Vec<f32> { (0..n).map(|i| (i as f32 * 0.37).sin()).collect() };
        let mut g = Graph::new();
        let x = g.add_input("image").unwrap();
        let c = g.add("conv", NodeOp::Conv2d { out_channels: 2, weights: fill(18), bias: vec![0.1, -0.1], windows: [same, same] }, &[x]).unwrap();
        let p = g.add("pool", NodeOp::Pool { kind: Pooling::Max, windows: [pool, pool] }, &[c]).unwrap();
        let d = g.add("fc", NodeOp::Dense { outputs: 3, weights: fill(24), bias: vec![0.0, 0.5, -0.5] }, &[p]).unwrap();
        g.add("probs", NodeOp::Softmax, &[d]).unwrap();
        g
    }

    #[test]
    fn a_model_round_trips_and_gives_the_same_scores() {
        let g = classifier();
        let image = feature(1, 4, 4, (0..16).map(|i| i as f32 / 16.0).collect());
        let before = g.run(&[("image", &image)]).unwrap();
        assert_eq!(before.dims(), [3, 1, 1]);
        let bytes = g.to_bytes();
        let back = Graph::from_bytes(&bytes).unwrap();
        assert_eq!(back, g);
        assert_eq!(back.run(&[("image", &image)]).unwrap(), before);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn every_truncation_of_a_model_is_refused() {
        let bytes = classifier().to_bytes();
        for n in 0..bytes.len() {
            assert!(Graph::from_bytes(&bytes[..n]).is_err(), "prefix of {n} bytes was accepted");
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(Graph::from_bytes(&extra), Err(GraphError::Archive("bytes after the last node")));
    }

    fn header(count: u32, output: u32) -> Vec<u8> {
        let mut b = b"BNNG".to_vec();
        b.push(1);
        b.extend_from_slice(&count.to_le_bytes());
        b.extend_from_slice(&output.to_le_bytes());
        b
    }

    fn input_node(b: &mut Vec<u8>, name: &str) {
        b.extend_from_slice(&(name.len() as u16).to_le_bytes());
        b.extend_from_slice(name.as_bytes());
        b.push(0);
        b.extend_from_slice(&0u32.to_le_bytes());
    }

    #[test]
    fn hostile_model_data_is_refused_without_allocating() {
        assert_eq!(Graph::from_bytes(b"nope"), Err(GraphError::Archive("not a model file")));
        let mut bad_version = header(1, 0);
        bad_version[4] = 9;
        assert_eq!(Graph::from_bytes(&bad_version), Err(GraphError::Archive("unknown version")));
        assert_eq!(Graph::from_bytes(&header(0, 0)), Err(GraphError::Archive("no nodes")));
        assert_eq!(Graph::from_bytes(&header(u32::MAX, 0)), Err(GraphError::TooManyNodes));
        assert_eq!(Graph::from_bytes(&header(1, 1)), Err(GraphError::Archive("the output node does not exist")));

        // A dense node that claims four billion weights but brings none.
        let mut huge = header(2, 1);
        input_node(&mut huge, "x");
        huge.extend_from_slice(&1u16.to_le_bytes());
        huge.push(b'd');
        huge.push(3);
        huge.extend_from_slice(&1u32.to_le_bytes());
        huge.extend_from_slice(&0u32.to_le_bytes());
        huge.extend_from_slice(&2u32.to_le_bytes());
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Graph::from_bytes(&huge), Err(GraphError::Archive("a weight list is longer than the data")));

        // An edge that points forward.
        let mut forward = header(1, 0);
        forward.extend_from_slice(&1u16.to_le_bytes());
        forward.push(b'r');
        forward.push(4);
        forward.extend_from_slice(&1u32.to_le_bytes());
        forward.extend_from_slice(&7u32.to_le_bytes());
        assert_eq!(Graph::from_bytes(&forward), Err(GraphError::UnknownNode(7)));

        // An input count the remaining bytes cannot hold.
        let mut many = header(1, 0);
        many.extend_from_slice(&1u16.to_le_bytes());
        many.push(b'r');
        many.push(4);
        many.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Graph::from_bytes(&many), Err(GraphError::Archive("the data ends inside a node")));

        // Two nodes with one name, and an unknown node kind.
        let mut dup = header(2, 1);
        input_node(&mut dup, "x");
        input_node(&mut dup, "x");
        assert_eq!(Graph::from_bytes(&dup), Err(GraphError::DuplicateName("x".into())));
        let mut kind = header(1, 0);
        kind.extend_from_slice(&1u16.to_le_bytes());
        kind.push(b'k');
        kind.push(200);
        kind.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(Graph::from_bytes(&kind), Err(GraphError::Archive("unknown node kind")));
    }

    fn lstm() -> RecurrentWeights {
        let rows = 4 * 2;
        let fill = |n: usize| -> Vec<f32> { (0..n).map(|i| ((i as f32) * 0.5).cos() * 0.3).collect() };
        RecurrentWeights::new(Cell::Lstm, 2, 3, fill(rows * 3), fill(rows * 2), vec![0.1; rows]).unwrap()
    }

    #[test]
    fn an_lstm_sequence_equals_stepping_by_hand() {
        let w = lstm();
        let xs = vec![vec![1.0, 0.0, -1.0], vec![0.5, 0.5, 0.5], vec![-1.0, 2.0, 0.0]];
        let run = lstm_sequence(&w, &xs, None).unwrap();
        assert_eq!(run.outputs.len(), 3);
        let (mut h, mut c) = (vec![0.0; 2], vec![0.0; 2]);
        for x in &xs {
            let (nh, nc) = lstm_step(&w, x, &h, &c).unwrap();
            h = nh;
            c = nc;
        }
        assert_eq!((run.hidden.clone(), run.cell.clone()), (h, c));
        // Resuming from the state of a shorter run gives the same final state.
        let first = lstm_sequence(&w, &xs[..2], None).unwrap();
        let rest = lstm_sequence(&w, &xs[2..], Some((first.hidden.as_slice(), first.cell.as_slice()))).unwrap();
        assert_eq!(rest.hidden, run.hidden);
        assert!(run.hidden.iter().all(|v| v.abs() < 1.0));
        assert_eq!(lstm_sequence(&w, &[], None).unwrap().outputs.len(), 0);
        assert_eq!(lstm_sequence(&w, &[vec![1.0]], None), Err(LayerError::Length { expected: 3, got: 1 }));
    }

    #[test]
    fn a_gru_sequence_refuses_the_wrong_cell() {
        let w = lstm();
        assert_eq!(gru_sequence(&w, &[], None), Err(LayerError::WrongCell));
        let rows = 3 * 2;
        let g = RecurrentWeights::new(Cell::Gru, 2, 1, vec![0.2; rows], vec![0.1; rows * 2], vec![0.0; rows]).unwrap();
        let run = gru_sequence(&g, &[vec![1.0], vec![1.0]], None).unwrap();
        assert_eq!(run.outputs.len(), 2);
        assert!(run.cell.is_empty());
        assert_eq!(lstm_sequence(&g, &[], None), Err(LayerError::WrongCell));
    }

    #[test]
    fn a_batch_goes_through_a_fully_connected_layer() {
        let input = [1.0, 2.0, 3.0, 4.0];
        let weights = [1.0, 0.0, 0.0, -1.0];
        let out = matrix_inference(&input, 2, 2, &weights, &[0.5, 0.0], 2, Activation::Relu).unwrap();
        // Row [1,2]: 1+0.5 = 1.5 and -2 -> 0. Row [3,4]: 3.5 and -4 -> 0.
        assert_eq!(out, vec![1.5, 0.0, 3.5, 0.0]);
        assert_eq!(matrix_inference(&input, 3, 2, &weights, &[0.0, 0.0], 2, Activation::Identity), Err(LayerError::Length { expected: 6, got: 4 }));
        assert_eq!(matrix_inference(&input, 2, 2, &weights, &[0.0], 2, Activation::Identity), Err(LayerError::Length { expected: 2, got: 1 }));
        assert_eq!(matrix_inference(&[], 0, 2, &weights, &[0.0, 0.0], 2, Activation::Identity), Ok(Vec::new()));
    }
}
