# Parity sdk-os-03: OS-handled rows that gained SDK models

## What the models mirror

- `bifrost_app::sample_buffer` (`springboard/sdk/src/sample_buffer.rs`): one coded sample with its timing (presentation,
  decode, duration), its sync flag and its bytes, mirroring the timing queries of a sample buffer. It converts the
  demuxer's `springboard::media::demux::Sample`. Presentation ordering of a batch is `sort_by_presentation`.
- `bifrost_app::background_download` (`springboard/sdk/src/background_download.rs`): a download spec with an exact size and
  a SHA-256 digest. `stage` verifies a payload, writes it to `<name>.part` and renames it, so a failed payload never reaches
  its final name. The same sequence is used by the shell's system update (`springboard/src/shell/update.rs`).
- `bifrost_app::crash_report` (existing): `CrashReason` and `ProcessCrash::reason` already carry the crash reason text.
  The row now cites them, and the headless tests in `springboard/sdk/tests/parity_sdk-os-03.rs` cover the reason.

## What is wired

Nothing in the shell or apps calls these models yet. The sample model reads the demuxer's output, and the download
model accepts bytes from any caller.

## What remains

- Transfers, background scheduling and resume are not in the SDK. The caller supplies the payload.
- No shared sample buffer object, data-ready callbacks or sample attachments.
- The shell does not pass a crashed app's reason to an app's reporter.
- Rows still `os` in this task: video picture hand-off, audio output, media capture, device and drawable setup, vertex
  batching, tar package indexing, file-sharing staging, the speaker service, keyboard and mouse game input, the crash
  record, the Console crash view and the markup controller. Each needs a hardware, shell-internal or service decision first.
