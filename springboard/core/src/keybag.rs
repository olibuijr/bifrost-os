//! The keybag: the device-lock key store behind data protection classes (`keychain_protection`).
//!
//! Three class keys exist, one per way an item can become readable: while the session is unlocked, after the first
//! unlock since boot, and always. The bag is stored with
//!
//! * the *always* key in the clear (that class is readable at any time by definition),
//! * the *after first unlock* and *when unlocked* keys each wrapped under a key-encryption key (KEK) derived from the
//!   user's password with PBKDF2-HMAC-SHA256 (RFC 8018) over a random salt. A wrapped key is the class key XORed with an
//!   HKDF-SHA256 pad that is unique to the KEK and the class, plus an HMAC-SHA256 tag over the label and the wrapped
//!   bytes; the tag is how a wrong password is told from a right one.
//!
//! A bag read from disk is locked: only the *always* key is available. `unlock` loads the other two. `lock` forgets the
//! *when unlocked* key (zeroing it) and keeps *after first unlock* until the machine stops, exactly the lifetimes the
//! class names promise. Random bytes come from the caller (`create` takes the entropy), so the module does no I/O and
//! its results are repeatable in tests.
//!
//! Wire form (`BKB1`): magic, iterations (u32 BE), salt (16), then wrapped key and tag for *when unlocked*, wrapped key
//! and tag for *after first unlock*, and the *always* key; 4 + 4 + 16 + 64 + 64 + 32 = 184 bytes.

use crate::hkdf::{hkdf_expand, hmac_sha256};

/// PBKDF2 rounds for a new bag.
pub const DEFAULT_ITERATIONS: u32 = 100_000;
/// Random bytes `create` needs: salt (16) and the three class keys (96).
pub const ENTROPY_LEN: usize = 112;
const MAGIC: &[u8; 4] = b"BKB1";
const BAG_LEN: usize = 184;
const MAX_ITERATIONS: u32 = 10_000_000;

/// PBKDF2 with HMAC-SHA256 producing one 32-byte block (RFC 8018 section 5.2).
pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut msg = salt.to_vec();
    msg.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(password, &msg);
    let mut t = u;
    for _ in 1..iterations.max(1) {
        u = hmac_sha256(password, &u);
        for (a, b) in t.iter_mut().zip(u.iter()) {
            *a ^= b;
        }
    }
    t
}

/// The class of a key: when it can be had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyClass {
    /// Only while the session is unlocked.
    WhenUnlocked,
    /// From the first unlock since boot, also while locked.
    AfterFirstUnlock,
    /// At any time.
    Always,
}

/// Why a keybag call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BagError {
    /// The password does not open the bag.
    BadPassword,
    /// The key of this class is not available in the current state.
    Locked(KeyClass),
    /// The bytes are not a keybag.
    Corrupt,
    /// The iteration count is zero or beyond the accepted limit.
    BadIterations,
}

impl std::fmt::Display for BagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BagError::BadPassword => write!(f, "keybag: wrong password"),
            BagError::Locked(c) => write!(f, "keybag: the {c:?} key is not available while locked"),
            BagError::Corrupt => write!(f, "keybag: not a keybag"),
            BagError::BadIterations => write!(f, "keybag: unacceptable iteration count"),
        }
    }
}

impl std::error::Error for BagError {}

type Wrapped = ([u8; 32], [u8; 32]);

const LABEL_WHEN_UNLOCKED: &[u8] = b"bifrost keybag wrap when-unlocked";
const LABEL_AFTER_FIRST_UNLOCK: &[u8] = b"bifrost keybag wrap after-first-unlock";

fn pad(kek: &[u8; 32], label: &[u8]) -> [u8; 32] {
    let v = hkdf_expand(kek, label, 32).expect("32 bytes is within the HKDF limit");
    v.try_into().expect("32 bytes")
}

fn tag(kek: &[u8; 32], label: &[u8], wrapped: &[u8; 32]) -> [u8; 32] {
    let mut msg = label.to_vec();
    msg.extend_from_slice(wrapped);
    hmac_sha256(kek, &msg)
}

