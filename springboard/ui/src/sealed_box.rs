//! Sealed bytes: a password-sealed container (PBKDF2-HMAC-SHA256 into AES-256-GCM) and a raw-key seal. Used for
//! encrypted archives, encrypted machine learning models and any file an app wants to keep private. The caller
//! supplies the random salt and nonce. Container layout: `BSB1`, iterations (u32), salt (16), nonce (12), then the
//! ciphertext and tag. The magic and header are authenticated as additional data.

use bifrost_core::{aes_gcm, hkdf};

pub const MAGIC: &[u8; 4] = b"BSB1";
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
const HEADER: usize = 4 + 4 + SALT_LEN + NONCE_LEN;
/// Iterations for new containers.
pub const DEFAULT_ITERATIONS: u32 = 150_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes are not a sealed container.
    NotSealed,
    /// The header is cut short or holds an impossible value.
    Malformed,
    /// Wrong password or key, or the bytes were changed.
    Authentication,
}

/// PBKDF2-HMAC-SHA256, one 32-byte block.
pub fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut first = salt.to_vec();
    first.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hkdf::hmac_sha256(password, &first);
    let mut t = u;
    for _ in 1..iterations.max(1) {
        u = hkdf::hmac_sha256(password, &u);
        for (a, b) in t.iter_mut().zip(u.iter()) {
            *a ^= b;
        }
    }
    t
}

fn header(iterations: u32, salt: &[u8; SALT_LEN], nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
    let mut h = MAGIC.to_vec();
    h.extend_from_slice(&iterations.to_be_bytes());
    h.extend_from_slice(salt);
    h.extend_from_slice(nonce);
    h
}

/// Seals `data` under a password.
pub fn seal_with_password(data: &[u8], password: &str, salt: &[u8; SALT_LEN], nonce: &[u8; NONCE_LEN], iterations: u32) -> Vec<u8> {
    let key = pbkdf2(password.as_bytes(), salt, iterations);
    let head = header(iterations, salt, nonce);
    let sealed = aes_gcm::seal(&key, nonce, &head, data).unwrap_or_default();
    let mut out = head;
    out.extend_from_slice(&sealed);
    out
}

/// True when `bytes` begin like a sealed container.
pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER + 16 && &bytes[..4] == MAGIC
}

/// Opens a container made by `seal_with_password`.
pub fn open_with_password(sealed: &[u8], password: &str) -> Result<Vec<u8>, Error> {
    if sealed.len() < 4 || &sealed[..4] != MAGIC {
        return Err(Error::NotSealed);
    }
    if sealed.len() < HEADER + 16 {
        return Err(Error::Malformed);
    }
    let iterations = u32::from_be_bytes([sealed[4], sealed[5], sealed[6], sealed[7]]);
    if iterations == 0 || iterations > 10_000_000 {
        return Err(Error::Malformed);
    }
    let salt = &sealed[8..8 + SALT_LEN];
    let nonce = &sealed[8 + SALT_LEN..HEADER];
    let key = pbkdf2(password.as_bytes(), salt, iterations);
    aes_gcm::open(&key, nonce, &sealed[..HEADER], &sealed[HEADER..]).map_err(|_| Error::Authentication)
}

/// Seals under a raw 16- or 32-byte key (for a key kept in the keychain). Output: nonce, then ciphertext and tag.
pub fn seal_with_key(data: &[u8], key: &[u8], nonce: &[u8; NONCE_LEN], context: &[u8]) -> Result<Vec<u8>, Error> {
    let sealed = aes_gcm::seal(key, nonce, context, data).map_err(|_| Error::Malformed)?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Ok(out)
}

pub fn open_with_key(sealed: &[u8], key: &[u8], context: &[u8]) -> Result<Vec<u8>, Error> {
    if sealed.len() < NONCE_LEN + 16 {
        return Err(Error::Malformed);
    }
    aes_gcm::open(key, &sealed[..NONCE_LEN], context, &sealed[NONCE_LEN..]).map_err(|_| Error::Authentication)
}

/// An archive of named entries, each sealed under one password, so a viewer can list the names without it
/// (`names_visible`) or hide them (the default). Entry layout: u32 count, then per entry u32 length and the sealed bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Archive {
    pub entries: Vec<(String, Vec<u8>)>,
}

