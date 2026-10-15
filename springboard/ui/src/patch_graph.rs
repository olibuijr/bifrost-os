//! Node-graph composition model: patches with typed input and output ports, connections between ports, and evaluation
//! in dependency order. An app builds a [`PatchGraph`] from [`PatchSpec`]s, connects them with [`PatchGraph::connect`]
//! (which rejects mismatched port types, a second source for one input, and cycles) and runs it with
//! [`PatchGraph::evaluate`], supplying the behaviour of each patch as a closure. The model draws nothing and holds no
//! rendering context; a composition view would be built on top of it.

use std::collections::BTreeSet;
use std::fmt;

/// The type a port carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortType {
    Number,
    Boolean,
    Text,
}

/// A value passed along a connection. Its [`Value::kind`] must match the port it is produced on.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Boolean(bool),
    Text(String),
}

impl Value {
    pub fn kind(&self) -> PortType {
        match self {
            Value::Number(_) => PortType::Number,
            Value::Boolean(_) => PortType::Boolean,
            Value::Text(_) => PortType::Text,
        }
    }
}

/// A named, typed port on a patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub kind: PortType,
}

impl Port {
    pub fn new(name: &str, kind: PortType) -> Port {
        Port { name: name.into(), kind }
    }
}

/// A patch's name and ports. Input and output ports are addressed by their index in these lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchSpec {
    pub name: String,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
}

impl PatchSpec {
    pub fn new(name: &str, inputs: Vec<Port>, outputs: Vec<Port>) -> PatchSpec {
        PatchSpec { name: name.into(), inputs, outputs }
    }
}

/// One port of one patch, by patch index and port index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub patch: usize,
    pub port: usize,
}

/// Why a connection or an evaluation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// No patch has this index.
    UnknownPatch(usize),
    /// The patch has no output port at this index (the source side of a connection).
    NoOutputPort(Endpoint),
    /// The patch has no input port at this index (the destination side of a connection).
    NoInputPort(Endpoint),
    /// The two ports carry different types.
    TypeMismatch { from: PortType, to: PortType },
    /// The input already has a source. An input takes one connection.
    InputInUse(Endpoint),
    /// The connection would make a patch depend on its own output.
    Cycle,
    /// A patch returned a different number of values than it has output ports.
    OutputCount { patch: usize, expected: usize, found: usize },
    /// A patch returned a value whose kind does not match the output port it fills.
    ValueKind { patch: usize, port: usize },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::UnknownPatch(p) => write!(f, "no patch with index {p}"),
            GraphError::NoOutputPort(e) => write!(f, "patch {} has no output port {}", e.patch, e.port),
            GraphError::NoInputPort(e) => write!(f, "patch {} has no input port {}", e.patch, e.port),
            GraphError::TypeMismatch { from, to } => write!(f, "cannot connect a {from:?} port to a {to:?} port"),
            GraphError::InputInUse(e) => write!(f, "input port {} of patch {} already has a source", e.port, e.patch),
            GraphError::Cycle => f.write_str("the connection would make a patch depend on itself"),
            GraphError::OutputCount { patch, expected, found } => {
                write!(f, "patch {patch} returned {found} values for {expected} output ports")
            }
            GraphError::ValueKind { patch, port } => write!(f, "patch {patch} returned a value of the wrong kind for output {port}"),
        }
    }
}

/// A directed connection from an output port to an input port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connection {
    pub from: Endpoint,
    pub to: Endpoint,
}

/// A composition: the patches and the connections between their ports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatchGraph {
    patches: Vec<PatchSpec>,
    connections: Vec<Connection>,
}

impl PatchGraph {
    pub fn new() -> PatchGraph {
        PatchGraph::default()
    }

    /// Adds a patch and returns its index.
    pub fn add_patch(&mut self, spec: PatchSpec) -> usize {
        self.patches.push(spec);
        self.patches.len() - 1
    }