fn wrap(kek: &[u8; 32], label: &[u8], key: &[u8; 32]) -> Wrapped {
    let p = pad(kek, label);
    let mut w = [0u8; 32];
    for i in 0..32 {
        w[i] = key[i] ^ p[i];
    }
    (w, tag(kek, label, &w))
}

fn unwrap(kek: &[u8; 32], label: &[u8], wrapped: &Wrapped) -> Option<[u8; 32]> {
    let expect = tag(kek, label, &wrapped.0);
    let diff = expect.iter().zip(wrapped.1.iter()).fold(0u8, |a, (x, y)| a | (x ^ y));
    if diff != 0 {
        return None;
    }
    let p = pad(kek, label);
    let mut k = [0u8; 32];
    for i in 0..32 {
        k[i] = wrapped.0[i] ^ p[i];
    }
    Some(k)
}

fn zero(slot: &mut Option<[u8; 32]>) {
    if let Some(k) = slot.as_mut() {
        k.fill(0);
    }
    *slot = None;
}

/// A keybag with its lock state.
pub struct Keybag {
    iterations: u32,
    salt: [u8; 16],
    wrapped_when_unlocked: Wrapped,
    wrapped_after_first_unlock: Wrapped,
    always: [u8; 32],
    when_unlocked: Option<[u8; 32]>,
    after_first_unlock: Option<[u8; 32]>,
}

impl std::fmt::Debug for Keybag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keybag").field("unlocked", &self.unlocked()).field("unlocked_since_boot", &self.unlocked_since_boot()).finish()
    }
}

impl Keybag {
    /// Makes a bag protected by `password`. The bag starts unlocked, as at the end of setting a password. `entropy` is
    /// [`ENTROPY_LEN`] random bytes from the system generator; reusing it reuses the keys.
    pub fn create(password: &[u8], entropy: &[u8; ENTROPY_LEN], iterations: u32) -> Result<Keybag, BagError> {
        if iterations == 0 || iterations > MAX_ITERATIONS {
            return Err(BagError::BadIterations);
        }
        let salt: [u8; 16] = entropy[..16].try_into().unwrap();
        let wu: [u8; 32] = entropy[16..48].try_into().unwrap();
        let afu: [u8; 32] = entropy[48..80].try_into().unwrap();
        let always: [u8; 32] = entropy[80..112].try_into().unwrap();
        let kek = pbkdf2_hmac_sha256(password, &salt, iterations);
        Ok(Keybag {
            iterations,
            salt,
            wrapped_when_unlocked: wrap(&kek, LABEL_WHEN_UNLOCKED, &wu),
            wrapped_after_first_unlock: wrap(&kek, LABEL_AFTER_FIRST_UNLOCK, &afu),
            always,
            when_unlocked: Some(wu),
            after_first_unlock: Some(afu),
        })
    }

    /// True while the *when unlocked* key is loaded.
    pub fn unlocked(&self) -> bool {
        self.when_unlocked.is_some()
    }

    /// True once the *after first unlock* key has been loaded since the bag was read.
    pub fn unlocked_since_boot(&self) -> bool {
        self.after_first_unlock.is_some()
    }

    /// Opens the bag with `password`, loading both wrapped keys. A wrong password changes nothing.
    pub fn unlock(&mut self, password: &[u8]) -> Result<(), BagError> {
        let kek = pbkdf2_hmac_sha256(password, &self.salt, self.iterations);
        let wu = unwrap(&kek, LABEL_WHEN_UNLOCKED, &self.wrapped_when_unlocked).ok_or(BagError::BadPassword)?;
        let afu = unwrap(&kek, LABEL_AFTER_FIRST_UNLOCK, &self.wrapped_after_first_unlock).ok_or(BagError::BadPassword)?;
        self.when_unlocked = Some(wu);
        self.after_first_unlock = Some(afu);
        Ok(())
    }

    /// Locks the session: forgets the *when unlocked* key. The *after first unlock* key stays until shutdown.
    pub fn lock(&mut self) {
        zero(&mut self.when_unlocked);
    }

    /// The key of `class`, or `Locked` when the state does not allow it.
    pub fn class_key(&self, class: KeyClass) -> Result<[u8; 32], BagError> {
        match class {
            KeyClass::Always => Ok(self.always),
            KeyClass::AfterFirstUnlock => self.after_first_unlock.ok_or(BagError::Locked(class)),
            KeyClass::WhenUnlocked => self.when_unlocked.ok_or(BagError::Locked(class)),
        }
    }

