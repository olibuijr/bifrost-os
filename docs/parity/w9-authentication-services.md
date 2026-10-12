# Authentication Services: credential import and export (wave 9)

Row: "Credential import and export" (gap, Apple: ASCredentialExportManager, ASCredentialImportManager, CXF).

## What exists now

- Model: `springboard/ui/src/credential_exchange.rs`, shared by the shell and apps. The SDK re-export is
  `springboard/sdk/src/credential_exchange.rs` (`bifrost_app::credential_exchange`).
- Format: a header line `bifrost-credential-exchange 1`, then one password record per line with three tab-separated,
  escaped fields (service, user name, password). A newer version number is refused. Passkey private keys are never exported.
- `export(records)` writes a document. `import(text)` returns an `ImportReport` (records in document order plus a duplicate
  count) or an `ExchangeError` that names the bad line. A bad document is refused as a whole.
- Duplicates (same service and user name) replace the earlier password, so the last copy in the document wins.

## Decisions

- The format is BifrOSt's own, not Apple's CXF. CXF is not public enough to match byte for byte, so the API shape is kept
  (export and import of password records, a version check, refusal on errors) but the document is not interoperable with
  Apple's files. This is the reason the row stays partial.
- The model does no file I/O. Saving to a file or using the share sheet is the caller's job.

## Not done

- No import or export of passkeys, notes or payment cards.
- No CXF reader or writer, so Apple-exported files cannot be imported.
- No UI (no import panel in Passwords and no export menu item). Not tested on Midget.

## Status

Proposed status: partial (pure model with host unit tests, not wired into an app or the shell). Parity needs a test on Midget.