    pub fn patch(&self, index: usize) -> Option<&PatchSpec> {
        self.patches.get(index)
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// Connects an output port to an input port. Refused when a port does not exist, the types differ, the input
    /// already has a source, or the connection would close a loop.
    pub fn connect(&mut self, from: Endpoint, to: Endpoint) -> Result<(), GraphError> {
        let source = self.patches.get(from.patch).ok_or(GraphError::UnknownPatch(from.patch))?;
        let target = self.patches.get(to.patch).ok_or(GraphError::UnknownPatch(to.patch))?;
        let out = source.outputs.get(from.port).ok_or(GraphError::NoOutputPort(from))?;
        let input = target.inputs.get(to.port).ok_or(GraphError::NoInputPort(to))?;
        if out.kind != input.kind {
            return Err(GraphError::TypeMismatch { from: out.kind, to: input.kind });
        }
        if self.connections.iter().any(|c| c.to == to) {
            return Err(GraphError::InputInUse(to));
        }
        if self.reaches(to.patch, from.patch) {
            return Err(GraphError::Cycle);
        }
        self.connections.push(Connection { from, to });
        Ok(())
    }

    /// The output port feeding an input, if one is connected.
    pub fn source_of(&self, input: Endpoint) -> Option<Endpoint> {
        self.connections.iter().find(|c| c.to == input).map(|c| c.from)
    }

    /// Patch indices in an order where every patch comes after the patches that feed it. Ties go to the lower index,
    /// so the order is the same on every run. Connections never form a loop, so every patch appears once.
    pub fn evaluation_order(&self) -> Vec<usize> {
        let count = self.patches.len();
        let mut pending = vec![0usize; count];
        for c in &self.connections {
            pending[c.to.patch] += 1;
        }
        let mut ready: BTreeSet<usize> = (0..count).filter(|&p| pending[p] == 0).collect();
        let mut order = Vec::with_capacity(count);
        while let Some(patch) = ready.pop_first() {
            order.push(patch);
            for c in self.connections.iter().filter(|c| c.from.patch == patch) {
                pending[c.to.patch] -= 1;
                if pending[c.to.patch] == 0 {
                    ready.insert(c.to.patch);
                }
            }
        }
        order
    }

    /// Runs every patch in dependency order. `run` receives the patch index and one value per input port (`None` for
    /// an unconnected input) and returns one value per output port. The result holds each patch's outputs by patch
    /// index.
    pub fn evaluate<F>(&self, mut run: F) -> Result<Vec<Vec<Value>>, GraphError>
    where
        F: FnMut(usize, &[Option<Value>]) -> Vec<Value>,
    {
        let mut results: Vec<Vec<Value>> = vec![Vec::new(); self.patches.len()];
        for patch in self.evaluation_order() {
            let spec = &self.patches[patch];
            let inputs: Vec<Option<Value>> = (0..spec.inputs.len())
                .map(|port| {
                    self.source_of(Endpoint { patch, port })
                        .and_then(|from| results[from.patch].get(from.port).cloned())
                })
                .collect();
            let produced = run(patch, inputs.as_slice());
            if produced.len() != spec.outputs.len() {
                return Err(GraphError::OutputCount { patch, expected: spec.outputs.len(), found: produced.len() });
            }
            if let Some(port) = produced.iter().zip(&spec.outputs).position(|(value, out)| value.kind() != out.kind) {
                return Err(GraphError::ValueKind { patch, port });
            }
            results[patch] = produced;
        }
        Ok(results)
    }

    /// True when `target` can be reached from `start` by following connections (a patch reaches itself).
    fn reaches(&self, start: usize, target: usize) -> bool {
        let mut seen = vec![false; self.patches.len()];
        let mut stack = vec![start];
        while let Some(patch) = stack.pop() {
            if patch == target {
                return true;
            }
            if std::mem::replace(&mut seen[patch], true) {
                continue;
            }
            stack.extend(self.connections.iter().filter(|c| c.from.patch == patch).map(|c| c.to.patch));
        }
        false
    }
}
