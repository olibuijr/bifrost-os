//! A symbolic compute graph. Inputs, constants and operations are recorded as nodes; each node's tensor type is
//! inferred when it is added, so a bad shape or element type is refused at construction. `schedule` returns the
//! nodes an output needs, in dependency order. `compile` turns the outputs into an `Executable`: the schedule, the
//! feeds it needs and their types, and the graph options it was built with. Feeds are checked against that
//! signature before a run. No value is computed here.
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

/// Element-wise unary operators. The result has the input's shape and element type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unary {
    Negate,
    Absolute,
    Square,
    /// Floating-point element types only.
    Sqrt,
    /// Floating-point element types only.
    Exponential,
}

/// What a node computes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An input supplied when the graph runs.
    Placeholder,
    /// A value fixed when the graph is built.
    Constant,
    Binary(Binary, TensorId, TensorId),
    Unary(Unary, TensorId),
    /// Clamps each element to zero from below. Shape and type are kept.
    Relu(TensorId),
    Reshape(TensorId, Shape),
    /// Converts each element to another element type. Shape is kept.
    Cast(TensorId, DataType),
    /// Permutes the dimensions: output dimension `i` is input dimension `perm[i]`.
    Transpose(TensorId, Vec<usize>),
    /// Batched matrix product over the last two dimensions; leading dimensions broadcast.
    Matmul(TensorId, TensorId),
    /// Joins the inputs along an axis; every other dimension must agree.
    Concat(Vec<TensorId>, usize),
}

impl Kind {
    /// The nodes this one reads.
    pub fn inputs(&self) -> Vec<TensorId> {
        match self {
            Kind::Placeholder | Kind::Constant => Vec::new(),
            Kind::Binary(_, a, b) | Kind::Matmul(a, b) => vec![*a, *b],
            Kind::Relu(x) | Kind::Reshape(x, _) | Kind::Unary(_, x) | Kind::Cast(x, _) | Kind::Transpose(x, _) => {
                vec![*x]
            }
            Kind::Concat(parts, _) => parts.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: Kind,
    pub ty: TensorType,
}

/// Why a node could not be added, scheduled, compiled or fed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// The id does not name a node of this graph.
    Unknown(TensorId),
    /// The operands have different element types.
    DataType(DataType, DataType),
    /// The shapes do not fit the operator (see `ShapeError`).
    Shape(ShapeError),
    /// The operator does not accept this element type (arithmetic on Bool, or a non-float input to Sqrt).
    Unsupported(DataType),
    /// An executable needs a placeholder that the feeds do not name.
    MissingFeed(TensorId),
    /// A feed names a node that is not a placeholder of the executable.
    NotAFeed(TensorId),
    /// A feed's type differs from the placeholder's type.
    FeedType(TensorId),
}

impl From<ShapeError> for GraphError {
    fn from(e: ShapeError) -> Self {
        GraphError::Shape(e)
    }
}

/// Graph-wide settings that are fixed into every executable compiled from the graph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Allow reduced-precision paths for faster math. The engine may ignore this, so results can differ slightly.
    pub reduced_precision_fast_math: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    nodes: Vec<Node>,
    options: Options,
    /// the payload of each constant node, so a run can read it (`graph_run`)
    constants: Vec<(TensorId, TensorData)>,
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

    pub fn options(&self) -> Options {
        self.options
    }

    pub fn set_options(&mut self, options: Options) {
        self.options = options;
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
        let id = self.push(Kind::Constant, ty);
        self.constants.push((id, data));
        id
    }

