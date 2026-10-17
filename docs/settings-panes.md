# Settings panes from apps and Settings intents

Code: `springboard/src/shell/settings_panes.rs` (panes declared by apps), `settings_intents.rs` (intents that open
Settings pages), the SDK side in `springboard/sdk/src/settings_panes.rs`.

## Panes declared by an app

- An app lists panes in `manifest.toml` as `[[settings_pane]]` tables with an `id`, a `title` and an optional `symbol`.
  A table without a valid id and title is dropped. A repeated id keeps only its first table.
- Settings' root list shows each app that declares panes. Its page lists the panes.
- Picking a pane sends `ae.settings.openPane` with `{"id": ...}` over the app's control socket. The app must be running
  and must have installed the handler. Otherwise the shell logs the refusal and changes nothing.

## Settings intents

- Each intent is a runnable shortcut that opens one Settings page (for example Open Date & Time, Open Keyboard, Open
  Network, Open Notifications, Open Mouse, Open Trackpad, Open Game Controllers, Open Backup). The intent names its page
  by `PageId`.
- An intent never writes a setting. Values change only through the page's own actions.
- Search lists these intents among its Shortcuts (`settings_intent_rows` in `quicksearch.rs`), and the agent socket can
  list them too. Choosing one opens the page with `Settings::open_page_id`.
