# Connect News to the system services (network path, widget timeline)

News (`springboard/apps/news-desktop`, version 1.3.0, changelog entry extended, no version bump) now uses two shell
system services over the named service channel (`docs/bifrost-ui.md` "System services"). The shell and the SDK are
unchanged; the wiring is the app's own code in `src/services.rs` and `src/main.rs`. This is an app-side connection, so
no matrix row changes. The rows stay as the services report them (`network` partial, `widgetkit` partial).

## What is connected

| Service | Matrix slug | Feature | Where it is used in News |
|---|---|---|---|
| `network` (`bifrost_app::network_path`) | `network` (Network path monitoring) | Offline reading state | `services.rs` requests the path once at start (`network.path`) and follows the `pathUpdate` event. `News::set_online` stops feed and picture fetches while the path is not satisfied, keeps the cached headlines and saved pages readable, and refetches every feed at once when the path comes back. |
| `widgets_timeline` (`bifrost_app::widgets_timeline`) | `widgetkit` (Timeline updates) | Top stories timeline | `services::publish_top` reloads the timeline `top-stories` with the top four headlines in Today's order, one every 30 minutes (`SLOT_SECS`), with reload policy `at_end`. It is sent when the headlines change (a feed arrives) and when the shell sends the `reload` event. Without headlines the timeline is removed. |
| `notification-schedule` | `user-notifications` (Notification triggers) | Daily digest | Not connected. News has no daily digest and the brief says to connect it only if the app has one. |

## How a user sees it

- **Offline reading.** Turn the network off. News keeps the headlines it already has and the saved pages in the
  Reading List, and it stops asking the feeds for new items. A list with nothing cached says "You are offline" (the
  Reading List never says this). Turn the network on again and the feeds refresh at once. Pictures that were not
  loaded yet appear when the path is back. A banner for the offline state while headlines are shown is not drawn
  (see gaps).
- **Top stories in the widget area.** Notification Centre's widget area shows the current top headline from News'
  timeline. The headlines rotate every 30 minutes through the top four, and the timeline is refreshed when the
  headlines change or the shell asks for it.

## How to verify on the test PC (Midget)

Use Midget only; the lead builds once (`cd springboard && cargo build --offline --profile fast --bin bifrost-news`),
then deploy with `bifrost pc midget ota` until `phase=accepted`.

1. Offline reading: open News with the network up and let the feeds load. Unplug the Ethernet cable (Midget uses the
   wired VLAN 20). Open a fresh News window or press Refresh: the feeds do not refetch (`bifrost pc log` shows no
   feed request), the saved headlines stay listed, and a list with nothing cached says "You are offline". Plug the
   cable back in: the feeds refresh without pressing Refresh.
2. Reading List: with the network down, open a saved page and check the article and link card still show; add and
   remove a page (the toolbar button works offline).
3. Top stories: open Notification Centre (swipe down from the left two thirds of the status bar, or tap the clock).
   The widget area shows the first top headline from News. Wait 30 minutes (or check that the entry rotates when the
   timeline date passes) and confirm the next headline shows.
4. Shell reload: when the timeline runs to its last entry, the shell sends `reload`; News answers with a new timeline
   in the log (`bifrost pc log`, service `widgets_timeline`).
5. Older shell: a shell without the `network` service answers "no service network". News keeps the default (online),
   so nothing breaks.
6. Light and Dark: `bifrost pc midget appearance light|dark`, then `bifrost pc midget screen /tmp/news.png` for the
   offline list message.
7. Host checks (no build run in this round): `cargo test -p bifrost-news services` (the glue tests) and
   `cargo test -p bifrost-news --test compliance` (the snapshot is unchanged for the online state).

## Tests (written by reading only; the lead's build runs them)

`src/services.rs` tests, using `Ctx::testing` and `take_sent`:

- top entries follow Today's order (first item of each feed, then the second round), capped at four, spaced 30 minutes;
- no headlines give no entries;
- the reload call carries the kind, the entries and the `at_end` policy; no headlines send `remove`;
- the reload event is recognised for News' timeline only;
- a satisfied path is online, unsatisfied and requires-connection paths are offline;
- an error reply and unrelated events leave the state alone.

## Remaining gaps

- **No daily digest.** News has no digest, so `notification-schedule` is not connected. A digest would be a new
  feature with its own permission and copy, so it is left for a later round.
- **No offline banner over cached headlines.** Offline with headlines listed, the list shows no notice; the message
  appears only when nothing is cached. A banner would change the drawn list and needs a snapshot bless after a visual
  check.
- **Widget timeline has no picture or link.** Entries carry a title and the feed name only. Tapping the entry does not
  open the article, and the shell gives no interactivity.
- **Timeline in memory.** The shell does not keep timelines across a restart; News sends the timeline again at its
  first tick after start, from the cached feeds.
- **Path is pulled once and pushed by the shell.** The shell pushes `pathUpdate` at its tick rate. There is no separate
  signal from the network manager, and reachability of a named host is not probed.
- **Picture retry.** Pictures that were skipped while offline load when the path comes back, through the row redraw;
  the feed fetch and the picture job share the same dispatch pool as before.
- **Cargo.lock.** `bifrost-core` is a new dependency of `bifrost-news`; `springboard/Cargo.lock` must record it. The
  lead's build updates it, or a `--locked` build needs `cargo update -p bifrost-news` first (this also applies to the
  version 1.3.0 entry noted in `wire-app-news.md`).
- **Not run.** Nothing in this branch was built or tested here (no-build rule); the tests and the Midget checks are the
  lead's.
