# Tablet bring-up scripts

Scripts for the Samsung Tab A8 build of the springboard shell, driven over adb.

| Script | What it does |
|---|---|
| `camprobe.sh BIN [args]` | Cross-builds a springboard binary for aarch64 Android, pushes it and runs it. |
| `lead-anim-run.sh` | Opens, leaves and reopens an app, then the switcher, and prints per-phase frame stats. |
| `lead-anim-bench.sh` | Pager swipes plus the shell's own frame averages. |
| `capture-audio.sh NAME` | Read-only dump of the audio chain (mixer, PCM, amplifier registers, dmesg). |

Environment:

- `ANDROID_SERIAL`: the tablet's adb address or serial (required).
- `TAB_CLI`: path to the Tab A8 `tools/tab.py`, which the two `lead-anim` scripts call for taps and swipes.
  That CLI lives with the Tab A8 project, not in this repository (required by those two scripts).
- `ANDROID_NDK_HOME`: optional; defaults to the newest NDK under `~/Android/Sdk`.
- `ANIM_OUT`, `CAPTURE_DIR`: optional output locations.
