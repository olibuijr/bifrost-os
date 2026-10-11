//! Tensor types for compute graphs: element data types, shapes with row-major strides, the broadcasting rule that
//! element-wise operators use, and dense payloads whose byte length is checked against their type.
//! Pure data: nothing here allocates a device buffer or runs arithmetic beyond the size checks.

/// Element type of a tensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataType {
    Float16,
    BFloat16,
    Float32,
    Int8,
    UInt8,
    Int16,
    Int32,
    Bool,
}

impl DataType {
    /// True for the floating-point element types.
    pub fn is_float(self) -> bool {
        matches!(self, DataType::Float16 | DataType::BFloat16 | DataType::Float32)
    }

    /// Bytes per element.
    pub fn size(self) -> usize {
        match self {
            DataType::Int8 | DataType::UInt8 | DataType::Bool => 1,
            DataType::Float16 | DataType::BFloat16 | DataType::Int16 => 2,
            DataType::Float32 | DataType::Int32 => 4,
        }
    }
}

/// Why a shape, reshape or payload was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// Two shapes do not broadcast: some aligned dimensions differ and neither is 1.
    Incompatible,
    /// A reshape would change the element count.
    ElementCount,
    /// A payload's byte length differs from what its type needs.
    ByteCount,
    /// The operator needs a different number of dimensions (for example a matrix product on a vector).
    Rank,
    /// A concatenation or reduction axis is outside the tensor's dimensions.
    Axis,
    /// A transpose order is not a permutation of the dimensions.
    Permutation,
    /// A concatenation was given no inputs.
    Empty,
}

/// The dimensions of a tensor, outermost first. A scalar has no dimensions and one element.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Shape(Vec<usize>);

impl Shape {
    pub fn new(dims: &[usize]) -> Self {
        Shape(dims.to_vec())
    }

    pub fn dims(&self) -> &[usize] {
        &self.0
    }

    pub fn rank(&self) -> usize {
        self.0.len()
    }

    /// Number of elements, or `None` when the product overflows.
    pub fn elements(&self) -> Option<usize> {
        self.0.iter().try_fold(1usize, |acc, &d| acc.checked_mul(d))
    }

    /// Row-major strides in elements (not bytes). The last dimension is contiguous.
    pub fn strides(&self) -> Vec<usize> {
        let mut out = vec![1usize; self.0.len()];
        for i in (0..self.0.len().saturating_sub(1)).rev() {
            out[i] = out[i + 1] * self.0[i + 1];
        }
        out
    }

    /// NumPy-style broadcast: shapes align from the end, and a pair of dimensions is compatible when they are equal
    /// or one of them is 1. A missing leading dimension counts as 1.
    pub fn broadcast(&self, other: &Shape) -> Result<Shape, ShapeError> {
        let n = self.rank().max(other.rank());
        let mut out = Vec::with_capacity(n);
        for k in (1..=n).rev() {
            let a = dim_from_end(&self.0, k);
            let b = dim_from_end(&other.0, k);
            out.push(match (a, b) {
                _ if a == b => a,
                (1, _) => b,
                (_, 1) => a,
                _ => return Err(ShapeError::Incompatible),
            });
        }
        Ok(Shape(out))
    }

    /// A reshape to `dims`, which must keep the element count.
    pub fn reshaped(&self, dims: &[usize]) -> Result<Shape, ShapeError> {
        let to = Shape::new(dims);
        match (self.elements(), to.elements()) {
            (Some(a), Some(b)) if a == b => Ok(to),
            _ => Err(ShapeError::ElementCount),
        }
    }
}

/// The `k`-th dimension counted from the end (1 is the last). Missing leading dimensions are 1.
fn dim_from_end(dims: &[usize], k: usize) -> usize {
    if k <= dims.len() {
        dims[dims.len() - k]
    } else {
        1
    }
}

/// A tensor's element type and shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorType {
    pub data_type: DataType,
    pub shape: Shape,
}

impl TensorType {
    pub fn new(data_type: DataType, dims: &[usize]) -> Self {
        TensorType { data_type, shape: Shape::new(dims) }
    }

    /// Bytes a dense payload of this type takes, or `None` when the size overflows.
    pub fn byte_len(&self) -> Option<usize> {
        self.shape.elements()?.checked_mul(self.data_type.size())
    }
}

