# Wave 5, theme-08: cryptographic assets on a smart card

Theme: "Using cryptographic assets stored on a smart card" (docs/parity-matrix/themes.json, theme-08).
Branch `parity/w5-theme-08`, built on `parity/integration` (waves 1 to 4). Pure Rust, std only, no unsafe, no
hardware or network access. Host unit tests only: nothing was built or run (wave rule: no builds in coding
waves). Not wired into apps or the shell, not tested on the VM or Midget.

## What was built

Four modules in `springboard/ui/src` (one `pub mod` line each in `lib.rs`), plus the tests in each module.

| Module | Mirrors | Behaviour |
|---|---|---|
| `token_objects.rs` | `TKSmartCardToken` (objects on the card), `TKSmartCardTokenSession` (use gated by an authenticated session) | Store of a token's private keys, public keys and certificates. Each private key carries a bit set of uses (sign, decrypt, derive). `use_key` clears a key for one use and returns a `KeyRef` with no key material. It refuses a session for another token (`TokenNotFound`), an unauthenticated or locked session (`AuthenticationNeeded`), a missing object (`ObjectNotFound`), and a use the key does not allow or a non-key object (`BadParameter`). |
| `token_registration.rs` | `TKSmartCardTokenRegistrationManager` (registered tokens, register and unregister), the sign-in part of "Authenticating Users with a Cryptographic Token" | Registers a token to one account, optionally persistent. A non-persistent registration is dropped when its token is removed; a persistent one (for example an HSM) stays. `authenticate` signs a user in only when the token is present in the `TokenWatcher` and registered to that account, then passes the driver's verdict to the existing `TokenSession`. A registration to another account does not count as a failed attempt. An enforcement switch refuses password sign-in for accounts that have a registered token. The enforcement rule is BifrOSt's own policy, not an Apple behaviour. |
| `certificate_panel.rs` | `SFCertificatePanel` (certificate details for a chain, and the evaluation status of each certificate) | Chain of 1 to 16 certificates (`MAX_CHAIN` from `certificate_trust`), each well-formed (subject, issuer, serial present; not-before not after not-after). Validity at a caller-supplied time (Unix seconds): not yet valid, valid, expired, with both ends valid. Selection, chain validity, and the detail rows (Subject, Issuer, Serial number, Status). Details only; trust editing stays in `certificate_trust.rs`. No certificate parsing. |
| `identity_chooser.rs` | `SFChooseIdentityPanel` (the list of identities, default and alternate button titles, informative text) | Identity = certificate plus a has-private-key flag. The policy filters by key requirement and by validity (expired allowed or not; not yet valid never listed). Listed sorted by subject, first row selected by default. `confirm` returns the selected identity or `NothingSelected`. Button titles default to Choose and Cancel. The table cell view is not modelled. |

Existing modules reused, not duplicated: `token_watch.rs` (`TokenWatcher`, `TokenSession`, `TokenError`), and
`certificate_trust.rs` (`MAX_CHAIN`).

## REA evidence (official rendered pages)

| API | Official page | REA evidence id |
|---|---|---|
| TKSmartCardTokenRegistrationManager | developer.apple.com/documentation/cryptotokenkit/tksmartcardtokenregistrationmanager | ev_bcdb33fce675270a86f23c17365892f775652f685a9fcc9b66daacea5b2a4ffe |
| TKSmartCardToken | developer.apple.com/documentation/cryptotokenkit/tksmartcardtoken | ev_cd7098efb9db30ebc085cce0887150b226a364a189f1b47cfa3015a08c222336 |
| TKSmartCardTokenSession | developer.apple.com/documentation/cryptotokenkit/tksmartcardtokensession | ev_589c6eeec205d0ef5451ecc3901ac6431408c52d405ea8c80d7165a191c5c365 |
| SFCertificatePanel | developer.apple.com/documentation/securityinterface/sfcertificatepanel | ev_acebd3af7d076253d907b6b0dfd44510cc2b023cf96dfeeba8b7c570065bff8d |
| SFChooseIdentityPanel | developer.apple.com/documentation/securityinterface/sfchooseidentitypanel | ev_280e0d92a35790253005a37c50c15b878eb24cb478d4b9be9a55ecdda9586e51 |

Facts used from the rendered pages: the registration manager lists `registeredSmartCardTokens`, registers
(`registerSmartCard(tokenID:promptMessage:)`) and unregisters (`unregisterSmartCard(tokenID:)`) a token; a
token session is used to access and send APDUs to the card; the certificate panel displays the certificates of a
chain with their evaluation status and does not show trust settings (that is the trust panel's job); the choose
identity panel lists identities (a certificate with its private key) and can show any certificate's contents.
The DocC JSON path returned 404 for the security-interface pages in this session, so the rendered pages are the
evidence used for those APIs.

## Matrix rows closed (status moves to partial)

Framework slugs and feature names are exactly as written in `docs/parity-matrix/data/<slug>.json`.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| cryptotokenkit | Smart card token driver and token | partial | springboard/ui/src/token_objects.rs |
| cryptotokenkit | Smart card authentication configuration | partial | springboard/ui/src/token_registration.rs |
| cryptotokenkit | Authenticating users with a cryptographic token | partial | springboard/ui/src/token_registration.rs |
| cryptotokenkit | Persistent tokens (HSM) | partial | springboard/ui/src/token_registration.rs |
| cryptotokenkit | Using cryptographic assets stored on a smart card | partial | springboard/ui/src/token_objects.rs |
| security-interface | Certificate panel | partial | springboard/ui/src/certificate_panel.rs |
| security-interface | Choose identity panel and table cell | partial | springboard/ui/src/identity_chooser.rs |

## Not closed, and why

- No smart card reader, PC/SC or driver process is opened. Slot and token presence still come from the
  existing `smartcard_slot.rs` and `token_watch.rs` models, which are not wired to hardware.
- Keys stay on the card in the model's terms only: `token_objects.rs` holds no key material and performs no
  signing or decryption. The actual cryptographic operation needs a driver and is not built.
- The "table cell" half of the identity row is a view and is not modelled.
- Persistence is a flag on the registration. Nothing is written to disk, so a restart loses registrations.
- CryptoKit, GSS, StoreKit JWS, Core ML, CloudKit and Archive rows in this theme were left alone: they need
  real cryptography or services that cannot be written honestly in std-only Rust in this wave.

## Checks run

- `python3 tools/check-names.py`: exit 0 (no platform or product names in code).
- Hand-worked test expectations per module (no build run, per the wave rule): 7 in `token_objects.rs`,
  7 in `token_registration.rs`, 5 in `certificate_panel.rs`, 7 in `identity_chooser.rs`.
