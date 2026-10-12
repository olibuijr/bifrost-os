//! Named state buffers that a stateful model keeps from one prediction to the next. A [`StateConstraint`] fixes a
//! buffer's name and shape. [`ModelState::new`] creates every buffer zeroed, and the caller reads, writes, adds to and
//! resets the buffers between predictions. The buffers are plain tensors: nothing here runs a model, and a buffer is
//! only ever changed through a checked write or add, so its shape always matches its constraint.
use crate::ml_tensor::{Tensor, TensorError};

/// The name and shape that a state buffer must keep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateConstraint {
    name: String,
    shape: Vec<usize>,
}

impl StateConstraint {
    pub fn new(name: &str, shape: &[usize]) -> Self {
        StateConstraint { name: name.to_owned(), shape: shape.to_vec() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn shape(&self) -> &[usize] {
        &self.shape
    }
}

/// Why a state buffer could not be created, read or changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateError {
    /// Two constraints share a name.
    Duplicate(String),
    /// No buffer has this name.
    Unknown(String),
    /// A value's shape differs from the buffer's constraint.
    Shape { expected: Vec<usize>, found: Vec<usize> },
    /// A tensor could not be created for the sum.
    Tensor(TensorError),
}

/// The state buffers of one stateful model, in the order their constraints were given.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelState {
    buffers: Vec<(StateConstraint, Tensor)>,
}

impl ModelState {
    /// Creates one zeroed buffer per constraint. Names must be unique.
    pub fn new(constraints: &[StateConstraint]) -> Result<Self, StateError> {
        for (i, c) in constraints.iter().enumerate() {
            if constraints[..i].iter().any(|other| other.name == c.name) {
                return Err(StateError::Duplicate(c.name.clone()));
            }
        }
        let buffers = constraints.iter().map(|c| (c.clone(), Tensor::zeros(&c.shape))).collect();
        Ok(ModelState { buffers })
    }

    /// The buffer names, in constraint order.
    pub fn names(&self) -> Vec<&str> {
        self.buffers.iter().map(|(c, _)| c.name()).collect()
    }

    /// The current contents of a buffer.
    pub fn read(&self, name: &str) -> Result<&Tensor, StateError> {
        let i = self.position(name)?;
        Ok(&self.buffers[i].1)
    }

    /// Replaces a buffer's contents. The value must have the buffer's declared shape.
    pub fn write(&mut self, name: &str, value: Tensor) -> Result<(), StateError> {
        let i = self.position(name)?;
        let expected = self.buffers[i].0.shape.clone();
        if value.shape() != expected.as_slice() {
            return Err(StateError::Shape { expected, found: value.shape().to_vec() });
        }
        self.buffers[i].1 = value;
        Ok(())
    }

    /// Adds `delta` to a buffer element by element, which is how a running total is kept across predictions.
    pub fn add(&mut self, name: &str, delta: &Tensor) -> Result<(), StateError> {
        let i = self.position(name)?;
        let expected = self.buffers[i].0.shape.clone();
        if delta.shape() != expected.as_slice() {
            return Err(StateError::Shape { expected, found: delta.shape().to_vec() });
        }
        let sum: Vec<f32> = self.buffers[i].1.values().iter().zip(delta.values()).map(|(a, b)| a + b).collect();
        self.buffers[i].1 = Tensor::new(&expected, sum).map_err(StateError::Tensor)?;
        Ok(())
    }

    /// Zeroes every buffer, as at the start of a new sequence.
    pub fn reset(&mut self) {
        for (c, t) in self.buffers.iter_mut() {
            *t = Tensor::zeros(&c.shape);
        }
    }

    fn position(&self, name: &str) -> Result<usize, StateError> {
        self.buffers
            .iter()
            .position(|(c, _)| c.name == name)
            .ok_or_else(|| StateError::Unknown(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constraints() -> Vec<StateConstraint> {
        vec![StateConstraint::new("acc", &[2]), StateConstraint::new("h", &[1, 3])]
    }

    #[test]
    fn new_creates_a_zeroed_buffer_of_each_declared_shape() {
        let s = ModelState::new(&constraints()).unwrap();
        assert_eq!(s.names(), vec!["acc", "h"]);
        assert_eq!(s.read("acc").unwrap().values(), &[0.0, 0.0]);
        let h = s.read("h").unwrap();
        assert_eq!((h.shape(), h.values().len()), (&[1usize, 3][..], 3));
    }

    #[test]
    fn duplicate_names_are_refused() {
        let dup = vec![StateConstraint::new("acc", &[2]), StateConstraint::new("acc", &[4])];
        assert_eq!(ModelState::new(&dup), Err(StateError::Duplicate("acc".into())));
    }

    #[test]
    fn write_replaces_a_buffer_of_its_declared_shape() {
        let mut s = ModelState::new(&constraints()).unwrap();
        s.write("acc", Tensor::new(&[2], vec![1.0, 2.0]).unwrap()).unwrap();
        assert_eq!(s.read("acc").unwrap().values(), &[1.0, 2.0]);
    }

    #[test]
    fn write_refuses_a_value_of_another_shape() {
        let mut s = ModelState::new(&constraints()).unwrap();
        let err = s.write("acc", Tensor::zeros(&[3]));
        assert_eq!(err, Err(StateError::Shape { expected: vec![2], found: vec![3] }));
        assert_eq!(s.read("acc").unwrap().values(), &[0.0, 0.0], "a refused write leaves the buffer as it was");
    }

    #[test]
    fn add_keeps_a_running_total_across_predictions() {
        let mut s = ModelState::new(&constraints()).unwrap();
        s.add("acc", &Tensor::new(&[2], vec![1.0, 2.0]).unwrap()).unwrap();
        s.add("acc", &Tensor::new(&[2], vec![0.5, 0.5]).unwrap()).unwrap();
        assert_eq!(s.read("acc").unwrap().values(), &[1.5, 2.5]);
    }

    #[test]
    fn reset_zeroes_every_buffer() {
        let mut s = ModelState::new(&constraints()).unwrap();
        s.write("acc", Tensor::new(&[2], vec![3.0, 4.0]).unwrap()).unwrap();
        s.write("h", Tensor::new(&[1, 3], vec![1.0, 1.0, 1.0]).unwrap()).unwrap();
        s.reset();
        assert_eq!(s.read("acc").unwrap().values(), &[0.0, 0.0]);
        assert_eq!(s.read("h").unwrap().values(), &[0.0, 0.0, 0.0]);
    }

    #[test]
    fn an_unknown_buffer_name_is_refused() {
        let mut s = ModelState::new(&constraints()).unwrap();
        assert_eq!(s.read("missing"), Err(StateError::Unknown("missing".into())));
        assert_eq!(s.add("missing", &Tensor::zeros(&[2])), Err(StateError::Unknown("missing".into())));
    }
}
