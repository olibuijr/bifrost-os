#!/usr/bin/env python3
"""Resolve git conflicts where both sides only insert text into the common base: keep both insertions.

usage: git config merge.conflictstyle diff3   # the base section is needed
       git merge BRANCH                       # conflicts
       python3 tools/git-union.py FILE...     # rewrites each FILE; prints `unresolved N` for hunks it left alone
Hunks where a side also changes or deletes base text stay as conflict markers for a human. Review the diff afterwards:
insertion-only conflicts (duplicate Cargo.toml lines, two `mod tests`) can still need a manual dedupe.
"""
import re,sys
def ins(b,x):
    # x = b[:i] + add + b[i:]? return (i, add) or None
    p=0
    while p<min(len(b),len(x)) and b[p]==x[p]: p+=1
    s=0
    while s<min(len(b)-p,len(x)-p) and b[len(b)-1-s]==x[len(x)-1-s]: s+=1
    if p+s!=len(b): return None
    return p, x[p:len(x)-s]
bad=0
for path in sys.argv[1:]:
    t=open(path).read()
    def fix(m):
        global bad
        o,b,th=m.group(1),m.group(2),m.group(3)
        a=ins(b,o); c=ins(b,th)
        if a is None or c is None: bad+=1; return m.group(0)
        (i,x),(j,y)=a,c
        if i==j: return b[:i]+x+y+b[i:]
        if i<j: return b[:i]+x+b[i:j]+y+b[j:]
        return b[:j]+y+b[j:i]+x+b[i:]
    t=re.sub(r'<<<<<<< [^\n]*\n(.*?)\|\|\|\|\|\|\| [^\n]*\n(.*?)=======\n(.*?)>>>>>>> [^\n]*\n',fix,t,flags=re.S)
    open(path,'w').write(t)
print('unresolved',bad)
