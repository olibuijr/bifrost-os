#!/usr/bin/env bash
# Tab A8: 16 pager swipes (~240 animated frames), then the shell's own 120-frame window averages.
# Needs ANDROID_SERIAL (the tablet adb address) and TAB_CLI (path to the Tab A8 tools/tab.py).
set -euo pipefail
: "${ANDROID_SERIAL:?set ANDROID_SERIAL to the tablet adb address}"
: "${TAB_CLI:?set TAB_CLI to the Tab A8 tools/tab.py}"
export ANDROID_SERIAL
T="python3 $TAB_CLI"
for i in $(seq 8); do $T swipe 1000 900 200 900 250 >/dev/null; sleep 0.9; $T swipe 200 900 1000 900 250 >/dev/null; sleep 0.9; done
adb shell 'grep "frame draw" /data/local/springboard/log | tail -2 | cut -c1-90'
