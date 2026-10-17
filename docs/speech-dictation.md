# Speech: text to speech, live recognition and Dictation

Code: `springboard/src/shell/services/speech.rs` (the `speech` service, text to speech), `services/recognizer.rs` (the
`speech_recognizer` service, live recognition), `pages.rs` and `settings.rs` (Keyboard > Dictation). SDK clients are
`bifrost_app::speech` and `bifrost_app::speech_whisper`.

## Text to speech (`services/speech.rs`)

- Uses the installed engine, `espeak-ng` or `espeak`. Each app has at most one utterance: a new `speak` replaces the
  current one. Stopping, or the app quitting, kills the child process.

## Live recognition (`services/recognizer.rs`)

- A `gst-launch-1.0` pipeline captures the microphone and feeds a whisper element with a local model. No audio leaves the
  machine.
- Consent comes first. Settings > Keyboard > Dictation records the person's answer, then starts the model download in a
  thread. "Remove" withdraws consent and removes the model.
- Recognition is refused without consent, without the model installed, for a locale other than English, or when the
  pipeline program is missing. Each app has at most one recognition; a new `start` replaces it, and the shell ends it when
  the app quits.
- Only final transcripts are results. Hypotheses are not reported as text.

## Who uses it

- Dictation is the Settings page for the model. The voice-control parser (`accessibility/voice_control.rs`) reads final
  transcripts, but no Voice Control session is started yet (see `docs/accessibility-input.md`).
