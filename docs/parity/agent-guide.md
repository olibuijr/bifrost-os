# Parity agent guide

For coders in a parity wave. You write code and commit on your own branch. You never build, test, run a checker or touch the
lab machine: a hook blocks it, and the lead builds and tests everything after merging. Read neighbouring code very carefully so
the code compiles the first time.

## What the work is for

BifrOSt mirrors macOS 27. The parity matrix under `docs/parity-matrix/` lists, row by row, what the reference system has and what
BifrOSt has. A row has a status: `gap` (nothing), `partial` (code exists), `parity` (verified on the lab machine, set only by
the lead), `os` (handled by the OS rather than an SDK type), `n/a` (does not apply). Your job is to make rows better:

- gap row: write the capability, then the row becomes `partial`;
- partial row that cannot be verified yet: write the missing tests so the lead's verifier can run them;
- partial row with missing behaviour: finish the behaviour.

## Where things are

| Need | Location |
|---|---|
| Repo rules, design rules, name guard | `AGENTS.md`, `CLAUDE.md` (Design section), `docs/design-golden-gate.md` |
| SDK crates | `springboard/sdk/src` (`bifrost_app`), `springboard/ui/src` (`bifrost_ui`), `springboard/core/src` (`bifrost_core`), `springboard/render/src` (`bifrost_render`), `springboard/layers/src` (`bifrost_layers`), `springboard/proto/src` (`bifrost_proto`) |
| Shell | `springboard/src/shell/` |
| Apps | `springboard/apps/<name>-desktop/` (a few are `springboard/apps/<name>/`) |
| SDK API docs | `docs/bifrost-ui.md` |
| Matrix rows | `docs/parity-matrix/data/<slug>.json` (SDK), `docs/parity-matrix/apps/<slug>.json` (apps), `docs/parity-matrix/os/<category>/*.json` (OS layer) |
| Matrix rules and row schema | `docs/parity-matrix/README.md`, `docs/parity-matrix/agent-brief.md` |
| How the verifier finds a row's tests | `tools/parity/verify.py` (`plan_row`, `classify`) |
| Code search | the `akurai_index` tools: `code_search`, `symbol_lookup`, `search_all` (repo `bifrost-os`); load them with ToolSearch |

## Rules that always apply

- Work only in your own worktree, and use absolute paths for every file and every git command.
- Name guard: no platform or product names in code, comments, identifiers or strings, and none of the reference system's type
  prefixes or brand names. Use BifrOSt terms ("search panel", "the reference app"). Matrix JSON rows may name the reference API in their
  `apple` field because that field is data; code, tests and docs may not.
- Tests are TDD and deterministic: no clock, no network, no display, no device, no randomness.
- No stubs, no `todo!()`, no commented-out code, no dead code without a reason.
- Commit messages and comments are plain, professional prose. Subject in conventional form (`feat(x): ...`, `test(x): ...`), a body that says
  what changed and why, and the final line `Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>`.
- Never reformat or re-serialise JSON. Edit matrix rows in place with the Edit tool so only the changed lines differ.

## Example 1: a matrix row before and after (SDK data file)

Edit only the fields that changed. Status stays `partial` unless you took a `gap` row to `partial`. Cite real paths.

```json
{
  "feature": "App Transport Security (secure network links)",
  "apple": "<the reference API names, as already written in the row>",
  "bifrost": "springboard/sdk/src/url.rs; springboard/src/shell/update.rs (HTTPS only)",
  "status": "partial",
  "note": "The shell's update client is HTTPS only. The SDK URL client uses rustls by default."
}
```

becomes

```json
{
  "feature": "App Transport Security (secure network links)",
  "apple": "<the reference API names, as already written in the row>",
  "bifrost": "springboard/sdk/src/url.rs (parse_url, resolve)",
  "status": "partial",
  "note": "Scheme and TLS choice, port defaults and redirect resolution are tested headless in url::transport_security_tests. Gap: redirects may still move an https request to http, and there is no manifest-level exception list."
}
```

A gap row takes the same shape: `"status": "gap"` becomes `"partial"`, `bifrost` names the new file (`path::symbol` is fine), and `note` is one honest
sentence about what works and what remains.

## Example 2: a logic test the verifier can run (integration test in a new file)

New tests go in new files so parallel agents do not collide: `springboard/<crate>/tests/parity_<TASK ID>.rs`. The verifier links a row to
tests that mention the row's module by path, so import with the literal path of the cited file's module
(`springboard/layers/src/layer.rs` is `bifrost_layers::layer`).

