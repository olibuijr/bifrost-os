//! Error codes shared by the server, the client library and the drivers. Owner: task 01.

/// What went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioError {
    UnknownObject,
    UnknownProperty,
    /// The property exists but cannot be set.
    NotSettable,
    /// The value's type does not match the property.
    BadValueType,
    /// The value is out of range (an unsupported rate or buffer size).
    BadValue,
    /// The device is hogged by another process.
    Hogged,
    /// The caller lacks the permission (microphone, capture, exclusive access).
    PermissionDenied,
    /// The device went away.
    DeviceGone,
    /// The server is not reachable.
    NoServer,
    /// A protocol message did not decode.
    Protocol,
    /// An underlying I/O or driver failure.
    Io,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_compare() {
        assert_eq!(AudioError::Hogged, AudioError::Hogged);
        assert_ne!(AudioError::Io, AudioError::Protocol);
    }
}
