//! ContentKit for apps: names a file's type from its leading bytes and its name together.
//!
//! [`identify`] sniffs the bytes first, with the same signatures the shell's content sniffer uses. A ZIP container
//! is refined by the file name, because a `.jar` or `.apk` is a more specific type that conforms to the ZIP type.
//! When the bytes name nothing, the file name's extension decides, as `uttype::type_for_extension` does.
//!
//! ```ignore
//! use bifrost_app::content_kit::identify;
//! let head = &bytes[..bytes.len().min(bifrost_app::content_kit::SNIFF_LEN)];
//! let kind = identify(Some("bundle.jar"), head); // Some("com.sun.java-archive")
//! ```

pub use bifrost_ui::content_sniff::{sniff, SNIFF_LEN};

use bifrost_ui::uttype;

/// The sniffed container type that a file name may refine.
const CONTAINER: &str = "public.zip-archive";

/// The type identifier for a file, from its name (optional) and its leading bytes. `None` when neither names a type.
pub fn identify(name: Option<&str>, head: &[u8]) -> Option<&'static str> {
    let by_name = name.and_then(uttype::type_for_extension);
    match sniff(head) {
        Some(CONTAINER) => by_name.filter(|t| uttype::conforms(t, CONTAINER)).or(Some(CONTAINER)),
        Some(sniffed) => Some(sniffed),
        None => by_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZIP: &[u8] = b"PK\x03\x04 rest of a zip";
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";

    #[test]
    fn the_name_refines_a_zip_container() {
        assert_eq!(identify(Some("bundle.jar"), ZIP), Some("com.sun.java-archive"));
        assert_eq!(identify(Some("app.apk"), ZIP), Some("com.android.package-archive"));
        assert_eq!(identify(Some("data.bin"), ZIP), Some("public.zip-archive"), "an unknown name keeps the container");
        assert_eq!(identify(None, ZIP), Some("public.zip-archive"));
    }

    #[test]
    fn the_bytes_win_over_a_wrong_name() {
        assert_eq!(identify(Some("notes.txt"), PNG), Some("public.png"));
    }

    #[test]
    fn the_name_decides_when_the_bytes_name_nothing() {
        assert_eq!(identify(Some("notes.zip"), b""), Some("public.zip-archive"));
        assert_eq!(identify(None, b""), None);
        assert_eq!(identify(None, b"#!/bin/sh\necho hi\n"), Some("public.shell-script"));
    }
}
