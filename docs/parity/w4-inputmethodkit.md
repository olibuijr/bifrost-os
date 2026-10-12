# InputMethodKit parity, wave 4

Pure Rust models for the provider side of text input. They are host-tested and not yet wired into the shell or apps,
so the matrix rows move to `partial`, not `parity`.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/imsession.rs` (`InputSession`) | `IMKInputController`, `IMKServerInput` composition queries, `commitComposition`, `updateComposition`, `cancelComposition`, `candidateSelected`, `inputControllerWillClose` | One session per client. Keeps the raw keystrokes (original string), the converted text (composed string), a candidate list and a candidate window model (`candidates.rs`). Commit returns the converted text, cancel returns the raw keystrokes, close returns any pending text and refuses later updates. |
| `springboard/ui/src/imroute.rs` (`route_key`, `route_mouse`) | `IMKServerInput` key binding (`inputText(_:client:)`, `didCommand(by:client:)`), `handle(_:client:)`, `inputText(_:key:modifiers:client:)`, `IMKMouseHandling` | Decides where a key down goes: provider (handle or text data mode), command from the shortcut registry, plain text, or the field. `route_mouse` sends a mouse down to the provider only over the marked range when the provider handles mouse events. |
| `springboard/ui/src/imstate.rs` (`MethodState`) | `IMKStateSetting` (`activateServer`, `deactivateServer`, `recognizedEvents`, `modes`, `value(forTag:client:)`, `setValue(_:forTag:client:)`) | Active flag, recognized event mask, named input modes with one current mode, and per-client values under integer tags. A closed client's values are dropped with `forget_client`. |

Only `springboard/ui/src/lib.rs` changed (three `pub mod` lines after `candidates`).

## Official API names mirrored

- InputMethodKit landing page: https://developer.apple.com/documentation/inputmethodkit
- IMKInputController: https://developer.apple.com/documentation/inputmethodkit/imkinputcontroller
- IMKServerInput: https://developer.apple.com/documentation/inputmethodkit/imkserverinput
- IMKStateSetting: https://developer.apple.com/documentation/inputmethodkit/imkstatesetting
- IMKMouseHandling (docc only, not REA-recorded): https://developer.apple.com/documentation/inputmethodkit/imkmousehandling

## REA evidence

| Page | evidence_id |
|---|---|
| InputMethodKit | ev_4f76b381a49e71bf77bec3bd4cf89ff199682ce9f119dac1893dd155febd1ba1 |
| IMKServerInput | ev_7aa64715087049591aa99482930a721ab4c0b6a5a2bd62e0d169dc1fc722ec5b |
| IMKInputController | ev_174418e7339be3454e258a6cc496de04ac01010826df86b75531ae6406a54b03 |
| IMKStateSetting | ev_b589d4abcb28d6a9be365d9f45c83794e3709fcfc5aef9a2b45d824095483ed4 |

## Matrix rows closed (new status partial)

| Framework | Feature (as in data/inputmethodkit.json) | Status | BifrOSt path |
|---|---|---|---|
| InputMethodKit | Per-session input controller | partial | springboard/ui/src/imsession.rs |
| InputMethodKit | Composed, original string and candidate queries | partial | springboard/ui/src/imsession.rs |
| InputMethodKit | Key binding and text events to the input method | partial | springboard/ui/src/imroute.rs |
| InputMethodKit | Input method state | partial | springboard/ui/src/imstate.rs |

Not claimed: "Input method server and connections" (no shell input_method server), "Mouse handling for input methods"
(`route_mouse` exists but nothing calls it yet), "Input method registration and Info dictionary" (no registration).

## Not done

- The shell does not create `InputSession` or `MethodState`, and the on-screen keyboard still does not emit Preedit.
- No server-side connection, so a third-party input method cannot connect.
- Candidate drawing and placement remain as wave 3 left them.
