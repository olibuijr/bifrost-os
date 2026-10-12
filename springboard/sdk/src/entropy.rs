//! Randomization services (`SecRandomCopyBytes`): cryptographically secure random bytes for apps.
//!
//! The bytes come from the kernel's random pool (`/dev/urandom`), the same source the shell uses for password salts
//! and keychain keys. The call is synchronous and runs in the app, not in the shell: nothing is sent over the socket.
//! If the pool cannot be read the call fails with [`Error::Unavailable`] and the caller's buffer is left unchanged.
//! Use [`below`] for an unbiased whole number in a range; a plain `% n` would favour the low values.

use std::fs::File;
use std::io::Read;

/// The kernel random pool that every call reads from.
pub const RANDOM_POOL: &str = "/dev/urandom";

/// Why a random draw failed (the Security framework's `errSecAllocate` role).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The random pool could not be opened or read in full.
    Unavailable,
}

/// Fills `buf` with random bytes. An empty buffer succeeds without touching the pool.
pub fn copy_bytes(buf: &mut [u8]) -> Result<(), Error> {
    if buf.is_empty() {
        return Ok(());
    }
    // Read into a scratch buffer first, so a short read never leaves the caller with half-filled bytes.
    let mut scratch = vec![0u8; buf.len()];
    let mut pool = File::open(RANDOM_POOL).map_err(|_| Error::Unavailable)?;
    pool.read_exact(&mut scratch).map_err(|_| Error::Unavailable)?;
    buf.copy_from_slice(&scratch);
    Ok(())
}

/// `count` random bytes in a new vector.
pub fn bytes(count: usize) -> Result<Vec<u8>, Error> {
    let mut out = vec![0u8; count];
    copy_bytes(&mut out)?;
    Ok(out)
}

/// One random 64-bit word.
pub fn random_u64() -> Result<u64, Error> {
    let mut word = [0u8; 8];
    copy_bytes(&mut word)?;
    Ok(u64::from_le_bytes(word))
}

/// The first value a draw is accepted at for a range of `bound` values. Draws at or above it are rejected, so the
/// accepted values split into whole groups of `bound` and every residue is equally likely.
pub fn accept_limit(bound: u64) -> u64 {
    u64::MAX - u64::MAX % bound
}

/// A uniform whole number in `[0, bound)`. A bound of 0 gives 0.
pub fn below(bound: u64) -> Result<u64, Error> {
    if bound == 0 {
        return Ok(0);
    }
    let limit = accept_limit(bound);
    loop {
        let x = random_u64()?;
        if x < limit {
            return Ok(x % bound);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_buffer_succeeds_and_stays_empty() {
        let mut empty: [u8; 0] = [];
        assert_eq!(copy_bytes(&mut empty), Ok(()));
        assert_eq!(bytes(0).unwrap().len(), 0);
    }

    #[test]
    fn requested_length_is_filled_and_two_draws_differ() {
        // 32 random bytes are all zero with probability 2^-256, so two draws that match would be a broken pool.
        let a = bytes(32).unwrap();
        let b = bytes(32).unwrap();
        assert_eq!(a.len(), 32);
        assert_eq!(b.len(), 32);
        assert_ne!(a, b);
        assert_ne!(a, vec![0u8; 32]);
    }

    #[test]
    fn accept_limit_is_the_largest_multiple_of_the_bound() {
        // u64::MAX is 18446744073709551615. Ten values: 18446744073709551615 % 10 = 5, so the limit is ...610.
        assert_eq!(accept_limit(10), 18_446_744_073_709_551_610);
        // Four values: u64::MAX % 4 = 3, so the limit is ...612.
        assert_eq!(accept_limit(4), 18_446_744_073_709_551_612);
        // Two values: u64::MAX % 2 = 1, so the limit is ...614.
        assert_eq!(accept_limit(2), 18_446_744_073_709_551_614);
    }

    #[test]
    fn below_zero_is_zero_and_below_one_is_always_zero() {
        assert_eq!(below(0), Ok(0));
        for _ in 0..16 {
            assert_eq!(below(1), Ok(0));
        }
    }

    #[test]
    fn below_stays_inside_the_range() {
        for _ in 0..1000 {
            let v = below(10).unwrap();
            assert!(v < 10, "draw {v} is outside [0, 10)");
        }
    }

    #[test]
    fn random_u64_draws_a_full_word() {
        // Two independent 64-bit words are equal with probability 2^-64.
        assert_ne!(random_u64().unwrap(), random_u64().unwrap());
    }
}
