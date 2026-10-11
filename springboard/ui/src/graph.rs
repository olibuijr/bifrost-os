//! A symbolic compute graph. Inputs, constants and operations are recorded as nodes; each node's tensor type is
//! inferred when it is added, so a bad shape or element type is refused at construction. `schedule` returns the
//! nodes an output needs, in dependency order. No value is computed here.
use crate::tensor::{DataType, Shape, ShapeError, TensorData, TensorType};

/// Index of a node, which is also the tensor it produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TensorId(pub usize);

/// Element-wise binary operators. Both operands must share an element type and broadcast to a common shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Maximum,
    Minimum,
}

/// What a node computes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An input supplied when the graph runs.
    Placeholder,
    /// A value fixed when the graph is built.
    Constant,
    Binary(Binary, TensorId, TensorId),
    /// Clamps each element to zero from below. Shape and type are kept.
    Relu(TensorId),
    Reshape(TensorId, Shape),
}

impl Kind {
    /// The nodes this one reads.
    pub fn inputs(&self) -> Vec<TensorId> {
        match self {
            Kind::Placeholder | Kind::Constant => Vec::new(),
            Kind::Binary(_, a, b) => vec![*a, *b],
            Kind::Relu(x) | Kind::Reshape(x, _) => vec![*x],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: Kind,
    pub ty: TensorType,
}

/// Why a node could not be added or scheduled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// The id does not name a node of this graph.
    Unknown(TensorId),
    /// The operands have different element types.
    DataType(DataType, DataType),
    /// The shapes do not fit the operator (see `ShapeError`).
    Shape(ShapeError),
}

impl From<ShapeError> for GraphError {
    fn from(e: ShapeError) -> Self {
        GraphError::Shape(e)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    nodes: Vec<Node>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of nodes recorded so far.
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    pub fn node(&self, id: TensorId) -> Option<&Node> {
        self.nodes.get(id.0)
    }

    fn push(&mut self, kind: Kind, ty: TensorType) -> TensorId {
        self.nodes.push(Node { kind, ty });
        TensorId(self.nodes.len() - 1)
    }

    fn type_of(&self, id: TensorId) -> Result<&TensorType, GraphError> {
        self.node(id).map(|n| &n.ty).ok_or(GraphError::Unknown(id))
    }

    /// An input supplied at run time.
    pub fn placeholder(&mut self, data_type: DataType, dims: &[usize]) -> TensorId {
        self.push(Kind::Placeholder, TensorType::new(data_type, dims))
    }

    /// A constant whose type is the payload's type.
    pub fn constant(&mut self, data: TensorData) -> TensorId {
        let ty = data.ty().clone();
        self.push(Kind::Constant, ty)
    }

    /// An element-wise operator. The result has the broadcast shape and the operands' element type.
    pub fn binary(&mut self, op: Binary, a: TensorId, b: TensorId) -> Result<TensorId, GraphError> {
        let (ta, tb) = (self.type_of(a)?.clone(), self.type_of(b)?.clone());
        if ta.data_type != tb.data_type {
            return Err(GraphError::DataType(ta.data_type, tb.data_type));
        }
        let shape = ta.shape.broadcast(&tb.shape)?;
        Ok(self.push(Kind::Binary(op, a, b), TensorType { data_type: ta.data_type, shape }))
    }

    pub fn relu(&mut self, x: TensorId) -> Result<TensorId, GraphError> {
        let ty = self.type_of(x)?.clone();
        Ok(self.push(Kind::Relu(x), ty))
    }

    /// A reshape of `x` to `dims`. The element count must not change.
    pub fn reshape(&mut self, x: TensorId, dims: &[usize]) -> Result<TensorId, GraphError> {
        let ty = self.type_of(x)?.clone();
        let shape = ty.shape.reshaped(dims)?;
        Ok(self.push(Kind::Reshape(x, shape.clone()), TensorType { data_type: ty.data_type, shape }))
    }

    /// The nodes needed for `outputs`, each listed after everything it reads, and each listed once.
    pub fn schedule(&self, outputs: &[TensorId]) -> Result<Vec<TensorId>, GraphError> {
        let mut done = vec![false; self.nodes.len()];
        let mut order = Vec::new();
        for &out in outputs {
            self.visit(out, &mut done, &mut order)?;
        }
        Ok(order)
    }

    fn visit(&self, id: TensorId, done: &mut [bool], order: &mut Vec<TensorId>) -> Result<(), GraphError> {
        let node = self.node(id).ok_or(GraphError::Unknown(id))?;
        if done[id.0] {
            return Ok(());
        }
        for input in node.kind.inputs() {
            self.visit(input, done, order)?;
        }
        done[id.0] = true;
        order.push(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_operator_broadcasts_and_keeps_the_element_type() {
        let mut g = Graph::new();
        let x = g.placeholder(DataType::Float32, &[4, 1, 3]);
        let bias = g.constant(TensorData::from_f32(&[2, 1], &[0.5, -0.5]).unwrap());
        let y = g.binary(Binary::Add, x, bias).unwrap();
        let ty = &g.node(y).unwrap().ty;
        assert_eq!(ty, &TensorType::new(DataType::Float32, &[4, 2, 3]));
        assert_eq!(g.count(), 3);
    }

    #[test]
    fn operands_of_different_element_types_are_refused() {
        let mut g = Graph::new();
        let f = g.placeholder(DataType::Float32, &[2]);
        let i = g.placeholder(DataType::Int32, &[2]);
        assert_eq!(
            g.binary(Binary::Multiply, f, i),
            Err(GraphError::DataType(DataType::Float32, DataType::Int32))
        );
        assert_eq!(g.count(), 2, "a refused operator adds no node");
    }

    #[test]
    fn incompatible_shapes_are_refused_with_the_shape_error() {
        let mut g = Graph::new();
        let a = g.placeholder(DataType::Float16, &[2, 3]);
        let b = g.placeholder(DataType::Float16, &[4]);
        assert_eq!(g.binary(Binary::Add, a, b), Err(GraphError::Shape(ShapeError::Incompatible)));
    }

    #[test]
    fn a_reshape_keeps_the_count_and_a_bad_one_is_refused() {
        let mut g = Graph::new();
        let x = g.placeholder(DataType::Float32, &[2, 3, 4]);
        let r = g.reshape(x, &[6, 4]).unwrap();
        assert_eq!(g.node(r).unwrap().ty.shape, Shape::new(&[6, 4]));
        assert_eq!(g.reshape(x, &[5, 5]), Err(GraphError::Shape(ShapeError::ElementCount)));
    }

    #[test]
    fn schedule_lists_dependencies_first_and_leaves_out_unused_nodes() {
        let mut g = Graph::new();
        let a = g.placeholder(DataType::Float32, &[3]);
        let b = g.placeholder(DataType::Float32, &[3]);
        let sum = g.binary(Binary::Add, a, b).unwrap();
        let act = g.relu(sum).unwrap();
        let unused = g.binary(Binary::Multiply, a, a).unwrap();
        let order = g.schedule(&[act]).unwrap();
        assert_eq!(order, vec![a, b, sum, act], "inputs come before the nodes that read them");
        assert!(!order.contains(&unused));
    }

    #[test]
    fn a_shared_input_is_scheduled_once() {
        let mut g = Graph::new();
        let x = g.placeholder(DataType::Float32, &[1]);
        let y = g.binary(Binary::Add, x, x).unwrap();
        let z = g.binary(Binary::Maximum, y, x).unwrap();
        let order = g.schedule(&[z, y]).unwrap();
        assert_eq!(order, vec![x, y, z]);
    }

    #[test]
    fn an_unknown_output_is_an_error_not_a_panic() {
        let mut g = Graph::new();
        g.placeholder(DataType::Float32, &[1]);
        assert_eq!(g.schedule(&[TensorId(9)]), Err(GraphError::Unknown(TensorId(9))));
        assert_eq!(g.relu(TensorId(3)), Err(GraphError::Unknown(TensorId(3))));
    }
}
