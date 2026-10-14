//! Media extensions for apps: the entitlements a media extension declares, and the check that admits a format reader
//! into the reader registry (format reader entitlement parity). The registry is `bifrost_ui::format_readers`; this
//! module decides who may add to it. The extension still runs in-process: no separate extension process hosts a
//! format reader or a video decoder yet.
//!
//! Rules of this model:
//! - A format reader is registered only from a declaration that holds [`Entitlement::FormatReader`].
//! - A declaration that lacks the entitlement is refused before the registry is touched.

use bifrost_ui::format_readers::{FormatReader, ReaderRegistry, RegistryError};

/// What a media extension may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entitlement {
    /// Read a container or a file format and offer it to apps.
    FormatReader,
    /// Decode a video codec.
    VideoDecoder,
    /// Process camera raw files.
    RawProcessor,
}

/// An extension's identity and the entitlements it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub bundle_id: String,
    pub entitlements: Vec<Entitlement>,
}

impl Declaration {
    /// A declaration for `bundle_id` with these entitlements.
    pub fn new(bundle_id: &str, entitlements: &[Entitlement]) -> Declaration {
        Declaration { bundle_id: bundle_id.to_string(), entitlements: entitlements.to_vec() }
    }

    /// Whether the declaration holds `entitlement`.
    pub fn grants(&self, entitlement: Entitlement) -> bool {
        self.entitlements.contains(&entitlement)
    }
}

/// Why an extension's reader is not registered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionError {
    /// The declaration lacks this entitlement.
    MissingEntitlement(Entitlement),
    /// The registry refused the reader.
    Registry(RegistryError),
}

/// Registers a reader that `declaration` offers, when the declaration holds the format reader entitlement.
pub fn register_reader(
    registry: &mut ReaderRegistry,
    declaration: &Declaration,
    reader: FormatReader,
) -> Result<(), ExtensionError> {
    if !declaration.grants(Entitlement::FormatReader) {
        return Err(ExtensionError::MissingEntitlement(Entitlement::FormatReader));
    }
    registry.register(reader).map_err(ExtensionError::Registry)
}
