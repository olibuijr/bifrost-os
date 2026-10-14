//! Encrypted models: the text form of a model sealed under a password or a raw key, so a model shipped in an app bundle
//! or downloaded stays private. The bytes are a `sealed_box` container. Opening checks the authentication tag before
//! any text is read, then parses the text with [`Model::parse`]. The caller supplies the random salt and nonce.

use crate::ml_model::{Model, ModelError};
use crate::sealed_box::{self, Error as SealError, NONCE_LEN, SALT_LEN};

/// Why an encrypted model could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The container could not be opened: a wrong password or key, changed bytes, or not a sealed container.
    Seal(SealError),
    /// The opened bytes are not UTF-8 text.
    NotText,
    /// The opened text is not a valid model.
    Model(ModelError),
}

/// Seals a model's text form under `password`. `iterations` is the PBKDF2 work factor; new models use
/// `sealed_box::DEFAULT_ITERATIONS`.
pub fn seal_with_password(model_text: &str, password: &str, salt: &[u8; SALT_LEN], nonce: &[u8; NONCE_LEN], iterations: u32) -> Vec<u8> {
    sealed_box::seal_with_password(model_text.as_bytes(), password, salt, nonce, iterations)
}

/// Opens a model sealed by [`seal_with_password`].
pub fn open_with_password(sealed: &[u8], password: &str) -> Result<Model, OpenError> {
    let plain = sealed_box::open_with_password(sealed, password).map_err(OpenError::Seal)?;
    parse_plain(&plain)
}

/// Seals a model's text form under a raw 16- or 32-byte key, such as one kept in the keychain. `context` is
/// authenticated but not stored, so opening needs the same context.
pub fn seal_with_key(model_text: &str, key: &[u8], nonce: &[u8; NONCE_LEN], context: &[u8]) -> Result<Vec<u8>, SealError> {
    sealed_box::seal_with_key(model_text.as_bytes(), key, nonce, context)
}

/// Opens a model sealed by [`seal_with_key`] with the same key and context.
pub fn open_with_key(sealed: &[u8], key: &[u8], context: &[u8]) -> Result<Model, OpenError> {
    let plain = sealed_box::open_with_key(sealed, key, context).map_err(OpenError::Seal)?;
    parse_plain(&plain)
}

fn parse_plain(plain: &[u8]) -> Result<Model, OpenError> {
    let text = std::str::from_utf8(plain).map_err(|_| OpenError::NotText)?;
    Model::parse(text).map_err(OpenError::Model)
}
