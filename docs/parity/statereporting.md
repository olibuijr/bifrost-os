# StateReporting parity

Apple's StateReporting framework lets an app name a domain, report one active state per domain and attach
metadata, so that crash and metric reports can be grouped by app state. This note records the pure-Rust models
BifrOSt now has for the reporting side and the signposts that complement it. Matrix data: `docs/parity-matrix/data/statereporting.json`.

## What was built

- `springboard/ui/src/state_report.rs` (`bifrost_ui::state_report`): the reporting model.
  - `StateReporter`: one per domain. `new(domain)` checks the reverse-DNS name and returns None if it is invalid.
    `report_transition(to, stable, volatile)` is a no-op when the label and stable metadata are unchanged; otherwise
    it queues a transition and replaces the volatile metadata. `report_volatile_update(volatile)` changes only the
    volatile metadata and queues a volatile report. `drain()` takes the queue.
  - `ReportableMetadata` trait with `metadata_dictionary()`, and `MetadataValue` (`Date`, `FloatingPoint`,
    `Integer`, `String`). Plain strings, integers and floats convert with `From`.
- `springboard/core/src/signpost.rs` (`bifrost_core::signpost`, re-exported by the SDK log module as `bifrost_app::log::signpost`):
  - `Signposter`: named intervals and events with an explicit time argument. `begin_interval` returns an
    `IntervalState`; `end_interval` checks the name, records the interval and returns its length; `event` records a point.
    `make_id` gives ids unique within the signposter. `disabled()` emits nothing. `drain()` takes the records.

The caller supplies times, so the models have no clock and no I/O. Nothing sends a report anywhere yet.

## Official API names mirrored

- `StateReporter`, `StateReporter.domain`, `reportTransition(to:stableMetadata:volatileMetadata:)`,
  `reportVolatileMetadataUpdate(_:)`, `reporter(for:stableMetadata:volatileMetadata:)`
  (https://developer.apple.com/documentation/statereporting/statereporter). The per-domain singleton is not
  enforced: callers keep one reporter per domain.
- `ReportableMetadata` and `metadataDictionary`
  (https://developer.apple.com/documentation/statereporting/reportablemetadata).
- `ReportableMetadataValue` cases `date`, `floatingPoint`, `integer`, `string`
  (https://developer.apple.com/documentation/statereporting/reportablemetadatavalue).
- `OSSignposter`, `beginInterval(_:id:)`, `endInterval(_:_:)`, `makeSignpostID()`, `disabled`, `isEnabled`
  (https://developer.apple.com/documentation/os/ossignposter).

## REA evidence

- `ev_34e0b6322dd08ba13cb8773169de052bde468b7f7ebad7416fa8c472f1e49323`: rendered StateReporter page.
- `ev_9d5163a3f50f14d882621862bd51be98bbc2418ed3c5c5a6c40ad7bb7b5935c5`: rendered ReportableMetadataValue page.
- The StateReporting landing page evidence is already in the matrix: `ev_4cfeac83cd2c0731855ab3128406c1847326babab9f3f5b2823e289fbcb188b3`.
- OSSignposter: official DocC JSON only (`tools/parity/docc.py os/ossignposter`); no REA page was captured for it.

## Not built (remaining gaps)

- No `#[derive]`-style macro for `ReportableMetadata`. Metadata is written by hand.
- No rate limiting. The official page warns that too-frequent reports can be dropped; the queue is unbounded.
- Reporter types are not checked per domain: the official fatal error for reusing a domain with other metadata types is not reproduced.
- Reports go nowhere: crash logs (`springboard/src/shell/diag.rs`) do not carry app state.
- No metric payload to the developer, the equivalent of the framework's MetricKit delivery (matrix row marked n/a).
- Signposts are not written to the unified log or to a trace file.

## Matrix rows closed

| Framework | Feature (as in `data/statereporting.json`) | New status | BifrOSt path |
|---|---|---|---|
| statereporting | StateReporter (domains and state transitions) | partial | springboard/ui/src/state_report.rs |
| statereporting | Reportable metadata (protocol, values, macros) | partial | springboard/ui/src/state_report.rs (no macro) |
| statereporting | Points of Interest and signposts for Instruments | partial | springboard/core/src/signpost.rs, springboard/sdk/src/log.rs |

Tests: `springboard/ui/src/state_report.rs` (6 tests), `springboard/core/src/signpost.rs` (6 tests). Not run: builds are out of scope for this round.