    /// Changes the password: opens the bag with `old`, then re-wraps both keys under a KEK from `new` and
    /// `new_salt`. The class keys themselves do not change, so data sealed under them stays readable. A wrong `old`
    /// changes nothing.
    pub fn change_password(&mut self, old: &[u8], new: &[u8], new_salt: [u8; 16]) -> Result<(), BagError> {
        let was = (self.when_unlocked, self.after_first_unlock);
        self.unlock(old)?;
        let (wu, afu) = (self.when_unlocked.unwrap(), self.after_first_unlock.unwrap());
        let kek = pbkdf2_hmac_sha256(new, &new_salt, self.iterations);
        self.salt = new_salt;
        self.wrapped_when_unlocked = wrap(&kek, LABEL_WHEN_UNLOCKED, &wu);
        self.wrapped_after_first_unlock = wrap(&kek, LABEL_AFTER_FIRST_UNLOCK, &afu);
        // the session state stays what it was before the call
        if was.0.is_none() {
            zero(&mut self.when_unlocked);
        }
        if was.1.is_none() {
            zero(&mut self.after_first_unlock);
        }
        Ok(())
    }

    /// The stored form. It holds no password and no unwrapped key except the *always* key.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BAG_LEN);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.iterations.to_be_bytes());
        out.extend_from_slice(&self.salt);
        for w in [&self.wrapped_when_unlocked, &self.wrapped_after_first_unlock] {
            out.extend_from_slice(&w.0);
            out.extend_from_slice(&w.1);
        }
        out.extend_from_slice(&self.always);
        out
    }

    /// Reads a stored bag. It comes back locked.
    pub fn from_bytes(bytes: &[u8]) -> Result<Keybag, BagError> {
        if bytes.len() != BAG_LEN || &bytes[..4] != MAGIC {
            return Err(BagError::Corrupt);
        }
        let iterations = u32::from_be_bytes(bytes[4..8].try_into().unwrap());
        if iterations == 0 || iterations > MAX_ITERATIONS {
            return Err(BagError::BadIterations);
        }
        let arr = |from: usize| -> [u8; 32] { bytes[from..from + 32].try_into().unwrap() };
        Ok(Keybag {
            iterations,
            salt: bytes[8..24].try_into().unwrap(),
            wrapped_when_unlocked: (arr(24), arr(56)),
            wrapped_after_first_unlock: (arr(88), arr(120)),
            always: arr(152),
            when_unlocked: None,
            after_first_unlock: None,
        })
    }
}

