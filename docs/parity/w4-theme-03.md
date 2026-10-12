# Wave 4, theme-03: key agreement (X25519, SharedSecret)

## What was built

`springboard/ui/src/x25519.rs` (pure Rust, std only, no unsafe): the X25519 function of RFC 7748
over GF(2^255 - 19) in five 51-bit limbs, with a Montgomery ladder over all 255 bits. Swaps are masks,
not branches. On top of it:

- `PrivateKey` (`from_raw`, `raw`, `public_key`, `shared_secret`), `PublicKey` (`from_raw`, `raw`),
  `SharedSecret` (`bytes`, constant-time `PartialEq`), `KeyError` (`BadLength`, `LowOrderPoint`).
- `shared_secret` refuses an all-zero result, which is what a small-order peer key produces.
- Debug output never prints key or secret bytes.

Randomness is the caller's job: keys come from 32 raw bytes.

## Official API mirrored

- `Curve25519.KeyAgreement` (`PrivateKey`, `PublicKey`, `sharedSecretFromKeyAgreement`):
  https://developer.apple.com/documentation/cryptokit/curve25519/keyagreement
- `SharedSecret`: https://developer.apple.com/documentation/cryptokit/sharedsecret
- `SharedSecret` derivation (`hkdfDerivedSymmetricKey`, `x963DerivedSymmetricKey`) is not built: both need
  SHA-256, which the UI crate does not have.

## REA evidence

- `ev_e26e2faf998e28d0b8850f8f6cd7c1297a2b260b28cb1d985883ca83ce429936`
  (rendered page `Curve25519.KeyAgreement`, iOS 13.0+ and macOS 10.15+).

## Tests

Seven tests in `x25519.rs`: RFC 7748 section 5.2 vector one, one iteration from the base point,
RFC 7748 section 6.1 Diffie-Hellman (both sides agree), zero peer key refused, key length errors and
their message, field encoding (p reduces to 0, 1 encodes as 01), and small field arithmetic
(2 * 3 = 6, p - 1 encoding). The RFC vectors were copied from the RFC, not computed here. No build was run,
so the tests have not been executed in this round.

## Matrix rows closed

| framework slug | feature | status | BifrOSt path |
|---|---|---|---|
| apple-cryptokit | Key agreement (X25519, SharedSecret) | partial | springboard/ui/src/x25519.rs |

Partial because: X25519 agreement and the secret type are built, but derivation (HKDF, X9.63) is missing,
there is no SDK wiring for apps, and key generation is left to the caller.

## Not closed (theme rows that stay gaps)

Only one of the 34 theme rows is a key agreement row. The other theme rows are smart-card, passkey,
Wi-Fi keychain, GSS, Multipeer and similar rows, which need different subsystems and were not started.
