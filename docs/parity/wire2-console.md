# Wire 2: Console app (security events, code requirements)

Branch `wire2/console`. App `springboard/apps/console-desktop` (Console 1.1.0 becomes 1.2.0). Only that folder changed,
plus this note. Not built: the lead builds once (`cargo test -p bifrost-console` from `springboard/`).

## What is now wired

| Matrix slug | Feature (as in data/<slug>.json) | BifrOSt path | Status after this wire |
|---|---|---|---|
| `endpoint-security` | Socket, pseudoterminal, clock and kernel events | `springboard/core/src/security_event.rs` (`Kind`, `Event`, `Subscription`, `Kind::ALL`), adapter `apps/console-desktop/src/security.rs` | Partial: the Console lists the 13 valid (kind, phase) pairs as a source, each row with its category, its phase (waits for a verdict or reported after), and whether the default subscription wants it. Search filters the list. No event is delivered: there is no kernel feed, so the list is what can be subscribed to, not a log of what happened. |
| `lightweightcoderequirements` | Checking code requirements for running processes | `springboard/core/src/codereq.rs` (`Identity`, `Requirement`, `Test`, `check_on_disk`, `check_process`, `hash_hex`), adapter `apps/console-desktop/src/identity.rs` | Partial: the detail pane of a live message or a signpost checks its process's executable against two requirements (unsigned code; signed by the platform). The executable is read from `/proc/<pid>/exe` and hashed. Nothing enforces the result. |

Pure models reused, not rewritten: `security_event::{Kind, Phase, Category, Event, Subscription}`,
`codereq::{Identity, Requirement, Test, check_on_disk, check_process, hash_hex}`, `sha256` through `codereq::hash_hex`.
The adapters only format rows and lines.

Not wired (no source data or no signing): the authorization path (`Phase::Authorize` verdicts), the other
endpoint-security categories (process, file, mount, session), the `Constraint` and `Scope` builder (the pane builds the
two requirements directly), and the code-directory, team and signing-identifier tests (BifrOSt has no code signature to
read).

## How a user reaches it

- **Sidebar > Reports > Security Events** (info glyph, `info.circle`). The table lists the 13 events. Each row's
  Category column is the event's category, its Type column is Default for an event that waits for a verdict and Info
  for one that is reported after it happens, and its Message reads `<event> (<phase>): subscribed` or `not subscribed`.
- **Filtering**: type in the toolbar search. `category:Socket` keeps the socket rows (6); `category:Kernel` keeps the
  kernel rows (3); `not subscribed` finds the rows the subscription lacks (none by default); free text matches the
  message. The footer reads `N of 13 messages`.
- **Detail pane** of a security row: the Details section says whether the event waits for a verdict, its category,
  and whether the subscription wants it.
- The footer shows: "No event feed yet. These rows are the events BifrOSt can subscribe to."
- **Detail pane > Details** on a live message or a signpost row with a PID: `Executable: <path>`, `SHA-256: <first 16
  hex digits>…` (not computed for a program over 64 MB), `Valid signature: no, the program has no code signature`,
  `Unsigned code: satisfied`, `Signed by the platform: not satisfied`. A row whose process has gone shows no lines.

## How to verify on the test PC (Midget)

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`). Midget is the
   test machine; the VM is skipped by owner rule.
2. `bifrost pc midget open-app is.olibuijr.console.desktop`, then `bifrost pc midget tap-window APP X Y` on the
   Security Events row (sidebar, under Reports; use `bifrost pc midget state` for the window-local position), then
   `bifrost pc midget screen /tmp/console-security.png`.
3. Expect 13 rows, the footer `13 messages` and the no-feed note. Type `category:Socket` in the search (tap the search
   field first): 6 rows and `6 of 13 messages`.
4. Select a Security row: the pane's Details say "Waits for a verdict..." for an Authorize row, "Reports the event after
   it happens..." for a Notify row.
5. Back to This Mac (Live), select a message from an app that has a PID (any row from `springboard` or `bifrost-notes`):
   the Details section shows the Executable line, the SHA-256 line, the signature line and the two requirement results.

## Host tests

Unit tests in the app (run with `cargo test -p bifrost-console` from `springboard/` when the lead builds once):
- `src/security.rs`: 13 valid events, the default subscription wants every row, a partial subscription marks the rows it
  lacks, categories and levels of rows, the detail text, the source keeps the newest rows within the limit.
- `src/identity.rs`: SHA-256 of `abc` is the standard vector, the hash is shortened to 16 digits, an unsigned program's
  lines (signature no, unsigned satisfied, platform not satisfied), the too-large hash line, no detail for pid 0 or a
  missing pid, and the test process has an executable detail.
- `src/wire_tests.rs`: the Security Events view (13 rows, the footer and the note) and the `category:Socket` filter (6 rows).

Note for the lead:
- `tests/compliance.rs` snapshots the window. The new sidebar row changes the golden; re-bless with `BIFROST_BLESS=1`
  after the first build and check the snapshot diff is only the new row.
- `springboard/Cargo.lock` pins `bifrost-console` at 1.1.0. Building without `--locked` updates it to 1.2.0; the lock
  update belongs in the release commit.

## REA evidence

- `ev_c46a2b208efdf203d586970ea89451b6c555dc1d1e11b450ed704df58606c996`: Endpoint Security page
  (`documentation/endpointsecurity`), read with `tools/parity/rea-page.sh`. The DocC collection page (`docc.py
  endpointsecurity`) lists Client, Message and Event Types.
- `ev_afbd58b69300a8ecc8f167a86953f47055415b85277f10e314c49b3be48bb222`: LightweightCodeRequirements page
  (`documentation/lightweightcoderequirements`), read with `tools/parity/rea-page.sh`. The DocC collection page (`docc.py
  lightweightcoderequirements`) lists "Checking code requirements for running processes", with `allOf(requirement:)`,
  `anyOf(requirement:)`, `ProcessCodeRequirement` and `SecTaskValidateForRequirement(task:requirement:)`. The rendered page
  says a validation result carries whether the code has a valid signature and whether it satisfies the requirement, which
  is what the pane shows.

## Design notes

- The Security Events row reuses the kit's source list, table, footer and detail pane. The rows use the existing Level
  labels (Default, Info). No new colours, type sizes or radii.
- Rows carry no date (0 renders as an empty time and "Unknown" in the pane), since no event has happened.
- The code requirement lines are text in the existing Details section of the pane, not a new panel.
- The pinned testing rows (`BIFROST_TEST_PIN=1`) get no code requirement lines, so the pinned snapshot does not depend on
  the test process's PIDs.

## Remaining gaps

- No event feed: nothing publishes security events to the Console, so the list is the subscription's contents. The
  authorization path (allow or deny before an event) is not modelled as a feed.
- Subscriptions are fixed to "every category" at startup. Nothing lets the user change them from the window.
- Code requirements are checked for the Journal's processes only (live and signposts), not for the crash reports, whose
  process has exited.
- The Constraint builder (`codereq::Constraint`, `Scope`) and the signing tests (team, signing identifier, code
  directory hash, flags, validation category, entitlements) are not wired. There is no signature to test.
- Apps still get no SDK API for code requirements.

## Matrix rows touched

| slug | feature (as in data/<slug>.json) | BifrOSt path |
|---|---|---|
| `endpoint-security` | Socket, pseudoterminal, clock and kernel events | `springboard/core/src/security_event.rs`; app `springboard/apps/console-desktop/src/security.rs` |
| `lightweightcoderequirements` | Checking code requirements for running processes | `springboard/core/src/codereq.rs`; app `springboard/apps/console-desktop/src/identity.rs` |
