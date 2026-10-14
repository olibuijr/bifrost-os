# Home Screen quick actions

Status: partial. Built on `parity/team-p-quick-actions`; not yet verified on the tablet.

An app offers a short list of quick actions, shown from the app's icon. Static items are declared in the manifest and
never change. Dynamic items are set while the app runs and replace the previous dynamic list. A choice is delivered to
the app as an input, with the item's type and user info.

## What is built

| Layer | Path | What it does |
|---|---|---|
| Model | `springboard/ui/src/quick_actions.rs` | `ShortcutItem` (type, title, subtitle, symbol, user info); limits (four items, 64-byte types, 48-character text); `clean` and `clean_all` checks; `combine`, which lists dynamic items first, then static ones, without repeated types and capped at four. |
| SDK | `springboard/sdk/src/quick_actions.rs` | `set_items(ctx, &[ShortcutItem])` checks the list and publishes it; `clear(ctx)` removes the dynamic list; `QuickActionPick` is the chosen item. |
| SDK input | `springboard/sdk/src/runtime.rs` | `Input::QuickAction(QuickActionPick)`, delivered when the shell sends `ToApp::QuickAction`. |
| Protocol | `springboard/proto/src/lib.rs` | `ToApp::QuickAction { kind, user_info }`, tag `0x164`; at most 64 user info pairs on the wire. |
| Manifest | `springboard/src/shell/quick_actions.rs` (`parse_static`), `springboard/src/shell/apphost.rs` | `[[quick_action]]` tables become `Manifest::quick_actions`. |
| Service | `springboard/src/shell/services/quick_actions.rs` | The `quick_actions` system service: `set`, `clear`, `state`. The list is kept on the app's `AppHost`. |
| Shell | `springboard/src/shell/quick_actions.rs` (`quick_actions_for`), `springboard/src/shell/desktop_bar.rs` | The combined list for an app; the Dock's menu shows the list above the app's own items, and choosing one sends `ToApp::QuickAction`. |

## How an app uses it

Static items go in the manifest. Each `[[quick_action]]` table needs a `type` and a `title`; `subtitle`, `symbol` and
`user_info` are optional. `user_info` is a list of `"key=value"` strings, and a value cannot contain a comma.

```toml
[[quick_action]]
type = "compose"
title = "New Message"
subtitle = "Work"
symbol = "pencil"
user_info = ["account=work"]

[[quick_action]]
type = "search"
title = "Search"
```

Dynamic items are set from the app, usually when its state changes:

```rust
use bifrost_app::quick_actions::{self, ShortcutItem};

let mut compose = ShortcutItem::new("compose", "New Message");
compose.user_info = vec![("account".into(), "work".into())];
quick_actions::set_items(ctx, &[compose])?;   // replaces the dynamic list; Err when the list breaks the limits
quick_actions::clear(ctx);                    // removes the dynamic list; static items stay
```

The app handles a choice in its input handler. The type says which action it is:

```rust
Input::QuickAction(pick) => match pick.kind.as_str() {
    "compose" => open_compose(ctx, &pick.user_info),
    _ => return false,
},
```

## Behaviour

- Dynamic items come first, then static ones. A type that appears twice is shown once, and the first copy wins. The
  shell shows at most four items.
- `set_items` checks the whole list before sending it. The shell checks it again (`clean_all`) and refuses the call
  with an error message if the limits are broken. A refused call leaves the previous list in place.
- The service answers with `null` on success. `state` returns `{"count": n}` for the dynamic list.
- The service works only for an installed app that is running, as the `dock` service does.

## Desktop and tablet

- Desktop (pointer and keyboard): a secondary click on an app's Dock icon opens the Dock's menu. The app's quick
  actions come first, then its own Dock menu items. Choosing one delivers `Input::QuickAction`.
- Tablet: the model, the service and the protocol work the same on the tablet, and the app receives the choice in the same
  way. The tablet draws no Dock menu, so a quick action cannot be chosen there yet. The Home Screen long press on an app
  icon still starts edit mode and does not show the quick actions. Drawing a menu for touch on the Home Screen is the next step.

## Not yet done

- The tablet Home Screen menu for a long press on an icon (see above).
- Icons for the `symbol` field. The item carries the symbol name, and no shell code draws it yet.
- Verification on the tablet and on Midget. Nothing here has been built or run yet; the lead's build and tests come first.

## Tests

Unit tests cover the model (`springboard/ui/src/quick_actions.rs`), the SDK params
(`springboard/sdk/src/quick_actions.rs`), the protocol round trip (`springboard/proto/src/lib.rs`,
`quick_action_tests`), the manifest tables (`springboard/src/shell/quick_actions.rs`) and the service
(`springboard/src/shell/services/quick_actions.rs`).
