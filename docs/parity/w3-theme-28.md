# Wave 3, theme 28: code signing validity events

Theme: "Code signing validity events" (`theme-28` in `docs/parity-matrix/themes.json`). Pure Rust, std only, no
hardware or network. Three modules close four matrix rows. None of them is wired into the shell or the SDK yet.

## What was built

- `springboard/core/src/codesign_events.rs`: `classify(current, launch)` compares a program's current identity
  (`codereq::Identity`) with the identity recorded at launch and returns `Valid` or `Invalid(Reason)`. Reasons are
  `Unsigned`, `CodeChanged` (code directory hash differs) and `IdentityChanged` (team or signing identifier differs).
  A field is compared only when the launch record has it. `Monitor::observe(pid, validity)` emits one `Event` (pid,
  reason, sequence number) when a process first becomes invalid. Invalidation is sticky until `Monitor::exit(pid)`.
- `springboard/core/src/backup_exclusion.rs`: `ExclusionSet` keeps the set of items excluded from backup, keyed by a
  lexically normalized absolute path (`normalize`: `.` and empty parts dropped, `..` resolved and stopping at the root,
  trailing slash removed). Relative and empty paths return `BackupError::NotAbsolute`. The flag applies to the exact
  item and is not inherited by children.
- `springboard/ui/src/one_time_code.rs`: `find_code(text)` returns the first run of 4 to 8 digits that stands alone and
  has one of the keywords (`code`, `verification`, `one-time`, `otp`, `kóði`, `staðfest`) within 40 bytes before it.
  It returns byte offsets, so `text[start..end]` is the code. Text without a keyword gives no code.

## Official API names mirrored

- Endpoint Security: the code-signing invalidation event type (`es_event_cs_invalidated_t`, "a type for an event that
  indicates the invalidation of a process' code signing status") and its validation categories. BifrOSt's categories
  are its own (`Reason`), because the category list was not read from the official page.
- Core Services, Backup Core: per-item backup exclusion set and query (`CSBackupSetItemExcluded`,
  `CSBackupIsItemExcluded`). The official collection describes these as low-level backup settings for files and
  folders. Whether the official query also reports inherited exclusions was not verified, so BifrOSt reports the item
  only.
- Security, one-time codes: the one-time code text content type (`textContentType oneTimeCode`), "streamline entry of
  authentication and recovery codes".
- Authentication Services: verification codes delivered to the system (`ASDeliveredVerificationCodesManager`,
  `ASVerificationCode`). BifrOSt has no message store, so `find_code` covers only the text rule.
- LightweightCodeRequirements was read for context only. Its `allOf`/`anyOf` builders and `BuildError` already exist
  in `springboard/core/src/codereq.rs` from wave 2, so this theme does not claim that row.

Official DocC fetches for per-symbol paths returned 404. The API facts above come from the collection and landing
pages and the REA-rendered pages listed below.

## REA evidence

| Page | evidence_id |
|---|---|
| https://developer.apple.com/documentation/endpointsecurity | ev_a30e69ab6274997c40997e83935574d76147daf7566bfbe764df42dc199caee7 |
| https://developer.apple.com/documentation/coreservices | ev_e172baa89f0067535716697c7aab6902b3cc85f395038baabfe6f099ea3b5a91 |
| https://developer.apple.com/documentation/security/one-time-codes | ev_f51a2a627a4aced2934fed6ec09f1999b70973e43e4118a903cf8fa3f2a43508 |
| https://developer.apple.com/documentation/authenticationservices | ev_4bb125166759d684f8f0a58fb5a8f06b0c51b1dcd905d2d181b5fbc3d4c65381 |
| https://developer.apple.com/documentation/lightweightcoderequirements | ev_cd88c774fcd97d83faeb7c27d5bde7a8333b3f04f75d2224cbff04c401920ed3 |

## Matrix rows closed

| slug | feature | status | BifrOSt path |
|---|---|---|---|
| endpoint-security | Code signing validity events | partial | springboard/core/src/codesign_events.rs |
| core-services | Backup exclusion | partial | springboard/core/src/backup_exclusion.rs |
| authentication-services | One-time passcodes | partial | springboard/ui/src/one_time_code.rs |
| security | One-time codes | partial | springboard/ui/src/one_time_code.rs |

## Gaps that remain

- No live code-signing reader. The `Identity` values come from the caller, and BifrOSt does not sign code, so
  nothing produces a real invalidation yet.
- The monitor has no feed from the kernel or from any process table, and no subscriber API for apps.
- The event reasons are BifrOSt's own. The official validation categories were not mapped one to one.
- The backup set is not read by the backup service, and there is no exclusion by path or by file type.
- The one-time code finder reads only digits. It has no alphanumeric codes, no split codes such as `123 456`, and no
  SMS or message store. No text field offers the code yet.
- `Code signing services` (Security) and the StoreKit signed-transaction row are not closed.

## Verification

Per the task, nothing was built or run. Each new module has a `#[cfg(test)] mod tests` block with 5 to 7 tests,
and the expected values were worked out by hand from the rules. `python3 tools/check-names.py` reports no hits.
