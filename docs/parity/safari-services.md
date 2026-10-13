# Safari Services parity: pure-Rust models (round 1)

Framework: Safari Services, https://developer.apple.com/documentation/safariservices (macOS 10.12+).
Matrix row: `docs/parity-matrix/data/safari-services.json` (framework status stays `partial`).

## What was built

Three std-only pure models. None has a build or test run yet; the build stage runs them.

| Gap (official name) | Module | Mirrors | Scope |
|---|---|---|---|
| Safari Reading List | `springboard/ui/src/reading_list.rs` (`ReadingList`, `Item`, `Error`) | `SSReadingList.addItem(with:title:previewText:)`, `supportsURL(_:)`, `SSReadingListError` | Add, remove, contains, newest-first order, a bounded size, and an escaped one-line-per-item text form. Our own rules: only http and https addresses with a host; a repeat moves to the top; an empty title falls back to the address. Tests: 7. |
| Content blockers | `springboard/ui/src/content_rules.rs` (`Rule`, `RuleList`, `ResourceType`, `Action`, `Verdict`) | `SFContentBlockerManager` state and reload concepts, and the content-blocker rule model | Block and allow rules with a url-filter subset: `*`, `^`, leading `||`, leading `|`, trailing `|`. Rules can be limited to resource types and to page hosts (`if_domain`, `unless_domain`). An allow match beats any block match. Tests: 8. |
| Universal links and associated domains | `springboard/core/src/app_links.rs` (`AppLinks`) | `SFUniversalLink` (`webpageURL`, `applicationURL`, `isEnabled`) and the associated domains concept | Claims a host's path prefixes for an app, enable and disable per app, unclaim, and longest-prefix resolution on segment boundaries. Query, fragment and port are ignored. Tests: 5. |

Module placement follows the brief: pure models in `springboard/ui` and `springboard/core`. Each new module has exactly one `pub mod` line in its crate's `lib.rs`. The reading list and content rules are both in `springboard/ui/src/lib.rs`; `app_links` is in `springboard/core/src/lib.rs`.

## Official API shape used

From `docc.py` on the official pages:

- `SSReadingList`: `default()`, `supportsURL(_:)`, `addItem(with:title:previewText:)`. Error domain and `SSReadingListError.Code` are listed on the class page but their codes are not reproduced; our error type has one variant, `UnsupportedUrl`, and says so in the doc comment.
- `SFContentBlockerManager`: `getStateOfContentBlocker(withIdentifier:completionHandler:)`, `reloadContentBlocker(withIdentifier:completionHandler:)`. Only the rule matching is built; the state and reload calls need an extension host and are not built.
- `SFUniversalLink`: `init(webpageURL:)`, `applicationURL`, `isEnabled`, `webpageURL`. The model covers the association and enable flag; the host lookup is not built.

## REA evidence

- `ev_a78c80dd67e48d793cee4c4f5446d4aa9d0db04c06f40d51af20509bab326bb7`: rendered Safari Services landing page (`https://developer.apple.com/documentation/safariservices`), recorded with `rea inspect-web-page` through the parity CDP browser on 127.0.0.1:9333. It lists the Content blockers, Associated domains and Safari Reading List sections used here.

The matrix row's previous evidence id, `ev_db48755bdd049ac960cdd61e81535120a7dc9a2c45e56e809f496825a5b51dc8`, is kept in git history.

## Remaining gaps

- Content rules are not loaded into the web view and have no SDK entry point, state or reload reporting, and no CSS hiding rules.
- The reading list has no SDK API, no Browser app wiring and no share target. Reader mode in `springboard/src/shell/browser/reader.rs` is a separate feature.
- Universal links have no association fetch, entitlement check, SDK API or hook in `springboard/sdk/src/workspace.rs`.
- No standard in-app browser sheet, no web extension host, no shared authentication session, no system-wide browser settings object, and no Safari bookmark or password import.
- Web view input gaps are unchanged (hover, drag selection, context menus, file panels, load cancel).

## Verification status

- No cargo, rustc or tests were run in this round, as the task required. The build stage must run `cargo test` for `bifrost-ui` and `bifrost-core` (or the crate names in their Cargo.toml).
- `python3 tools/check-names.py` exits 0 for this change.

## Round: web extension model and Safari-shaped authentication session (parity-safari)

- `springboard/ui/src/safari_extension.rs` (SDK: `bifrost_app::safari_extension`): the identity (lowercase reverse-DNS bundle
  id), kind (content blocker or web extension), host permissions (`https://*.example.com`, `*://*`), a new extension starts
  disabled, and a registry answers `allowed_for(url)` only for enabled extensions whose patterns match. Messages are a name
  and a string dictionary, with plain-token names. Decision: this is the model shape only. Nothing runs extension code, and
  there are no page, tab or window proxies yet. The Browser app does not consult the registry.
- `springboard/ui/src/safari_auth.rs` (SDK: `bifrost_app::safari_auth`): an `SFAuthenticationSession`-shaped surface over
  `webauth_session`. It gives the start URL and callback scheme, one completion (callback URL, canceled login, or presentation
  failure) reported once. Decision: Apple deprecated this API for the web authentication session, so the model is kept
  only for the older name. Its numeric error codes are not mirrored; the Rust enum names the cases. Shared cookies between
  the Browser app and apps are still not wired.
- Still gaps: a web extension host and Safari page or tab proxies, the open-url wiring for the in-app browser sheet
  (`browser_sheet.rs`), and content-blocker enforcement in the web view. Status for the matrix rows is partial, not parity:
  the device test (Midget) has not run.
