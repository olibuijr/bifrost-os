//! Error codes for configuring and running a guest virtual machine. The code names and their descriptions follow the
//! official Virtualization documentation for its error code enumeration. This module holds the codes, their text
//! and their grouping only; no hypervisor raises them here.

use std::fmt;

/// Error codes a guest virtual machine can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VmErrorCode {
    InternalError,
    InvalidConfiguration,
    InvalidState,
    InvalidStateTransition,
    InvalidDiskImage,
    LimitExceeded,
    NetworkError,
    NotSupported,
    OutOfDiskSpace,
    OperationCancelled,
    InstallationFailed,
    InstallationRequiresUpdate,
    InvalidRestoreImage,
    InvalidRestoreImageCatalog,
    NoSupportedRestoreImages,
    RestoreImageCatalogLoadFailed,
    RestoreImageLoadFailed,
    NetworkBlockDeviceNegotiationFailed,
    NetworkBlockDeviceDisconnected,
    RestoreFromSave,
    SaveFailed,
    DeviceAlreadyAttached,
    DeviceInitializationFailure,
    DeviceNotFound,
    UsbControllerNotFound,
}

impl VmErrorCode {
    /// Every code, in the order the official enumeration lists them.
    pub const ALL: [VmErrorCode; 25] = [
        VmErrorCode::InternalError,
        VmErrorCode::InvalidConfiguration,
        VmErrorCode::InvalidState,
        VmErrorCode::InvalidStateTransition,
        VmErrorCode::InvalidDiskImage,
        VmErrorCode::LimitExceeded,
        VmErrorCode::NetworkError,
        VmErrorCode::NotSupported,
        VmErrorCode::OutOfDiskSpace,
        VmErrorCode::OperationCancelled,
        VmErrorCode::InstallationFailed,
        VmErrorCode::InstallationRequiresUpdate,
        VmErrorCode::InvalidRestoreImage,
        VmErrorCode::InvalidRestoreImageCatalog,
        VmErrorCode::NoSupportedRestoreImages,
        VmErrorCode::RestoreImageCatalogLoadFailed,
        VmErrorCode::RestoreImageLoadFailed,
        VmErrorCode::NetworkBlockDeviceNegotiationFailed,
        VmErrorCode::NetworkBlockDeviceDisconnected,
        VmErrorCode::RestoreFromSave,
        VmErrorCode::SaveFailed,
        VmErrorCode::DeviceAlreadyAttached,
        VmErrorCode::DeviceInitializationFailure,
        VmErrorCode::DeviceNotFound,
        VmErrorCode::UsbControllerNotFound,
    ];

