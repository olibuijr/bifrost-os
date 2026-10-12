//! PDF access permissions and write options as a pure model. A document carries a permission word (the `P` entry of
//! its encryption dictionary, a signed 32-bit value). The word says which operations a reader may perform: print,
//! modify, copy text, annotate, fill forms, extract for accessibility, assemble and print at high quality.
//!
//! The bit layout follows the PDF 2.0 standard (ISO 32000-2, table on access permissions): bit 3 print, bit 4 modify,
//! bit 5 copy, bit 6 annotate, bit 9 fill forms, bit 10 extract for accessibility, bit 11 assemble, bit 12 print high
//! quality. Bits 1 and 2 are zero, bits 7 and 8 and bits 13 to 32 are set. This module encodes and decodes that word
//! and checks write options. It does not encrypt, decrypt or write a file.
//!
//! Rules: a write that asks for any restriction, or that sets a password, needs encryption, which this model cannot
//! do, so `check_write` refuses it. An unrestricted write (all operations allowed, no passwords) is accepted.
//! A document without an encryption dictionary reads as fully permitted.

/// The reserved bits that must be set in a permission word: bits 7 and 8 and bits 13 to 32.
const RESERVED_SET: u32 = 0xFFFF_F0C0;

const PRINT: u32 = 1 << 2;
const MODIFY: u32 = 1 << 3;
const COPY: u32 = 1 << 4;
const ANNOTATE: u32 = 1 << 5;
const FILL_FORMS: u32 = 1 << 8;
const ACCESSIBILITY: u32 = 1 << 9;
const ASSEMBLE: u32 = 1 << 10;
const PRINT_HIGH: u32 = 1 << 11;

/// The operations a reader may perform on a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessPermissions {
    pub print: bool,
    pub modify: bool,
    pub copy: bool,
    pub annotate: bool,
    pub fill_forms: bool,
    pub accessibility: bool,
    pub assemble: bool,
    pub print_high_quality: bool,
}

impl AccessPermissions {
    /// Every operation allowed.
    pub const ALL: AccessPermissions = AccessPermissions {
        print: true,
        modify: true,
        copy: true,
        annotate: true,
        fill_forms: true,
        accessibility: true,
        assemble: true,
        print_high_quality: true,
    };

    /// No operation allowed.
    pub const NONE: AccessPermissions = AccessPermissions {
        print: false,
        modify: false,
        copy: false,
        annotate: false,
        fill_forms: false,
        accessibility: false,
        assemble: false,
        print_high_quality: false,
    };

    /// The permission word for this set of operations.
    pub fn to_word(self) -> i32 {
        let mut w = RESERVED_SET;
        let bits = [
            (self.print, PRINT),
            (self.modify, MODIFY),
            (self.copy, COPY),
            (self.annotate, ANNOTATE),
            (self.fill_forms, FILL_FORMS),
            (self.accessibility, ACCESSIBILITY),
            (self.assemble, ASSEMBLE),
            (self.print_high_quality, PRINT_HIGH),
        ];
        for (on, bit) in bits {
            if on {
                w |= bit;
            }
        }
        w as i32
    }

    /// Reads the operations from a permission word. Reserved bits are ignored.
    pub fn from_word(word: i32) -> AccessPermissions {
        let w = word as u32;
        AccessPermissions {
            print: w & PRINT != 0,
            modify: w & MODIFY != 0,
            copy: w & COPY != 0,
            annotate: w & ANNOTATE != 0,
            fill_forms: w & FILL_FORMS != 0,
            accessibility: w & ACCESSIBILITY != 0,
            assemble: w & ASSEMBLE != 0,
            print_high_quality: w & PRINT_HIGH != 0,
        }
    }

    /// True when every operation is allowed.
    pub fn is_unrestricted(self) -> bool {
        self == AccessPermissions::ALL
    }
}

