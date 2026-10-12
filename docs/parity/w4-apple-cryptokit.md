# Wave 4: CryptoKit hashing, MAC and key derivation in pure Rust

Framework: Apple CryptoKit (`docs/parity-matrix/data/apple-cryptokit.json`). Branch `parity/w4-apple-cryptokit`.

## What was built

All three modules are pure Rust, std only, with no `unsafe` and no new crates. They live in the shell's core
crate (`springboard/core/src/`), next to `sha256.rs`, and each has one `pub mod` line in `lib.rs`.

- `springboard/core/src/sha512.rs`: SHA-512 and SHA-384 (FIPS 180-4), incremental (`Sha512::new`, `Sha512::new_384`,
  `update`, `finish`), with one-shot `digest512` and `digest384`. The constants are derived from prime roots, not
  copied from memory: the 80 cube-root constants, the SHA-512 square-root IV and the SHA-384 IV.
- `springboard/core/src/sha3.rs`: SHA3-256, SHA3-384 and SHA3-512 over the Keccak-f[1600] sponge (FIPS 202), with
  `Sha3::new_256/new_384/new_512`, `update`, `finish` and one-shot `sha3_256/384/512`. The round constants follow
  the rc(t) LFSR of FIPS 202.
- `springboard/core/src/hkdf.rs`: HMAC-SHA256 (RFC 2104, `hmac_sha256`), HKDF-Extract (`hkdf_extract`),
  HKDF-Expand (`hkdf_expand`, rejects more than 255 hash blocks) and the one-call `hkdf_sha256`, per RFC 5869.

Each module has a `#[cfg(test)] mod tests`. The expected values are published test vectors (NIST and RFC
4231/5869 cases, plus the empty string, "abc" and one million "a"). They were also checked against Python's stdlib
`hashlib` and `hmac` before the Rust was written. The Keccak code was checked by a line-for-line Python
transliteration against `hashlib` for lengths 0 to 1000 bytes, which covers the rate boundary.

## Official API names mirrored

| Official API (developer.apple.com/documentation/cryptokit) | BifrOSt |
|---|---|
| `SHA512`, `SHA512.Digest`, `init()`, `update(bufferPointer:)`, `finalize()`, `byteCount`, `blockByteCount` | `sha512::Sha512`, `new`, `update`, `finish`, `digest512` |
| `SHA384` | `sha512::Sha512::new_384`, `digest384` |
| `SHA3_256`, `SHA3_384`, `SHA3_512` (and digest types) | `sha3::Sha3::new_256/new_384/new_512`, `sha3_256/384/512` |
| `HKDF` `deriveKey(inputKeyMaterial:salt:info:outputByteCount:)`, `extract(inputKeyMaterial:salt:)`, `expand(pseudoRandomKey:info:outputByteCount:)` | `hkdf::hkdf_sha256`, `hkdf_extract`, `hkdf_expand` |
| `HMAC` (SHA-256 instance) | `hkdf::hmac_sha256` |

REA evidence of the rendered official pages (one `rea-page.sh` run each):

- CryptoKit landing page: `ev_dc5cf4c819d1c307e1e4087c93b6b0ae9af5cec13de2bf128003a5480c09c799`
- SHA512: `ev_ba4ff5a37808e54793930dcb09d3482704fab217a3085e899676b6b7ecd95cb5`
- SHA384: `ev_0c2fe4838825b4fa01dc86a546df8986b1fa9e1451debaa3b90f36545144e098`
- SHA3_256: `ev_2a26cf72745baaf815eae80a0a0803e7fba4fc3d6dcf140714c9c8ae2fafa868`
- HKDF: `ev_e85bf2363b7b2bbd0ef3efe90e64b690aaef37422c86412fb04f53a1a11714b6`

The HKDF page says the key derivation is RFC 5869 compliant, which is what `hkdf.rs` implements. Apple's functions
return a `SymmetricKey`; `hkdf.rs` returns plain bytes, because there is no typed key object yet.

## Matrix rows closed

Status is `partial` for each row. The matrix itself is not edited here; the lead updates it.

| Framework slug | Feature (as written in the data file) | New status | BifrOSt path |
|---|---|---|---|
| apple-cryptokit | SHA-2 hashing (SHA256, SHA384, SHA512, HashFunction) | partial | `springboard/core/src/sha512.rs` (`digest512`, `digest384`, `Sha512`), with `springboard/core/src/sha256.rs` |
| apple-cryptokit | SHA-3 hashing | partial | `springboard/core/src/sha3.rs` (`sha3_256`, `sha3_384`, `sha3_512`, `Sha3`) |
| apple-cryptokit | HMAC message authentication | partial | `springboard/core/src/hkdf.rs` (`hmac_sha256`) |
| apple-cryptokit | Per-app key derivation (HKDF) | partial | `springboard/core/src/hkdf.rs` (`hkdf_sha256`, `hkdf_extract`, `hkdf_expand`) |

## Not done, and why

- No app-facing SDK type. Apps still get no `HashFunction` protocol, no `SymmetricKey` and no HMAC or HKDF
  call through `springboard/sdk/src/`. The core functions are the pure-Rust layer that an SDK type would wrap.
- The shell's keychain keeps its own private `hmac` (`springboard/src/keychain.rs`) and its `derive` function. The
  change was not made: the wave may not build, so the keychain's crypto path stays as it was. Switching it to
  `bifrost_core::hkdf::hmac_sha256` is a follow-up for a round that can build and test it.
- Not started: AES-GCM, ChaCha20-Poly1305 with Poly1305, the NIST and Curve25519 primitives, HPKE and the
  post-quantum KEMs. These need bigger, timing-sensitive code, and the wave covers only the hashing and KDF rows.
- Nothing here was built or run, because of this wave's rule: no cargo, rustc or tests. The tests and the
  `tools/check-names.py` guard (no hits) are the next steps for the integration build.
