//! Resources (`FSResource` parity): the things a file system module is asked to act on. A block device is a device
//! node directly under `/dev`, a path URL is an absolute file path, and a generic URL is any other URL with a scheme.
//! Pure model: no device is opened, no path is read and no URL is fetched. The checks only decide whether the text
//! names a resource of the right kind.
//!
//! Gaps: no resource is opened or mounted here, and a generic URL is kept as text without percent-decoding.

/// A thing a file system module can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    /// A block device node under `/dev`, such as `/dev/sdb1`.
    BlockDevice(String),
    /// An absolute file path, such as `/Volumes/archive.img`, with no `.` or `..` parts.
    PathUrl(String),
    /// Any other URL: a scheme, a colon, and a non-empty location, such as `https://example.com/disk.img`.
    GenericUrl(String),
}

/// Why a piece of text was refused as a resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceError {
    /// The text is empty.
    Empty,
    /// The text contains a control character, such as a line break or a NUL.
    ControlCharacter,
    /// A block device is not a single node name under `/dev`.
    NotADeviceNode,
    /// A path URL does not start with `/`.
    NotAbsolutePath,
    /// A path has a `.` or `..` part.
    DotSegment,
    /// A generic URL has no scheme, or the scheme is not a letter followed by letters, digits, `+`, `-` or `.`.
    BadScheme,
    /// A generic URL has nothing after its colon.
    MissingLocation,
}

const DEVICE_DIR: &str = "/dev/";

impl Resource {
    /// A block device node. The name after `/dev/` is one component made of letters, digits, `-`, `_` and `.`,
    /// and it is not made of dots alone.
    pub fn block_device(node: &str) -> Result<Self, ResourceError> {
        check_text(node)?;
        let name = node.strip_prefix(DEVICE_DIR).ok_or(ResourceError::NotADeviceNode)?;
        let name_ok = !name.bytes().all(|b| b == b'.')
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if !name_ok {
            return Err(ResourceError::NotADeviceNode);
        }
        Ok(Resource::BlockDevice(node.to_string()))
    }

    /// An absolute path. Trailing slashes and repeated slashes are allowed. A `.` or `..` part is refused.
    pub fn path_url(path: &str) -> Result<Self, ResourceError> {
        check_text(path)?;
        if !path.starts_with('/') {
            return Err(ResourceError::NotAbsolutePath);
        }
        if path.split('/').any(|part| part == "." || part == "..") {
            return Err(ResourceError::DotSegment);
        }
        Ok(Resource::PathUrl(path.to_string()))
    }

    /// Any URL with a scheme. The scheme must start with a letter and continue with letters, digits, `+`, `-` or
    /// `.`, and the text after the colon must not be empty.
    pub fn generic_url(url: &str) -> Result<Self, ResourceError> {
        check_text(url)?;
        let (scheme, location) = url.split_once(':').ok_or(ResourceError::BadScheme)?;
        let mut chars = scheme.chars();
        let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
        let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if !starts_with_letter || !rest_ok {
            return Err(ResourceError::BadScheme);
        }
        if location.is_empty() {
            return Err(ResourceError::MissingLocation);
        }
        Ok(Resource::GenericUrl(url.to_string()))
    }

    /// Picks the kind from the text: a `/dev/` prefix is a block device, another `/` prefix is a path, and
    /// anything else is a generic URL.
    pub fn parse(text: &str) -> Result<Self, ResourceError> {
        if text.starts_with(DEVICE_DIR) {
            Self::block_device(text)
        } else if text.starts_with('/') {
            Self::path_url(text)
        } else {
            Self::generic_url(text)
        }
    }

    /// The text the resource was made from.
    pub fn text(&self) -> &str {
        match self {
            Resource::BlockDevice(text) | Resource::PathUrl(text) | Resource::GenericUrl(text) => text,
        }
    }
}

/// Refuses empty text and text with control characters, which no resource name may contain.
fn check_text(text: &str) -> Result<(), ResourceError> {
    if text.is_empty() {
        return Err(ResourceError::Empty);
    }
    if text.chars().any(char::is_control) {
        return Err(ResourceError::ControlCharacter);
    }
    Ok(())
}
