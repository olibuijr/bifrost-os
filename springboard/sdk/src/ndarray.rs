//! Multidimensional arrays: `NDArray`, `NDArray.ScalarType` and `NDArrayDescriptor`.
//!
//! Storage is packed and row-major (`preferredStrides`: the last dimension is contiguous). The scalar type is
//! carried by the [`Scalar`] trait, so an `NDArray<f32>` and an `NDArray<u8>` cannot be mixed by mistake. Not
//! modelled: dynamic dimensions, interleaved layouts, `Float16`, `Int64`, and the raw or typed memory views.

use std::fmt;

/// The scalar types an array can hold (`NDArray.ScalarType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScalarType {
    Int8,
    Int16,
    Int32,
    UInt8,
    Float32,
    Float64,
}

impl ScalarType {
    /// Bytes per scalar in the packed storage.
    pub fn byte_size(self) -> usize {
        match self {
            ScalarType::Int8 | ScalarType::UInt8 => 1,
            ScalarType::Int16 => 2,
            ScalarType::Int32 | ScalarType::Float32 => 4,
            ScalarType::Float64 => 8,
        }
    }
}

/// A Rust type that can be stored in an [`NDArray`].
pub trait Scalar: Copy + Default + PartialEq + fmt::Debug {
    const SCALAR_TYPE: ScalarType;
}

impl Scalar for i8 {
    const SCALAR_TYPE: ScalarType = ScalarType::Int8;
}
impl Scalar for i16 {
    const SCALAR_TYPE: ScalarType = ScalarType::Int16;
}
impl Scalar for i32 {
    const SCALAR_TYPE: ScalarType = ScalarType::Int32;
}
impl Scalar for u8 {
    const SCALAR_TYPE: ScalarType = ScalarType::UInt8;
}
impl Scalar for f32 {
    const SCALAR_TYPE: ScalarType = ScalarType::Float32;
}
impl Scalar for f64 {
    const SCALAR_TYPE: ScalarType = ScalarType::Float64;
}

/// Errors from building or indexing an array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NDArrayError {
    /// No dimensions, a zero-length dimension, or a size that overflows.
    InvalidShape(Vec<usize>),
    /// The number of scalars does not match the shape.
    CountMismatch { expected: usize, got: usize },
    /// The index has the wrong rank, or a coordinate is past its dimension.
    IndexOutOfBounds { shape: Vec<usize>, index: Vec<usize> },
}

impl fmt::Display for NDArrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NDArrayError::InvalidShape(shape) => write!(f, "invalid array shape {shape:?}"),
            NDArrayError::CountMismatch { expected, got } => {
                write!(f, "expected {expected} scalars, got {got}")
            }
            NDArrayError::IndexOutOfBounds { shape, index } => {
                write!(f, "index {index:?} is outside shape {shape:?}")
            }
        }
    }
}

impl std::error::Error for NDArrayError {}

/// `NDArrayDescriptor`: the shape and scalar type of an array, without its values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NDArrayDescriptor {
    shape: Vec<usize>,
    scalar_type: ScalarType,
    count: usize,
}

fn checked_count(shape: &[usize]) -> Option<usize> {
    shape.iter().try_fold(1usize, |acc, &d| acc.checked_mul(d))
}

impl NDArrayDescriptor {
    /// Validates the shape: at least one dimension, every dimension non-zero, and the byte size representable.
    pub fn new(shape: &[usize], scalar_type: ScalarType) -> Result<Self, NDArrayError> {
        let invalid = || NDArrayError::InvalidShape(shape.to_vec());
        if shape.is_empty() || shape.contains(&0) {
            return Err(invalid());
        }
        let count = checked_count(shape).ok_or_else(|| invalid())?;
        if count.checked_mul(scalar_type.byte_size()).is_none() {
            return Err(invalid());
        }
        Ok(Self { shape: shape.to_vec(), scalar_type, count })
    }

    /// `shape`: the length of each dimension.
    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    /// `scalarType`.
    pub fn scalar_type(&self) -> ScalarType {
        self.scalar_type
    }

    /// `rank`: the number of dimensions.
    pub fn rank(&self) -> usize {
        self.shape.len()
    }

    /// The number of scalars (the product of the dimensions).
    pub fn element_count(&self) -> usize {
        self.count
    }

    /// `preferredStrides`: distance in scalars between neighbours along each dimension, row-major.
    pub fn preferred_strides(&self) -> Vec<usize> {
        let mut strides = vec![1usize; self.shape.len()];
        for i in (0..self.shape.len().saturating_sub(1)).rev() {
            strides[i] = strides[i + 1] * self.shape[i + 1];
        }
        strides
    }

    /// `minimumByteCount`: the storage the packed array needs.
    pub fn minimum_byte_count(&self) -> usize {
        self.count * self.scalar_type.byte_size()
    }