    /// The one-sentence description from the official page.
    pub fn description(self) -> &'static str {
        match self {
            VmErrorCode::InternalError => "An internal error, such as the VM unexpectedly stopping.",
            VmErrorCode::InvalidConfiguration => "An invalid configuration error.",
            VmErrorCode::InvalidState => "An invalid state error.",
            VmErrorCode::InvalidStateTransition => "An invalid state transition error.",
            VmErrorCode::InvalidDiskImage => "An invalid disk-image error.",
            VmErrorCode::LimitExceeded => "Unable to create an additional VM.",
            VmErrorCode::NetworkError => "A network error, such as a failed connection error, occurred.",
            VmErrorCode::NotSupported => "The host computer or operating system isn't supported.",
            VmErrorCode::OutOfDiskSpace => "The host is out of disk space.",
            VmErrorCode::OperationCancelled => {
                "The user canceled an installation, or the app canceled the installation of a guest OS."
            }
            VmErrorCode::InstallationFailed => "An error occurred during installation.",
            VmErrorCode::InstallationRequiresUpdate => {
                "The VM requires a software update in order to complete the installation."
            }
            VmErrorCode::InvalidRestoreImage => "The restore image is invalid.",
            VmErrorCode::InvalidRestoreImageCatalog => "The restore image catalog is invalid.",
            VmErrorCode::NoSupportedRestoreImages => "The restore image catalog has no supported restore images.",
            VmErrorCode::RestoreImageCatalogLoadFailed => "The restore image catalog failed to load.",
            VmErrorCode::RestoreImageLoadFailed => "The restore image failed to load.",
            VmErrorCode::NetworkBlockDeviceNegotiationFailed => {
                "The connection or the negotiation with the network block device server failed."
            }
            VmErrorCode::NetworkBlockDeviceDisconnected => {
                "The network block device client disconnected from the server."
            }
            VmErrorCode::RestoreFromSave => "The VM failed to restore from save file.",
            VmErrorCode::SaveFailed => "The VM failed to save to the save file.",
            VmErrorCode::DeviceAlreadyAttached => "The device already has an attachment to the VM.",
            VmErrorCode::DeviceInitializationFailure => "A device initialization failure.",
            VmErrorCode::DeviceNotFound => "The framework can't find the device.",
            VmErrorCode::UsbControllerNotFound => "The framework can't find the controller.",
        }
    }

    /// Codes raised while restoring an image or a saved state (BifrOSt grouping).
    pub fn is_restore_related(self) -> bool {
        matches!(
            self,
            VmErrorCode::InvalidRestoreImage
                | VmErrorCode::InvalidRestoreImageCatalog
                | VmErrorCode::NoSupportedRestoreImages
                | VmErrorCode::RestoreImageCatalogLoadFailed
                | VmErrorCode::RestoreImageLoadFailed
                | VmErrorCode::RestoreFromSave
        )
    }

    /// Codes about attaching or finding a device (BifrOSt grouping).
    pub fn is_device_related(self) -> bool {
        matches!(
            self,
            VmErrorCode::DeviceAlreadyAttached
                | VmErrorCode::DeviceInitializationFailure
                | VmErrorCode::DeviceNotFound
                | VmErrorCode::UsbControllerNotFound
        )
    }
}

/// A reported error: a code plus optional detail text for the log or the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmError {
    pub code: VmErrorCode,
    pub detail: Option<String>,
}

impl VmError {
    pub fn new(code: VmErrorCode) -> Self {
        Self { code, detail: None }
    }

    pub fn with_detail(code: VmErrorCode, detail: &str) -> Self {
        Self {
            code,
            detail: Some(detail.to_string()),
        }
    }
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{} ({})", self.code.description(), detail),
            None => f.write_str(self.code.description()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_lists_25_distinct_codes_with_distinct_descriptions() {
        assert_eq!(VmErrorCode::ALL.len(), 25);
        let texts: HashSet<&str> = VmErrorCode::ALL.iter().map(|c| c.description()).collect();
        assert_eq!(texts.len(), 25);
    }

    #[test]
    fn descriptions_match_the_official_text() {
        assert_eq!(VmErrorCode::InvalidState.description(), "An invalid state error.");
        assert_eq!(
            VmErrorCode::DeviceNotFound.description(),
            "The framework can't find the device."
        );
        assert_eq!(
            VmErrorCode::SaveFailed.description(),
            "The VM failed to save to the save file."
        );
    }

    #[test]
    fn restore_group_has_six_codes_and_excludes_save() {
        let count = VmErrorCode::ALL.iter().filter(|c| c.is_restore_related()).count();
        assert_eq!(count, 6);
        assert!(VmErrorCode::RestoreFromSave.is_restore_related());
        assert!(!VmErrorCode::SaveFailed.is_restore_related());
    }

    #[test]
    fn device_group_has_four_codes() {
        let count = VmErrorCode::ALL.iter().filter(|c| c.is_device_related()).count();
        assert_eq!(count, 4);
        assert!(VmErrorCode::UsbControllerNotFound.is_device_related());
        assert!(!VmErrorCode::NetworkError.is_device_related());
    }

    #[test]
    fn display_appends_detail_in_parentheses() {
        let plain = VmError::new(VmErrorCode::InvalidState);
        assert_eq!(plain.to_string(), "An invalid state error.");
        let detailed = VmError::with_detail(VmErrorCode::InvalidState, "VM is paused");
        assert_eq!(detailed.to_string(), "An invalid state error. (VM is paused)");
        assert_eq!(detailed.code, VmErrorCode::InvalidState);
    }
}
