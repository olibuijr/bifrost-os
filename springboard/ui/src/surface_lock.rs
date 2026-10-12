//! CPU lock state for a shared buffer. Any number of read locks may be held together; a write lock is exclusive.
//! Releasing a write lock advances the seed, so a holder can tell that the contents changed while it was not looking.
//! Pure model: nothing is mapped or synchronised here. The reader count never wraps; the seed wraps as a token.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockError {
    /// A write lock is held, so no other lock can be taken.
    WriteLocked,
    /// Readers are active, so a write lock cannot be taken yet.
    ReadersActive,
    /// The release has no matching lock.
    NotLocked,
    /// The reader count would pass its maximum.
    Overflow,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceLock {
    readers: u32,
    writer: bool,
    seed: u32,
}

impl SurfaceLock {
    pub fn new() -> Self {
        SurfaceLock {
            readers: 0,
            writer: false,
            seed: 0,
        }
    }

    /// Changes every time a write lock is released.
    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn readers(&self) -> u32 {
        self.readers
    }

    pub fn is_write_locked(&self) -> bool {
        self.writer
    }

    pub fn is_locked(&self) -> bool {
        self.writer || self.readers > 0
    }

    /// Takes a read lock (`read_only`) or the exclusive write lock.
    pub fn lock(&mut self, read_only: bool) -> Result<(), LockError> {
        if self.writer {
            return Err(LockError::WriteLocked);
        }
        if read_only {
            self.readers = self.readers.checked_add(1).ok_or(LockError::Overflow)?;
            Ok(())
        } else {
            if self.readers > 0 {
                return Err(LockError::ReadersActive);
            }
            self.writer = true;
            Ok(())
        }
    }

    /// Releases one lock of the given kind and returns the seed after the release.
    pub fn unlock(&mut self, read_only: bool) -> Result<u32, LockError> {
        if read_only {
            if self.readers == 0 {
                return Err(LockError::NotLocked);
            }
            self.readers -= 1;
        } else {
            if !self.writer {
                return Err(LockError::NotLocked);
            }
            self.writer = false;
            self.seed = self.seed.wrapping_add(1);
        }
        Ok(self.seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_buffer_is_unlocked_with_seed_zero() {
        let l = SurfaceLock::new();
        assert!(!l.is_locked());
        assert_eq!(l.seed(), 0);
        assert_eq!(l.readers(), 0);
    }

    #[test]
    fn several_readers_share_the_buffer() {
        let mut l = SurfaceLock::new();
        assert_eq!(l.lock(true), Ok(()));
        assert_eq!(l.lock(true), Ok(()));
        assert_eq!(l.readers(), 2);
        assert!(l.is_locked());
    }

    #[test]
    fn a_reader_blocks_a_writer_and_a_writer_blocks_everyone() {
        let mut l = SurfaceLock::new();
        l.lock(true).unwrap();
        assert_eq!(l.lock(false), Err(LockError::ReadersActive));
        l.unlock(true).unwrap();
        assert_eq!(l.lock(false), Ok(()));
        assert_eq!(l.lock(true), Err(LockError::WriteLocked));
        assert_eq!(l.lock(false), Err(LockError::WriteLocked));
        assert!(l.is_write_locked());
    }

    #[test]
    fn releasing_a_write_lock_advances_the_seed_but_a_read_release_does_not() {
        let mut l = SurfaceLock::new();
        l.lock(false).unwrap();
        assert_eq!(l.unlock(false), Ok(1));
        l.lock(true).unwrap();
        assert_eq!(l.unlock(true), Ok(1));
        l.lock(false).unwrap();
        assert_eq!(l.unlock(false), Ok(2));
        assert_eq!(l.seed(), 2);
    }

    #[test]
    fn a_release_without_a_matching_lock_is_an_error() {
        let mut l = SurfaceLock::new();
        assert_eq!(l.unlock(true), Err(LockError::NotLocked));
        assert_eq!(l.unlock(false), Err(LockError::NotLocked));
        l.lock(true).unwrap();
        assert_eq!(l.unlock(false), Err(LockError::NotLocked));
        assert_eq!(l.readers(), 1);
    }
}