impl Archive {
    pub fn add(&mut self, name: &str, data: &[u8]) {
        self.entries.retain(|(n, _)| n != name);
        self.entries.push((name.to_string(), data.to_vec()));
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice())
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = (self.entries.len() as u32).to_be_bytes().to_vec();
        for (name, data) in &self.entries {
            let nb = name.as_bytes();
            out.extend_from_slice(&(nb.len() as u32).to_be_bytes());
            out.extend_from_slice(nb);
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    fn decode(mut b: &[u8]) -> Option<Archive> {
        fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
            if b.len() < n {
                return None;
            }
            let (head, rest) = b.split_at(n);
            *b = rest;
            Some(head)
        }
        fn u32_of(b: &mut &[u8]) -> Option<usize> {
            let h = take(b, 4)?;
            Some(u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize)
        }
        let count = u32_of(&mut b)?;
        let mut entries = vec![];
        for _ in 0..count {
            let n = u32_of(&mut b)?;
            let name = String::from_utf8(take(&mut b, n)?.to_vec()).ok()?;
            let d = u32_of(&mut b)?;
            entries.push((name, take(&mut b, d)?.to_vec()));
        }
        Some(Archive { entries })
    }

    /// The whole archive sealed under a password (names included).
    pub fn seal(&self, password: &str, salt: &[u8; SALT_LEN], nonce: &[u8; NONCE_LEN], iterations: u32) -> Vec<u8> {
        seal_with_password(&self.encode(), password, salt, nonce, iterations)
    }

    pub fn open(sealed: &[u8], password: &str) -> Result<Archive, Error> {
        let plain = open_with_password(sealed, password)?;
        Archive::decode(&plain).ok_or(Error::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: [u8; SALT_LEN] = [3; SALT_LEN];
    const NONCE: [u8; NONCE_LEN] = [5; NONCE_LEN];

    #[test]
    fn password_round_trip_and_failures() {
        let sealed = seal_with_password(b"secret data", "pw", &SALT, &NONCE, 4);
        assert!(is_sealed(&sealed));
        assert!(!sealed.windows(6).any(|w| w == b"secret"));
        assert_eq!(open_with_password(&sealed, "pw").unwrap(), b"secret data");
        assert_eq!(open_with_password(&sealed, "no"), Err(Error::Authentication));
        assert_eq!(open_with_password(b"plain bytes", "pw"), Err(Error::NotSealed));
        assert_eq!(open_with_password(b"BSB1short", "pw"), Err(Error::Malformed));
        let mut bad = sealed.clone();
        bad[9] ^= 1; // the salt is authenticated and also changes the key
        assert_eq!(open_with_password(&bad, "pw"), Err(Error::Authentication));
        let mut zero = sealed.clone();
        zero[4..8].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(open_with_password(&zero, "pw"), Err(Error::Malformed));
    }

    #[test]
    fn pbkdf2_single_round_is_one_hmac() {
        let mut first = b"salt".to_vec();
        first.extend_from_slice(&[0, 0, 0, 1]);
        assert_eq!(pbkdf2(b"pw", b"salt", 1), hkdf::hmac_sha256(b"pw", &first));
        assert_ne!(pbkdf2(b"pw", b"salt", 2), pbkdf2(b"pw", b"salt", 1));
    }

    #[test]
    fn raw_key_seal_binds_the_context() {
        let key = [9u8; 32];
        let sealed = seal_with_key(b"model weights", &key, &NONCE, b"model:v1").unwrap();
        assert_eq!(open_with_key(&sealed, &key, b"model:v1").unwrap(), b"model weights");
        assert_eq!(open_with_key(&sealed, &key, b"model:v2"), Err(Error::Authentication));
        assert_eq!(open_with_key(&sealed, &[8u8; 32], b"model:v1"), Err(Error::Authentication));
        assert_eq!(open_with_key(&sealed[..10], &key, b""), Err(Error::Malformed));
        assert_eq!(seal_with_key(b"x", &[1u8; 5], &NONCE, b""), Err(Error::Malformed));
    }

    #[test]
    fn archive_round_trip() {
        let mut a = Archive::default();
        a.add("a.txt", b"one");
        a.add("dir/b.bin", &[0, 1, 2]);
        a.add("a.txt", b"uno"); // replaces
        let sealed = a.seal("pw", &SALT, &NONCE, 3);
        let back = Archive::open(&sealed, "pw").unwrap();
        assert_eq!(back.get("a.txt"), Some(&b"uno"[..]));
        assert_eq!(back.get("dir/b.bin"), Some(&[0u8, 1, 2][..]));
        assert_eq!(back.entries.len(), 2);
        assert!(Archive::open(&sealed, "bad").is_err());
        assert_eq!(Archive::decode(&[0, 0, 0, 9]), None);
    }
}
