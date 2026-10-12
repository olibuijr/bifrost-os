# Wave 4, theme-08: file metadata search and statistical summaries

Theme: "File metadata search (MDQuery, kMDItem*, MDLabel)", size 25 rows. This wave closes two gap rows, both
as partial. Both pieces are pure models with host unit tests. Neither is wired into apps or the shell, and neither
has been built or run on the VM or Midget, as the wave rules require.

## What was built

1. **Attribute predicates for file metadata** (`springboard/ui/src/metaquery.rs`, extended). A record can now carry
   named, typed attributes (`Value`: text, number, date, flag, list). The module adds:
   - `keys` constants: `display-name`, `content-type`, `keywords`, `tags`.
   - Predicates: `HasAttribute`, `AttributeEquals` (text ignores case, other types compare exactly),
     `AttributeContains` (substring for text, whole-item match for a list), `AttributeAtLeast` and `AttributeAtMost`
     (numbers and dates, inclusive). A missing attribute, or one of another type, does not match.
   - `Metadata::with_attribute` and `Metadata::attribute`.
   The existing name, extension, size, time, all/any/not, sort and limit parts are unchanged.
2. **Statistical summaries for a table column** (`springboard/ui/src/column_summary.rs`, new, one `pub mod` line in
   `springboard/ui/src/lib.rs`):
   - `NumericSummary::new(&[Option<f64>])`: some and none counts, total count, mean, sample standard deviation,
     min, max, median, first and third quartile. NaN counts as missing.
   - `CategoricalSummary::new(&[Option<T>])`: some and none counts, total count, unique count, and the mode (all
     tied values, ascending).

## Official API names mirrored

- Tabular Data, Statistical Summaries: `NumericSummary` (`someCount`, `noneCount`, `totalCount`, `mean`,
  `standardDeviation`, `min`, `max`, `median`, `firstQuartile`, `thirdQuartile`), `CategoricalSummary`
  (`someCount`, `noneCount`, `uniqueCount`, `mode`, `totalCount`). Source: `docs/parity-matrix` row "Statistical
  summaries". DocC: `python3 -I tools/parity/docc.py tabulardata`, section "Statistical Summaries".
- Core Services, File Metadata: `MDQuery` and the `kMDItem*` attribute constants, read here as predicates over typed
  attributes. The official page lists the constant families (for example `kMDItemKeywords` and the label keys), and the file-metadata collection covers `MDLabel` as a class.
  DocC: `python3 -I tools/parity/docc.py coreservices/file_metadata`, `coreservices/mdquery`.

The official pages do not state the divisor of the numeric standard deviation or the quartile interpolation. The
module records its choice (sample deviation, nearest-rank quartiles, median as the mean of the middle pair) in its
own docs.

## REA evidence (rendered official pages, captured with `tools/parity/rea-page.sh`)

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/tabulardata/numericsummary | ev_75e48bb7763564d7ff38405b3e357644e3b03d9448f9e11ae6b2e7f01ac1649e |
| https://developer.apple.com/documentation/tabulardata/categoricalsummary | ev_96ec69eed22c685b27c857a87346df12e34ccede2299320fe0e4977cb3ad7346 |
| https://developer.apple.com/documentation/coreservices/file_metadata | ev_9c66d5380162c0f1197cee682f49adb2f66d8e93f08a74b26c786be76f3693d9 |
| https://developer.apple.com/documentation/coreservices/mdquery | ev_8cab77ab3369ddd291c5731093a38deb94c9c35135d610119fc15eac688bc2a5 |

## Matrix rows closed (status partial)

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | Status | BifrOSt path |
|---|---|---|---|
| core-services | File metadata search (MDQuery, kMDItem*, MDLabel) | partial | springboard/ui/src/metaquery.rs |
| tabular-data | Statistical summaries | partial | springboard/ui/src/column_summary.rs |

## Gaps that remain

- File metadata: no importers that fill attributes from file content, no label colours or label sets (`MDLabel`),
  no live updates while a query runs, no query over the whole file system (the caller supplies the records), no
  sort by attribute.
- Statistical summaries: no type-erased summary (`AnyCategoricalSummary`), no data frame or typed column to read
  from, and the statistics are not yet wired to any app.
- Other theme-08 rows (Spotlight indexing, cloud search, thumbnails, Visual Look Up and so on) are not touched here.
