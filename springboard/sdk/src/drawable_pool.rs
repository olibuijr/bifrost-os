//! Drawable pool: the small, fixed set of frame buffers an application draws into before a frame is shown.
//!
//! A drawable moves through three states. `next_drawable` takes a free drawable for drawing. `present` hands the drawn
//! frame to the presenter, which keeps it until the display no longer needs it, and `release` returns it to the pool.
//! A drawing that will not be shown goes back with `abandon`. The pool holds two or three drawables, as the presenter
//! expects; when all of them are busy `next_drawable` gives None and the caller skips the frame.

/// The fewest drawables a pool may hold: one being shown and one being drawn.
pub const MIN_DRAWABLES: usize = 2;
/// The most drawables a pool may hold.
pub const MAX_DRAWABLES: usize = 3;

/// The position of a drawable in its pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawableId(pub usize);

/// Why a pool call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoolError {
    /// The pool size is outside `MIN_DRAWABLES..=MAX_DRAWABLES`.
    CountOutOfRange(usize),
    /// A width or height is zero.
    EmptySize,
    /// The drawable is not in the state the call needs (for example, it was not being drawn).
    WrongState(DrawableId),
    /// The size cannot change while a drawable is being drawn or shown.
    InUse,
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolError::CountOutOfRange(n) => write!(f, "a drawable pool holds {MIN_DRAWABLES} to {MAX_DRAWABLES} drawables, not {n}"),
            PoolError::EmptySize => write!(f, "a drawable needs a width and a height above zero"),
            PoolError::WrongState(id) => write!(f, "drawable {} is not in the state this call needs", id.0),
            PoolError::InUse => write!(f, "the drawable size cannot change while a drawable is in use"),
        }
    }
}

impl std::error::Error for PoolError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    Drawing,
    Queued,
}

/// A fixed set of drawables of one size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawablePool {
    width: u32,
    height: u32,
    states: Vec<State>,
}

impl DrawablePool {
    /// A pool of `count` free drawables, each `width` by `height` pixels.
    pub fn new(count: usize, width: u32, height: u32) -> Result<DrawablePool, PoolError> {
        if !(MIN_DRAWABLES..=MAX_DRAWABLES).contains(&count) {
            return Err(PoolError::CountOutOfRange(count));
        }
        check_size(width, height)?;
        Ok(DrawablePool { width, height, states: vec![State::Free; count] })
    }

    /// The number of drawables in the pool.
    pub fn count(&self) -> usize {
        self.states.len()
    }

    /// The size of every drawable in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The number of drawables that `next_drawable` could hand out now.
    pub fn free_count(&self) -> usize {
        self.states.iter().filter(|s| **s == State::Free).count()
    }

    /// Takes a free drawable for drawing, or None when every drawable is busy.
    pub fn next_drawable(&mut self) -> Option<DrawableId> {
        let index = self.states.iter().position(|s| *s == State::Free)?;
        self.states[index] = State::Drawing;
        Some(DrawableId(index))
    }

    /// Hands a drawn frame to the presenter. The drawable stays busy until it is released.
    pub fn present(&mut self, id: DrawableId) -> Result<(), PoolError> {
        self.move_from(id, State::Drawing, State::Queued)
    }

    /// Returns a presented drawable to the pool once the display no longer needs it.
    pub fn release(&mut self, id: DrawableId) -> Result<(), PoolError> {
        self.move_from(id, State::Queued, State::Free)
    }

    /// Returns a drawable that is being drawn to the pool without presenting it.
    pub fn abandon(&mut self, id: DrawableId) -> Result<(), PoolError> {
        self.move_from(id, State::Drawing, State::Free)
    }

    /// Changes the size of every drawable. Refused while any drawable is being drawn or shown.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), PoolError> {
        check_size(width, height)?;
        if self.states.iter().any(|s| *s != State::Free) {
            return Err(PoolError::InUse);
        }
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn move_from(&mut self, id: DrawableId, from: State, to: State) -> Result<(), PoolError> {
        match self.states.get_mut(id.0) {
            Some(state) if *state == from => {
                *state = to;
                Ok(())
            }
            _ => Err(PoolError::WrongState(id)),
        }
    }
}

fn check_size(width: u32, height: u32) -> Result<(), PoolError> {
    if width == 0 || height == 0 {
        Err(PoolError::EmptySize)
    } else {
        Ok(())
    }
}
