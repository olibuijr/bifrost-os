# Wave 3, theme 18: web extensions (match patterns, URL filters, call directory)

Branch `parity/w3-theme-18`, built on `parity/integration` (waves 1 and 2). Pure Rust, std only, no new crates,
no unsafe. Nothing was built or run: the code and its tests were checked by reading only, as the wave rules
require. The lead runs `cargo test -p bifrost-ui` (or the crate's test target) before merging.

## What was built

| Module | Crate | What it does |
|---|---|---|
| `springboard/ui/src/match_pattern.rs` | bifrost-ui | Parses `<all_urls>` and `scheme://host/path` patterns (`*` scheme is http or https; `*.domain` host; `*` glob path). Matches a URL, ignoring the port, query and fragment. Rejects malformed patterns with a typed `PatternError`. |
| `springboard/ui/src/url_filter.rs` | bifrost-ui | Allow and deny lists over match patterns. Verdict: allow when an allow pattern matches, deny when a deny pattern matches, allow otherwise. `should_connect` for the caller. Built on `match_pattern`. |
| `springboard/ui/src/call_directory.rs` | bifrost-ui | Blocking and identification entries keyed by phone number. Full or incremental requests (`begin_request`). Strictly ascending numbers within a request, range 1 to `PHONE_NUMBER_MAX`. Lookup by number, removal by list. |

`springboard/ui/src/lib.rs` gained three `pub mod` lines (`match_pattern`, `url_filter`, `call_directory`).
Each module has a `#[cfg(test)] mod tests` with 5 to 7 behaviour tests whose expected values were worked out by
hand from the rules in the code (for example `https://*.example.com/*` against `https://notexample.com/` is
false; `*://*/*` deny against `ftp://files.example/x` is allow, because `*` covers http and https only).

## Official API names mirrored

Source: developer.apple.com DocC JSON, read with `tools/parity/docc.py`.

- Web extensions: `WKWebExtension.MatchPattern` (`init(string:)`, `matches(_:)`, `matchesAllURLs`,
  `matchesAllHosts`, `allURLs()`, `allHostsAndSchemes()`, scheme/host/path parts). Doc:
  https://developer.apple.com/documentation/webkit/wkwebextension/matchpattern
- URL filters: `NEURLFilter.verdict(for:)` and `NEURLFilter.Verdict` (allow or deny; the page says "Don't connect
  to any URL that receives a 'deny' verdict"). Doc: https://developer.apple.com/documentation/networkextension/neurlfilter
- Caller ID and blocking: `CXCallDirectoryExtensionContext` (`addIdentificationEntry(withNextSequentialPhoneNumber:label:)`,
  `addBlockingEntry(withNextSequentialPhoneNumber:)`, `removeBlockingEntry(withPhoneNumber:)`,
  `removeIdentificationEntry(withPhoneNumber:)`, `removeAllBlockingEntries()`, `removeAllIdentificationEntries()`,
  `isIncremental`). Doc: https://developer.apple.com/documentation/callkit/cxcalldirectoryextensioncontext

## REA evidence ids (rendered official pages)

- `ev_bce540b569486a4379e95d0018a3aa0b44edc135fc1cbea08a57084ceb0232e3`: WKWebExtension.MatchPattern
- `ev_8df6a35b696d815e80c5f99419bcfae9762f923f8e4b08d99fce23c098048ae2`: NEURLFilter
- `ev_921bd8a2c941d5e55a2188d13e1ec2e35d757c99c83a5690aa3cab532bb81269`: CXCallDirectoryExtensionContext

## Matrix rows closed (new status partial)

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| webkit | Web extensions | partial | springboard/ui/src/match_pattern.rs |
| network-extension | URL filters | partial | springboard/ui/src/url_filter.rs |
| callkit | Caller ID and blocking | partial | springboard/ui/src/call_directory.rs |

Each row is partial, not parity. Only the matching, filter and directory models exist:

- Web extensions: there is no manifest loading, no runtime, no content-script injection and no tab or window
  model. Only match patterns are built.
- URL filters: the verdict model exists. The filter is not wired to a connection path, and it is not a system
  content filter provider (the "Content filter providers" row stays a gap).
- Caller ID and blocking: the model exists. No system call directory extension is registered, and nothing in the
  PC desktop calls the directory.

## Rows not closed (and why)

- Quarantine properties (core-services): not built. The official quarantine key page did not load from the DocC
  path tried, so no key names or values are claimed.
- Web-history deletion (STWebHistory), Web-usage reporting (STWebpageController): not built in this theme. The
  browser's history store is app-level (springboard/apps/browser/src/store.rs), and no Screen Time counterpart
  exists to mirror.
- Content filter providers, Contextual menus, DOM node snapshot, Native object export (JSExport), Index descriptions:
  outside a pure model in this theme.

## Owner rule

`python3 tools/check-names.py` exits 0 after this change. The three modules contain no Apple platform or product
names; the official names above appear only in this document.
