//! GSS-API function status (`GSS_S_COMPLETE`, `GSS_S_FAILURE`, `GSS_C_MECH_CODE`, `gss_display_status`,
//! `GSSCreateError`): the two status codes a call returns, a major code and a minor code.
//!
//! The major code packs three bitfields: a calling error, a routine error and supplementary information. A call that
//! runs to completion leaves all three at zero, so `MajorStatus::COMPLETE` compares equal to the zero code. The field
//! widths are 8, 8 and 16 bits, the layout RFC 2744 section 1.2.1 defines; the official reference names the fields and the
//! masks but does not print the numeric values, so the numbers here follow that RFC. The minor code is mechanism
//! specific and is carried as an opaque `u32`.
//!
//! Not here: the mechanism that produces minor codes, and error objects that cross to the shared error type (see
//! `crate::error::AppError` for the form a caller that only needs a domain and code uses).

use std::fmt;

/// `GSS_S_COMPLETE`: no field is set.
pub const COMPLETE: u32 = 0;

/// `GSS_C_CALLING_ERROR_OFFSET`.
pub const CALLING_ERROR_OFFSET: u32 = 24;
/// `GSS_C_ROUTINE_ERROR_OFFSET`.
pub const ROUTINE_ERROR_OFFSET: u32 = 16;
/// `GSS_C_SUPPLEMENTARY_OFFSET`.
pub const SUPPLEMENTARY_OFFSET: u32 = 0;
/// `GSS_C_CALLING_ERROR_MASK`: an 8-bit field.
pub const CALLING_ERROR_MASK: u32 = 0xFF;
/// `GSS_C_ROUTINE_ERROR_MASK`: an 8-bit field.
pub const ROUTINE_ERROR_MASK: u32 = 0xFF;
/// `GSS_C_SUPPLEMENTARY_MASK`: a 16-bit field.
pub const SUPPLEMENTARY_MASK: u32 = 0xFFFF;

/// `GSS_C_GSS_CODE`: the status code comes from the GSS-API itself.
pub const GSS_CODE: u32 = 1;
/// `GSS_C_MECH_CODE`: the status code comes from the underlying mechanism, such as Kerberos.
pub const MECH_CODE: u32 = 2;

/// The calling errors: the caller passed a bad parameter. The names follow the "Calling Errors" section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallingError {
    /// `GSS_S_CALL_INACCESSIBLE_READ`: a required input parameter could not be read.
    InaccessibleRead,
    /// `GSS_S_CALL_INACCESSIBLE_WRITE`: a required output parameter failed to write.
    InaccessibleWrite,
    /// `GSS_S_CALL_BAD_STRUCTURE`: an improperly formatted parameter.
    BadStructure,
}

impl CallingError {
    /// The field value, already shifted into place by the offset in `MajorStatus::compose`.
    pub const fn field(self) -> u32 {
        match self {
            CallingError::InaccessibleRead => 1,
            CallingError::InaccessibleWrite => 2,
            CallingError::BadStructure => 3,
        }
    }

    /// The text `gss_display_status` gives, from the official abstract.
    pub const fn description(self) -> &'static str {
        match self {
            CallingError::InaccessibleRead => "A required input parameter could not be read.",
            CallingError::InaccessibleWrite => "A required output parameter failed to write.",
            CallingError::BadStructure => "Improperly formatted parameter.",
        }
    }

    /// The calling error for a field value, or `None` when the value names no calling error.
    pub const fn from_field(field: u32) -> Option<CallingError> {
        match field {
            1 => Some(CallingError::InaccessibleRead),
            2 => Some(CallingError::InaccessibleWrite),
            3 => Some(CallingError::BadStructure),
            _ => None,
        }
    }
}

/// The routine errors: the operation failed. The names follow the "Routine Errors" section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutineError {
    BadMech,
    BadName,
    BadNameType,
    BadBindings,
    BadStatus,
    BadMic,
    NoCred,
    NoContext,
    DefectiveToken,
    DefectiveCredential,
    CredentialsExpired,
    ContextExpired,
    Failure,
    BadQop,
    Unauthorized,
    Unavailable,
    DuplicateElement,
    NameNotMn,
    BadMechAttr,
    CredUnavail,
}

impl RoutineError {
    /// Every routine error, in field-value order.
    pub const ALL: [RoutineError; 20] = [
        RoutineError::BadMech,
        RoutineError::BadName,
        RoutineError::BadNameType,
        RoutineError::BadBindings,
        RoutineError::BadStatus,
        RoutineError::BadMic,
        RoutineError::NoCred,
        RoutineError::NoContext,
        RoutineError::DefectiveToken,
        RoutineError::DefectiveCredential,
        RoutineError::CredentialsExpired,
        RoutineError::ContextExpired,
        RoutineError::Failure,
        RoutineError::BadQop,
        RoutineError::Unauthorized,
        RoutineError::Unavailable,
        RoutineError::DuplicateElement,
        RoutineError::NameNotMn,
        RoutineError::BadMechAttr,
        RoutineError::CredUnavail,
    ];

