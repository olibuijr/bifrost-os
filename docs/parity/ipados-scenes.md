# Scenes: several windows of one app

Status: built on `parity/team-p-scenes`; not yet verified on the tablet or on Midget. The code has unit tests only, and the
lead builds and checks it later.

This covers two rows of the tablet platform parity data (`docs/parity-matrix/ipados/data/`):

- `uikit-app-scene-lifecycle.json`, "UIScene, UIWindowScene, UISceneSession, UISceneDelegate, multiple instances".
- `uikit-windows-multitasking.json`, "UIWindowScene, multiple windows of one app".

## What is built

The model has four layers. Each one lives in its own module, and the shared files only gain a `pub mod` line.

| Layer | File | What it does |
|---|---|---|
| App side | `springboard/sdk/src/scene_session.rs` | `SceneSessions` registry, `SceneDelegate` callbacks, `SceneState`, `SceneEvent`, wire codecs for the `scenes` service |
| Shell service | `springboard/src/shell/services/scene_services.rs` | `SceneWindows` table and the `scenes` service: request, reopen, list, focus, close |
| Shared geometry | `springboard/ui/src/window_scene.rs` | `SizeRestrictions` (`UIWindowScene.sizeRestrictions`), `cascade` placement, `contains` |
| Window server | `springboard/wm/src/scene_stack.rs` | Back-to-front stack of one app's scene surfaces: raise, remove, front, hit order |

### Scene sessions in the app

A scene is one window with its own session. `SceneSessions` keeps the sessions of one app. Each session has:

- `persistent_id`, which the shell assigns and which survives a relaunch;
- `role`, always `windowApplication`;
- `state`, one of `Unattached`, `ForegroundActive`, `ForegroundInactive` or `Background`;
- `user_info`, the key and value pairs the app passed when it asked for the scene;
- `restorable`, a `RestorableState` (see `springboard/sdk/src/restoration.rs`) that the app fills and reads per scene.

The state is derived, not set by the app. The registry focuses one scene at a time. When the app is in front, the focused
scene is `ForegroundActive` and the others are `ForegroundInactive`. When the app is in the background, every scene is
`Background`. `transition_events` gives the callbacks for each change, in the platform's order:

| From to | Callbacks |
|---|---|
| connect in front | `WillConnect`, `WillEnterForeground`, `DidBecomeActive` |
| connect in background | `WillConnect`, `DidEnterBackground` |
| active to inactive | `WillResignActive` |
| inactive to active | `DidBecomeActive` |
| inactive to background | `DidEnterBackground` |
| active to background | `WillResignActive`, `DidEnterBackground` |
| any to disconnected | `WillResignActive` (if active), `DidEnterBackground` (if not in background), `DidDisconnect` |

The app implements `SceneDelegate::scene_event` and receives each callback after the scene has its new state.

### The shell's scene windows

The shell owns the windows. `SceneWindows` keeps every scene window front to back. Each entry has the app id, the scene
identifier (`window-N`), a window number (from 20001 up, above the app numbers of the `windows` service), a frame in points
and a title.

- A new window cascades from the top-left corner of the screen. Each new window steps 28 points right and down, and the
  cascade wraps before it leaves the screen. On the desktop the screen starts below the menu bar. On the handheld it is
  the whole screen.
- Size restrictions come from the request (`min_width`, `min_height`, `max_width`, `max_height`). A window never leaves
  them, and a window larger than the screen is shrunk to fit.
- An app may hold at most eight windows (`MAX_PER_APP`).
- `reopen` puts back a window with its saved identifier and frame. The frame is moved and shrunk into the screen.
  An identifier already open is answered as it is, and a later `window-N` skips it.

### Window server stack

`SceneStack` is the stacking order of one app's surfaces. `push` and `raise` put a surface in front, `remove` closes one,
and the next surface in front takes focus. `front_to_back` gives the hit-test order, and `back_to_front` gives the paint
order.

## How an app uses it

An app asks the shell for a window, then registers the scene with the id the shell answers with.

```rust
use bifrost_app::scene_session::{self, SceneAnswer, SceneDelegate, SceneEvent, SceneSession, SceneSessions};

// The registry and its delegate are separate fields, so both can be borrowed at once.
struct Notes {
    scenes: SceneSessions,
    windows: NoteWindows,
}

#[derive(Default)]
struct NoteWindows;

impl SceneDelegate for NoteWindows {
    fn scene_event(&mut self, scene: &SceneSession, event: SceneEvent) {
        if event == SceneEvent::WillConnect {
            // build the content of this window from scene.user_info
        }
    }
}

// the app asks for a second window; the answer arrives in App::service_reply
let info = [("document".to_string(), "essay.txt".to_string())];
let params = scene_session::request_params("Notes", 480.0, 360.0, &info);
let request = bifrost_app::service::call(ctx, scene_session::SERVICE, scene_session::METHOD_REQUEST, &params);

// in App::service_reply(ctx, id, result), when id is the request:
let answer = SceneAnswer::decode(result)?;
notes.scenes.request(&mut notes.windows, Some(&answer.scene), info.to_vec())?;
```

The app moves focus with `focus` and ends a window with `close`, both on the same service. `list` answers the windows the
app holds, front first. An app that restores after a relaunch calls `reopen` for each saved scene, with the saved
identifier and frame, and then `request` on the registry for each one.

The app notifies the registry when it comes to the front or goes to the background, from its existing `resume` and
`suspend` callbacks: `notes.scenes.set_foreground(&mut notes.windows, true)` and `set_foreground(&mut notes.windows, false)`.

## Desktop and tablet

The same API works on both form factors. The geometry is shared, and only the screen rectangle differs. On the desktop, a
scene window is a floating window below the menu bar and cascades from its corner. Pointer and keyboard focus use the
window server's normal rules, and the registry's focus follows the shell's `focus` call. On the tablet the screen is the
whole panel, and the cascade starts at the top-left corner. No gesture is added for scene windows yet.

## What is not done

- The desk holds one window per app (each desk window names its app, and the windows service lists one per app). A scene window therefore has no surface yet.
  The next step is a second entry per app in the desk, using `SceneStack` for the stacking order.
- Scene windows are not in the `windows` service list. `window_services::records` reads only the desk's app windows.
- The table lives in memory. Scene windows do not survive a shell restart; the app's restorable state brings them back
  through `reopen`.
- No keyboard shortcut, touch gesture or Dock menu entry opens a new scene window.
- The `SizeRestrictions` come from the request. There is no per-window size class; the trait collection row is separate.
- Nothing here runs on the tablet yet. The lead builds the crates, runs the unit tests and checks a scene on Midget.

## Tests

Each module has unit tests: the transition table and the registry (`scene_session.rs`), the table and reopen rules
(`scene_services.rs`), the restrictions and cascade (`window_scene.rs`), and the stack order (`scene_stack.rs`).
