#!/usr/bin/env python3
"""Repeatable Midget benchmark: 10s idle + 30 alternating Files selections.
Open Files at Home in grid view, with identical background windows for both runs.
Enable `tools/midget.py trace on` first. Run from the repository root.
Results are one-core CPU percentages; shell draw timings exclude Wayland submission.
"""
import subprocess,time,json,re,statistics,sys
label=sys.argv[1]
window=sys.argv[2] if len(sys.argv)>2 else "External(0)"
if not re.fullmatch(r"[a-zA-Z0-9_-]+",label): raise SystemExit("Use a simple benchmark label")
from pathlib import Path
Path("out/render-performance").mkdir(parents=True,exist_ok=True)
hz=int(subprocess.check_output(['ssh','midget','getconf CLK_TCK'],text=True))
def ssh(command): return subprocess.check_output(['ssh','midget',command],text=True)
def cpu():
    return json.loads(ssh("python3 - <<'REMOTE'\nimport glob,json,os\nr={}\nfor p in glob.glob('/proc/[0-9]*/stat'):\n try:\n  s=open(p).read(); n=s[s.index('(')+1:s.rindex(')')]; v=s[s.rindex(')')+2:].split()\n  if n in ('shell','springboard','labwc','bifrost-files'):r[p.split('/')[2]]=[n,int(v[11])+int(v[12])]\n except: pass\nprint(json.dumps(r))\nREMOTE"))
def pct(a,b,seconds):
 r={}
 for pid,(name,ticks) in b.items():
  if pid in a:r[name]=r.get(name,0)+(ticks-a[pid][1])/hz/seconds*100
 return r
start=int(ssh('date +%s').strip())
a=cpu();t=time.monotonic();time.sleep(10);b=cpu();idle=pct(a,b,time.monotonic()-t)
a=cpu();t=time.monotonic()
for i in range(30):
 subprocess.run(['python3','tools/midget.py','tap-window',window,str(345 if i%2==0 else 485),'215'],stdout=subprocess.DEVNULL,check=True)
 time.sleep(.35)
b=cpu();elapsed=time.monotonic()-t
logs=ssh(f'journalctl -u springboard --since @{start} --no-pager -o cat')
open(f'out/render-performance/{label}.log','w').write(logs)
frames=[float(v) for v in re.findall(r'springboard: frame draw ([0-9.]+) present',logs)]
result={'label':label,'idle_cpu_percent_one_core':idle,'selection_cpu_percent_one_core':pct(a,b,elapsed),'selection_elapsed_s':elapsed,'shell_draw_ms_median':statistics.median(frames) if frames else None,'shell_draw_ms_p95':sorted(frames)[min(len(frames)-1,int(len(frames)*.95))] if frames else None,'frames':len(frames),'app_stats':re.findall(r'app frames:.*',logs)}
open(f'out/render-performance/{label}.json','w').write(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
