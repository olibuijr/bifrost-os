#!/bin/bash
# Kernel latency A/B on Midget: scheduling latency (cyclictest) idle and under stress, then real ALSA playback
# xruns at small periods under the same stress. Usage: kbench.sh LABEL [SECONDS]. Results in /root/kbench/LABEL.
set -u
L=$1; D=${2:-60}; O=/root/kbench/$L
rm -rf "$O"; mkdir -p "$O"
{ uname -a; cat /proc/cmdline; grep -o 'Dynamic Preempt: [a-z]*' <(dmesg) | tail -1; } > "$O/kernel.txt"
STRESS="stress-ng --cpu 8 --io 2 --vm 2 --vm-bytes 256M --hdd 1 --hdd-bytes 64M --timer 4 --switch 2"

ct() { cyclictest -m -S -p 95 -i 200 -h 3000 -q -D "${D}s" > "$O/$1.txt" 2>&1; }

ct cyclic-idle
$STRESS --timeout $((D * 5 + 60))s >/dev/null 2>&1 &
SP=$!
sleep 5
ct cyclic-stress

# Real audio: playback straight to the HDA analog output at period P with 2 periods, under the stress above.
# aplay prints "underrun!!!" for every xrun; -v shows the period and buffer the hardware granted.
for P in 32 64 128; do
  timeout "${D}s" chrt -f 80 aplay -v -D hw:0,0 -f S16_LE -r 48000 -c 2 \
    --period-size=$P --buffer-size=$((P * 2)) /dev/zero > "$O/aplay-$P.txt" 2>&1
done
kill $SP 2>/dev/null; pkill -x stress-ng 2>/dev/null
echo done > "$O/DONE"
