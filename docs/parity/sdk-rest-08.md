# Parity sdk-rest-08: failure capture, sessions, schedulers, WebDriver wire model

Covers the rows Uncaught exception capture and logging, Unclean-exit detection at next start (exception handling),
Schedulers (the publisher framework) and WebDriver and automated testing (the web view framework). The developer test
rig (hypervisor) is unchanged: it is host tooling for an Arch Linux guest, not an app surface.

## What it mirrors

- Exception handling: the monitored-failure decision of the reference handler, now with a capture step that writes the
  failure to a log file. The unclean-exit mark is the start and exit record the shell already keeps.
- Publisher schedulers: the scheduler contract (run now, run after a delay within a tolerance) with an immediate
  scheduler and the dispatch queues.
- Web view automation: the W3C WebDriver wire format for error codes, locators, timeouts and element references.

## What is wired

| Piece | Where | Tests |
|---|---|---|
| Failure capture into crash.log | `springboard/sdk/src/exception_handler.rs` (`ExceptionHandler::capture`, `Failure::log_line`) | `sdk/tests/parity_sdk-rest-08.rs` |
| App session marks | `exception_handler.rs` (`start_session`, `end_session`) over `springboard/src/bare/crashlog.rs` | `springboard/tests/parity_sdk-rest-08.rs`, `sdk/tests/parity_sdk-rest-08.rs` |
| Fallible log writes | `springboard/src/bare/crashlog.rs` (`try_append`, `try_write_last_exit`) | `springboard/tests/parity_sdk-rest-08.rs` |
| Scheduler trait | `springboard/sdk/src/schedulers.rs` | `sdk/tests/parity_sdk-rest-08.rs` |
| WebDriver wire model | `springboard/sdk/src/webdriver.rs` | `sdk/tests/parity_sdk-rest-08.rs` |

The shell's process panic hook is unchanged: `crashlog::install` still captures panics for the whole process.

## What needs the lab machine

- Checking that a crash written by an app appears in the shell's diagnostics (the shell reads only its own directory).
- Starting a real page under automation: there is no HTTP endpoint in the browser shell yet, and `navigator.webdriver`
  stays a fixed value.
- Confirming the unclean-exit note after a real power loss or kill on the device.

## Not done

- No `receive(on:)` or `subscribe(on:)` operator. Publishers are single-threaded.
- No app-facing panic hook. Installing one from an app would duplicate the shell's hook.
