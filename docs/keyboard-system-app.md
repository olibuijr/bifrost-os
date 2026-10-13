# On-screen keyboard as a System App (built 2026-10-09, shell 0.4.77, keyboard 1.1.0)

Status: done in code and tests; the plan below is kept for the decisions. Check 2026-10-13: the package (`springboard/system/keyboard`) is
still 1.1.0 with `min_shell = "0.4.77"` and its CHANGELOG stops at 1.1.0, while the crate `springboard/apps/keyboard` is 1.1.1 in
`Cargo.toml`; the manifest and the crate version should move together on the next keyboard release. Whether 1.1.0 is published
to the store is not recorded in the repo: check `bifrost pc app info keyboard` before relying on "Shipping".

Related, built separately: the composing input methods in `springboard/src/shell/input_methods/` (Hangul 2-set, Telex and VNI) with
an `InputController` shaped like the reference input-method kit, and the keyboard viewer (`shell/keyboard_viewer.rs`). The
controller is not wired into `kb_input.rs` yet, so the keyboard does not route keys to it.

Goal: the keyboard runs as its own package (`is.olibuijr.keyboard`, `kind = "system"`) instead of living in
`springboard/src/shell/keyboard.rs` (2,078 lines) and the `kb_*` modules. Apps keep requesting it
(`ToShell::WantsKeyboard`) but own no keyboard code. The keyboard can then update through the store/OTA path
without a shell restart, like the Settings plan in `docs/settings-system-app.md`.

## What the shell does today (the surface we must replace)
- `Keyboard::new / resize / set_mode / set_predictive / set_return_kind / set_shown / set_auto_caps / set_kb_prefs`:
  the shell configures it from the focused field (mode, return key, caret).
- `Keyboard::down / moved / up / tick / pinch` take touch events; `up` and `tick` return `Out` (text, delete,
  return, dismiss, emoji).
- `Keyboard::draw(surface, ui, oy, t)`: drawn by the shell at `kb_top()` (`mod.rs` ~3250-3374), so it is part of
  the shell frame.
- `caret_moved`, `take_clicks`, `centre` (hwkeys/gesture hooks) also reach into it.
- `hwkeys.rs` and `predict.rs` read the keyboard package's word lists and emoji font (already package assets).

## Steps
1. **Stop the leaks (no behaviour change).** Route every keyboard touch through one `KeyboardHost` trait in the shell
   (`show`, `hide`, `configure(mode, return, caret)`, `touch(ev)`, `frame_top()`), and make `Keyboard` implement it.
   Tests stay green. Fix the current failing keyboard tests (62 at last run) in this step.
2. **Wire protocol (DONE 2026-10-09, API first).** Add to `proto`: `ToShell::Keyboard(Surface…)` is NOT needed; use
   `ToKeyboard::{Configure{mode,return,caret,predictive}, Touch{..}, Show, Hide}` and
   `FromKeyboard::{Text(String), Delete, Return, Dismiss, Height(px)}`, sent over the existing app socket
   (`bifrost_app` runtime). Only the shell and the `kind = "system"` keyboard may use it.
3. **Surface.** The keyboard draws into its own buffer and the shell composites it at the bottom edge (same path
   as the dock layer). Height comes from the keyboard (`FromKeyboard::Height`), so the shell's `kb_top()` follows
   the real height, not a constant.
4. **Crate.** `springboard/apps/keyboard` (workspace member, depends on `render`, `ui`, `sdk`). Move
   `keyboard.rs`, `kb_*.rs`, `predict.rs`, `emoji.rs` into it. The preview/test harness moves with it.
5. **Package.** `system/keyboard/manifest.toml` already exists (assets); add `kind = "system"`, the binary,
   and `provides = ["keyboard"]`. Installed with the other system packages (`proto/src/svc.rs` already lists it).
6. **Apps.** Remove any keyboard-specific state from app code (today only the `WantsKeyboard` call; check
   `apps/*` for `kb`/`keyboard` helpers in step 1). Apps just get `Text`/`Delete`/`Return` as normal input.
7. **Remove the built-in** once the system app passes the same tests. Keep a minimal fallback for recovery.

## Risks
- Latency: every key crosses a socket. Measure key-to-glyph time before and after (`tools/bench.py`).
- Focus: the keyboard must be told about focus changes before the next key, or keys go to the wrong field.
- Predictive text and emoji depend on package assets; a missing package must degrade to plain letters, not a blank bar.
- Shell-only gestures on the keyboard (shift swipe, trackpad, pinch) move with it and need their own tests.

