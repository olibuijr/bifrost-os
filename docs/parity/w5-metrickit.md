# Wave 5: MetricKit gap rows (network and disk write metrics)

Branch `parity/w5-metrickit`, started from `parity/integration` (wave 4, 236401fa).

## What was built

Two pure models in `springboard/core` (std only, no hardware, no clock, no network). Both take the caller's figures
and keep per-app totals; neither is wired into the shell or the SDK yet.

- `springboard/core/src/app_transfer.rs`: per-app upload and download byte totals split by link (Wi-Fi or
  cellular). `Ledger::record` adds bytes (zero bytes create no entry, totals saturate), `totals`, `apps`,
  `link_sum`, `take`. `ALL_METRICS` lists the four (link, direction) totals in the order the official report lists
  them.
- `springboard/core/src/app_disk_writes.rs`: per-app running logical write totals (`DiskLedger::record_write`),
  a `Storage` snapshot (file count, binary, cache and clone sizes, `size_total`), and `check_window`, which reports a
  write exception when the bytes written inside a window are strictly above a limit. A zero limit means no limit. A
  counter that went backwards reads as zero bytes.

`springboard/core/src/lib.rs` gained one `pub mod` line per module.

The models do not measure anything themselves. Nothing in the shell or the network daemon feeds them, and the
network daemon still does not attribute bytes to apps.

## Official API names mirrored

Official pages (developer.apple.com, DocC JSON via `tools/parity/docc.py`):

- `TotalWiFiUploadMetric`, `TotalWiFiDownloadMetric`, `TotalCellularUploadMetric`, `TotalCellularDownloadMetric`
  (each a `Measurement<UnitInformationStorage>` `value`, reported in interval entries only).
- `LogicalDiskWritesMetric` (the total data written to disk or other long-term storage, interval entries only).
- `DiskWriteExceptionDiagnostic` (`totalBytesWritten` over the exception period). Its call-stack field is not
  modelled: there is no stack capture behind it.
- `TotalFileSizeMetric` (`binaryFileSize`, `cacheFolderSize`, `cloneSize`) and `TotalFileCountMetric`.

The official framework page documents that reports cover the previous 24 hours and are delivered at most once a day,
through `MetricManager`. Delivery to the app is not part of this wave.

Apple does not publish a numeric disk-write exception limit on the pages read. `check_window` takes the limit from the
caller and does not supply one.

## REA evidence

- MetricKit landing page: `ev_689243ca9a9298770f376831ffac29c2c2b3a889e6113bd803b430ebfa3043ab`
- TotalWiFiUploadMetric: `ev_25e40257470d8b1a897a40ebaf57805db83f6f873c0596af7beccea260af1607`
- LogicalDiskWritesMetric: `ev_a50739b7c0a423ca8f9aa904f049a9f5fc7b6570868d127e55fbe98d4a552f74`

## Matrix rows closed

These rows go to `new status partial` in the matrix (the lead updates `docs/parity-matrix/` centrally). Each module
has host unit tests and is not yet wired into the shell or apps.

| Framework slug | Feature (as written in `docs/parity-matrix/data/metrickit.json`) | New status | BifrOSt path |
|---|---|---|---|
| metrickit | Network data metrics (Wi-Fi and cellular upload and download) | partial | springboard/core/src/app_transfer.rs |
| metrickit | Disk write and storage metrics (LogicalDiskWritesMetric, TotalFileSizeMetric, DiskWriteExceptionDiagnostic) | partial | springboard/core/src/app_disk_writes.rs |

## Not done

- No build and no test run (the task forbids them). The tests are written from hand-worked values and are not yet
  compiled or run.
- No SDK type gives an app its own totals, and no daily report is produced from them (the report path is
  `springboard/sdk/src/daily_metric_report.rs` from wave 4).
- The network daemon and the cgroup layer in `springboard/src/bare/app_manager.rs` do not call these models.
