# Security Interface, wave 3

Official framework: https://developer.apple.com/documentation/securityinterface (macOS 10.3). The framework is a set of
panels and views for certificates, identities, keychain settings and authorization. This wave builds the three
keychain and certificate panels that have no UI in BifrOSt, as pure models in `bifrost-ui`. Each model holds the state
and the rules; the shell draws it with the Golden Gate kit later. None of the three models reads a store, a clock or a
certificate file.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/keychain_save.rs` | `SFKeychainSavePanel` (`setPassword(_:)`, `runModal`/`beginSheet` create flow, `error()`, `keychain()`) | Form state for creating a keychain: name (trimmed, 1 to 64 bytes, no slash, colon or control character), password (8 to 1024 bytes) and its confirmation. `save()` returns the outcome; `error()` gives the last rejection, `keychain()` the last created name. A successful save clears both password fields. |
| `springboard/ui/src/keychain_settings.rs` | `SFKeychainSettingsPanel` (`runModal(for:keychain:)`, `beginSheet(... settings:keychain:)`) | Two settings, lock on sleep and idle lock after 1 to 1440 minutes (or never). Edits stay pending until `commit()`; `revert()` drops them; `is_dirty()` reports the difference. |
| `springboard/ui/src/certificate_trust.rs` | `SFCertificateTrustPanel` (`runModal(for:message:)`, `beginSheet(... trust:message:)`, `informativeText()`, `setInformativeText(_:)`) | A chain of 1 to 16 certificates, leaf first. Each entry has the system's default trust (supplied by the caller) and the user's choice: use default, always trust or never trust. `chain_trusted()` is true when every entry is effectively trusted. The panel does not evaluate signatures. |

Each module is declared with one `pub mod` line in `springboard/ui/src/lib.rs`. Each has a `#[cfg(test)] mod tests`
with hand-checked behaviour tests (8 in the save panel, 7 in the settings panel, 8 in the trust panel).

## Limits

- No shell panel or sheet draws these models yet. A row becomes `parity` only when the shell shows them and a
  Midget screenshot confirms it, as the parity rules require. Until then the rows are `partial`.
- Certificate panel, certificate view and the authorization lock view are not closed here. They need certificate
  parsing (DER, X.509) and an SDK surface, and are left as gaps.
- The keychain itself stores only generic passwords (`springboard/sdk/src/keychain.rs`). The save panel creates a
  keychain name and password in this model; nothing yet writes a new keychain store from it.
- Trust evaluation stays with the caller. There is no certificate chain builder in this wave.

## REA evidence

| Page | evidence_id |
|---|---|
| Security Interface landing page | `ev_002e7e8036c28adfe13bd4cb955689883ec165c5dcb7f94e5b8b836a60efe5b9` |
| SFKeychainSavePanel | `ev_e612611d444db5f8f598eadfea2c72c11058486c109fed803d9bd83db0e80b65` |
| SFKeychainSettingsPanel | `ev_e4df5502a4a4a2a73b040f9bbc7f9576a8fd2fcb55c4d89a0e664ff2166daf71` |
| SFCertificateTrustPanel | `ev_d147d189a649586e7331eb0a5c53d975dc47b2407512e3fadc57e1d61af3345a` |

Official DocC was read with `python3 -I tools/parity/docc.py securityinterface`, `securityinterface/sfkeychainsavepanel`,
`securityinterface/sfkeychainsettingspanel` and `securityinterface/sfcertificatetrustpanel`.

## Matrix rows closed

Status moves from `gap` to `partial` for each row. Framework slug: `security-interface`
(`docs/parity-matrix/data/security-interface.json`).

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| security-interface | Keychain save panel | partial | springboard/ui/src/keychain_save.rs |
| security-interface | Keychain settings panel | partial | springboard/ui/src/keychain_settings.rs |
| security-interface | Certificate trust editor | partial | springboard/ui/src/certificate_trust.rs |

## Not verified

No build or test was run in this wave, as the wave rules require. The integration build and the test run happen at the
end of the parity integration. The code was checked by hand against the Rust rules it uses.
