# Wave 5: CryptoKit legacy digests (MD5, SHA-1)

Branch `parity/w5-apple-cryptokit`, based on `parity/integration`. Closes one gap row of the CryptoKit framework.

## What was built

- `springboard/core/src/md5.rs`: MD5 (RFC 1321), incremental `Md5` (`new`, `update`, `finish`) and `digest`. The
  additive constants are computed from the RFC's sine definition at first use, not typed in.
- `springboard/core/src/sha1.rs`: SHA-1 (FIPS 180-4), incremental `Sha1` (`new`, `update`, `finish`) and `digest`.
- `springboard/sdk/src/digest.rs` (extended, wave 4 module): a new `insecure` namespace with `md5(data) -> [u8; 16]`
  and `sha1(data) -> [u8; 20]`, wrapping the core digests. Its module doc and the `insecure` doc warn that both are not
  collision resistant and must not authenticate data.
- Both core modules are registered with one `pub mod` line each in `springboard/core/src/lib.rs`.

The core modules are std only, with no unsafe, no new crates and no hardware or network access.

## Official API mirrored

Official page: https://developer.apple.com/documentation/cryptokit/insecure (the framework page is
https://developer.apple.com/documentation/cryptokit). Mirrored symbols:

- `Insecure` (container for legacy algorithms): `insecure` module.
- `Insecure.MD5` with `init`, `update`, `finalize` and `byteCount`: `md5` function and the `Md5` incremental state.
- `Insecure.SHA1` with `init`, `update`, `finalize` and `byteCount`: `sha1` function and the `Sha1` incremental state.
- `Insecure.MD5Digest` and `Insecure.SHA1Digest` are represented as `[u8; 16]` and `[u8; 20]`.

Official warning, quoted from the page: "These algorithms aren't considered cryptographically secure, but the framework
provides them for backward compatibility with older services that require them. For new services, avoid these
algorithms." The BifrOSt doc comments say the same.

Not mirrored: the typed `Digest` protocol and `HashFunction` conformance, `blockByteCount`, and the `Digest` value
types. Apps get byte arrays, not Digest objects, as with the rest of the SDK digest module.

## Verification

No build or test was run, per the wave rules. The tests were written against published vectors, so a later host test
run will check them:

- RFC 1321 test suite for MD5: `""`, `"a"`, `"abc"`, and the quick brown fox sentence.
- FIPS 180 vectors for SHA-1: `""`, `"abc"`, the 56-byte `abcdbcdecdef...` message, and one million `a` characters.
- MD5 first and last constants `0xd76aa478` and `0xeb86d391` (from the sine rule).
- Chunked updates equal one-shot digests at every length from 0 to 130 bytes, around the 64-byte block boundary.
- A 56-byte message that needs an extra padding block, and a changed final byte that changes the digest.

## REA evidence

- Insecure page: `ev_5ad7a33fa438e914f4413d5a978a5c3cbc49200195adc63e9227915bdeccf167`
- CryptoKit framework landing page: `ev_8d281c67ba14dbe9b9cba9a2daceb775cf6ea92cdac840336486c0752725755c`

## Matrix rows closed

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| apple-cryptokit | Legacy insecure algorithms (Insecure: MD5, SHA-1) | partial | springboard/core/src/md5.rs, springboard/core/src/sha1.rs, springboard/sdk/src/digest.rs (`insecure`) |

Partial, not parity: the app-facing API is a byte-array function, not the framework's Digest types, and the insecure
digests are not yet wired into a shell or app feature.

## Not built this wave

The CryptoKit gap rows for AES-GCM, public-key signatures (Ed25519, P-256, P-384, P-521), HPKE and post-quantum KEM
and signatures were judged too large to write and check correctly without a build and test run. AES-GCM needs an S-box
and GF(2^128) arithmetic that would be unverified here. They remain gaps.
