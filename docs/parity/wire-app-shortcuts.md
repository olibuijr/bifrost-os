# Wire: Shortcuts app (automation parity)

Shortcuts 1.3.0 (`springboard/apps/shortcuts-desktop/`) now uses three parity models that were only host-tested
before: the parameter resolution model, the workflow model and the scripting model. Nothing new was written as a
model; the app adds small adapters (`src/automation.rs`) over the existing ones. Std only, no unsafe, and the only
manifest change is the two `[[intent]]` entries plus the version. The change is not built yet: the lead's build
runs the app's unit tests (`cargo test -p bifrost-shortcuts`).

Matrix rows are not edited here; `docs/parity-matrix/` is updated centrally.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | Model used | BifrOSt path |
|---|---|---|---|
| siri-kit | Parameter resolution, confirmation and disambiguation | `bifrost_app::controls::intent_resolve` (`ParamSpec`, `Kind::Entity`, `resolve`) | `src/automation.rs` `resolve`, used by `run-shortcut` and `plan-shortcut` |
| app-intents | Intent parameters (entity type) | `intents::Param` with `ParamType::Entity("Shortcut")` | `src/automation.rs` `declared`, `entities`; `src/main.rs` `intents`, `entities` |
| app-intents-testing | Intent test helpers (typed result from a run) | `intents::run`, `intents::dispatch`, `IntentResult::at` | unit tests in `src/automation.rs` |
| automator | Workflow and controller (run of ordered steps) | `controls::workflow::{Workflow, WorkflowController, Step}` | `src/automation.rs` `plan_text` |
| scripting-bridge | Class mapping, properties, element lookup, error reporting | `controls::scripting::{Dictionary, Target, Object}` | `src/automation.rs` (`dictionary`, `script_get`, `script_set`); `src/main.rs` `rpc` |

The intent ids are `run-shortcut` and `plan-shortcut`. The scripting methods are `script.dictionary`, `script.get`
and `script.set`. `src/widget.rs` gained `list()` and `edit_list()` so the open window and the intents and scripts
share one list and one save path.

### Behaviour

- **Resolution.** The `shortcut` parameter is an id (the index shown as `id` in `entity.query`) or a name. An exact
  id or an exact name wins. A partial name matches when only one shortcut contains it. Several matches stop with
  the list of ids to pick from. No match stops with "no matching item".
- **Plan (dry run).** `plan-shortcut` returns the steps in order, one line per action (`Set Wi-Fi: Off`). The
  output is the last step's result of a workflow where each step appends its line. A shortcut with no actions
  stops with "has no actions".
- **Run.** `run-shortcut` resolves the shortcut and then replies that it cannot run on this device yet. It never
  reports a successful run. The window's Run button still shows its own "cannot run" toast.
- **Scripting.** A `shortcut` object has `name` and `color` (read-write; colour is a palette index 0 to 7) and
  `actions` (read-only count). Shortcuts are found by name; with duplicate names the first one. Values are checked
  the way the window checks them (non-empty name of at most 40 characters without control characters). A failed
  change leaves the list untouched. With a window open the change goes through the window and is saved; with no
  window the saved file is changed, so the next launch shows it.

## How a user reaches it

- Other apps and tools use the app's control socket: `intent.list`, `intent.plan-shortcut`,
  `intent.run-shortcut`, `entity.query` with `type: "Shortcut"`, and `script.*` (see the `bifrost-app-control`
  skill). The manifest lists the two intents for Search.
- A person in the Shortcuts window uses the same list; edits made through `script.set` appear at once.
- Nothing in the window itself shows a disambiguation prompt yet. The window's own Run button still shows the
  "cannot run" toast.

## How to verify on the test PC (Midget)

Midget is the only test machine; the VM is not used.

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. `bifrost pc midget open-app is.olibuijr.shortcuts.desktop`, then
   `bifrost pc midget screen /tmp/shortcuts-1.3.0.png`. The window must look as before (sidebar, toolbar, tiles).
3. Through the control socket of the open app (`bifrost pc control methods` then `call`):
   - `intent.list` shows `run-shortcut` and `plan-shortcut`.
   - `intent.plan-shortcut` with `{"shortcut": "Bedtime"}` returns the three lines of the default Bedtime shortcut.
   - `intent.run-shortcut` with `{"shortcut": "Bedtime"}` returns the "cannot run on this device yet" error.
   - Create two shortcuts whose names both contain "Save", then `intent.run-shortcut` with `"Save"` returns the
     choose-an-id error listing both ids.
   - `entity.query` with `{"type": "Shortcut", "string": "bed"}` returns the Bedtime entity.
   - `script.get` `{"shortcut": "Bedtime", "property": "actions"}` returns 3; `script.set` the name to "Night"
     makes the window show "Night" at once; `script.set` `actions` is refused.
4. Screenshot the window after step 3 for the APP-02 check.

Host unit tests (for the build stage): `cargo test -p bifrost-shortcuts` runs `automation::tests` (intent
resolution, dry run, dispatch, entity query, scripting, and the manifest/declaration match) and the existing model
tests.

## REA evidence

- `ev_410046ef507f693f7917b8b944b3ccda05f53c5f35f94875ca3b9e5204b93c1e`: rendered page
  https://developer.apple.com/documentation/appintents/intentparameter (captured for this change). Its text lists
  the primitive, framework and custom parameter types and the request-value and confirmation methods used by the
  resolution model. DocC facts (`tools/parity/docc.py appintents/intentparameter`): a property wrapper for an input
  argument of an intent.
- Earlier evidence reused for the other rows, from `docs/parity/app-intents.md`, `app-intents-testing.md` and
  `automator.md`: `ev_c441b370fcc37cac5c16394d85f699df60d53a7ab076421d2ed3b4044f81eeb0` (App Intents landing page),
  `ev_aa9407bd3a501bfe1821013f0787fe5c21fb49af97d4c4a41c99b653cb3d977f` (App Intents Testing landing page),
  `ev_a34202871a969c8341f73586cb24c93d7dc9782b14d30adf09f095d151b2859d` (Automator landing page).
- `ev_17fa3380a6666bef213625b91e2abcb65a786d5b82e9c31d5304a03b04bb45ba` (Scripting Bridge landing page, from
  `scripting-bridge.md`).

## Remaining gaps

- The shell cannot run shortcut actions, so `run-shortcut` only resolves and reports. Running needs an action
  runner in the shell (Wi-Fi, Low Power, Do Not Disturb, brightness, open app, screenshot, timer).
- The window has no disambiguation or parameter prompt; resolution is reachable only through the control socket.
- The resolution model has no typed Double, Date or Currency parameter in this app; only the entity parameter is
  used. Confirmation (`needs_confirmation`) is not requested.
- Donations, relevance ranking and the authentication policy at dispatch are not used.
- Workflow steps do nothing but describe actions: no variables between steps and no process running.
- Scripting: no per-object last error (the error is returned per call), no `array(byApplying:)` filtering, and no
  dictionary served as a file. Duplicate names are found by the first match only. The locator base (0) is the
  model's choice; the official text does not state it.
- Not built or run in this stage. Cargo.toml gained the in-repo `bifrost-core` path dependency (for `Json`), as the
  calculator app has; the lead's build updates `Cargo.lock`.