    /// The field value, before the offset is applied. `BadMic` shares the value of `BadSig` in RFC 2744.
    pub const fn field(self) -> u32 {
        match self {
            RoutineError::BadMech => 1,
            RoutineError::BadName => 2,
            RoutineError::BadNameType => 3,
            RoutineError::BadBindings => 4,
            RoutineError::BadStatus => 5,
            RoutineError::BadMic => 6,
            RoutineError::NoCred => 7,
            RoutineError::NoContext => 8,
            RoutineError::DefectiveToken => 9,
            RoutineError::DefectiveCredential => 10,
            RoutineError::CredentialsExpired => 11,
            RoutineError::ContextExpired => 12,
            RoutineError::Failure => 13,
            RoutineError::BadQop => 14,
            RoutineError::Unauthorized => 15,
            RoutineError::Unavailable => 16,
            RoutineError::DuplicateElement => 17,
            RoutineError::NameNotMn => 18,
            RoutineError::BadMechAttr => 19,
            RoutineError::CredUnavail => 20,
        }
    }

    /// The text `gss_display_status` gives, from the official abstract.
    pub const fn description(self) -> &'static str {
        match self {
            RoutineError::BadMech => "Unsupported mechanism.",
            RoutineError::BadName => "Invalid name.",
            RoutineError::BadNameType => "Unsupported name type.",
            RoutineError::BadBindings => "Channel bindings mismatch.",
            RoutineError::BadStatus => "Invalid status selector.",
            RoutineError::BadMic => "Failed token integrity check.",
            RoutineError::NoCred => "No valid credentials.",
            RoutineError::NoContext => "No valid security context.",
            RoutineError::DefectiveToken => "Defective token.",
            RoutineError::DefectiveCredential => "Defective credential.",
            RoutineError::CredentialsExpired => "Expired credential.",
            RoutineError::ContextExpired => "Expired context.",
            RoutineError::Failure => "An unspecified error.",
            RoutineError::BadQop => "Unsupported QOP value.",
            RoutineError::Unauthorized => "Unauthorized operation.",
            RoutineError::Unavailable => "Unavailable operation.",
            RoutineError::DuplicateElement => "A duplicate credential element requested.",
            RoutineError::NameNotMn => "The name contains multimechanism elements.",
            RoutineError::BadMechAttr => "Unknown mechanism attribute.",
            RoutineError::CredUnavail => "Unavailable credential.",
        }
    }

    /// The routine error for a field value, or `None` when the value names none. Value 6 maps to `BadMic`.
    pub fn from_field(field: u32) -> Option<RoutineError> {
        RoutineError::ALL.iter().copied().find(|e| e.field() == field)
    }
}

/// A `OM_uint32` major status code: the function's return value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MajorStatus(u32);

impl MajorStatus {
    /// `GSS_S_COMPLETE`.
    pub const COMPLETE: MajorStatus = MajorStatus(COMPLETE);

    /// Wraps a raw major code as a function returns it.
    pub const fn from_raw(raw: u32) -> MajorStatus {
        MajorStatus(raw)
    }

    /// The raw 32-bit code.
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Joins the three fields. `None` when a field is wider than its mask, so a value never bleeds into a neighbour.
    pub const fn compose(calling: u32, routine: u32, supplementary: u32) -> Option<MajorStatus> {
        if calling > CALLING_ERROR_MASK || routine > ROUTINE_ERROR_MASK || supplementary > SUPPLEMENTARY_MASK {
            return None;
        }
        Some(MajorStatus((calling << CALLING_ERROR_OFFSET) | (routine << ROUTINE_ERROR_OFFSET) | supplementary))
    }

    /// The calling error field, as the extraction macro reads it.
    pub const fn calling_error_field(self) -> u32 {
        (self.0 >> CALLING_ERROR_OFFSET) & CALLING_ERROR_MASK
    }

    /// The routine error field.
    pub const fn routine_error_field(self) -> u32 {
        (self.0 >> ROUTINE_ERROR_OFFSET) & ROUTINE_ERROR_MASK
    }

    /// The supplementary information field.
    pub const fn supplementary_field(self) -> u32 {
        self.0 & SUPPLEMENTARY_MASK
    }

    /// The calling error, when the calling error field names one.
    pub const fn calling_error(self) -> Option<CallingError> {
        CallingError::from_field(self.calling_error_field())
    }

    /// The routine error, when the routine error field names one.
    pub fn routine_error(self) -> Option<RoutineError> {
        RoutineError::from_field(self.routine_error_field())
    }

    /// True when every field is zero: the test the page describes as a direct comparison with `GSS_S_COMPLETE`.
    pub const fn is_complete(self) -> bool {
        self.0 == COMPLETE
    }