## API as built (step 2)
- `bifrost_proto`: `ToApp::KeyboardConfig(KeyboardConfig { layout: KeyboardLayout, return_key: ReturnKey, flags })`
  (`kbflags::{PREDICTIVE, AUTO_CAPS, ACCESSORY, SHOWN}`) and `ToShell::KeyboardOut(KeyboardOut::{Text, Backspace, Enter, Hide, Named, Height})`.
  Tags 15 and 0x10f; round-trip and rejection tests live in `proto/src/lib.rs`. Touches reuse `ToApp::Touch`/`Configure`.
- SDK (`bifrost_app`): `App::keyboard_config(ctx, config) -> bool` (default no-op) and `Ctx::keyboard_out`,
  `keyboard_insert_text`, `keyboard_delete_backward`, `keyboard_height`.
- Shell: `apphost::KEYBOARD_APP_ID` is the only app whose `KeyboardOut` is forwarded; every other app's is dropped
  (test `only_the_keyboard_system_app_may_type_into_the_focused_field`).
- Apple parity: `UIKeyInput.insertText/deleteBackward/hasText` map to `Text`/`Backspace`; `UIReturnKeyType` to `ReturnKey`
  (Search, Go, Done, Next, Send, Return; Apple's Join, Route, Yahoo, Google, Emergency Call, Continue are not
  mapped: no feature uses them); `UIInputViewController` height to `Height`. `UITextInput` selection and marked text
  (composition) are not carried yet, so predictive replacement still happens inside the shell until step 3.

## As built (steps 1 to 7)
- **Crate** `springboard/apps/keyboard` (`bifrost-keyboard`): `keyboard.rs`, `kb_*`, `predict.rs`, `emoji.rs` and the word lists moved
  there with their tests (25 keyboard tests + emoji/predict). `main.rs` is the `App`: `keyboard_config` applies the field's configuration,
  touches go to the moved `Keyboard`, typed output goes to `ctx.keyboard_out`, the height is reported when it changes.
- **Shell client** `springboard/src/shell/keyboard.rs`: same type name, same calls (`set_shown`, `h()`, `offset`, `down/moved/up`, `tick`,
  `draw`) so Terminal, Browser, Settings search, Wi-Fi, Spotlight, Deildu and Files keep their code. One keyboard process is
  shared per thread (a service); the client shown last owns it. Typed output arrives from `tick`, not from `up`.
- **App backend**: in an app process (the SDK runtime calls `use_app_backend`) the same type asks the shell for the keyboard
  (`WantsKeyboard`), reads its height from `safe_bottom` and draws nothing; the shell draws it over the app and sends text as input.
- **Shell-owned state**: input languages go in `KeyboardConfig` (`languages`, `language`), a globe-key change comes back as
  `KeyboardOut::Language`; the app never reads `kb_layouts.ini` (`kb_layouts::use_remote_state`). Emoji recents live in the app's data
  dir, word lists beside its executable (the sandbox exposes only those). The sandbox's `BIFROST_HARDWARE_KEYBOARD=1` is ignored by it.
- **Package**: `system/keyboard` (`exec = "bifrost-keyboard"`, `min_shell = "0.4.77"`); `tools/pc-app.py publish keyboard` builds the crate and packs
  the system directory. The shell finds it through the install registry (`kind = "system"`, `provides = ["keyboard"]`) and spawns it on demand;
  it restarts the process when the package updates, and 2 s after a crash.
- **Tests**: `apps/keyboard/tests/process.rs` (real process over `AppHost`), `tests/client.rs` (the shell client against the real process:
  show, frame drawn, tap, typed text, hide). In shell unit tests a stand-in replaces the process (`centre(label)` + `up` types the key);
  auto-repeat is tested in the keyboard crate.

## Shipping
1. `tools/pc-app.py publish keyboard` (store publish), 2. `bifrost pc publish`, 3. update Midget (Settings > Software Update; the store check
installs 1.1.0 for the system package). Order matters: a 0.4.77 shell without the 1.1.0 package shows no on-screen keyboard (height 0).
The PC desktop never raises the on-screen keyboard (hardware keyboard), so nothing is visible there until a touch presentation uses it.

## Wayland applications (compositor side, 2026-10-13)

`bifrost-wm` serves `zwp_text_input_v3`, `zwp_input_method_v2` and `zwp_virtual_keyboard_v1` (`docs/window-server.md`, "Text
input, input method and virtual keyboard"). For Wayland applications the shell is the input method: it forwards `activate`,
`deactivate`, the surrounding text and the content type to this app as `KeyboardConfig`, `TextState` and `InputTraits`, and turns
`KeyboardOut` back into `commit_string`, preedit and virtual-keyboard keys. The app itself needs no change.
