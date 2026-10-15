# Encrypted archive wrapper

`encrypted_archive` seals and opens caller-provided archive bytes using a versioned `BFEA` header and the SDK's
ChaCha20-Poly1305 key type. The header is authenticated as additional data. Integration tests cover the byte layout,
round-trip, wrong keys, tampering, malformed input and unsupported versions.

This is intentionally recorded as partial, not Apple Archive parity: BFEA is a BifrOSt-specific wrapper, does not
compress, and does not implement Apple's encrypted archive profiles or key derivation/storage. Callers own the key.
