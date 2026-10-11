#!/bin/bash
# Tab A8: read-only dump of the audio chain over USB: capture-audio.sh NAME
# Needs ANDROID_SERIAL. Output goes to CAPTURE_DIR/NAME (default ~/.local/share/bifrost-os/audio-capture).
set -euo pipefail
: "${ANDROID_SERIAL:?set ANDROID_SERIAL to the tablet adb serial}"
D="${CAPTURE_DIR:-$HOME/.local/share/bifrost-os/audio-capture}/$1"; mkdir -p "$D"
A="adb -s $ANDROID_SERIAL shell"
$A "/data/local/springboard/audiotest controls" > "$D/controls.txt" 2>&1
$A "cat /proc/interrupts" > "$D/interrupts.txt"
$A "for i in 1 2 3; do cat /proc/asound/card0/pcm0p/sub0/status /proc/asound/card0/pcm3p/sub0/status 2>&1; sleep 1; done; cat /proc/asound/card0/pcm0p/sub0/hw_params /proc/asound/card0/pcm0p/sub0/sw_params /proc/asound/card0/pcm3p/sub0/hw_params" > "$D/pcm.txt" 2>&1
for d in 3-0034 3-0035 3-0036 3-0037; do $A "cat /sys/bus/i2c/devices/$d/reg" > "$D/amp-$d.txt" 2>&1; done
$A "ls /sys/kernel/debug/asoc /sys/kernel/debug/regmap 2>&1 | head -50" > "$D/debugfs.txt" 2>&1
$A "dmesg | grep -vE 'wlan|wpa|WCN|charger|fgu|sgm4154|servicemanager|init:|gxy' | tail -400" > "$D/dmesg.txt" 2>&1
$A "ps -A | grep -iE 'audio|media|vendor.sprd|agdsp|modem' " > "$D/ps.txt" 2>&1
echo "done $D"
