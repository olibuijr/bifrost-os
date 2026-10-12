//! Use counts for a shared buffer. A count above zero means the buffer is in use and must not be reused or purged.
//! Pure model: the count never wraps. A decrement at zero and an increment past the maximum are errors.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseError {
    Underflow,
    Overflow,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UseCount {
    count: u32,
}

impl UseCount {
    pub fn new() -> Self {
        UseCount { count: 0 }
    }

    /// A counter that starts at `count` (used when a buffer is handed over with holders already attached).
    pub fn with_count(count: u32) -> Self {
        UseCount { count }
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn is_in_use(&self) -> bool {
        self.count > 0
    }

    /// Adds one holder and returns the new count.
    pub fn increment(&mut self) -> Result<u32, UseError> {
        self.count = self.count.checked_add(1).ok_or(UseError::Overflow)?;
        Ok(self.count)
    }

    /// Removes one holder and returns the new count. Fails at zero and leaves the count unchanged.
    pub fn decrement(&mut self) -> Result<u32, UseError> {
        if self.count == 0 {
            return Err(UseError::Underflow);
        }
        self.count -= 1;
        Ok(self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_buffer_is_not_in_use() {
        let u = UseCount::new();
        assert_eq!(u.count(), 0);
        assert!(!u.is_in_use());
    }

    #[test]
    fn increments_count_up_and_mark_the_buffer_in_use() {
        let mut u = UseCount::new();
        assert_eq!(u.increment(), Ok(1));
        assert_eq!(u.increment(), Ok(2));
        assert_eq!(u.count(), 2);
        assert!(u.is_in_use());
    }

    #[test]
    fn the_buffer_stays_in_use_until_every_holder_releases_it() {
        let mut u = UseCount::new();
        u.increment().unwrap();
        u.increment().unwrap();
        assert_eq!(u.decrement(), Ok(1));
        assert!(u.is_in_use());
        assert_eq!(u.decrement(), Ok(0));
        assert!(!u.is_in_use());
    }

    #[test]
    fn decrementing_at_zero_is_an_error_and_keeps_the_count() {
        let mut u = UseCount::new();
        assert_eq!(u.decrement(), Err(UseError::Underflow));
        assert_eq!(u.count(), 0);
    }

    #[test]
    fn incrementing_past_the_maximum_is_an_error_and_keeps_the_count() {
        let mut u = UseCount::with_count(u32::MAX);
        assert_eq!(u.increment(), Err(UseError::Overflow));
        assert_eq!(u.count(), u32::MAX);
        assert!(u.is_in_use());
    }

    #[test]
    fn a_handed_over_buffer_starts_with_its_holders() {
        let mut u = UseCount::with_count(3);
        assert!(u.is_in_use());
        assert_eq!(u.decrement(), Ok(2));
    }
}
