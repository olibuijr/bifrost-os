//! TLS options for a secure connection as a pure model: the protocol version range, the application protocol names
//! offered in the handshake, the server name, and how the peer certificate is trusted. The shell's HTTP client is
//! fixed to rustls with the webpki roots. This type checks an options set before any client is built from it, so
//! an invalid range, an empty pin set or a malformed name is refused with a reason instead of failing in the handshake.

/// A TLS protocol version. The variants are ordered from oldest to newest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TlsVersion {
    Tls12,
    Tls13,
}

/// How the peer certificate is trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// The certificate chain must end in a root of the built-in trust store.
    SystemRoots,
    /// The peer must present one of these certificates, identified by the SHA-256 digest of its DER encoding.
    Pinned(Vec<[u8; 32]>),
}

/// Why an options set was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsOptionsError {
    /// The minimum version is newer than the maximum.
    InvalidVersionRange,
    /// An application protocol name is empty or longer than 255 bytes (RFC 7301).
    InvalidProtocolName,
    /// The server name is empty, longer than 253 bytes, or contains a character a host name cannot hold.
    InvalidServerName,
    /// A pinned trust set has no certificates, so no peer could ever be accepted.
    EmptyPinSet,
}

/// The TLS options of one connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsOptions {
    min_version: TlsVersion,
    max_version: TlsVersion,
    alpn: Vec<String>,
    server_name: Option<String>,
    trust: Trust,
}

impl Default for TlsOptions {
    /// TLS 1.2 to 1.3, no application protocols, no explicit server name, and the system roots.
    fn default() -> Self {
        TlsOptions {
            min_version: TlsVersion::Tls12,
            max_version: TlsVersion::Tls13,
            alpn: Vec::new(),
            server_name: None,
            trust: Trust::SystemRoots,
        }
    }
}

impl TlsOptions {
    /// The default options: TLS 1.2 to 1.3 against the system roots.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the version range. Refuses a minimum newer than the maximum.
    pub fn with_versions(mut self, min: TlsVersion, max: TlsVersion) -> Result<Self, TlsOptionsError> {
        if min > max {
            return Err(TlsOptionsError::InvalidVersionRange);
        }
        self.min_version = min;
        self.max_version = max;
        Ok(self)
    }

    /// Sets the application protocols offered in the handshake, in preference order. Refuses an empty or
    /// overlong name.
    pub fn with_alpn(mut self, protocols: &[&str]) -> Result<Self, TlsOptionsError> {
        if protocols.iter().any(|p| p.is_empty() || p.len() > 255) {
            return Err(TlsOptionsError::InvalidProtocolName);
        }
        self.alpn = protocols.iter().map(|p| p.to_string()).collect();
        Ok(self)
    }

    /// Sets the server name sent in the handshake and checked against the certificate.
    pub fn with_server_name(mut self, name: &str) -> Result<Self, TlsOptionsError> {
        let valid = !name.is_empty()
            && name.len() <= 253
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
        if !valid {
            return Err(TlsOptionsError::InvalidServerName);
        }
        self.server_name = Some(name.to_string());
        Ok(self)
    }

    /// Replaces the trust policy. A pinned set must hold at least one digest.
    pub fn with_trust(mut self, trust: Trust) -> Result<Self, TlsOptionsError> {
        if let Trust::Pinned(pins) = &trust {
            if pins.is_empty() {
                return Err(TlsOptionsError::EmptyPinSet);
            }
        }
        self.trust = trust;
        Ok(self)
    }

    /// The lowest version the connection may use.
    pub fn min_version(&self) -> TlsVersion {
        self.min_version
    }

    /// The highest version the connection may use.
    pub fn max_version(&self) -> TlsVersion {
        self.max_version
    }

    /// True when the connection may use `version`.
    pub fn allows(&self, version: TlsVersion) -> bool {
        version >= self.min_version && version <= self.max_version
    }

    /// The application protocols offered, in preference order.
    pub fn alpn(&self) -> &[String] {
        &self.alpn
    }

    /// The server name, when one was set.
    pub fn server_name(&self) -> Option<&str> {
        self.server_name.as_deref()
    }

    /// The trust policy.
    pub fn trust(&self) -> &Trust {
        &self.trust
    }
}
