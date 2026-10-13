# Wave 3, theme 14: stream errors, crash reporting and service errors

Theme: "Stream errors and lifecycle" (themes.json `theme-14`). This wave closes three gap rows that fit pure Rust
models: the crash reporter view (binary images, symbolication, the reporter hook) and the Service Management error
cases. The stream-error and exception-mask rows stay open (see "Not built").

## What was built

- `springboard/sdk/src/service_management.rs`: `ServiceError`, the eleven error cases of the Service Management
  Errors reference, with `code()` (1 to 11 in declaration order), `from_code()`, `all()`, `Display` text from the
  reference abstracts, and `to_error()` returning an `AppError` in `ERROR_DOMAIN` (`SMAppServiceErrorDomain`).
  The reference gives no numeric values, so the numbers are BifrOSt's own. Five new tests, plus the existing one.
- `springboard/sdk/src/crash_report.rs` (new, one `pub mod` line in `springboard/sdk/src/lib.rs`):
  `BinaryImageInfo` (name, load address, size, sorted symbol table with inline chains), `ProcessCrash` (reason and
  images) with `symbolicate_address`, `symbolicate_addresses` and `symbol_address`, `SymbolicatedFrame`, and the
  `CrashReporter` trait with `process_crash_report`. Inline chains expand to one frame per name, innermost first,
  with `inline` set on every frame except the containing function. Nine tests with hand-worked values.

## Official API names mirrored

- Service Management Errors: the `kSMError*` cases (AlreadyRegistered, AuthorizationFailure, InternalFailure,
  InvalidPlist, InvalidSignature, JobMustBeEnabled, JobNotFound, JobPlistNotFound, LaunchDeniedByUser,
  ServiceUnavailable, ToolNotValid) and the error domain constant. Source: DocC
  `python3 -I tools/parity/docc.py servicemanagement --depth 2`.
- CrashedProcess: `reason`, `binaryImages`, `symbolicateAddress(_:)`, `symbolicateAddresses(_:)`,
  `symbolAddress(imageName:symbolName:)`, `SymbolicatedFrame`, `BinaryImageInfo`, and the `processCrashReport(process:)`
  hook of CrashReporterExtension. Signatures from the rendered page: `symbolicateAddress(UInt64) -> [SymbolicatedFrame]`,
  `symbolAddress(imageName: String?, symbolName: String) -> UInt64`. The BifrOSt versions return `Option` where the
  official call returns a plain value on failure, and the field layouts of `BinaryImageInfo` and `SymbolicatedFrame`
  are our own (the DocC abstracts do not give them).

## REA evidence

- `ev_da3ff92b885fb029e6b8c691cb055ded996f73cc5f3d87da559c6840280e0305`: Service Management Errors, rendered page
  (`https://developer.apple.com/documentation/servicemanagement/service-management-errors`).
- `ev_57057ec7af49369bfe1bd457612bd1140e1b6925ea69994fc668351a3e86b877`: CrashedProcess, rendered page
  (`https://developer.apple.com/documentation/crashreportextension/crashedprocess`).

## Matrix rows closed (status `partial`)

| Framework slug | Feature (as in data file) | Status | BifrOSt path |
|---|---|---|---|
| crashreportextension | Crash reporter extension point (CrashReporterExtension, processCrashReport(process:)) | partial | springboard/sdk/src/crash_report.rs |
| crashreportextension | Binary images and symbolication (binaryImages, symbolicateAddress(es), SymbolicatedFrame) | partial | springboard/sdk/src/crash_report.rs |
| service-management | Service Management error constants | partial | springboard/sdk/src/service_management.rs |

Why partial: the shell does not call an app's reporter when another app crashes (the reporter hook is only a trait
the caller drives), the crashed process's binary images are supplied by the caller and not read from a live
process, and the Service Management shell reply still carries only a status, so the shell does not yet report a
failure as a `ServiceError`.

## Not built

- ScreenCaptureKit stream errors (`stream error` rows): BifrOSt has no screen capture pipeline to report errors for.
- Exception Handling mask, halt mask and delegate approval: the official pages were not reachable through the DocC
  tool (404 on the guessed paths), so the mask constants have no verified values. Left as gaps.
- Corpse port and the Mach port hook: no equivalent on Linux.

## Verification

- `python3 tools/check-names.py` exits 0 (no Apple platform or product names in `.rs` files).
- No builds were run (the wave rules forbid it). Expected values in the tests were worked out by hand from the code.