    /// `gss_display_status` for this major code: the calling error text and the routine error text joined by a
    /// space. An unknown field value is named by its number rather than dropped. A complete code reads "Complete."
    pub fn describe(self) -> String {
        if self.is_complete() {
            return "Complete.".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        match self.calling_error_field() {
            0 => {}
            f => match CallingError::from_field(f) {
                Some(e) => parts.push(e.description().to_string()),
                None => parts.push(format!("Unknown calling error {f}.")),
            },
        }
        match self.routine_error_field() {
            0 => {}
            f => match RoutineError::from_field(f) {
                Some(e) => parts.push(e.description().to_string()),
                None => parts.push(format!("Unknown routine error {f}.")),
            },
        }
        if self.supplementary_field() != 0 {
            parts.push(format!("Supplementary information 0x{:x}.", self.supplementary_field()));
        }
        parts.join(" ")
    }
}

impl fmt::Display for MajorStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// A failed call: the major code and the minor code, as `GSSCreateError` pairs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusError {
    pub major: MajorStatus,
    /// The minor code. Its meaning depends on the mechanism named by `kind`.
    pub minor: u32,
    /// `GSS_C_GSS_CODE` or `GSS_C_MECH_CODE`.
    pub kind: u32,
}

impl StatusError {
    /// A status error whose minor code comes from the GSS-API itself.
    pub const fn new(major: MajorStatus, minor: u32) -> StatusError {
        StatusError { major, minor, kind: GSS_CODE }
    }

    /// A status error whose minor code comes from the mechanism.
    pub const fn from_mechanism(major: MajorStatus, minor: u32) -> StatusError {
        StatusError { major, minor, kind: MECH_CODE }
    }

    /// True when the minor code comes from the underlying mechanism.
    pub const fn is_mechanism(&self) -> bool {
        self.kind == MECH_CODE
    }
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let origin = if self.is_mechanism() { "mechanism" } else { "GSS" };
        write!(f, "{} ({} minor status 0x{:x})", self.major.describe(), origin, self.minor)
    }
}

impl std::error::Error for StatusError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_code_splits_into_its_fields() {
        // FAILURE is routine field 13 at bit 16: 13 << 16 = 0x000D_0000.
        let s = MajorStatus::from_raw(0x000D_0000);
        assert_eq!(s.calling_error_field(), 0);
        assert_eq!(s.routine_error_field(), 13);
        assert_eq!(s.supplementary_field(), 0);
        assert_eq!(s.routine_error(), Some(RoutineError::Failure));
        assert!(!s.is_complete());
    }

    #[test]
    fn compose_then_read_back_round_trips() {
        // Calling 1 at bit 24, routine 2 at bit 16, supplementary 5: 0x0102_0005.
        let s = MajorStatus::compose(1, 2, 5).expect("all fields fit");
        assert_eq!(s.raw(), 0x0102_0005);
        assert_eq!(s.calling_error(), Some(CallingError::InaccessibleRead));
        assert_eq!(s.routine_error(), Some(RoutineError::BadName));
        assert_eq!(s.supplementary_field(), 5);
    }

    #[test]
    fn a_field_wider_than_its_mask_is_refused() {
        // The routine field is 8 bits, so 256 does not fit and must not spill into the calling field.
        assert_eq!(MajorStatus::compose(0, 256, 0), None);
        assert_eq!(MajorStatus::compose(256, 0, 0), None);
        assert_eq!(MajorStatus::compose(0, 0, 0x1_0000), None);
    }

    #[test]
    fn zero_is_complete_and_reads_complete() {
        let s = MajorStatus::COMPLETE;
        assert!(s.is_complete());
        assert_eq!(s.describe(), "Complete.");
        assert_eq!(MajorStatus::from_raw(0), s);
    }

    #[test]
    fn describe_joins_the_calling_and_routine_text() {
        let s = MajorStatus::compose(3, 1, 0).unwrap();
        assert_eq!(s.describe(), "Improperly formatted parameter. Unsupported mechanism.");
    }

    #[test]
    fn an_unknown_routine_field_is_named_by_its_number() {
        // Routine value 99 names no error; the text keeps the number instead of dropping it.
        let s = MajorStatus::compose(0, 99, 0).unwrap();
        assert_eq!(s.routine_error(), None);
        assert_eq!(s.describe(), "Unknown routine error 99.");
    }

    #[test]
    fn status_error_shows_the_minor_code_and_its_origin() {
        let gss = StatusError::new(MajorStatus::compose(0, 13, 0).unwrap(), 0x2a);
        assert_eq!(gss.to_string(), "An unspecified error. (GSS minor status 0x2a)");
        let mech = StatusError::from_mechanism(MajorStatus::compose(0, 1, 0).unwrap(), 0x10);
        assert!(mech.is_mechanism());
        assert_eq!(mech.to_string(), "Unsupported mechanism. (mechanism minor status 0x10)");
    }
}
