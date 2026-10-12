# Wire 4: Browser app (`springboard/apps/browser`, 0.12.0)

Fourth wiring round for the Browser, focused on the WebKit rows: navigation and the back-forward list, web data
management, and web views and configuration. One of the three focus items is wired: the back-forward list, as a History
menu. The other two are not, for the reasons under "Remaining gaps". No cargo, rustc or tests were run (the lead builds
once), so the first compile is the lead's.

## What is now wired

| Piece | Model | Where in the app |
|---|---|---|
| History menu: Back and Forward submenus of the selected tab's pages | none in `springboard/ui`; the app's own per-tab list (`tabdiscard::Discarder`, the same list the Back button uses for revived tabs) | `springboard/apps/browser/src/tabdiscard.rs`: `history_menu` builds the items, `menu_entries` picks the entries around the current one, `history_label` shortens each address, `jump_to` and `jump_step` move the tab one engine step per finished load, `step_back` is the Back button's rule. `main.rs`: `menus()` adds the History menu, `menu_action` dispatches the item ids. |

### Why this is the app's own list and not a shared model

`springboard/ui/src` has no back-forward list model. The WebKit row names `WKBackForwardList` (an object that holds the
previously loaded pages and the current one, with `backList`, `forwardList`, `backItem` and `forwardItem`; REA evidence
below). The Browser already keeps that list per tab for revived (discarded) tabs in `tabdiscard.rs`, since Servo 0.7.0 cannot
inject history. The menu reads that list, so there is one list, not a second model. Writing a shared model would duplicate
it; the shared-model route is listed under gaps.

### How the jump works

- `history_menu` gives each entry the item id `HISTORY_BASE` (1000) plus its index in the tab's list. `history_target`
  maps an id back to an index (ids 1000 to 1049, the list keeps at most 50 entries).
- `jump_to(target)` records the target index for the tab and takes the first step. Each step is one Back (`go_back`, or
  the app-side previous entry for a revived tab) or Forward (`go_forward`). The next step is taken on `LoadFinished`,
  until the tab's current index reaches the target.
- A jump ends when it reaches its target, when the engine has no step left in that direction, or when the user opens a
  new page (`track` reports a new entry, and the jump is cleared).
- The menu is recomputed after each event by the shell, so the entries follow the tab.

## How a user reaches it