/// A dense, row-major payload for a tensor type. The byte length is checked once, at construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorData {
    ty: TensorType,
    bytes: Vec<u8>,
}

impl TensorData {
    pub fn new(ty: TensorType, bytes: Vec<u8>) -> Result<Self, ShapeError> {
        match ty.byte_len() {
            Some(n) if n == bytes.len() => Ok(TensorData { ty, bytes }),
            _ => Err(ShapeError::ByteCount),
        }
    }

    /// A Float32 tensor from values in row-major order, stored little-endian.
    pub fn from_f32(dims: &[usize], values: &[f32]) -> Result<Self, ShapeError> {
        let bytes = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        Self::new(TensorType::new(DataType::Float32, dims), bytes)
    }

    pub fn ty(&self) -> &TensorType {
        &self.ty
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The values as f32 when the element type is Float32, otherwise `None`.
    pub fn f32_values(&self) -> Option<Vec<f32>> {
        if self.ty.data_type != DataType::Float32 {
            return None;
        }
        Some(
            self.bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_sizes_and_payload_byte_lengths() {
        assert_eq!(DataType::Bool.size(), 1);
        assert_eq!(DataType::BFloat16.size(), 2);
        assert_eq!(DataType::Int32.size(), 4);
        assert_eq!(TensorType::new(DataType::Float32, &[2, 3]).byte_len(), Some(24));
        assert_eq!(TensorType::new(DataType::Float16, &[]).byte_len(), Some(2), "a scalar holds one element");
        assert_eq!(TensorType::new(DataType::Int8, &[usize::MAX, 2]).byte_len(), None, "overflow is reported");
    }

    #[test]
    fn strides_are_row_major_and_the_last_dimension_is_contiguous() {
        assert_eq!(Shape::new(&[2, 3, 4]).strides(), vec![12, 4, 1]);
        assert_eq!(Shape::new(&[5]).strides(), vec![1]);
        assert!(Shape::new(&[]).strides().is_empty());
        assert_eq!(Shape::new(&[]).elements(), Some(1));
    }

    #[test]
    fn broadcasting_aligns_from_the_end_and_stretches_ones() {
        let a = Shape::new(&[4, 1, 3]);
        let b = Shape::new(&[2, 1]);
        assert_eq!(a.broadcast(&b).unwrap(), Shape::new(&[4, 2, 3]));
        assert_eq!(Shape::new(&[3]).broadcast(&Shape::new(&[2, 3])).unwrap(), Shape::new(&[2, 3]));
        assert_eq!(Shape::new(&[]).broadcast(&Shape::new(&[7])).unwrap(), Shape::new(&[7]));
    }

    #[test]
    fn shapes_that_do_not_broadcast_are_refused() {
        let err = Shape::new(&[2, 3]).broadcast(&Shape::new(&[4]));
        assert_eq!(err, Err(ShapeError::Incompatible));
        assert_eq!(Shape::new(&[2, 3]).broadcast(&Shape::new(&[3, 2])), Err(ShapeError::Incompatible));
    }

    #[test]
    fn a_reshape_must_keep_the_element_count() {
        let s = Shape::new(&[2, 3, 4]);
        assert_eq!(s.reshaped(&[6, 4]).unwrap(), Shape::new(&[6, 4]));
        assert_eq!(s.reshaped(&[5, 5]), Err(ShapeError::ElementCount));
    }

    #[test]
    fn f32_payloads_round_trip_and_wrong_byte_counts_are_refused() {
        let d = TensorData::from_f32(&[2, 2], &[1.0, -2.5, 3.0, 0.25]).unwrap();
        assert_eq!(d.bytes().len(), 16);
        assert_eq!(d.f32_values(), Some(vec![1.0, -2.5, 3.0, 0.25]));
        assert_eq!(d.ty(), &TensorType::new(DataType::Float32, &[2, 2]));
        let short = TensorData::new(TensorType::new(DataType::Float32, &[2]), vec![0; 7]);
        assert_eq!(short, Err(ShapeError::ByteCount));
        let ints = TensorData::new(TensorType::new(DataType::Int16, &[1]), vec![1, 0]).unwrap();
        assert_eq!(ints.f32_values(), None, "only Float32 payloads read back as f32");
    }
}