```rust
//! Parity sdk-tests-01a: headless tests for the layer material model. No test composes a frame.

use bifrost_layers::geometry::Color;
use bifrost_layers::layer::{Layer, LayerId, Material};

#[test]
fn a_new_layer_has_no_material() {
    let layer = Layer::new(LayerId(7));
    assert_eq!(layer.material, Material::None);
    assert_eq!(Material::default(), Material::None);
}

#[test]
fn glass_materials_compare_by_all_their_parameters() {
    let clear = Color::CLEAR;
    let a = Material::Glass { blur: 10.0, tint: clear, saturation: 1.0 };
    let b = Material::Glass { blur: 12.0, tint: clear, saturation: 1.0 };
    assert_ne!(a, b, "a different blur radius is a different material");
}
```

Name tests as sentences that state the behaviour. Assert concrete values. Each test drives the public API.

## Example 3: a headless drawing test (rows about views)

Rows whose cited code is drawing or window code are promotable only when their tests render to memory and mention `headless`
(or `offscreen`, `raster`, `snapshot`, `pixel`) and never mention a compositor (`wayland`, `wl_`, `drm`, `kms`, `libinput`, `winit`).
The SDK testing kit renders a view tree offscreen:

```rust
//! Headless tests of the view tree through the testing kit: queries, presses and drawn pixels.
use bifrost_app::assert_element;
use bifrost_app::testing::TestWindow;
use bifrost_app::view::{Button, Label, Stack, TextField};
use bifrost_ui::responder::Responder;
use bifrost_ui::Role;

#[derive(Default)]
struct Model {
    actions: Vec<String>,
}
impl Responder for Model {
    fn perform(&mut self, action: &str) -> bool {
        self.actions.push(action.into());
        true
    }
}

fn window() -> TestWindow<Model> {
    let root = Stack::vstack(8.0, vec![Box::new(Button::new("Save", "save")), Box::new(TextField::new("Name", "name")), Box::new(Label::new("Hello"))]);
    TestWindow::new(root, Model::default(), 320, 200)
}

#[test]
fn headless_pressing_a_button_reaches_the_model() {
    let mut w = window();
    assert_element!(w, Role::Button, "Save");
    assert!(w.element(Role::Button, "Save").unwrap().press());
    assert!(w.model.actions.contains(&"save".to_string()));
}

#[test]
fn headless_the_drawn_frame_has_the_window_size() {
    let mut w = window();
    assert_eq!(w.draw().len(), 320 * 200, "one 0xAARRGGBB pixel per point at scale one");
}
```

`TestWindow::draw()` returns the frame as `Vec<u32>`; the API is in `springboard/sdk/src/testing.rs`.

## Example 4: a new SDK module

A model that apps and the shell both need lives in `springboard/ui/src` (never in the shell, because the SDK crate depends on the shell
crate). Register it with one line in the crate's `lib.rs` (`pub mod csv;` style), write the module with its own doc comment, and test it from
`springboard/<crate>/tests/parity_<TASK ID>.rs`. Document it in a new file `docs/parity/<slug>.md` (what it mirrors, what is wired, what needs
the lab machine), not in shared docs.

```rust
//! CSV reader: splits text into records of fields. Quoted fields keep commas, doubled quotes and line breaks.

/// Parses `text` into rows of fields.
pub fn parse_rows(text: &str) -> Vec<Vec<String>> {
    // ...
}
```

## Example 5: wiring a feature into an app menu

Apps build their menu bar with the SDK kit. New commands go in menus, not on the default toolbar (the compliance snapshot would change).
From `springboard/apps/stickies-desktop/src/main.rs`:

```rust
AppMenuItem::new(MENU_NEW, "New Sticky Note").key("cmd+n"),
AppMenuItem::new(MENU_DUPLICATE, "Duplicate Sticky Note").key("cmd+d").enabled(has_focus),
AppMenuItem::new(MENU_DELETE, "Delete Sticky Note").key("cmd+shift+backspace").enabled(has_focus),
AppMenuItem::separator(),
AppMenuItem::new(MENU_IMPORT, "Import Text...").key("cmd+o"),
```

Define the `MENU_*` constant next to the others, handle it in the app's `Input::Menu(id)` match, put the logic in a tested module, and bump the app
version (`Cargo.toml` and `manifest.toml` together, minor) with a `CHANGELOG.md` entry in your last commit. App crates already have the
test kit as a dev-dependency (`springboard = { path = "../..", features = ["testkit"] }`).

## Example 6: a commit

```text
test(layers): headless tests for the layer material model

Cover the default material, the glass parameters and their equality so the
verifier can run the material rows. No test composes a frame.

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
```

## Merge safety

Other agents edit the same crates at the same time. Do not touch `HANDOFF.md`, `docs/apple-parity-matrix.md` (generated), `Cargo.lock`, or files
owned by another task. Add at most a one-line `mod` declaration to a shared `lib.rs` or `mod.rs`. Prefix scratch files with your task id; the scratch
directory is shared.