- **History > Back** and **History > Forward** (the menu bar, History menu, after View). Each submenu lists up to ten
  pages on its side of the current page, nearest first, shortened to 60 characters. The list is empty (with "No Earlier
  Pages" or "No Later Pages", greyed out) on a new tab.
- Choosing a page moves the tab there. The Back and Forward buttons and the address bar keep working as before.

## How to verify on the test PC (Midget)

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. Open a new tab and load three pages in turn (for example `https://example.com/`, then two more links on the page).
3. Open the History menu: Back lists the two earlier pages, nearest first. Forward is greyed out with "No Later Pages".
4. Choose the first Back item: the tab goes to that page (one step at a time, so a page with several entries back takes
   more than one load). Forward now lists the pages you left.
5. Choose a Forward item: the tab goes forward to it. Then open a new link from the page: Forward is empty again
   ("No Later Pages"), as with the Back and Forward buttons.
6. Take a Midget screenshot with `bifrost pc midget screen PATH` and check the menu with the bifrost-hig checklist (the
   History menu is an app menu after View; its submenus are standard).
7. To check the engine step rule on a revived tab, lower the discard idle time (`BIFROST_TAB_DISCARD_MIN=1`) and open
   several pages: a discarded tab's earlier pages should still jump from the menu.

Unit tests (not run; the lead runs them), all in `tabdiscard::tests`: `menu_lists_back_nearest_first_and_forward_after_current`,
`menu_limit_keeps_the_nearest_entries`, `menu_labels_drop_scheme_and_trailing_slash`, `history_menu_ids_map_back_to_entries`,
`track_reports_only_new_entries`, `jump_target_is_kept_until_cleared_or_forgotten`, and the existing
`app_history_survives_and_steps_back` (its `track` calls still work; `track` now returns a bool that the test ignores).
Expected values were worked out by hand: a list `[a, b, c, d]` at index 1 with a limit of 2 gives back `[(0, a)]` and
forward `[(2, c), (3, d)]`; `https://example.com/a/` labels as `example.com/a`; a 70-character host and path label as 60
characters and an ellipsis.

## REA evidence

- `ev_cff149f10ab3e352c5c120de9442b655d34986cc614f335f92969696f51a55d2`: official page "WKBackForwardList" (rendered with
  `tools/parity/rea-page.sh`). It says the object manages the list of previously loaded pages that the web view uses for
  backward and forward navigation, and that each web view creates one automatically. `docc.py webkit/wkbackforwardlist`
  lists `backItem`, `currentItem`, `forwardItem`, `item(at:)`, `backList` and `forwardList` (the names the app's list
  mirrors in behaviour; no Swift or Objective-C names are used in code).
- `ev_d7c8b17e8ed5fadf02a14e2ef8d29bec251781107bdc234628dd1eed5947b5c4`: official page "WKHTTPCookieStore" (rendered).
  It says the store manages the cookies for a web view, with `getAllCookies`, `setCookie` and `delete`. Read with
  `docc.py webkit/wkhttpcookiestore`. This is the API the Browser would need for website data, and BifrOSt does not expose
  it (see gaps).
- `docc.py webkit/wkwebsitedatastore` (no rendered evidence id this round): "An object that manages cookies, disk and
  memory caches, and other types of data for a web view."

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status after this round |
|---|---|---|---|
| webkit | Navigation and back-forward list | `springboard/apps/browser/src/tabdiscard.rs` (`history_menu`, `jump_to`, `jump_step`); `springboard/apps/browser/src/main.rs` (`menus`, `menu_action`) | still `partial` (the History menu works from the app's list; no back-forward list object in the SDK, and no navigation policy callback) |
| webkit | Web data management (cookies, website data, URL scheme handling) | none wired (`springboard/sdk/src/webview.rs` profile directory; `springboard/sdk/src/http_cookies.rs` is not connected to the engine) | still `partial` |
| webkit | Web views and configuration | none wired | still `partial` |

## Remaining gaps

- **Manage Website Data (not built):** the Settings sheet has only the all-or-nothing "Clear Website Data" row. A per-site
  list and per-site removal needs the engine's cookie and site-storage API, which the engine (`springboard/apps/browser/engine/src/main.rs`)
  uses only for the download cookie probe (`cookies_for_url`). Listing and deleting per host needs (a) a new engine
  protocol command for the cookies of a host and for deleting a host's site data, and (b) a Settings page listing those
  hosts. Servo 0.7.0 is not in the local cargo registry, so the exact `SiteDataManager` calls could not be checked here,
  and an unverified engine change could break the engine build. Not built in this round for that reason.
- **Per-site settings:** no model exists for per-site preferences (the matrix has no row for them either), so nothing is
  wired.
- **The `http_cookies` model:** `HTTPCookieStorage` is a pure model and would hold the list if the engine exposed the
  jar. It is not connected to the engine, so it is not used here. Connecting it is the first step of the gap above.
- **Back-forward list object:** the SDK has no `WKBackForwardList`-style object. Apps cannot read the list through
  `bifrost_app::webview`; only the Browser's own menu reads it. Moving the list into the SDK would make it reusable.
- **Jumps and engine events:** a jump relies on `LoadFinished` arriving after each engine step. This is not verified on
  Midget; if a step does not finish, the jump waits until the next load and then ends (the Back button still works).
- **No navigation policy callback:** the engine does not ask for decisions, so `nav_policy` is not called (as in wire 3).
- **Not built or run:** no cargo, rustc or tests were run in this round, per the wiring rules. The matrix data is not
  edited here; the lead updates `docs/parity-matrix/` after the build. `springboard/apps/browser/Cargo.toml` and
  `manifest.toml` are at 0.12.0; the lead's build updates `springboard/Cargo.lock` for the new version.