    /// The payload of a constant node.
    pub fn constant_data(&self, id: TensorId) -> Option<&TensorData> {
        self.constants.iter().find(|(c, _)| *c == id).map(|(_, d)| d)
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

    /// An element-wise unary operator. Bool is refused, and Sqrt and Exponential need a float element type.
    pub fn unary(&mut self, op: Unary, x: TensorId) -> Result<TensorId, GraphError> {
        let ty = self.type_of(x)?.clone();
        let refused = match op {
            Unary::Negate | Unary::Absolute | Unary::Square => ty.data_type == DataType::Bool,
            Unary::Sqrt | Unary::Exponential => !ty.data_type.is_float(),
        };
        if refused {
            return Err(GraphError::Unsupported(ty.data_type));
        }
        Ok(self.push(Kind::Unary(op, x), ty))
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

    /// Converts `x` to the element type `to`. The shape is kept.
    pub fn cast(&mut self, x: TensorId, to: DataType) -> Result<TensorId, GraphError> {
        let ty = self.type_of(x)?.clone();
        Ok(self.push(Kind::Cast(x, to), TensorType { data_type: to, shape: ty.shape }))
    }

    /// Permutes the dimensions of `x`. `perm` must name every dimension exactly once.
    pub fn transpose(&mut self, x: TensorId, perm: &[usize]) -> Result<TensorId, GraphError> {
        let ty = self.type_of(x)?.clone();
        let dims = ty.shape.dims();
        if perm.len() != dims.len() {
            return Err(ShapeError::Rank.into());
        }
        let mut seen = vec![false; perm.len()];
        for &p in perm {
            if p >= perm.len() || seen[p] {
                return Err(ShapeError::Permutation.into());
            }
            seen[p] = true;
        }
        let out: Vec<usize> = perm.iter().map(|&p| dims[p]).collect();
        let shape = Shape::new(&out);
        Ok(self.push(Kind::Transpose(x, perm.to_vec()), TensorType { data_type: ty.data_type, shape }))
    }

    /// Batched matrix product of `a` (`[.., m, k]`) and `b` (`[.., k, n]`). The leading dimensions broadcast, and
    /// the result is `[batch.., m, n]`. Both operands need at least two dimensions and the same element type.
    pub fn matmul(&mut self, a: TensorId, b: TensorId) -> Result<TensorId, GraphError> {
        let (ta, tb) = (self.type_of(a)?.clone(), self.type_of(b)?.clone());
        if ta.data_type != tb.data_type {
            return Err(GraphError::DataType(ta.data_type, tb.data_type));
        }
        let (da, db) = (ta.shape.dims(), tb.shape.dims());
        if da.len() < 2 || db.len() < 2 {
            return Err(ShapeError::Rank.into());
        }
        let (m, k) = (da[da.len() - 2], da[da.len() - 1]);
        let (k2, n) = (db[db.len() - 2], db[db.len() - 1]);
        if k != k2 {
            return Err(ShapeError::Incompatible.into());
        }
        let batch = Shape::new(&da[..da.len() - 2]).broadcast(&Shape::new(&db[..db.len() - 2]))?;
        let mut out = batch.dims().to_vec();
        out.push(m);
        out.push(n);
        Ok(self.push(Kind::Matmul(a, b), TensorType { data_type: ta.data_type, shape: Shape::new(&out) }))
    }

    /// Joins `parts` along `axis`. All parts share an element type and rank, and agree on every other dimension.
    pub fn concat(&mut self, parts: &[TensorId], axis: usize) -> Result<TensorId, GraphError> {
        let first = match parts.first() {
            Some(&p) => self.type_of(p)?.clone(),
            None => return Err(ShapeError::Empty.into()),
        };
        let base = first.shape.dims().to_vec();
        if axis >= base.len() {
            return Err(ShapeError::Axis.into());
        }
        let mut total = 0usize;
        for &p in parts {
            let t = self.type_of(p)?;
            if t.data_type != first.data_type {
                return Err(GraphError::DataType(first.data_type, t.data_type));
            }
            let d = t.shape.dims();
            if d.len() != base.len() {
                return Err(ShapeError::Rank.into());
            }
            if d.iter().zip(&base).enumerate().any(|(i, (x, y))| i != axis && x != y) {
                return Err(ShapeError::Incompatible.into());
            }
            total += d[axis];
        }
        let mut out = base;
        out[axis] = total;
        Ok(self.push(Kind::Concat(parts.to_vec(), axis), TensorType { data_type: first.data_type, shape: Shape::new(&out) }))
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

    /// Compiles the graph for `outputs`: the schedule, the placeholders it needs with their types, the output types
    /// and the graph options. The result is what a run is checked against.
    pub fn compile(&self, outputs: &[TensorId]) -> Result<Executable, GraphError> {
        let order = self.schedule(outputs)?;
        let feeds = order
            .iter()
            .filter(|&&id| matches!(self.node(id).map(|n| &n.kind), Some(Kind::Placeholder)))
            .map(|&id| -> Result<(TensorId, TensorType), GraphError> { Ok((id, self.type_of(id)?.clone())) })
            .collect::<Result<Vec<_>, GraphError>>()?;
        let outputs = outputs
            .iter()
            .map(|&id| -> Result<(TensorId, TensorType), GraphError> { Ok((id, self.type_of(id)?.clone())) })
            .collect::<Result<Vec<_>, GraphError>>()?;
        Ok(Executable {
            order,
            feeds,
            outputs,
            reduced_precision_fast_math: self.options.reduced_precision_fast_math,
        })
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

/// A compiled graph: what a run needs, and what it produces. Built by `Graph::compile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Executable {
    order: Vec<TensorId>,
    feeds: Vec<(TensorId, TensorType)>,
    outputs: Vec<(TensorId, TensorType)>,
    reduced_precision_fast_math: bool,
}

impl Executable {
    /// The nodes to run, in dependency order.
    pub fn order(&self) -> &[TensorId] {
        &self.order
    }

    /// The placeholders this executable reads, with the types they must be fed with.
    pub fn feeds(&self) -> &[(TensorId, TensorType)] {
        &self.feeds
    }

    /// The requested outputs, with their types.
    pub fn outputs(&self) -> &[(TensorId, TensorType)] {
        &self.outputs
    }

    /// Whether reduced-precision fast math was allowed when the graph was compiled.
    pub fn reduced_precision_fast_math(&self) -> bool {
        self.reduced_precision_fast_math
    }

    /// Checks a set of feeds against the signature: every placeholder is fed once with its exact type, and no
    /// feed names anything else.
    pub fn check_feeds(&self, feeds: &[(TensorId, TensorType)]) -> Result<(), GraphError> {
        for (id, ty) in &self.feeds {
            match feeds.iter().find(|(f, _)| f == id) {
                None => return Err(GraphError::MissingFeed(*id)),
                Some((_, given)) if given != ty => return Err(GraphError::FeedType(*id)),
                Some(_) => {}
            }
        }
        for (id, _) in feeds {
            if !self.feeds.iter().any(|(f, _)| f == id) {
                return Err(GraphError::NotAFeed(*id));
            }
        }
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

    #[test]
    fn matmul_broadcasts_the_batch_and_refuses_a_mismatched_inner_dimension() {
        let mut g = Graph::new();
        let a = g.placeholder(DataType::Float32, &[8, 1, 4, 5]);
        let b = g.placeholder(DataType::Float32, &[3, 5, 2]);
        let y = g.matmul(a, b).unwrap();
        assert_eq!(g.node(y).unwrap().ty, TensorType::new(DataType::Float32, &[8, 3, 4, 2]));
        let bad = g.placeholder(DataType::Float32, &[4, 2]);
        assert_eq!(g.matmul(a, bad), Err(GraphError::Shape(ShapeError::Incompatible)));
        let vector = g.placeholder(DataType::Float32, &[5]);
        assert_eq!(g.matmul(vector, b), Err(GraphError::Shape(ShapeError::Rank)));
    }

    #[test]
    fn transpose_permutes_dimensions_and_refuses_a_bad_order() {
        let mut g = Graph::new();
        let x = g.placeholder(DataType::Int32, &[2, 3, 4]);
        let t = g.transpose(x, &[2, 0, 1]).unwrap();
        assert_eq!(g.node(t).unwrap().ty.shape, Shape::new(&[4, 2, 3]));
        assert_eq!(g.transpose(x, &[0, 0, 1]), Err(GraphError::Shape(ShapeError::Permutation)));
        assert_eq!(g.transpose(x, &[0, 1]), Err(GraphError::Shape(ShapeError::Rank)));
    }

    #[test]
    fn concat_adds_the_axis_and_refuses_disagreeing_parts() {
        let mut g = Graph::new();
        let a = g.placeholder(DataType::Float16, &[2, 3]);
        let b = g.placeholder(DataType::Float16, &[2, 5]);
        let c = g.concat(&[a, b], 1).unwrap();
        assert_eq!(g.node(c).unwrap().ty.shape, Shape::new(&[2, 8]));
        let wrong_rows = g.placeholder(DataType::Float16, &[3, 5]);
        assert_eq!(g.concat(&[a, wrong_rows], 1), Err(GraphError::Shape(ShapeError::Incompatible)));
        assert_eq!(g.concat(&[a, b], 2), Err(GraphError::Shape(ShapeError::Axis)));
        assert_eq!(g.concat(&[], 0), Err(GraphError::Shape(ShapeError::Empty)));
    }

    #[test]
    fn cast_changes_the_element_type_and_keeps_the_shape() {
        let mut g = Graph::new();
        let x = g.placeholder(DataType::Float32, &[4, 2]);
        let y = g.cast(x, DataType::Float16).unwrap();
        assert_eq!(g.node(y).unwrap().ty, TensorType::new(DataType::Float16, &[4, 2]));
        assert_eq!(g.node(y).unwrap().kind, Kind::Cast(x, DataType::Float16));
    }

    #[test]
    fn unary_operators_refuse_bool_and_non_float_roots() {
        let mut g = Graph::new();
        let flag = g.placeholder(DataType::Bool, &[2]);
        let count = g.placeholder(DataType::Int32, &[2]);
        let real = g.placeholder(DataType::Float32, &[2]);
        assert_eq!(g.unary(Unary::Negate, flag), Err(GraphError::Unsupported(DataType::Bool)));
        assert_eq!(g.unary(Unary::Sqrt, count), Err(GraphError::Unsupported(DataType::Int32)));
        assert!(g.unary(Unary::Exponential, real).is_ok());
    }

    #[test]
    fn compile_reports_feeds_outputs_and_the_fast_math_option() {
        let mut g = Graph::new();
        g.set_options(Options { reduced_precision_fast_math: true });
        let a = g.placeholder(DataType::Float32, &[3]);
        let unused = g.placeholder(DataType::Float32, &[7]);
        let k = g.constant(TensorData::from_f32(&[3], &[1.0, 2.0, 3.0]).unwrap());
        let y = g.binary(Binary::Multiply, a, k).unwrap();
        let exe = g.compile(&[y]).unwrap();
        assert_eq!(exe.feeds(), &[(a, TensorType::new(DataType::Float32, &[3]))]);
        assert_eq!(exe.outputs(), &[(y, TensorType::new(DataType::Float32, &[3]))]);
        assert!(!exe.order().contains(&unused));
        assert!(exe.reduced_precision_fast_math());
    }

    #[test]
    fn check_feeds_refuses_missing_wrong_typed_and_extra_feeds() {
        let mut g = Graph::new();
        let a = g.placeholder(DataType::Float32, &[2]);
        let b = g.placeholder(DataType::Int32, &[2]);
        let y = g.binary(Binary::Add, a, a).unwrap();
        let z = g.cast(b, DataType::Float32).unwrap();
        let both = g.binary(Binary::Add, y, z).unwrap();
        let exe = g.compile(&[both]).unwrap();
        let f32_2 = TensorType::new(DataType::Float32, &[2]);
        let i32_2 = TensorType::new(DataType::Int32, &[2]);
        assert_eq!(exe.check_feeds(&[(a, f32_2.clone())]), Err(GraphError::MissingFeed(b)));
        assert_eq!(exe.check_feeds(&[(a, i32_2.clone()), (b, i32_2.clone())]), Err(GraphError::FeedType(a)));
        assert_eq!(
            exe.check_feeds(&[(a, f32_2.clone()), (b, i32_2.clone()), (y, f32_2.clone())]),
            Err(GraphError::NotAFeed(y))
        );
        assert_eq!(exe.check_feeds(&[(a, f32_2), (b, i32_2)]), Ok(()));
    }
}
