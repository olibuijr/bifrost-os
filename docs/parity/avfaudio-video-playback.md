# AVFAudio: playback through the video pipeline

Mirrors the audio output of a media player: the sound of a video goes to an output the app chooses.

## What is built

- `springboard/src/media/player.rs`: `Player::open_audio(path, audio)` opens a video with a caller-supplied `AudioOut`.
  `Player::open` now delegates to it with the speaker, so the shell's behaviour is unchanged.
- `springboard/sdk/src/video_playback.rs` (`bifrost_app::video_playback`): `VideoAudio` (`Speaker` or `Silent`),
  `audio_output`, `open`, and `VideoError` (`NotFound`, `Unreadable`), with the I/O error kinds mapped to typed errors.
- Tests: `springboard/sdk/tests/parity_sdk-rest-07-video.rs`, headless, using paths that do not exist.

## What needs the lab machine

- Sound from a real video on the speaker route, and the clock of a silent video, are checked only on Midget.
- The shell's video screen still opens its own player; it does not use this facade yet.
- Decoder choice is the player's (`default_decoder`); apps cannot pass their own decoder through the SDK yet.
