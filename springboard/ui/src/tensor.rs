//! Multi-dimensional numeric arrays for model inputs and outputs (`MLTensor`): a row-major grid of `f32` scalars with a
//! shape, reshaping, slicing along one axis (`MLTensorRangeExpression`) and element-wise minimum and maximum
//! (`pointwiseMin`, `pointwiseMax`). Running a model is not here; this is the data a runner hands to and from a model.
use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    shape: Vec<usize>,
    data: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TensorError {
    /// The number of values does not match the element count of the shape.
    CountMismatch { expected: usize, found: usize },
    /// An axis, index or range lies outside the shape.
    OutOfBounds,
    /// Element-wise operations need tensors of the same shape. Broadcasting is not implemented.
    ShapeMismatch,
}

/// Element count of a shape. An empty shape is a scalar with one element. Overflow counts as "too many".
fn count_of(shape: &[usize]) -> usize {
    shape.iter().try_fold(1usize, |n, d| n.checked_mul(*d)).unwrap_or(usize::MAX)
}

impl Tensor {
    pub fn new(shape: &[usize], data: Vec<f32>) -> Result<Self, TensorError> {
        let expected = count_of(shape);
        if expected != data.len() {
            return Err(TensorError::CountMismatch { expected, found: data.len() });
        }
        Ok(Self { shape: shape.to_vec(), data })
    }

    pub fn zeros(shape: &[usize]) -> Self {
        Self { shape: shape.to_vec(), data: vec![0.0; count_of(shape)] }
    }

    pub fn scalar(value: f32) -> Self {
        Self { shape: Vec::new(), data: vec![value] }
    }

    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    pub fn rank(&self) -> usize {
        self.shape.len()
    }

    /// The values in row-major order (the last axis changes fastest).
    pub fn values(&self) -> &[f32] {
        &self.data
    }

    /// The value at `index`, or `None` when the index has the wrong rank or is outside the shape.
    pub fn get(&self, index: &[usize]) -> Option<f32> {
        if index.len() != self.shape.len() {
            return None;
        }
        let mut offset = 0;
        for (i, dim) in index.iter().zip(&self.shape) {
            if i >= dim {
                return None;
            }
            offset = offset * dim + i;
        }
        self.data.get(offset).copied()
    }

    /// The same values under another shape with the same element count.
    pub fn reshape(&self, shape: &[usize]) -> Result<Self, TensorError> {
        Self::new(shape, self.data.clone())
    }

    /// The part of the tensor where `axis` is in `range`. The axis keeps its place with the range's length.
    pub fn slice(&self, axis: usize, range: Range<usize>) -> Result<Self, TensorError> {
        let dim = *self.shape.get(axis).ok_or(TensorError::OutOfBounds)?;
        if range.start > range.end || range.end > dim {
            return Err(TensorError::OutOfBounds);
        }
        let outer = count_of(&self.shape[..axis]);
        let inner = count_of(&self.shape[axis + 1..]);
        let mut data = Vec::with_capacity(outer * range.len() * inner);
        for o in 0..outer {
            for d in range.clone() {
                let start = (o * dim + d) * inner;
                data.extend_from_slice(&self.data[start..start + inner]);
            }
        }
        let mut shape = self.shape.clone();
        shape[axis] = range.len();
        Ok(Self { shape, data })
    }

    pub fn pointwise_min(&self, other: &Self) -> Result<Self, TensorError> {
        self.zip_with(other, f32::min)
    }

    pub fn pointwise_max(&self, other: &Self) -> Result<Self, TensorError> {
        self.zip_with(other, f32::max)
    }

    fn zip_with(&self, other: &Self, f: impl Fn(f32, f32) -> f32) -> Result<Self, TensorError> {
        if self.shape != other.shape {
            return Err(TensorError::ShapeMismatch);
        }
        let data = self.data.iter().zip(&other.data).map(|(a, b)| f(*a, *b)).collect();
        Ok(Self { shape: self.shape.clone(), data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Tensor {
        Tensor::new(&[2, 3], vec![0., 1., 2., 3., 4., 5.]).unwrap()
    }

    #[test]
    fn new_checks_the_count_and_scalars_have_one_element() {
        assert_eq!(
            Tensor::new(&[2, 2], vec![1.0]),
            Err(TensorError::CountMismatch { expected: 4, found: 1 })
        );
        let s = Tensor::scalar(7.5);
        assert_eq!((s.rank(), s.shape(), s.values()), (0, &[][..], &[7.5][..]));
        assert_eq!(Tensor::zeros(&[3, 0]).values().len(), 0, "a zero-sized axis holds no values");
    }

    #[test]
    fn get_reads_row_major_and_refuses_bad_indices() {
        let t = grid();
        assert_eq!(t.get(&[0, 0]), Some(0.0));
        assert_eq!(t.get(&[1, 2]), Some(5.0), "the last axis changes fastest");
        assert_eq!(t.get(&[2, 0]), None, "outside the first axis");
        assert_eq!(t.get(&[1]), None, "wrong rank");
        assert_eq!(Tensor::scalar(3.0).get(&[]), Some(3.0));
    }

    #[test]
    fn reshape_keeps_the_values_and_rejects_another_count() {
        let t = grid().reshape(&[3, 2]).unwrap();
        assert_eq!(t.shape(), &[3, 2]);
        assert_eq!(t.get(&[2, 1]), Some(5.0));
        assert!(matches!(grid().reshape(&[4]), Err(TensorError::CountMismatch { expected: 4, found: 6 })));
    }

    #[test]
    fn slice_along_the_first_and_last_axes() {
        let rows = grid().slice(0, 1..2).unwrap();
        assert_eq!((rows.shape(), rows.values()), (&[1, 3][..], &[3., 4., 5.][..]));
        let cols = grid().slice(1, 1..3).unwrap();
        assert_eq!((cols.shape(), cols.values()), (&[2, 2][..], &[1., 2., 4., 5.][..]));
        let none = grid().slice(1, 2..2).unwrap();
        assert_eq!(none.shape(), &[2, 0]);
    }

    #[test]
    fn slice_refuses_axes_and_ranges_outside_the_shape() {
        assert_eq!(grid().slice(2, 0..1), Err(TensorError::OutOfBounds));
        assert_eq!(grid().slice(1, 0..4), Err(TensorError::OutOfBounds));
        assert_eq!(grid().slice(1, 2..1), Err(TensorError::OutOfBounds));
    }

    #[test]
    fn pointwise_min_and_max_compare_each_element() {
        let a = Tensor::new(&[3], vec![1., 5., -2.]).unwrap();
        let b = Tensor::new(&[3], vec![4., 2., -3.]).unwrap();
        assert_eq!(a.pointwise_min(&b).unwrap().values(), &[1., 2., -3.]);
        assert_eq!(a.pointwise_max(&b).unwrap().values(), &[4., 5., -2.]);
    }

    #[test]
    fn pointwise_operations_need_the_same_shape() {
        let a = Tensor::new(&[2], vec![1., 2.]).unwrap();
        let b = Tensor::new(&[1, 2], vec![1., 2.]).unwrap();
        assert_eq!(a.pointwise_min(&b), Err(TensorError::ShapeMismatch), "no broadcasting");
    }
}
