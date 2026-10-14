# Parity sdk-rest-05: ChaCha20-Poly1305 and the streaming sound analyzer

## What is built

- **ChaCha20-Poly1305 (RFC 8439).** `springboard/core/src/chacha_poly.rs` holds the pure std model: the ChaCha20
  block function, Poly1305 with five 26-bit limbs, and `seal` and `open`. `springboard/sdk/src/chacha_poly.rs` gives
  apps `ChaChaPolyKey`, which validates the 32-byte key, draws a fresh nonce for each seal, and returns one sealed box
  (nonce, ciphertext, tag). Tests: `springboard/core/tests/parity_sdk-rest-05.rs` and
  `springboard/sdk/tests/parity_sdk-rest-05.rs`.
- **Streaming sound analysis.** `springboard/ui/src/sound_stream.rs` gives `StreamAnalyzer`, which takes ordered
  interleaved buffers at frame positions, averages channels to mono, and classifies each full window through the
  same `ClassifyRequest` windows as the file analyzer. Tests: `springboard/ui/tests/parity_sdk-rest-05.rs`.

## What is wired and what needs the lab machine

- Nothing in the shell or in apps calls the new modules yet. The keychain still seals with its own HMAC-SHA256
  construction.
- The audio service does not send live microphone buffers to apps; `StreamAnalyzer` is ready for that feed.
- All of this is tested on the host only. Midget verification is pending, and the rows stay `partial` until the lead
  verifies them there.

## Rows left as they were

- Post-quantum KEM and signatures (ML-KEM, X-Wing, ML-DSA): no FIPS 203 or FIPS 204 arithmetic exists in the repo.
- Device discovery extension (AVRouting): no shell-side browse service feeds receivers into the route picker.
- Rendering on macOS requirements (GLKit): left as `os`; the GPU stack is shell-side.
