# Wave 5: CryptoTokenKit parity (pure Rust)

Branch `parity/w5-cryptotokenkit`, on top of the wave 4 integration branch. Adds two pure models to `bifrost-ui`
and builds on the wave 3 token layer (`springboard/ui/src/token_watch.rs`: `TokenWatcher`, `TokenSession`,
`TokenError`). Nothing is wired into apps or the shell, and nothing has been built or run on the VM or Midget.
Tests were written by hand against the rules; they have not been executed.

## What was built

- `springboard/ui/src/token_objects.rs`: objects a token stores (keys, certificates, identities), kept as
  descriptions in `TokenObjectStore`. `list` returns descriptions without authentication. `find` returns an object
  only to a session of the same token, and a protected object needs an authenticated session. No key material is
  held.
- `springboard/ui/src/token_registry.rs`: `TokenRegistry`, the registered smart card token IDs (register and
  unregister, sorted, unique) and an `AuthPolicy` (`Optional` or `Required`). `login_allowed` decides whether a
  login may proceed: `Required` needs a registered card with an authenticated session.

Both modules are declared in `springboard/ui/src/lib.rs` (one `pub mod` line each).

## Official API names mirrored

- `TKTokenKeychainItem` (objectID, label, constraints; https://developer.apple.com/documentation/cryptotokenkit/tktokenkeychainitem)
  as `TokenObject` and `TokenObjectStore`. Constraints are reduced to a single `requires_auth` flag.
- `TKSmartCardTokenRegistrationManager` (`registeredSmartCardTokens`, `registerSmartCard(tokenID:)`,
  `unregisterSmartCard(tokenID:)`; https://developer.apple.com/documentation/cryptotokenkit/tksmartcardtokenregistrationmanager)
  as `TokenRegistry::registered`, `register`, `unregister`.
- Article "Authenticating Users with a Cryptographic Token" and "Configuring Smart Card Authentication", reduced to
  the login policy check in `TokenRegistry::login_allowed` and `AuthPolicy`.

## REA evidence

- `ev_33f37903128c46ba8bfd0b25da1ca6199c14579c4f19d2d1f01b7a80d41c3421`: rendered CryptoTokenKit landing page
  (https://developer.apple.com/documentation/cryptotokenkit). The page states that persistent tokens "aren't
  suitable for validating a user login", which is why the login policy here does not model persistent tokens.

## Matrix rows closed (status partial)

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| CryptoTokenKit | Using cryptographic assets stored on a smart card | partial | springboard/ui/src/token_objects.rs |
| CryptoTokenKit | Smart card authentication configuration | partial | springboard/ui/src/token_registry.rs |
| CryptoTokenKit | Authenticating users with a cryptographic token | partial | springboard/ui/src/token_registry.rs |

## Not built

- Smart card token driver and token session (`TKSmartCardTokenDriver`, `TKSmartCardToken`) stay a gap: they need an
  app-extension mechanism that does not exist.
- Persistent tokens (HSM) and the NFC session stay gap and n/a.
- No cryptographic operation is performed. The driver checks the PIN and signs; the model only records the outcome.
- The login flow is not connected to `springboard/src/shell/login.rs`.
