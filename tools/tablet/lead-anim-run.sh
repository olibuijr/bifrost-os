#!/usr/bin/env bash
# Tab A8: open an app, go home, open it again, open the switcher; print per-phase frame stats from the trace log.
# Needs ANDROID_SERIAL and TAB_CLI (see lead-anim-bench.sh). The raw trace goes to ANIM_OUT.
set -euo pipefail
: "${ANDROID_SERIAL:?set ANDROID_SERIAL to the tablet adb address}"
: "${TAB_CLI:?set TAB_CLI to the Tab A8 tools/tab.py}"
export ANDROID_SERIAL
ANIM_OUT="${ANIM_OUT:-$HOME/.local/share/bifrost-os/anim.txt}"
mkdir -p "$(dirname "$ANIM_OUT")"
T="python3 $TAB_CLI"; mark() { adb shell "echo MARK_$1 >> /data/local/springboard/log"; }
$T key home >/dev/null 2>&1; sleep 2.5
mark open; $T tap 426 156 >/dev/null; sleep 2
mark home; $T swipe 600 1915 600 1300 220 >/dev/null; sleep 2
mark open2; $T tap 426 156 >/dev/null; sleep 2
mark switcher; $T swipe 600 1915 600 1000 600 >/dev/null; sleep 2.5
mark end
adb shell 'sed -n "/^MARK_open$/,\$p" /data/local/springboard/log | grep -E "^MARK|frame draw [0-9.]+ present"' > "$ANIM_OUT"
ANIM_OUT="$ANIM_OUT" python3 - <<'P'
import os, re
ph=None; d={}
for l in open(os.environ['ANIM_OUT']):
    if l.startswith('MARK'): ph=l.strip(); d[ph]=[]; continue
    m=re.search(r'draw ([\d.]+) present ([\d.]+) damage (\d+)% gap ([\-\d.inf]+)',l)
    if m and ph and float(m.group(3))>0: d[ph].append((float(m.group(1)),float(m.group(2)),float(m.group(4))))
for k,v in d.items():
    act=[x for x in v if x[2]<200]
    if not act: print(k,'-'); continue
    tot=[a+b for a,b,_ in act]
    print('%-14s frames %3d  draw avg %5.1f max %5.1f  total>16.7ms: %2d  period>20ms: %2d'%(k,len(act),sum(a for a,_,_ in act)/len(act),max(a for a,_,_ in act),sum(1 for t in tot if t>16.7),sum(1 for *_,g in act if g>20)))
P
