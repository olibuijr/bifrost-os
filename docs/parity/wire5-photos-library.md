# Wire 5: Photo library (smart albums and sort by date in the Photos app)

Branch `wire5/photos-library`, from `main` (0.5.42). Wires two photo library models into the shell's Photos app:
the smart album rules (`bifrost_ui::photo_smart_albums`) and the fetch options (`bifrost_ui::photo_fetch`). Nothing
was built or run in this round (the round forbids builds); the tests are written with hand-worked expected values and
have not been executed. The lead builds once.

## What is now wired

| Piece | Before | Now |
|---|---|---|
| Favourites, Recents and Screenshots lists | the Photos app filtered its own list (`albums::recents`, `albums::screenshots`, a favourite filter) | the lists come from `models::smart_album` over `photo_smart_albums::members` (the same rules as the model, with its tests) |
| Order of every album's grid and viewer | library order (ascending time, path on ties), fixed | `models::by_date` over `photo_fetch::fetch`: oldest first (the default, the same order as before) or newest first, chosen in the album picker |
| Sort choice | none | kept in the Photos prefs file as `sort` = `newest` or `oldest`; read back at start |

The old helpers `albums::recents`, `albums::screenshots` and `albums::is_screenshot` (and `albums::RECENT_DAYS`) were
removed because nothing else used them. Their rules now live in the model, and the test in `models.rs` covers the same
cases (name prefix in any case, the 30-day window, a day of future tolerance).

Smart album rules, as the model has them: Screenshots is the name prefix `screenshot` (any case) and not trashed;
Recents is created within the last 30 days, and at most a day in the future; Favourites is marked and not trashed;
Recently Deleted is trashed (still the Trash list on disk, unchanged). Recents keeps the old boundary: a photo exactly
30 days old is out.

## How a user reaches it

1. Open Photos and tap **Albums** in the top bar (the album picker).
2. Pick **Favourites**, **Recents**, **Screenshots** and the other albums as before; their lists now come from the
   smart album rules.
3. The picker's last row, **Sort by Date**, shows the current order (**Oldest** or **Newest**). Tap it: the picker
   closes, the grid flips order and scrolls to the top for newest first (bottom for oldest first, as before). The
   choice stays after a restart.
4. Favourite toggle and Add to Album are unchanged: the viewer's heart and the viewer's Album button.

## How to verify on the test PC (Midget)

1. Deploy the lead's build (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. `bifrost pc midget open-app photos`. Read `bifrost pc midget state` to get the window geometry.
3. Tap Albums (`bifrost pc midget tap-window Photos X Y`, window-local pixels, left of the centre of the top bar). The
   picker lists All Photos, Recents, Favorites, Screenshots, Recently Deleted and the user albums, then **Sort by Date
   / Oldest** under them.
4. Tap **Sort by Date**: the grid now shows the newest month at the top. Open the first photo: it is the newest one.
5. Tap Albums, then Sort by Date again: back to oldest first. Restart the Photos app: the order is kept.
6. Favourites: mark one photo with the heart in the viewer, open Favourites in the picker, and check it is listed once.
   Screenshots: a file whose name starts with `screenshot` (any case) under `~/Pictures` appears in Screenshots.
7. Screenshot with `bifrost pc midget screen PATH` and check the picker row against the design rules (CONTENT_BG is not
   used on the picker; it uses the sheet colour and the named constants, as the other rows do).

## Host tests added (not run)

- `springboard/src/shell/photos/models.rs` (pure, hand-worked values; library in the test: times in days 99, 60,
  100 plus 5 seconds, 70, 10 after the epoch, "now" at day 100):
  - `oldest_first_is_the_library_order_by_time`: 10, 60, 70, 99, 100 (+5 s).
  - `newest_first_reverses_the_times`: the reverse order.
  - `items_with_the_same_time_keep_their_library_order_in_both_directions`: two photos at time 5 keep their order.
  - `screenshots_match_the_name_rule`: `Screenshot_1.png` and `screenshot_old.png`, in library order.
  - `recents_are_the_last_thirty_days_up_to_a_day_ahead`: day 99 and the day 100 plus 5 s picture; day 70 is out
    (exactly 30 days old).
  - `favourites_follow_the_mark_the_shell_passes_in`, and `a_negative_time_counts_as_the_epoch`.
- `springboard/src/shell/photos/sort_tests.rs`: `the_sort_row_flips_the_grid_between_oldest_and_newest_first` opens the
  picker, taps the sort row (the row is below the albums and the New Album row), checks the reversed order, the
  `sort` pref, and that a second tap restores the oldest-first order.
- `albums.rs`: the old screenshot and recents test is replaced by a date-range check (`in_range`), since those helpers
  moved to the model.

Check before merging: the existing `favorites_tests.rs` and `mod.rs` tests use the same library order (ascending time),
which is still the default.

## REA evidence

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/photos/phfetchoptions (predicate, sort descriptors, fetch limit, hidden assets) | `ev_a7f8e3d90847b69c7d0d8a0cac164e26a49ca5efa571bb1bec997f4c8db52431` |
| https://developer.apple.com/documentation/photos/phassetcollectionsubtype (smart album subtypes) | `ev_ad862b37b7ba81d7cadeb37414f8de1381b8ae3ebbbfdbfe19306c22556d952a` |
| https://developer.apple.com/documentation/photos/phasset/mediatype (media types of an asset) | `ev_42172fd08ae21d86e0164b1f2e7b9fd142bf99dd8acd541841823dc652fbeb07` |

The earlier evidence from the wave 5 PhotoKit note (`docs/parity/w5-photokit.md`) still applies to the smart album
subtypes and the caching manager.

## Remaining gaps

- **Favourites for apps**: the favourite mark is still a Photos preference (`fav:<path>`). No service call lets an app
  read or set it. The matrix row `Favorites` stays `os`. This needs a service method in `springboard/src/shell/services/`
  and an SDK client, which this round did not add.
- **Shared library access and limited selection** (`photo_access.rs`): no app asks the shell for library access, and no
  limited selection reaches an app. The Photos app reads `~/Pictures` directly.
- **Asset retrieval for apps** (`photo_fetch.rs` as an API): the model is used by the Photos app only. Apps cannot
  fetch assets, collections or sorted lists.
- **Change observation** (`photo_changes.rs`): the Photos app rescans and rebuilds; nothing emits a change diff to apps.
- **Media Types**: the library reads images only (the scan keeps image file names). There is no Videos or Live Photo
  album, so the Media Types part of the album list is not built. `photo_media::Kind` has the kinds for later.
- **Moments, Years and folders**: the sections are month groups inside the grid, not separate collections.
- **Limit and media filter** (`FetchOptions::limit` and `media`): the model has them, but the picker does not expose them.
- **Not built, not tested on Midget**: all code is unverified until the lead builds and runs the Photos tests.

## Matrix rows touched (for the lead to import; `docs/parity-matrix/data/photokit.json`)

| Slug | Feature (as written in the matrix) | Status | BifrOSt path |
|---|---|---|---|
| photokit | Smart albums (screenshots, recents, recently deleted) | partial | springboard/src/shell/photos/models.rs (smart album rules of springboard/ui/src/photo_smart_albums.rs) |
| photokit | Asset retrieval and fetch options | partial | springboard/src/shell/photos/models.rs (fetch model of springboard/ui/src/photo_fetch.rs; sort by date in the Photos picker) |
| photokit | Favorites | os | springboard/src/shell/photos/mod.rs (favourite mark; no app-facing read or write, see the gaps) |