/// The options given when a document is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteOptions {
    pub permissions: AccessPermissions,
    pub owner_password: Option<String>,
    pub user_password: Option<String>,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions { permissions: AccessPermissions::ALL, owner_password: None, user_password: None }
    }
}

/// Why a write was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// The options need encryption (a restriction or a password), which this model does not perform.
    NeedsEncryption,
    /// A password was set but is empty.
    EmptyPassword,
}

/// Checks write options against what the writer can do.
pub fn check_write(options: &WriteOptions) -> Result<(), WriteError> {
    for pw in [&options.owner_password, &options.user_password].into_iter().flatten() {
        if pw.is_empty() {
            return Err(WriteError::EmptyPassword);
        }
    }
    let has_password = options.owner_password.is_some() || options.user_password.is_some();
    if has_password || !options.permissions.is_unrestricted() {
        return Err(WriteError::NeedsEncryption);
    }
    Ok(())
}

/// The permissions a reader sees for a document. A document with no encryption dictionary has none, so it is
/// fully permitted.
pub fn effective_permissions(encryption_word: Option<i32>) -> AccessPermissions {
    match encryption_word {
        Some(w) => AccessPermissions::from_word(w),
        None => AccessPermissions::ALL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_allowed_is_minus_four_and_none_is_minus_3904() {
        // Operation bits 2 to 5 and 8 to 11 (0-based), plus the reserved bits, give -4 for all and -3904 for none.
        assert_eq!(AccessPermissions::ALL.to_word(), -4);
        assert_eq!(AccessPermissions::NONE.to_word(), -3904);
    }

    #[test]
    fn print_and_copy_map_to_their_bits() {
        let p = AccessPermissions { print: true, ..AccessPermissions::NONE };
        assert_eq!(p.to_word(), -3904 | 0x4);
        let c = AccessPermissions { copy: true, ..AccessPermissions::NONE };
        assert_eq!(c.to_word(), -3904 | 0x10);
    }

    #[test]
    fn decoding_inverts_encoding_for_every_single_operation() {
        let single = |f: fn(&mut AccessPermissions)| {
            let mut a = AccessPermissions::NONE;
            f(&mut a);
            a
        };
        let cases = [
            single(|a| a.print = true),
            single(|a| a.modify = true),
            single(|a| a.copy = true),
            single(|a| a.annotate = true),
            single(|a| a.fill_forms = true),
            single(|a| a.accessibility = true),
            single(|a| a.assemble = true),
            single(|a| a.print_high_quality = true),
        ];
        for a in cases {
            assert_eq!(AccessPermissions::from_word(a.to_word()), a);
        }
    }

    #[test]
    fn reserved_bits_do_not_grant_anything() {
        // Bit 1 and bit 7 set on top of an empty word: still no operation is allowed.
        assert_eq!(AccessPermissions::from_word(-3904), AccessPermissions::NONE);
        assert_eq!(AccessPermissions::from_word(-3904 | 0x2), AccessPermissions::NONE);
    }

    #[test]
    fn unrestricted_plain_write_is_accepted() {
        assert_eq!(check_write(&WriteOptions::default()), Ok(()));
    }

    #[test]
    fn restrictions_and_passwords_need_encryption() {
        let restricted = WriteOptions { permissions: AccessPermissions::NONE, ..WriteOptions::default() };
        assert_eq!(check_write(&restricted), Err(WriteError::NeedsEncryption));
        let locked = WriteOptions { owner_password: Some("owner".into()), ..WriteOptions::default() };
        assert_eq!(check_write(&locked), Err(WriteError::NeedsEncryption));
        let empty = WriteOptions { user_password: Some(String::new()), ..WriteOptions::default() };
        assert_eq!(check_write(&empty), Err(WriteError::EmptyPassword));
    }

    #[test]
    fn document_without_encryption_is_fully_permitted() {
        assert_eq!(effective_permissions(None), AccessPermissions::ALL);
        assert_eq!(effective_permissions(Some(-3904)), AccessPermissions::NONE);
    }
}
