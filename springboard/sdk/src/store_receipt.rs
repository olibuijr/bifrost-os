//! Purchase receipts for apps sold through the PC App Store. The store signs each purchase record; an app checks the
//! receipt before it unlocks a product. There are no reference-vendor payments here: the signer is the BifrOSt store, and the
//! signature check is supplied by the caller (a verifier over the canonical payload and the signature bytes), so the
//! key stays with whoever holds the store's trust root. This module fixes the shape: the canonical payload, the
//! signature format (a 64-byte Ed25519 signature as 128 lowercase hex digits) and the order of the checks.

use crate::digest::sha256;

/// The length of an Ed25519 signature in bytes, and of its lowercase hex form.
pub const SIGNATURE_BYTES: usize = 64;

/// One store receipt: the purchase the store signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub product: String,
    pub transaction_id: u64,
    /// The day of the purchase, counted from 1970-01-01.
    pub purchased_day: i64,
    /// The store's signature over [`Receipt::payload`], as lowercase hex.
    pub signature: String,
}

/// Why a receipt is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiptError {
    /// The product identifier is empty.
    EmptyProduct,
    /// The signature is not 128 lowercase hex digits.
    MalformedSignature,
    /// The verifier did not accept the signature over the payload.
    Unverified,
}

impl Receipt {
    /// The canonical bytes the store signs: the product, transaction id and purchase day, one per line. The product
    /// identifier must not contain a newline (`shape` refuses one).
    pub fn payload(&self) -> Vec<u8> {
        format!("{}\n{}\n{}\n", self.product, self.transaction_id, self.purchased_day).into_bytes()
    }

    /// The SHA-256 of the payload, for logs and for the store's audit trail. Not a signature.
    pub fn payload_digest(&self) -> [u8; 32] {
        sha256(&self.payload())
    }

    /// The structural checks, without a signature check: a product, no newline in it, and a signature of the right
    /// shape. Run this before asking the verifier.
    pub fn shape(&self) -> Result<(), ReceiptError> {
        if self.product.is_empty() || self.product.contains('\n') {
            return Err(ReceiptError::EmptyProduct);
        }
        let sig = &self.signature;
        let well_formed = sig.len() == SIGNATURE_BYTES * 2
            && sig.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !well_formed {
            return Err(ReceiptError::MalformedSignature);
        }
        Ok(())
    }

    /// Checks the shape, then asks `verify(payload, signature_bytes)` whether the store signed it.
    pub fn verify<F>(&self, verify: F) -> Result<(), ReceiptError>
    where
        F: Fn(&[u8], &[u8]) -> bool,
    {
        self.shape()?;
        let bytes = decode_hex(&self.signature).ok_or(ReceiptError::MalformedSignature)?;
        if verify(&self.payload(), &bytes) {
            Ok(())
        } else {
            Err(ReceiptError::Unverified)
        }
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(signature: &str) -> Receipt {
        Receipt { product: "is.example.pro".into(), transaction_id: 7, purchased_day: 20_000, signature: signature.into() }
    }

    fn good() -> String {
        "ab".repeat(SIGNATURE_BYTES)
    }

    #[test]
    fn the_payload_is_the_canonical_three_lines() {
        assert_eq!(receipt(&good()).payload(), b"is.example.pro\n7\n20000\n".to_vec());
    }

    #[test]
    fn a_product_with_a_newline_or_an_empty_product_is_refused() {
        let mut r = receipt(&good());
        r.product = String::new();
        assert_eq!(r.shape(), Err(ReceiptError::EmptyProduct));
        r.product = "a\nb".into();
        assert_eq!(r.shape(), Err(ReceiptError::EmptyProduct));
    }

    #[test]
    fn a_signature_must_be_128_lowercase_hex_digits() {
        assert_eq!(receipt("abcd").shape(), Err(ReceiptError::MalformedSignature));
        assert_eq!(receipt(&"AB".repeat(SIGNATURE_BYTES)).shape(), Err(ReceiptError::MalformedSignature));
        assert_eq!(receipt(&"zz".repeat(SIGNATURE_BYTES)).shape(), Err(ReceiptError::MalformedSignature));
        assert!(receipt(&good()).shape().is_ok());
    }

    #[test]
    fn the_verifier_sees_the_payload_and_the_decoded_signature() {
        let r = receipt(&good());
        let seen = std::cell::Cell::new(0usize);
        let result = r.verify(|payload, sig| {
            seen.set(sig.len());
            payload == r.payload().as_slice()
        });
        assert_eq!(result, Ok(()));
        assert_eq!(seen.get(), SIGNATURE_BYTES, "the signature is passed as bytes, not hex");
    }

    #[test]
    fn a_receipt_the_verifier_rejects_is_unverified() {
        assert_eq!(receipt(&good()).verify(|_, _| false), Err(ReceiptError::Unverified));
    }

    #[test]
    fn the_shape_is_checked_before_the_verifier_runs() {
        let called = std::cell::Cell::new(false);
        let result = receipt("nothex").verify(|_, _| {
            called.set(true);
            true
        });
        assert_eq!(result, Err(ReceiptError::MalformedSignature));
        assert!(!called.get());
    }

    #[test]
    fn the_payload_digest_is_stable_for_the_same_receipt() {
        assert_eq!(receipt(&good()).payload_digest(), receipt(&good()).payload_digest());
    }
}
