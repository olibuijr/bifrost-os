# Wave 4, theme-20: exception handling mask and resource diagnostics

Theme: "Exception handling mask (which exceptions are monitored)", 7 gap rows across 4 frameworks
(Exception Handling, MetricKit, Force Feedback, Service Management). Built in pure Rust, std only, no builds run.

## What was built

- `springboard/core/src/exception_policy.rs` (new, `bifrost-core`). The handling mask (log and handle bits for five
  condition kinds), the separate halt mask, a `Decision` per kind, the `Delegate` trait that approves each handle and
  log decision, a `Handler` that holds the masks and delegate, and `default_handler()`, a process-wide singleton behind
  `OnceLock<Mutex<_>>`. Nothing is monitored until a caller sets a mask. The halt bits only report a decision: nothing
  suspends a process or attaches a debugger. No hook reads the policy yet; the panic hook in
  `springboard/src/bare/crashlog.rs` is unchanged.
- `springboard/core/src/resource_exception.rs` (new, `bifrost-core`). Pure classifiers for a memory exception (peak
  above a limit) and a CPU exception (CPU time above a percentage of a wall-clock window, several cores allowed), and
  a `Diagnostic` with a crash-log style line. It takes no samples; the per-app limits in
  `springboard/src/bare/app_manager.rs` are the intended source. No call-stack tree is produced.
- `springboard/ui/src/access_control.rs` (extended). Two right names, `helper.install` and `daemon.modify`, and
  `helper_install_allowed(store, now)`, which requires both rights to be valid. The install itself is not implemented.
- `springboard/core/src/lib.rs`: one `pub mod` line for each new module.

## Official API names mirrored

- Exception Handling (https://developer.apple.com/documentation/exceptionhandling): the handler singleton
  (`default()`), the handling and hanging masks with their set and get methods, the delegate get and set, and the
  delegate decisions `shouldHandle` and `shouldLogException`. The mask constants are the logging and handling set
  (log and handle, for uncaught, uncaught system, uncaught runtime, top-level and other conditions) and the system
  hang set (halt for the same five kinds). Source: DocC `exceptionhandling`, `exceptionhandling/nsexceptionhandler`,
  `exceptionhandling/exceptionhandling-enumerations`, `logging-and-handling-constants`, `system-hang-constants`.
- MetricKit (https://developer.apple.com/documentation/metrickit): `MemoryExceptionDiagnostic` (the app terminates
  for exceeding the memory limit) and `CPUExceptionDiagnostic` (CPU time consumed in the scope of a CPU exception,
  and total sampled time). Source: DocC `metrickit/memoryexceptiondiagnostic`, `metrickit/cpuexceptiondiagnostic`.
- Service Management: the privileged helper rights are named by the authorization right names. Official page only
  described in the matrix row; no DocC fetch for this theme.

Our names differ on purpose: the owner rule forbids platform identifiers in `.rs` files, so the masks are `LOG_UNCAUGHT`,
`HANDLE_TOP_LEVEL`, `HANG_OTHER` and so on. The decision rules are our own, and the code comments say so.

## Decisions and limits

- Default masks are empty (nothing monitored). The official pages do not state Apple's defaults, so none is copied.
- A delegate is asked only about a bit that is set, and it can turn handling or logging off but never on.
  Halting is not subject to delegate approval, as the official delegate page describes approval for handling and logging only.
- Force Feedback "Hardware escape commands" is not closed. The escape commands need device access, which this theme
  rules out, so the row stays a gap.
- Nothing here is wired into the shell or an SDK API yet, so every closed row is `partial`.

## REA evidence

- Exception Handling landing page: `ev_d6f97567d1498c4642ba4ddb167942c17da0cb53c3d6a05dcbb53357302d4960`
- MetricKit landing page: `ev_45834e6d861d18dd85b0eb629337b6c7c67f4ed3cfcc4c81ad15f1da3a3a9a2d`

## Rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| exception-handling | Exception handling mask (which exceptions are monitored) | partial | springboard/core/src/exception_policy.rs (Mask, Handler::set_handling_mask, decide) |
| exception-handling | Halt-for-debugger mask (system hang constants) | partial | springboard/core/src/exception_policy.rs (HANG_* bits, Handler::set_hanging_mask) |
| exception-handling | Default handler singleton and delegate accessors | partial | springboard/core/src/exception_policy.rs (default_handler, Handler::delegate, set_delegate) |
| exception-handling | Delegate approval per exception | partial | springboard/core/src/exception_policy.rs (Delegate::should_handle, should_log) |
| metrickit | Memory and CPU exception diagnostics (MemoryExceptionDiagnostic, CPUExceptionDiagnostic) | partial | springboard/core/src/resource_exception.rs (classify_memory, classify_cpu, Diagnostic) |
| service-management | Privileged helper authorization | partial | springboard/ui/src/access_control.rs (RIGHT_HELPER_INSTALL, RIGHT_DAEMON_MODIFY, helper_install_allowed) |

Not closed: force-feedback "Hardware escape commands" (needs device access).

## Tests

Each new module has a `#[cfg(test)] mod tests` with hand-worked expectations (mask bit layout, decision table per kind,
delegate veto, halt independence, singleton round trip; memory strictly above the limit, CPU at 80 percent exactly, several
cores against 200 and 400, log line shape). `access_control.rs` gained three tests on the helper rights (both rights needed,
expiry at 160 s for a 60 s grant from 100, revocation). Not run: this wave's rule is no builds.