    /// Flat row-major position of `index`. Fails when the rank differs or a coordinate is out of range.
    pub fn offset(&self, index: &[usize]) -> Result<usize, NDArrayError> {
        let outside = index.len() != self.shape.len() || index.iter().zip(&self.shape).any(|(i, d)| i >= d);
        if outside {
            return Err(NDArrayError::IndexOutOfBounds { shape: self.shape.clone(), index: index.to_vec() });
        }
        Ok(index.iter().zip(self.preferred_strides()).map(|(i, s)| i * s).sum::<usize>())
    }
}

/// `NDArray`: a multidimensional array of scalars in row-major order.
#[derive(Clone, Debug, PartialEq)]
pub struct NDArray<T: Scalar> {
    descriptor: NDArrayDescriptor,
    scalars: Vec<T>,
}

impl<T: Scalar> NDArray<T> {
    /// `init(scalars:shape:)`: takes the scalars in row-major order. Their count must equal the shape's.
    pub fn new(scalars: Vec<T>, shape: &[usize]) -> Result<Self, NDArrayError> {
        let descriptor = NDArrayDescriptor::new(shape, T::SCALAR_TYPE)?;
        if scalars.len() != descriptor.element_count() {
            return Err(NDArrayError::CountMismatch { expected: descriptor.element_count(), got: scalars.len() });
        }
        Ok(Self { descriptor, scalars })
    }

    /// `init(shape:scalarType:)`: every scalar set to the type's default (zero).
    pub fn zeros(shape: &[usize]) -> Result<Self, NDArrayError> {
        let descriptor = NDArrayDescriptor::new(shape, T::SCALAR_TYPE)?;
        let scalars = vec![T::default(); descriptor.element_count()];
        Ok(Self { descriptor, scalars })
    }

    /// `descriptor`: the shape and scalar type.
    pub fn descriptor(&self) -> &NDArrayDescriptor {
        &self.descriptor
    }

    /// `shape`.
    pub fn shape(&self) -> &[usize] {
        self.descriptor.shape()
    }

    /// `scalarType`.
    pub fn scalar_type(&self) -> ScalarType {
        T::SCALAR_TYPE
    }

    /// The scalars in row-major order.
    pub fn scalars(&self) -> &[T] {
        &self.scalars
    }

    /// The scalar at `index`.
    pub fn get(&self, index: &[usize]) -> Result<T, NDArrayError> {
        let at = self.descriptor.offset(index)?;
        Ok(self.scalars[at])
    }

    /// Writes the scalar at `index`.
    pub fn set(&mut self, index: &[usize], value: T) -> Result<(), NDArrayError> {
        let at = self.descriptor.offset(index)?;
        self.scalars[at] = value;
        Ok(())
    }

    /// A copy with the same scalars in a new shape of the same element count.
    pub fn reshaped(&self, shape: &[usize]) -> Result<Self, NDArrayError> {
        Self::new(self.scalars.clone(), shape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_strides_are_row_major() {
        let d = NDArrayDescriptor::new(&[2, 3, 4], ScalarType::Float32).unwrap();
        assert_eq!(d.preferred_strides(), vec![12, 4, 1]);
        assert_eq!(d.element_count(), 24);
        assert_eq!(d.rank(), 3);
        assert_eq!(d.minimum_byte_count(), 96);
    }

    #[test]
    fn offset_maps_index_and_rejects_out_of_range() {
        let d = NDArrayDescriptor::new(&[2, 3], ScalarType::UInt8).unwrap();
        assert_eq!(d.offset(&[1, 2]), Ok(5));
        assert!(matches!(d.offset(&[2, 0]), Err(NDArrayError::IndexOutOfBounds { .. })));
        assert!(matches!(d.offset(&[0]), Err(NDArrayError::IndexOutOfBounds { .. })));
    }

    #[test]
    fn rejects_empty_zero_and_overflowing_shapes() {
        assert!(NDArrayDescriptor::new(&[], ScalarType::Float32).is_err());
        assert!(NDArrayDescriptor::new(&[3, 0], ScalarType::Float32).is_err());
        assert!(NDArrayDescriptor::new(&[usize::MAX, 2], ScalarType::Float64).is_err());
    }

    #[test]
    fn new_checks_the_scalar_count() {
        assert_eq!(
            NDArray::new(vec![1.0f32; 5], &[2, 3]),
            Err(NDArrayError::CountMismatch { expected: 6, got: 5 })
        );
        assert!(NDArray::new(vec![1.0f32; 6], &[2, 3]).is_ok());
    }

    #[test]
    fn set_then_get_and_reshape_keeps_row_major_order() {
        let mut a = NDArray::<i32>::zeros(&[2, 2]).unwrap();
        a.set(&[1, 0], 7).unwrap();
        assert_eq!(a.get(&[1, 0]), Ok(7));
        assert_eq!(a.scalar_type(), ScalarType::Int32);
        let flat = a.reshaped(&[4]).unwrap();
        assert_eq!(flat.scalars().to_vec(), vec![0, 0, 7, 0]);
        assert!(a.reshaped(&[3]).is_err());
    }
}
