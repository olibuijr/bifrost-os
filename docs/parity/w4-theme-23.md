# Parity wave 4, theme 23: web-usage reporting and web history

Theme: "Web-usage reporting (STWebpageController)", size 5, frameworks Screen Time, ScreenCaptureKit,
PermissionKit and Safari Services. This round builds the shared web-usage model and closes the three rows it can
honestly cover. It is pure Rust with std only, and it lives in the bifrost-ui crate (`springboard/ui/src`).

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/web_usage.rs` | STWebpageController | One controller per page or tab: address, reporting bundle, profile, recording suppression, playing-video and picture-in-picture flags, and a blocked flag computed from a `BlockList` of guardian rules. `usage_record(at)` turns a visit into a `UsageRecord`. Also `canonical_url`. |
| `springboard/ui/src/web_history.rs` | STWebHistory | A `WebHistoryStore` holds visits. A scoped `WebHistory` handle (by bundle, by profile, or both) fetches and deletes visits by address or by `TimeInterval`, a half-open interval. Handles borrow the store, so a deletion is seen by other handles. |
| `springboard/ui/src/browser_settings.rs` | SFSafariSettings (settings access only) | A system-wide `BrowserSettings` object for homepage, default search engine and restore-on-launch. Read and write by key (`browser.homepage`, `browser.searchEngine`, `browser.restoreSession`, the same keys the browser stores in its defaults). Values are validated; a refused value leaves the setting unchanged. |

`springboard/ui/src/lib.rs` gained three `pub mod` lines. `web_usage` and `web_history` share one canonical URL
rule. `browser_settings` reuses `reading_list::ReadingList::supports` as its homepage check, so there is no second
copy of that rule.

## Official API names mirrored

- STWebpageController (`suppressUsageRecording`, `url`, `urlIsBlocked`, `urlIsPlayingVideo`,
  `urlIsPictureInPicture`, `setBundleIdentifier(_:)`, `profileIdentifier`).
  https://developer.apple.com/documentation/screentime/stwebpagecontroller
- STWebHistory (`init(bundleIdentifier:)`, `init(bundleIdentifier:profileIdentifier:)`, `init(profileIdentifier:)`,
  `deleteAllHistory()`, `deleteHistory(during:)`, `deleteHistory(for:)`, `fetchAllHistory(completionHandler:)`,
  `fetchHistory(during:completionHandler:)`). https://developer.apple.com/documentation/screentime/stwebhistory
- SFSafariSettings (type method `checkAutoFillUserNamesAndPasswordsEnabled`, `openExtensionsSettings`,
  `openExportBrowsingDataSettings`). https://developer.apple.com/documentation/safariservices/sfsafarisettings
- Screen Time framework landing page, which says the APIs do nothing on visionOS.
  https://developer.apple.com/documentation/screentime

Official facts came from `tools/parity/docc.py` (DocC JSON) for the four pages above.

## REA evidence

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/screentime | ev_be0edb51da3c7a89b44519852c156a346d43939989538676867d7703706cc538 |
| https://developer.apple.com/documentation/screentime/stwebpagecontroller | ev_0a344e80548c7280793dc82a727286f073645011ce4306e9857b9b3c7c45e463 |
| https://developer.apple.com/documentation/screentime/stwebhistory | ev_ead75aef2b0e2923adf26e7c4afd2dc1bbcd37bb31d87733fde3c940ffb09662 |
| https://developer.apple.com/documentation/safariservices/sfsafarisettings | ev_aaa769a289e2f9c727ec9b9b2b8fdf847664bcd36b8f330175921ba37a8c2a87 |

## Deviations from the official API

- Fetches return their result directly (`BTreeSet<String>`). The official API uses completion handlers. This is a
  pure model with no asynchronous delivery.
- Deletions return the number of visits removed. The official API returns nothing.
- Visit timestamps are Unix seconds, passed in by the caller. Official `Date` values are not modelled.
- Rule syntax for `BlockList` is our own: a host, optionally followed by a path, with a segment boundary. It is not
  a parental-control policy format, and no guardian approval flow exists.
- `SFSafariSettings` has no SDK object that presents the extensions pane or the export sheet. The browser has no
  extension host. The AutoFill check is not implemented: BifrOSt has no AutoFill store. Those parts stay gaps.
- Nothing is wired. The browser app still keeps its own history and preferences (`springboard/apps/browser/src/store.rs`,
  `springboard/apps/browser/src/prefs.rs`), and the shell does not call `BlockList`. The browser's `Engine` enum
  duplicates `SearchEngine` in the ui crate. Switching the browser over is a follow-up.
- Nothing has been built or run. The brief forbids builds in this round. The unit tests were worked out by hand from
  the rules, but no compiler has checked the code yet.

## Matrix rows closed

| Framework slug | Feature (as written in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| screen-time | Web-usage reporting (STWebpageController) | partial | springboard/ui/src/web_usage.rs |
| screen-time | Web-history deletion (STWebHistory) | partial | springboard/ui/src/web_history.rs |
| safari-services | Safari settings access | partial | springboard/ui/src/browser_settings.rs |

Rows left open after this round: ScreenCaptureKit "Permission for screen recording and persistent capture"
(no permission model for capture), PermissionKit "Permission button UI (PermissionButton)" (no guardian request flow),
and the other Screen Time configuration-observer rows. The web-usage model is the base for the Screen Time
configuration work when that is picked up.

## Tests

Each new module has a `#[cfg(test)] mod tests` block. Between them there are 20 behaviour tests (7 in `web_usage`, 7 in `web_history`, 6 in `browser_settings`) covering canonical
URLs, subdomain and path block rules, controller usage records, half-open intervals, scoped fetch and delete, and
validated setting writes. The expected values were worked out by hand from the rules.