impl Drop for Keybag {
    fn drop(&mut self) {
        zero(&mut self.when_unlocked);
        zero(&mut self.after_first_unlock);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entropy() -> [u8; ENTROPY_LEN] {
        let mut e = [0u8; ENTROPY_LEN];
        for (i, b) in e.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        e
    }

    fn hex(b: &[u8]) -> String {
        crate::sha256::hex(b)
    }

    #[test]
    fn pbkdf2_matches_the_published_vectors() {
        assert_eq!(hex(&pbkdf2_hmac_sha256(b"password", b"salt", 1)), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
        assert_eq!(hex(&pbkdf2_hmac_sha256(b"password", b"salt", 2)), "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43");
    }

    #[test]
    fn a_new_bag_is_unlocked_and_a_read_one_is_locked() {
        let bag = Keybag::create(b"pw", &entropy(), 2).unwrap();
        assert!(bag.unlocked() && bag.unlocked_since_boot());
        let read = Keybag::from_bytes(&bag.to_bytes()).unwrap();
        assert!(!read.unlocked() && !read.unlocked_since_boot());
        assert_eq!(read.class_key(KeyClass::WhenUnlocked), Err(BagError::Locked(KeyClass::WhenUnlocked)));
        assert_eq!(read.class_key(KeyClass::AfterFirstUnlock), Err(BagError::Locked(KeyClass::AfterFirstUnlock)));
        assert_eq!(read.class_key(KeyClass::Always).unwrap(), bag.class_key(KeyClass::Always).unwrap());
    }

    #[test]
    fn unlock_recovers_the_class_keys_the_bag_was_made_with() {
        let bag = Keybag::create(b"pw", &entropy(), 2).unwrap();
        let (wu, afu) = (bag.class_key(KeyClass::WhenUnlocked).unwrap(), bag.class_key(KeyClass::AfterFirstUnlock).unwrap());
        let mut read = Keybag::from_bytes(&bag.to_bytes()).unwrap();
        read.unlock(b"pw").unwrap();
        assert_eq!(read.class_key(KeyClass::WhenUnlocked).unwrap(), wu);
        assert_eq!(read.class_key(KeyClass::AfterFirstUnlock).unwrap(), afu);
        assert_ne!(wu, afu);
        assert_ne!(afu, read.class_key(KeyClass::Always).unwrap());
    }

    #[test]
    fn a_wrong_password_opens_nothing() {
        let bag = Keybag::create(b"pw", &entropy(), 2).unwrap();
        let mut read = Keybag::from_bytes(&bag.to_bytes()).unwrap();
        assert_eq!(read.unlock(b"other"), Err(BagError::BadPassword));
        assert!(!read.unlocked_since_boot());
    }

    #[test]
    fn locking_forgets_only_the_when_unlocked_key() {
        let mut bag = Keybag::create(b"pw", &entropy(), 2).unwrap();
        bag.lock();
        assert!(!bag.unlocked() && bag.unlocked_since_boot());
        assert!(bag.class_key(KeyClass::WhenUnlocked).is_err());
        assert!(bag.class_key(KeyClass::AfterFirstUnlock).is_ok());
        bag.unlock(b"pw").unwrap();
        assert!(bag.unlocked());
    }

    #[test]
    fn changing_the_password_keeps_the_keys_and_needs_the_old_one() {
        let mut bag = Keybag::create(b"old", &entropy(), 2).unwrap();
        let wu = bag.class_key(KeyClass::WhenUnlocked).unwrap();
        assert_eq!(bag.change_password(b"nope", b"new", [9; 16]), Err(BagError::BadPassword));
        bag.change_password(b"old", b"new", [9; 16]).unwrap();
        let mut read = Keybag::from_bytes(&bag.to_bytes()).unwrap();
        assert_eq!(read.unlock(b"old"), Err(BagError::BadPassword));
        read.unlock(b"new").unwrap();
        assert_eq!(read.class_key(KeyClass::WhenUnlocked).unwrap(), wu);
    }

    #[test]
    fn a_locked_bag_stays_locked_after_a_password_change() {
        let mut bag = Keybag::create(b"old", &entropy(), 2).unwrap();
        bag.lock();
        bag.change_password(b"old", b"new", [1; 16]).unwrap();
        assert!(!bag.unlocked() && bag.unlocked_since_boot());
    }

    #[test]
    fn a_tampered_wrapped_key_fails_its_tag() {
        let bag = Keybag::create(b"pw", &entropy(), 2).unwrap();
        let mut bytes = bag.to_bytes();
        bytes[30] ^= 1;
        let mut read = Keybag::from_bytes(&bytes).unwrap();
        assert_eq!(read.unlock(b"pw"), Err(BagError::BadPassword));
    }

    #[test]
    fn foreign_or_short_bytes_are_refused() {
        assert_eq!(Keybag::from_bytes(b"BKB1").unwrap_err(), BagError::Corrupt);
        let bag = Keybag::create(b"pw", &entropy(), 2).unwrap();
        let mut bytes = bag.to_bytes();
        bytes[0] = b'X';
        assert_eq!(Keybag::from_bytes(&bytes).unwrap_err(), BagError::Corrupt);
        let mut zero_rounds = bag.to_bytes();
        zero_rounds[4..8].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(Keybag::from_bytes(&zero_rounds).unwrap_err(), BagError::BadIterations);
        assert_eq!(Keybag::create(b"pw", &entropy(), 0).unwrap_err(), BagError::BadIterations);
    }

    #[test]
    fn the_debug_form_shows_no_key() {
        let bag = Keybag::create(b"pw", &entropy(), 2).unwrap();
        let text = format!("{bag:?}");
        assert!(text.contains("unlocked"));
        assert!(!text.contains(&hex(&bag.class_key(KeyClass::Always).unwrap())));
    }
}
