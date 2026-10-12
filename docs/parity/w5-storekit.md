# Wave 5: StoreKit gap rows (parity/w5-storekit)

Two gap rows of the StoreKit framework (`docs/parity-matrix/data/storekit.json`) get pure-Rust models. Both are
partial: they are not wired into apps or the shell, and they have not been tested on the VM or Midget.

## What was built

### StoreKit testing (`springboard/ui/src/store_test.rs`)

A test session for purchase flows, the counterpart of the StoreKit testing harness. It keeps a day clock and drives the
existing `entitlement::Ledger` and `purchase::Purchase` models, so a test sees the entitlements an app would see.

- `TestSession::new(day)`, `day()`, `advance(days)` (the clock never runs backwards)
- `buy(product, renewal)`, `refund(id)`, `finish(id)`, `clear()`
- `entitled(product)`, `unfinished_ids()`, `ledger()`
- `approve(attempt, renewal)` and `cancel(attempt)` run a `purchase::Purchase` attempt to success or cancellation

Tests (8): a purchase is entitled from its day; the clock only moves forward; a subscription ends on its expiry day;
a grace period keeps access to its last day; a refund removes the entitlement from its day; finished purchases leave
the unfinished list; clear forgets every purchase and restarts ids; an approved attempt records a purchase and may
install; a cancelled attempt records nothing.

### Signed transaction verification, JWS (`springboard/ui/src/jws.rs`)

The structure layer of JWS Compact Serialization, with the verified or unverified result that Apple's API reports.

- `decode_b64url`: unpadded base64url, std only
- `parse_compact(token)`: three segments, base64url encoding, a non-empty header and payload, a 64-byte ES256
  signature, `alg` equal to ES256, and an `x5c` certificate chain. Errors use the `VerificationError` variants
  (`MissingRequiredProperties`, `InvalidEncoding`, `InvalidSignature`, plus `UnsupportedAlgorithm`, a BifrOSt case).
- `json_string_field`: reads a string value from a flat header object
- `VerificationResult<T>` (`Verified` / `Unverified`) with `payload_value()` and `unchecked_payload_value()`
- `verify_result(payload, signature_ok)`

Not done: the ECDSA P-256 signature check and certificate-chain validation. Std has no P-256 verifier and this wave adds
no crate, so `verify_result` takes the outcome of that check from its caller. Until a real verifier exists, nothing in
BifrOSt can report a signed transaction as verified.

Tests (8): base64url decodes known bytes; it rejects standard symbols and impossible lengths; the JSON field reader;
a two-segment token is an encoding error; a signature that is not 64 bytes is an encoding error; a header without
a certificate chain is missing properties; `verify_result` keeps the payload either way.

## Official API names mirrored

- StoreKit landing page: https://developer.apple.com/documentation/storekit
- `VerificationResult` (`verified`, `unverified`, `payloadValue`, `unsafePayloadValue`):
  https://developer.apple.com/documentation/storekit/verificationresult
- `VerificationResult.VerificationError`: https://developer.apple.com/documentation/storekit/verificationresult/verificationerror
- `Transaction`, `Transaction.updates`: https://developer.apple.com/documentation/storekit/transaction
- The StoreKit testing harness is listed under Related Documentation on the StoreKit landing page ("Store Kit Test").

## REA evidence

- `ev_442b3c211edd28b48f4818d8be816955fa84767cec731d0cf09c9101b8ae451d`: StoreKit landing page (rendered)
- `ev_b79f55bc330dba2e281e60130e801af28201b1a3207bd322eeef6aba97deaacc`: VerificationResult page (rendered)

## Matrix rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| StoreKit | StoreKit testing | partial | springboard/ui/src/store_test.rs |
| StoreKit | Signed transaction verification (JWS) | partial | springboard/ui/src/jws.rs |

## Not built

- App Store interactions (gap): needs an account-level store service that does not exist in the SDK yet.
- Promoted purchases (gap): tied to the store-side product page, which BifrOSt does not have.
- The matrix row "Transactions and entitlements" and the other partial rows are unchanged.
