# Parity wave 4: Security (`security`)

Two gap rows of the Security framework now have an app-facing SDK API in pure Rust (std only, no new crates, no
unsafe). Both are in `bifrost_app` (`springboard/sdk/src`). The Security framework's other gap rows (certificate and
trust evaluation, code signing, Authorization Services, key objects) need more than a pure model and are not in this
wave.

## What was built

- `springboard/sdk/src/entropy.rs`: random bytes for apps. `copy_bytes` fills a buffer from the kernel pool
  (`/dev/urandom`), reading into a scratch buffer first so a failed read leaves the caller's buffer unchanged.
  `bytes(n)`, `random_u64()`, `below(bound)` (unbiased by rejection sampling, limit from `accept_limit`) are built on it.
  Failures return `Error::Unavailable`. Six tests.
- `springboard/sdk/src/digest.rs`: security transforms that need no key store. `sha256` delegates to the shell's own
  `bifrost_core::sha256::digest`, so there is no second SHA-256. `hmac_sha256` implements RFC 2104 on that hash (keys
  longer than 64 bytes are hashed, shorter keys padded with zeros). `constant_time_eq` compares without an early exit
  and returns false at once on a length mismatch. Six tests.
- `springboard/sdk/src/lib.rs`: two `pub mod` lines (`entropy`, `digest`).

Not built: encryption, signatures and key objects (the rest of "Security transforms"). Those need asymmetric crypto
that is not in the repo and would need a new crate or a large hand-written implementation.

## Official API names mirrored

- `SecRandomCopyBytes(_:_:_:)`: "Generates an array of cryptographically secure random bytes." Mirrored by
  `entropy::copy_bytes`. Its result-code return maps to `Result<(), Error>`.
- `Randomization Services` (topic): mirrored by the `entropy` module.
- `Security Transforms` (topic): the hash and MAC part is mirrored by `digest::sha256` and `digest::hmac_sha256`.
  The MAC follows RFC 2104 and is not checked against a named Apple function on the Security Transforms page; the
  official page for the MAC API was not fetched in this wave.
- `Security` landing page: lists "Randomization Services" and "Security Transforms" under Cryptography.

Differences: the calls are synchronous, as Apple's are. The random source is the kernel pool, the same one the shell
uses. There is no random generator object (`rnd` parameter) and no default-generator constant.

## REA evidence

- `ev_516c4d9f420c2f75c93c9a1b34fc38f257349907743b35034a879c7c8f58fc4a`: rendered Security landing page
  (`https://developer.apple.com/documentation/security`), Cryptography section.
- `ev_9d28a236691e2cf66d3be2854e17299ca163d171c24d74a5909a85aceb2b44b5`: rendered `SecRandomCopyBytes(_:_:_:)`
  page (`https://developer.apple.com/documentation/security/secrandomcopybytes(_:_:_:)`), macOS 10.7+.

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/security.json`) | New status | BifrOSt path |
|---|---|---|---|
| security | Randomization services | partial | `springboard/sdk/src/entropy.rs` |
| security | Security transforms (encryption, signing, verification) | partial | `springboard/sdk/src/digest.rs` |

Both rows stay `partial`: the entropy row is missing the generator object and the crypto transforms (encrypt, sign,
verify) are not built. The lead updates the matrix centrally.

## Notes for the lead

- `springboard/src/keychain.rs` has its own private HMAC-SHA256 (`fn hmac`) and its own `random` helper. They were not
  switched to the new SDK modules, because the shell crate cannot depend on the SDK crate. A later change could move
  the shared pieces into `bifrost_core`.
- No build or test was run (the wave rule forbids it). The code and tests were checked by hand. The HMAC test vector is
  RFC 4231 case 2; the SHA-256 vector is FIPS 180-4 "abc".
