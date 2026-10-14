//! Result codes of the security services (the `OSStatus` values the keychain and the trust and key services return),
//! with a short meaning and a broad category for each code this platform reports.
//!
//! A code of zero is success. Any other code that is not in [`KNOWN_CODES`] has no meaning here and falls in
//! [`Category::Unknown`]. The keychain's own errors (`keychain::Error`) report their codes from this table.

/// A broad class of result code, for deciding how an app reacts to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// The operation completed.
    Success,
    /// The request had a bad argument, a bad reference or a bad size.
    Parameter,
    /// The system refused access: no permission, a failed authentication or a refused user interaction.
    Access,
    /// The item or keychain already exists.
    Duplicate,
    /// The item, attribute, class or keychain is not there.
    Missing,
    /// The store could not be read, written or changed.
    Storage,
    /// Stored data could not be decoded.
    Format,
    /// The service failed inside itself.
    Internal,
    /// The code is not in the table.
    Unknown,
}

/// Each code this platform reports, its meaning and its category.
pub const KNOWN_CODES: &[(i32, &str, Category)] = &[
    (0, "success", Category::Success),
    (-4, "function not implemented", Category::Internal),
    (-36, "I/O error", Category::Storage),
    (-49, "file is open for writing elsewhere", Category::Storage),
    (-50, "invalid parameter", Category::Parameter),
    (-108, "memory allocation failed", Category::Internal),
    (-128, "the user canceled the operation", Category::Access),
    (-909, "bad request", Category::Parameter),
    (-2070, "internal component failure", Category::Internal),
    (-25291, "the service is not available", Category::Access),
    (-25292, "the keychain is read-only", Category::Storage),
    (-25293, "authentication failed", Category::Access),
    (-25294, "no such keychain", Category::Missing),
    (-25295, "the keychain is invalid", Category::Storage),
    (-25296, "the keychain already exists", Category::Duplicate),
    (-25299, "the item already exists", Category::Duplicate),
    (-25300, "the item was not found", Category::Missing),
    (-25301, "the buffer is too small", Category::Parameter),
    (-25302, "the data is too large", Category::Parameter),
    (-25303, "the attribute was not found", Category::Missing),
    (-25304, "the item reference is invalid", Category::Parameter),
    (-25306, "the class was not found", Category::Missing),
    (-25307, "there is no default keychain", Category::Missing),
    (-25308, "user interaction is not allowed", Category::Access),
    (-25309, "the attribute is read-only", Category::Storage),
    (-25315, "user interaction is required", Category::Access),
    (-25316, "the data is not available", Category::Missing),
    (-25317, "the data cannot be modified", Category::Storage),
    (-26275, "the data could not be decoded", Category::Format),
    (-34018, "the app lacks the required entitlement", Category::Access),
];

fn lookup(code: i32) -> Option<&'static (i32, &'static str, Category)> {
    KNOWN_CODES.iter().find(|(known, _, _)| *known == code)
}

/// Whether a code reports success.
pub fn is_success(code: i32) -> bool {
    code == 0
}

/// The meaning of a known code, or `None` for a code this platform does not report.
pub fn meaning(code: i32) -> Option<&'static str> {
    lookup(code).map(|(_, text, _)| *text)
}

/// The category of a code. Codes outside the table are [`Category::Unknown`].
pub fn category(code: i32) -> Category {
    lookup(code).map_or(Category::Unknown, |(_, _, category)| *category)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_in_the_table_is_listed_once() {
        for (i, (code, _, _)) in KNOWN_CODES.iter().enumerate() {
            assert!(KNOWN_CODES[i + 1..].iter().all(|(other, _, _)| other != code), "{code} listed twice");
        }
    }

    #[test]
    fn zero_is_success_and_nothing_else_is() {
        assert!(is_success(0));
        assert!(!is_success(-25300));
        assert_eq!(category(0), Category::Success);
    }

    #[test]
    fn a_code_outside_the_table_has_no_meaning() {
        assert_eq!(meaning(-1), None);
        assert_eq!(category(-1), Category::Unknown);
    }
}
