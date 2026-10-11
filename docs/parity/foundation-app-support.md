# Foundation (App Support): parity notes

Framework: Foundation (App Support), https://developer.apple.com/documentation/foundation (macOS 10.0 and later).
Status after this round: `partial` (unchanged). Two of the six gaps now have a pure Rust model with tests.

## What was built

1. **Progress tree**: `springboard/ui/src/progress.rs` (`bifrost_ui::progress`, reached from apps as
   `bifrost_app::controls::progress`).
   - Mirrors `Progress`: `init(totalUnitCount:)` (`ProgressTree::root`), `init(totalUnitCount:parent:pendingUnitCount:)`
     (`ProgressTree::child`), `totalUnitCount`, `completedUnitCount`, `fractionCompleted`, `isFinished`,
     `isIndeterminate`, `isCancellable` and `cancel()`, `isPausable`, `isPaused`, `pause()` and `resume()`,
     `setUserInfoObject(_:forKey:)` and `userInfo` (text values only).
   - A child reports `pending` units of its parent. The parent's fraction is its own completed units plus each child's
     fraction times its share, over its total, clamped to 0..=1.
   - Differences from Foundation: `finish(id)` completes indeterminate work (Foundation sets completed to total).
     `cancel` and `pause` refuse finished work. Cancelling does not reach the children.
   - Not modelled: `ProgressManager`, `publish()`/`unpublish()`, file-operation fields, `estimatedTimeRemaining`,
     `throughput`, value observation (KVO), `localizedDescription` text.

2. **Error with domain and code**: `springboard/sdk/src/error.rs` (`bifrost_app::error::AppError`).
   - Mirrors `NSError`: `init(domain:code:userInfo:)` (`AppError::new` and `with_user_info`), `domain`, `code`, `userInfo`,
     `localizedDescription` (generic sentence when none is set), `localizedFailureReason`,
     `localizedRecoverySuggestion`, `localizedRecoveryOptions`, `helpAnchor`, `underlyingErrors`, and the standard
     `Error::source` chain (the first underlying error).
   - `From<URLError>` maps each URL failure to domain `bifrost.url` with codes 1 to 7 in declaration order. These codes
     are ours, not Foundation's numbers.
   - Not modelled: per-domain user info value providers (`setUserInfoValueProvider(forDomain:provider:)`), the recovery
     attempter (`recoveryAttempter`, `NSErrorRecoveryAttempting`), and the error domains and codes of the system.

## Official API names mirrored

- Foundation: `Progress` (Creating Progress Objects, Reporting Progress, Observing Progress, Controlling Progress,
  Inspecting Progress Information), `NSError` (Creating Error Objects, Getting Error Properties, Getting a Localized Error
  Description, Displaying a Help Anchor, Getting the Error Recovery Attempter, Instance Properties).
- Official pages were read with `python3 -I tools/parity/docc.py foundation/progress` and `foundation/nserror`.

## REA evidence

- `ev_e2ff5d55d77a4e9024c3fe26224756e2a348cab8ef8b4488a19a55f20b160e51`: rendered page, Progress.
- `ev_15a72d7495a8d097e8b6e4d6d85a93832977186208cea4b1ce8576b64804c164`: rendered page, NSError.
- Earlier framework evidence (landing page): `ev_63b0f1e52f23b2c8ab863af9b25e2447a76fb1b6f5bf6197613cd873163f9f3e`.

## Tests

- `progress.rs`: 7 tests (fraction, child shares, indeterminate work, clamping, cancel and pause, user info).
- `error.rs`: 5 tests (default and explicit description, URL mapping, recovery text, user info order and source chain).
- Tests were written but not run in this round (no builds in this stage).

## Remaining gaps

- ProgressManager and publishing progress to other processes; value observation of progress for views.
- Cancelling a progress object does not reach its children.
- Per-domain error user info providers and a recovery attempter that runs the chosen option.
- NSUserActivity Handoff between devices.
- Generic app extension host and NSItemProvider.
- Scripting dictionaries and an AppleScript-like automation layer (control socket RPC covers the agent case).
- NotificationQueue coalescing and typed notification payloads (user info is still text only).
