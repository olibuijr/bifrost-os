# Wave 4: Exception Handling (failure monitoring and crash records)

Framework: Exception Handling (`docs/parity-matrix/data/exception-handling.json`, official collection
https://developer.apple.com/documentation/exceptionhandling). This wave builds the monitoring model that the
framework's one class describes, and an app-facing reader for the crash records the shell keeps. It does not wire
the model into the shell's panic hook.

## What was built

- `springboard/sdk/src/exception_handler.rs` (new, one `pub mod` line in `springboard/sdk/src/lib.rs`):
  - `FailureKind` (Uncaught, UncaughtSystem, UncaughtRuntime, TopLevel, Other) with the bit layout: a log bit and a
    handle bit per kind in the handling mask, and one hang bit per kind in the hanging mask.
  - `handling` and `hanging` modules with the named bit constants and `ALL` masks. Bits outside `ALL` are dropped
    by `set_handling_mask` and `set_hanging_mask`.
  - `Failure` (kind, message, optional `file:line:col`) and `Decision` (`handle`, `log`, `halt`).
  - `ExceptionHandlerDelegate` trait (`should_handle`, `should_log`, both defaulting to yes). The handler asks the
    delegate only about failures whose bit is set in the mask, and passes the handling bits of that kind.
  - `ExceptionHandler`: masks, delegate accessors, and `decide(&Failure) -> Decision`. Without a delegate the mask
    decides alone. `halt` follows the hanging mask only; the handler never stops a thread or a process.
  - `default_handler()`: a process-wide `Mutex<ExceptionHandler>` created once with `OnceLock`.
  - `crash_records(dir)`: reads crash.log and crash.log.old through `springboard::shell::diag::load`, so the format
    is parsed by the shell's own parser and not copied.
  - Nine tests with values worked out by hand from the rules above (bit positions, mask filtering, delegate
    refusal, delegate receiving only its kind's bits, halt policy, the singleton, and a crash.log parse with
    `signature` "src/a.rs:3 boom").

## Official API names mirrored

- `NSExceptionHandler`: `default()`, `exceptionHandlingMask()`, `setExceptionHandlingMask(_:)`,
  `exceptionHangingMask()`, `setExceptionHangingMask(_:)`, `delegate()`, `setDelegate(_:)`.
- `NSExceptionHandlerDelegate`: `exceptionHandler(_:shouldHandle:mask:)`, `exceptionHandler(_:shouldLogException:mask:)`.
- Logging and Handling Constants: the ten log and handle masks for uncaught, uncaught system, uncaught runtime error,
  top-level and other exceptions. These map to `handling::LOG_UNCAUGHT` ... `handling::HANDLE_OTHER`.
- System Hang Constants: the five hang masks, mapped to `hanging::UNCAUGHT` ... `hanging::OTHER`.
- Sources: DocC `python3 -I tools/parity/docc.py exceptionhandling --depth 2`,
  `exceptionhandling/nsexceptionhandler --depth 2`, `exceptionhandling/exceptionhandling-enumerations --depth 2`.
  The paths that wave 3 reported as 404 are reachable now (`exceptionhandling/...`).

Differences from the official API, all deliberate:

- Rust has no exception objects. The caller classifies a panic or fault into a `FailureKind`.
- There is no singleton that installs a hook. The shell's panic hook (`springboard/src/bare/crashlog.rs`) is owned by
  the shell, and the SDK depends on the shell crate, so the shell cannot call the SDK back. The handler is a model
  an app or a shell service drives.
- `halt` is only a policy flag. Nothing in BifrOSt suspends execution for a debugger.
- The default mask is empty. The official pages do not state one.
- The `NSStackTraceKey` and the global string constants are Objective-C runtime concepts and stay n/a.

## REA evidence

- `ev_f1687a3b6d76d407a9d331f466d13df3d02190b035d669f0b400ce094c77ad06`: Exception Handling landing page
  (`https://developer.apple.com/documentation/exceptionhandling`).
- `ev_11b8eb72f05c2683a31a8bcc770bc8d581bac7c151f378ccc0353c77703c9dd2`: NSExceptionHandler class page (mask and
  delegate accessors, the "installs its uncaught exception handler" note).
- `ev_222e51e81ac8c4934e647cc0125c4b6de9ebfb28b593e01d59f2a60d1b7b2e5b`: NSExceptionHandlerDelegate protocol page
  (the two approval methods).

## Matrix rows closed

Framework slug `exception-handling`. Status `partial` for each.

| Framework slug | Feature (as in data file) | Status | BifrOSt path |
|---|---|---|---|
| exception-handling | Exception handling mask (which exceptions are monitored) | partial | springboard/sdk/src/exception_handler.rs |
| exception-handling | Halt-for-debugger mask (system hang constants) | partial | springboard/sdk/src/exception_handler.rs |
| exception-handling | Delegate approval per exception | partial | springboard/sdk/src/exception_handler.rs |
| exception-handling | Default handler singleton and delegate accessors | partial | springboard/sdk/src/exception_handler.rs |
| exception-handling | Crash record surfaced to the user and developer | partial | springboard/sdk/src/exception_handler.rs |

Not closed: "Uncaught exception capture and logging" (os). The shell's panic hook still writes crash.log without
asking any handler, so the capture itself stays a system capability.

Why partial: the model decides but does not run. Nothing calls `decide` on a real panic, `halt` is not enforced,
and an app can read crash records only from a directory it is given. The shell does not grant its crash log
directory to apps.

## Not built

- Wiring `default_handler()` into `springboard/src/bare/crashlog.rs`. That needs a shell-side call site and a
  decision on what "handle" ends (the thread or the process). It is the next step for the uncaught-capture row.
- A debugger halt. BifrOSt has no debugger hook on panic; coredumps come from systemd-coredump.

## Verification

- `python3 tools/check-names.py` exits 0 after the change.
- No cargo build or test run was made in this wave, by instruction. The tests in `exception_handler.rs` have not
  been executed.
