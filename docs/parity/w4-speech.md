# Wave 4: Speech parity (microphone consent, speech model assets)

Branch `parity/w4-speech`, from `parity/integration`. Two gap rows of the Speech framework were built as pure Rust
models. No builds or tests were run for this wave; the integration build runs the tests.

## What was built

- `springboard/ui/src/mic_consent.rs` (`bifrost_ui::mic_consent`, SDK `bifrost_app::microphone`): the microphone
  as an app sees it. It holds the person's consent answer (asked once, `Undetermined` / `Granted` / `Denied`), the input
  mute switch, and the one app that holds the recording. `begin(app)` checks consent, then mute, then ownership;
  `end(app)` releases the recording, and only the holder can end it. The audio service still opens the microphone; the
  shell reports the prompt answer and an app asks `begin` before it sends `RecordStart`.
- `springboard/ui/src/speech_assets.rs` (`bifrost_ui::speech_assets`, SDK `bifrost_app::speech_models`): the speech
  model assets of one app. Per-locale install lifecycle (`NotInstalled` -> `Requested` -> `Downloading` -> `Installed`,
  `Failed` can be requested again), progress clamped to the total, finish refused until every byte arrived, removal
  refused while a transfer runs, and a reservation set capped by the caller. Locale tags are checked and folded to
  lowercase.
- `springboard/ui/src/lib.rs`: one `pub mod` line each. `springboard/sdk/src/lib.rs`: two `pub use` re-exports next
  to the audio client.

## Official API names mirrored

- Speech: `AssetInventory` (`assetInstallationRequest`, `reserve(locale:)`, `release(reservedLocale:)`,
  `reservedLocales`, `maximumReservedLocales`, `status(forModules:)`), official page
  https://developer.apple.com/documentation/speech/assetinventory
- AVFAudio: `AVAudioApplication` (`requestRecordPermission`, `recordPermission`, `isInputMuted`, `setInputMuted`),
  official page https://developer.apple.com/documentation/avfaudio/avaudioapplication

The BifrOSt names are plain Rust (`Microphone`, `Consent`, `StartError`, `AssetInventory`, `InstallState`); the mapping
above is documentation only.

## REA evidence

- `ev_eebebac22b960fce0a49de22275800133856f928407e17c0f6f220e43136399b`: rendered page of `speech/assetinventory`
  (asset download, four-step install, reservation rules, status).
- `ev_ad5b0aca3014f6813773af6cb44b58a38f44f505667dd3e18a05ab5b30cd85ce`: rendered page of
  `avfaudio/avaudioapplication` (recording permission, input mute state).

Official DocC facts were read with `python3 -I tools/parity/docc.py speech/assetinventory` and
`python3 -I tools/parity/docc.py avfaudio/avaudioapplication`.

## Matrix rows closed

These rows are for the lead to update in `docs/parity-matrix/`. Status is `partial`, since neither module is wired into
an app or the shell yet, and the models are not yet tested on the VM or Midget.

| Framework slug | Feature (as written in data) | New status | BifrOSt path |
|---|---|---|---|
| speech | Asset and model installation | partial | springboard/ui/src/speech_assets.rs |
| speech | Microphone consent and ownership | partial | springboard/ui/src/mic_consent.rs (SDK: springboard/sdk/src/lib.rs `microphone`) |

## Not done in this wave

- Not wired: no shell prompt, no audio-service change, and no app calls `begin` or `request` yet.
- The speech recognizer, the system dictation key and the recognition engine are still missing (rows left as they are).
- The legacy recognizer API row stays a gap; it is not a Rust concept.
