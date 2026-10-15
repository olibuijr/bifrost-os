# Parity sdk-rest-02

Five matrix rows were `os` after earlier rounds. This round builds two thin SDK surfaces and records blockers for the rest.

## Built

| Row | Module | What it mirrors | Wired | Needs the lab machine |
|---|---|---|---|---|
| core-media: Video decoding and picture hand-off | `springboard/sdk/src/video_frame.rs` | Picture hand-off: a format (codec, size, pixel format), frames that must match it, presentation order | Uses `bifrost_app::pixel_buffer::PixelBuffer` and `bifrost_app::video_decode::Codec`. The decoder still runs in the shell. | Confirm that a decoded picture can reach an app process. |
| foundation-networking-lowlevel: launching child processes | `springboard/sdk/src/child_process.rs` | A checked launch (absolute program, NUL-free arguments, valid environment names), exit code or signal, captured output | Uses `std::process::Command`. Inherits the app environment. | Confirm that the child runs under the shell's sandbox policy. It does not today. |

Tests: `springboard/sdk/tests/parity_sdk-rest-02.rs`. They run headless. The process tests start only `/bin/sh`.

## Left as os, with the blocker

- Audio output to the speaker: an SDK client needs the audio service's request protocol, to be checked on the lab machine.
- Media capture: camera frames have no channel to an app process, and the camera permission flow sits in the shell.
- Markup view controller: the interactive editor needs the shell's touch handling split into a pure model first.
