#!/usr/bin/env python3
"""docc.py PATH [--depth 1|2]  Official Apple documentation (the DocC JSON developer.apple.com renders).
PATH like 'appkit' or 'appkit/nsundomanager'. Prints title, abstract, macOS availability and the topic
sections with each symbol's title, kind and abstract. --depth 2 also expands each child collection one level."""
import json, sys, urllib.request
def get(p):
    u = f"https://developer.apple.com/tutorials/data/documentation/{p.lower().strip('/')}.json"
    with urllib.request.urlopen(u, timeout=30) as r: return json.load(r)
def txt(a): return ''.join(x.get('text', '') or (x.get('code') or '') for x in (a or []))
def show(p, depth, indent=''):
    d = get(p); m = d.get('metadata', {}); refs = d.get('references', {})
    mac = [f"{x['name']} {x.get('introducedAt','')}{' deprecated '+str(x.get('deprecatedAt')) if x.get('deprecated') else ''}" for x in m.get('platforms', []) if x.get('name') in ('macOS', 'Mac Catalyst')]
    print(f"{indent}# {m.get('title')} ({m.get('role') or m.get('symbolKind')}) [{', '.join(mac)}] https://developer.apple.com/documentation/{p.lower()}")
    if txt(d.get('abstract')): print(f"{indent}  {txt(d.get('abstract'))}")
    for s in d.get('topicSections', []):
        print(f"{indent}## {s.get('title')}")
        for i in s.get('identifiers', []):
            r = refs.get(i, {}); t = r.get('title') or i.rsplit('/', 1)[-1]
            print(f"{indent}  - {t} ({r.get('kind','')}/{r.get('role','')}) {txt(r.get('abstract'))[:160]}")
            if depth > 1 and r.get('role') in ('collectionGroup', 'collection', 'article') and '/documentation/' in i:
                try: show(i.split('/documentation/', 1)[1], depth - 1, indent + '    ')
                except Exception as e: print(f"{indent}    (could not expand: {e})")
depth = int(sys.argv[sys.argv.index('--depth') + 1]) if '--depth' in sys.argv else 1
show(sys.argv[1], depth)
