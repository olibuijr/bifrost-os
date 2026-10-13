#!/usr/bin/env python3
"""bifrost pc parity: the reference SDK vs BifrOSt parity matrix (docs/parity-matrix/).

Data: docs/parity-matrix/index.json lists every framework the reference vendor documents for its desktop OS 27
(official developer documentation index); docs/parity-matrix/data/<slug>.json holds one framework's (or one part's)
feature rows, filled by agents from the official docs and this repo. This CLI reads them.

  status                         created vs remaining, overall and per group, with coverage
  coverage [--group G] [--sort coverage|name|group] [--min N] [--max N] [--json]
                                 coverage percentage per SDK/API
  show NAME                      one framework's rows (slug or name, case-insensitive substring)
  gaps [--group G] [--status gap,partial]
                                 what remains, most important gaps first
  check                          JSON validity, statuses, cited repo paths exist, every framework assessed
  build [OUT.md]                 regenerate docs/apple-parity-matrix.md
  import DIR [--new-only]        copy agent output files DIR/*.json into data/ (validated; --new-only keeps existing files)
  db-export DIR                  one JSON file per database document for the published artifact
                                 (frameworks/<slug>.json, meta/summary.json) plus batches.json (50 per batch)
  refresh-index                  re-read the official documentation index and rewrite index.json
  search QUERY [--top N] [--status gap,partial]
                                 semantic search over feature rows (Titan embeddings, then rerank)
  related                        nearest frameworks per framework by embedding -> related.json
  themes [--k N]                 cluster Gap rows across frameworks into work themes -> themes.json
  apply FILE.json [--dry-run]    apply closures [{slug, feature, status, bifrost, note}]: each updates the row it
                                 matches (exact feature, else rerank) or is appended as a new row
  verify plan|run|closures       promote partial rows to parity from Midget test evidence (tools/parity/verify.py):
                                 plan [--group G] [--slug S] [--out plan.json] classifies rows logic/ui/hardware/untested;
                                 run PLAN --version X runs the logic rows' tests on Midget -> results.json;
                                 closures RESULTS --version X emits apply closures + needs-manual.md

Coverage of a framework: rows that apply (status not n/a) score parity 1, partial 0.5, os 0.5, gap 0;
coverage = score / applicable rows. A framework with no applicable rows has no coverage (n/a).
"""
import argparse
import collections
import json
import os
import re
import shutil
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BASE = os.path.join(ROOT, "docs", "parity-matrix")
DATA = os.path.join(BASE, "data")
INDEX = os.path.join(BASE, "index.json")
MATRIX_MD = os.path.join(ROOT, "docs", "apple-parity-matrix.md")
RELATED = os.path.join(BASE, "related.json")
THEMES = os.path.join(BASE, "themes.json")
STATUSES = ["parity", "partial", "os", "gap", "n/a"]
LABEL = {"parity": "Parity", "partial": "Partial", "os": "OS only", "gap": "Gap", "n/a": "n/a"}
WEIGHT = {"parity": 1.0, "partial": 0.5, "os": 0.5, "gap": 0.0}
PARTS = {  # large frameworks split by official topic sections; slug prefix -> framework title
    "appkit-": "AppKit", "foundation-": "Foundation", "swiftui-": "SwiftUI", "avfoundation-": "AVFoundation",
}
DOCS_URL = "https://developer.apple.com/tutorials/data/documentation/"
PATH_RE = re.compile(r"(?<![\w/~.:-])((?:springboard|docs|tools|packaging)/[A-Za-z0-9_./-]+)")  # this repo only


def norm(s):
    s = (s or "").strip().lower()
    return {"na": "n/a", "none": "gap", "os only": "os", "missing": "gap"}.get(s, s if s in STATUSES else "gap")


def key(s):
    """Loose match for names and slugs: lower case, letters and digits only."""
    return re.sub(r"[^a-z0-9]", "", (s or "").lower())


def load_index():
    return json.load(open(INDEX))


def load_entries():
    idx = {r["slug"]: r for r in load_index()}
    by_title = {r["title"]: r for r in idx.values()}
    by_key = {key(r["slug"]): r for r in idx.values()} | {key(r["title"]): r for r in idx.values()}
    out, bad = [], []
    for f in sorted(os.listdir(DATA)) if os.path.isdir(DATA) else []:
        if not f.endswith(".json"):
            continue
        slug = f[:-5]
        try:
            d = json.load(open(os.path.join(DATA, f)))
        except Exception as e:  # noqa: BLE001 - report and continue
            bad.append(f"{slug}: {e}")
            continue
        meta = idx.get(slug) or by_key.get(key(slug)) or by_key.get(key(d.get("framework", "")))
        if not meta:
            parent = next((t for p, t in PARTS.items() if slug.startswith(p)), None)
            meta = by_title.get(parent, {})
        rows = [dict(r, status=norm(r.get("status"))) for r in d.get("features") or []]
        c = collections.Counter(r["status"] for r in rows)
        applicable = sum(c[s] for s in WEIGHT)
        score = sum(WEIGHT[s] * c[s] for s in WEIGHT)
        out.append(dict(
            slug=slug, index_slug=meta.get("slug"), name=d.get("framework") or meta.get("title", slug), group=meta.get("group", "Other"),
            parent=meta.get("title", d.get("framework", slug)), since=meta.get("macos_since", ""),
            deprecated=bool(meta.get("deprecated")), status=norm(d.get("status")), doc=d.get("doc") or meta.get("doc", ""),
            bifrost=d.get("bifrost", ""), summary=d.get("summary", ""), rea=d.get("rea_evidence", ""),
            gaps=[g for g in d.get("gaps") or [] if g], rows=rows, counts=c, applicable=applicable, score=score,
            coverage=round(100 * score / applicable, 1) if applicable else None,
        ))
    out.sort(key=lambda e: (e["group"], e["name"].lower()))
    return out, bad


def built_pct(rows):
    """Share of applicable rows that are built (partial or parity): the owner's 80% gate before deploying."""
    n = sum(rows[s] for s in WEIGHT)
    return (100 * (rows["partial"] + rows["parity"]) / n) if n else None


def pct(score, n):
    return f"{100 * score / n:.1f}%" if n else "n/a"


def totals(entries):
    fw = collections.Counter(e["status"] for e in entries)
    rows = collections.Counter()
    for e in entries:
        rows.update(e["counts"])
    applicable = sum(rows[s] for s in WEIGHT)
    score = sum(WEIGHT[s] * rows[s] for s in WEIGHT)
    return fw, rows, applicable, score


def by_group(entries):
    g = collections.defaultdict(list)
    for e in entries:
        g[e["group"]].append(e)
    return dict(sorted(g.items()))


def missing_slugs(entries):
    have = {e["slug"] for e in entries} | {s for e in entries for s in (e.get("index_slug"),) if s}
    parts = {p for p in PARTS if any(s.startswith(p) for s in have)}
    out = []
    for r in load_index():
        if r["slug"] in have:
            continue
        if any(r["title"] == PARTS[p] for p in parts):
            continue
        out.append(r["slug"])
    return out


# --- The OS layer (docs/macos-os-inventory.md): every component of the reference OS image -------------------------
OS_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "docs", "parity-matrix", "os")
OS_STATUSES = ("parity", "partial", "os", "gap", "n/a", "unassessed")


def load_os():
    shards = []
    if not os.path.isdir(OS_DIR):
        return shards
    for cat in sorted(os.listdir(OS_DIR)):
        d = os.path.join(OS_DIR, cat)
        if not os.path.isdir(d):
            continue
        for f in sorted(os.listdir(d)):
            if f.endswith(".json"):
                shards.append(json.load(open(os.path.join(d, f))))
    return shards


def os_totals(entries):
    c = {s: 0 for s in OS_STATUSES}
    for e in entries:
        c[e.get("status", "unassessed") if e.get("status") in OS_STATUSES else "unassessed"] += 1
    applicable = sum(c[s] for s in ("parity", "partial", "os", "gap"))
    score = c["parity"] + 0.5 * (c["partial"] + c["os"])
    return c, applicable, score


def cmd_os_status(a):
    shards = load_os()
    if not shards:
        print("OS layer: no entries yet (tools/parity/os_inventory.py ROOT)")
        return
    by = {}
    for sh in shards:
        by.setdefault(sh["category"], []).extend(sh["entries"])
    allc, alln, alls = os_totals([e for es in by.values() for e in es])
    print(f"OS layer ({shards[0].get('source', '')}): {sum(len(v) for v in by.values())} components, "
          f"{allc['unassessed']} unassessed, coverage {pct(alls, alln)} of {alln} assessed applicable")
    print(f"{'Category':<12} {'All':>6} {'Unass':>6} {'Parity':>6} {'Part':>5} {'OS':>4} {'Gap':>5} {'n/a':>5} {'Cov':>7}")
    for cat, es in sorted(by.items()):
        c, n, sc = os_totals(es)
        print(f"{cat:<12} {len(es):>6} {c['unassessed']:>6} {c['parity']:>6} {c['partial']:>5} {c['os']:>4} {c['gap']:>5} {c['n/a']:>5} {pct(sc, n):>7}")


def os_check():
    problems = []
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    for sh in load_os():
        for e in sh["entries"]:
            if e.get("status") not in OS_STATUSES:
                problems.append(f"os/{sh['category']}/{sh['shard']}: {e['id']}: bad status {e.get('status')!r}")
            for part in re.split(r"[;,]\s*", e.get("bifrost", "") or ""):
                m = re.match(r"(springboard/\S+|tools/\S+|packaging/\S+|docs/\S+)", part.strip())
                if m and not os.path.exists(os.path.join(root, m.group(1).split(":")[0].rstrip(")."))):
                    problems.append(f"os/{sh['category']}/{sh['shard']}: {e['id']}: cited path not found: {m.group(1)}")
    return problems


def cmd_status(a):
    entries, bad = load_entries()
    fw, rows, n, score = totals(entries)
    built = fw["parity"] + fw["partial"] + fw["os"]
    print(f"Assessed: {len(entries)} frameworks/parts; not yet assessed: {len(missing_slugs(entries))}; unreadable: {len(bad)}")
    print(f"Created (parity/partial/os): {built}  [parity {fw['parity']}, partial {fw['partial']}, os {fw['os']}]")
    print(f"Remaining: gap {fw['gap']} frameworks, plus missing features in {fw['partial']} partial ones; n/a {fw['n/a']}")
    print(f"Feature rows: {sum(rows.values())}; applicable {n}; coverage {pct(score, n)}; built {built_pct(rows):.1f}% "
          f"(gate 80%: partial or parity over applicable rows)")
    print()
    print(f"{'Group':<22} {'Cov':>7} {'Parity':>6} {'Part':>5} {'OS':>4} {'Gap':>4} {'n/a':>4}")
    for g, es in by_group(entries).items():
        f2, _, n2, s2 = totals(es)
        print(f"{g:<22} {pct(s2, n2):>7} {f2['parity']:>6} {f2['partial']:>5} {f2['os']:>4} {f2['gap']:>4} {f2['n/a']:>4}")


def cmd_coverage(a):
    entries, _ = load_entries()
    if a.group:
        entries = [e for e in entries if a.group.lower() in e["group"].lower()]
    if a.min is not None:
        entries = [e for e in entries if e["coverage"] is not None and e["coverage"] >= a.min]
    if a.max is not None:
        entries = [e for e in entries if e["coverage"] is not None and e["coverage"] <= a.max]
    key = {"coverage": lambda e: (-1 if e["coverage"] is None else e["coverage"]), "name": lambda e: e["name"].lower(),
           "group": lambda e: (e["group"], e["name"].lower())}[a.sort]
    entries.sort(key=key, reverse=a.sort == "coverage")
    if a.json:
        print(json.dumps([{k: e[k] for k in ("slug", "name", "group", "status", "coverage", "applicable")} |
                          {"counts": dict(e["counts"])} for e in entries], indent=1))
        return
    for e in entries:
        c = e["counts"]
        cov = "n/a" if e["coverage"] is None else f"{e['coverage']:.1f}%"
        print(f"{cov:>7}  {LABEL[e['status']]:<8} {e['name'][:52]:<52} {e['group']:<20} "
              f"P{c['parity']} p{c['partial']} o{c['os']} g{c['gap']} n{c['n/a']}")


def find(entries, name):
    q = name.lower()
    hits = [e for e in entries if e["slug"] == q or e["name"].lower() == q]
    return hits or [e for e in entries if q in e["slug"] or q in e["name"].lower()]


def cmd_show(a):
    entries, _ = load_entries()
    hits = find(entries, a.name)
    if not hits:
        sys.exit(f"no framework matches {a.name!r}")
    for e in hits:
        cov = "n/a" if e["coverage"] is None else f"{e['coverage']:.1f}%"
        print(f"# {e['name']}  [{LABEL[e['status']]}, coverage {cov}, since {e['since']}]  {e['doc']}")
        print(f"  BifrOSt: {e['bifrost']}")
        print(f"  {e['summary']}")
        print(f"  REA evidence: {e['rea'] or 'none'}")
        for r in e["rows"]:
            print(f"  - [{LABEL[r['status']]}] {r.get('feature')}: {r.get('apple')} -> {r.get('bifrost') or '-'}"
                  + (f"  ({r['note']})" if r.get("note") else ""))
        if e["gaps"]:
            print("  Gaps: " + "; ".join(e["gaps"]))
        print()


def cmd_gaps(a):
    entries, _ = load_entries()
    want = set((a.status or "gap,partial").split(","))
    for g, es in by_group(entries).items():
        if a.group and a.group.lower() not in g.lower():
            continue
        es = [e for e in es if e["status"] in want]
        if not es:
            continue
        print(f"## {g}")
        for e in sorted(es, key=lambda e: (e["coverage"] or 0)):
            cov = "n/a" if e["coverage"] is None else f"{e['coverage']:.0f}%"
            print(f"- {e['name']} ({LABEL[e['status']]}, {cov}): " + ("; ".join(e["gaps"][:5]) or "see matrix"))
        print()


def cited_paths(entries):
    ok, missing = 0, []
    for e in entries:
        for text in [e["bifrost"]] + [r.get("bifrost") or "" for r in e["rows"]]:
            for m in PATH_RE.finditer(str(text)):
                p = m.group(1).rstrip(".,;:)").split(":")[0]
                if os.path.exists(os.path.join(ROOT, p)):
                    ok += 1
                else:
                    missing.append((e["slug"], p))
    return ok, missing


def cmd_check(a):
    entries, bad = load_entries()
    problems = list(bad)
    for e in entries:
        if not e["rows"]:
            problems.append(f"{e['slug']}: no feature rows")
        if not e["doc"].startswith("https://developer.apple.com/"):
            problems.append(f"{e['slug']}: doc link is not the official documentation")
    ok, missing = cited_paths(entries)
    problems += [f"{s}: cited path not found: {p}" for s, p in missing]
    miss = missing_slugs(entries)
    problems += os_check()
    print(f"{len(entries)} entries, cited paths ok {ok}, not found {len(missing)}, not assessed {len(miss)}")
    if miss:
        print("not assessed: " + ", ".join(miss))
    for p in problems:
        print("FAIL " + p)
    sys.exit(1 if problems or miss else 0)


def esc(s):
    return str(s if s is not None else "").replace("|", "\\|").replace("\n", " ").strip()


def mark_paths(s):
    def fix(m):
        p = m.group(1).rstrip(".,;:)").split(":")[0]
        return m.group(1) if os.path.exists(os.path.join(ROOT, p)) else m.group(1) + " (not found)"
    return PATH_RE.sub(fix, s or "")


def cmd_build(a):
    entries, bad = load_entries()
    fw, rows, n, score = totals(entries)
    miss = missing_slugs(entries)
    o = []
    w = o.append
    w("# Apple macOS 27 SDK vs BifrOSt: index and parity matrix")
    w("")
    w("Generated by `bifrost pc parity build` from `docs/parity-matrix/` (do not edit by hand; edit the data files).")
    w("What BifrOSt has built against each framework Apple documents for macOS 27 (Golden Gate), and what remains.")
    w("Apple side: the official developer documentation only (developer.apple.com DocC data, read with")
    w("`tools/parity/docc.py`; each framework page was also rendered in Chromium and recorded with REA, whose")
    w("`evidence_id` is listed per framework). BifrOSt side: code and docs in this repo, cited by path; a cited path that")
    w("does not exist is marked \"(not found)\". Rows were filled by agents; treat a row as a pointer to check.")
    w("")
    w("Status: **Parity** app-facing API covers the main features; **Partial** some features; **OS only** the system")
    w("layer has it but apps get no SDK API; **Gap** relevant to a macOS-style desktop and missing; **n/a** tied to")
    w("Apple-only hardware, stores, services or other platforms. Coverage: rows that apply score Parity 1, Partial and")
    w("OS only 0.5, Gap 0, divided by the rows that apply (n/a rows excluded). AppKit, Foundation, SwiftUI and AVFoundation")
    w("are split into parts by their official topic sections.")
    w("")
    w("## Summary: created vs remaining")
    w("")
    w(f"- Coverage over all applicable feature rows: **{pct(score, n)}** ({n} applicable of {sum(rows.values())} rows).")
    w(f"- Built (Partial or Parity) share of applicable rows: **{built_pct(rows):.1f}%**; the deploy gate is 80%.")
    w(f"- Frameworks and parts assessed: **{len(entries)}**.")
    w(f"- Created: **{fw['parity'] + fw['partial'] + fw['os']}** (Parity {fw['parity']}, Partial {fw['partial']}, OS only {fw['os']}).")
    w(f"- Remaining: **{fw['gap']}** Gap, plus the missing features inside the {fw['partial']} Partial ones.")
    w(f"- Not applicable: **{fw['n/a']}**.")
    if miss:
        w(f"- Not yet assessed ({len(miss)}): {', '.join(miss)}.")
    w("")
    w("| Group | Coverage | Parity | Partial | OS only | Gap | n/a |")
    w("|---|---|---|---|---|---|---|")
    for g, es in by_group(entries).items():
        f2, _, n2, s2 = totals(es)
        w(f"| {g} | {pct(s2, n2)} | " + " | ".join(str(f2[s]) for s in STATUSES) + " |")
    w("")
    w("## Index")
    w("")
    w("| Framework | Group | macOS since | Status | Coverage | BifrOSt counterpart | Summary |")
    w("|---|---|---|---|---|---|---|")
    for e in entries:
        cov = "n/a" if e["coverage"] is None else f"{e['coverage']:.0f}%"
        dep = " (deprecated)" if e["deprecated"] else ""
        w(f"| [{esc(e['name'])}](#{e['slug']}){dep} | {e['group']} | {esc(e['since'])} | {LABEL[e['status']]} | {cov} | "
          f"{esc(mark_paths(e['bifrost']))} | {esc(e['summary'])} |")
    w("")
    w("## Remaining work (Gap and Partial, lowest coverage first)")
    w("")
    for g, es in by_group(entries).items():
        es = [e for e in es if e["status"] in ("gap", "partial")]
        if not es:
            continue
        w(f"### {g}")
        w("")
        for e in sorted(es, key=lambda e: (e["coverage"] or 0)):
            cov = "n/a" if e["coverage"] is None else f"{e['coverage']:.0f}%"
            w(f"- **{esc(e['name'])}** ({LABEL[e['status']]}, {cov}): " + ("; ".join(esc(x) for x in e["gaps"][:4]) or "see matrix"))
        w("")
    rel, themes = load_semantic_files()
    if themes:
        w("## Work themes (Gap rows clustered by meaning)")
        w("")
        w("Gap rows from all frameworks, embedded on Titan and clustered (`bifrost pc parity themes`); one theme is one")
        w("piece of work that closes gaps in several frameworks at once.")
        w("")
        w("| Theme | Gap rows | Frameworks |")
        w("|---|---|---|")
        for t in themes:
            w(f"| {esc(t['title'])} | {t['size']} | {esc(', '.join(f['name'] for f in t['frameworks'][:8]))}"
              + (f" and {len(t['frameworks']) - 8} more" if len(t["frameworks"]) > 8 else "") + " |")
        w("")
    names = {e["slug"]: e["name"] for e in entries}
    w("## Matrix")
    for e in entries:
        cov = "n/a" if e["coverage"] is None else f"{e['coverage']:.0f}%"
        w("")
        w(f'<a id="{e["slug"]}"></a>')
        w(f"### {esc(e['name'])}")
        w("")
        w(f"{LABEL[e['status']]} · coverage {cov} · macOS {esc(e['since'])} · [official docs]({e['doc']}) · REA `{esc(e['rea']) or 'none'}`")
        w("")
        if e["summary"]:
            w(esc(e["summary"]))
            w("")
        if e["rows"]:
            w("| Feature | Apple API | BifrOSt | Status | Note |")
            w("|---|---|---|---|---|")
            for r in e["rows"]:
                w(f"| {esc(r.get('feature'))} | {esc(r.get('apple'))} | {esc(mark_paths(r.get('bifrost')))} | "
                  f"{LABEL[r['status']]} | {esc(r.get('note'))} |")
            w("")
        if e["gaps"]:
            w("Gaps: " + "; ".join(esc(x) for x in e["gaps"]) + ".")
        near = [x for x in rel.get(e["slug"], []) if x["slug"] in names]
        if near:
            w("")
            w("Related: " + ", ".join(f"[{esc(names[x['slug']])}](#{x['slug']})" for x in near) + ".")
    w("")
    out = a.out or MATRIX_MD
    open(out, "w").write("\n".join(o))
    print(f"wrote {os.path.relpath(out, ROOT)}: {len(entries)} entries, {sum(rows.values())} rows, coverage {pct(score, n)}"
          + (f", {len(bad)} unreadable" if bad else ""))


def cmd_import(a):
    os.makedirs(DATA, exist_ok=True)
    n = 0
    for f in sorted(os.listdir(a.dir)):
        if not f.endswith(".json"):
            continue
        src = os.path.join(a.dir, f)
        if a.new_only and os.path.exists(os.path.join(DATA, f)):
            continue
        try:
            d = json.load(open(src))
            assert isinstance(d.get("features"), list) and d.get("framework")
        except Exception as e:  # noqa: BLE001
            print(f"skip {f}: {e}")
            continue
        shutil.copyfile(src, os.path.join(DATA, f))
        n += 1
    print(f"imported {n} files into {os.path.relpath(DATA, ROOT)}")


def cmd_db_export(a):
    entries, _ = load_entries()
    fw, rows, n, score = totals(entries)
    os.makedirs(os.path.join(a.dir, "frameworks"), exist_ok=True)
    os.makedirs(os.path.join(a.dir, "meta"), exist_ok=True)
    os.makedirs(os.path.join(a.dir, "themes"), exist_ok=True)
    rel, themes = load_semantic_files()
    names = {e["slug"]: e["name"] for e in entries}
    writes = []
    for e in entries:
        doc = {
            "slug": e["slug"], "name": e["name"], "parent": e["parent"], "group": e["group"], "since": e["since"],
            "deprecated": e["deprecated"], "status": e["status"], "coverage": e["coverage"], "applicable": e["applicable"],
            "counts": {s: e["counts"][s] for s in STATUSES}, "doc": e["doc"], "bifrost": e["bifrost"],
            "summary": e["summary"], "rea": e["rea"], "gaps": e["gaps"][:12],
            "features": [{k: r.get(k, "") for k in ("feature", "apple", "bifrost", "status", "note")} for r in e["rows"]],
        }
        p = os.path.join(a.dir, "frameworks", e["slug"] + ".json")
        json.dump(doc, open(p, "w"))
        writes.append({"op": "set", "collection": "frameworks", "doc_id": e["slug"], "file_path": os.path.abspath(p)})
    if rel:
        p = os.path.join(a.dir, "meta", "related.json")
        json.dump({s: [{"slug": x["slug"], "name": names.get(x["slug"], x["slug"]), "score": x["score"]}
                       for x in v if x["slug"] in names] for s, v in rel.items() if s in names}, open(p, "w"))
        writes.append({"op": "set", "collection": "meta", "doc_id": "related", "file_path": os.path.abspath(p)})
    for t in themes:
        p = os.path.join(a.dir, "themes", t["id"] + ".json")
        json.dump(t, open(p, "w"))
        writes.append({"op": "set", "collection": "themes", "doc_id": t["id"], "file_path": os.path.abspath(p)})
    groups = {}
    for g, es in by_group(entries).items():
        f2, r2, n2, s2 = totals(es)
        groups[g] = {"frameworks": {s: f2[s] for s in STATUSES}, "applicable": n2,
                     "coverage": round(100 * s2 / n2, 1) if n2 else None}
    summary = {"frameworks": len(entries), "status": {s: fw[s] for s in STATUSES}, "rows": {s: rows[s] for s in STATUSES},
               "applicable": n, "coverage": round(100 * score / n, 1) if n else None, "groups": groups,
               "built": round(built_pct(rows), 1) if n else None, "gate": 80,
               "not_assessed": missing_slugs(entries), "generated_by": "bifrost pc parity db-export",
               "source": "developer.apple.com official documentation + bifrost-os repo"}
    os_shards = load_os()
    if os_shards:
        os.makedirs(os.path.join(a.dir, "os"), exist_ok=True)
        os_all = []
        for sh in os_shards:
            es = []
            for e in sh["entries"]:
                f = dict(e.get("facts", {}))
                if isinstance(f.get("functions"), list) and len(f["functions"]) > 200:
                    f["functions"] = f["functions"][:200] + [f"... {len(e['facts']['functions']) - 200} more"]
                es.append({**{k: e.get(k, "") for k in ("id", "name", "path", "kind", "identifier", "version", "owner",
                                                          "description", "role", "doc", "bifrost", "status", "note", "rea")}, "facts": f})
            os_all.extend(sh["entries"])
            c, n, sc = os_totals(sh["entries"])
            doc = {"category": sh["category"], "shard": sh["shard"], "source": sh.get("source", ""), "counts": c, "entries": es}
            did = f"{sh['category']}--{sh['shard']}"
            p = os.path.join(a.dir, "os", did + ".json")
            json.dump(doc, open(p, "w"))
            writes.append({"op": "set", "collection": "os", "doc_id": did, "file_path": os.path.abspath(p)})
        c, n, sc = os_totals(os_all)
        cats = {}
        for sh in os_shards:
            cats.setdefault(sh["category"], []).extend(sh["entries"])
        summary["os"] = {"source": os_shards[0].get("source", ""), "components": len(os_all), "status": c,
                         "coverage": round(100 * sc / n, 1) if n else None,
                         "categories": {k: dict(zip(("status", "applicable"), os_totals(v)[:2])) for k, v in cats.items()}}
    p = os.path.join(a.dir, "meta", "summary.json")
    json.dump(summary, open(p, "w"))
    writes.append({"op": "set", "collection": "meta", "doc_id": "summary", "file_path": os.path.abspath(p)})
    batches = [writes[i:i + 50] for i in range(0, len(writes), 50)]
    json.dump(batches, open(os.path.join(a.dir, "batches.json"), "w"), indent=1)
    print(f"{len(writes)} documents in {len(batches)} batches: {os.path.join(a.dir, 'batches.json')}")


def cmd_refresh_index(a):
    def get(p):
        with urllib.request.urlopen(DOCS_URL + p + ".json", timeout=30) as r:
            return json.load(r)
    tech = get("technologies")
    old = {r["slug"]: r for r in load_index()} if os.path.exists(INDEX) else {}
    rows = []
    for s in tech["sections"]:
        if s.get("kind") != "technologies":
            continue
        for g in s["groups"]:
            for t in g["technologies"]:
                ident = t["destination"]["identifier"]
                if "/documentation/" not in ident:
                    continue
                path = ident.split("/documentation/", 1)[1].lower()
                try:
                    m = get(path).get("metadata", {})
                except Exception:  # noqa: BLE001 - pages without data are tool or overview pages
                    continue
                mac = [p for p in m.get("platforms", []) if p.get("name") == "macOS"]
                if not mac:
                    continue
                slug = re.sub(r"[^a-z0-9]+", "-", t["title"].lower()).strip("-")
                rows.append({"title": t["title"], "slug": slug, "path": path, "group": g["name"],
                             "doc": "https://developer.apple.com/documentation/" + path,
                             "macos_since": mac[0].get("introducedAt", ""),
                             "deprecated": mac[0].get("deprecatedAt") if mac[0].get("deprecated") else None,
                             "abstract": old.get(slug, {}).get("abstract", "")})
    rows.sort(key=lambda r: (r["group"], r["title"].lower()))
    json.dump(rows, open(INDEX, "w"), indent=1)
    print(f"{len(rows)} frameworks with macOS availability written to {os.path.relpath(INDEX, ROOT)}")


def _semantic():
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import semantic  # noqa: PLC0415 - needs numpy and the embedding service only for these commands
    return semantic


def cmd_search(a):
    sem = _semantic()
    entries, _ = load_entries()
    status = set(a.status.split(",")) if a.status else None
    hits, emodel, rmodel = sem.search(entries, a.query, top=a.top, status=status)
    print(f"# {a.query!r}  (embed {emodel}, rerank {rmodel})")
    for score, cos, e, r in hits:
        print(f"{score:5.2f} {cos:4.2f}  [{LABEL[r['status']]}] {e['name']} :: {r.get('feature')}")
        print(f"           ref: {r.get('apple')}")
        print(f"           bifrost: {r.get('bifrost') or '-'}" + (f"  ({r['note']})" if r.get("note") else ""))


def cmd_related(a):
    sem = _semantic()
    entries, _ = load_entries()
    rel, model = sem.related(entries, k=a.k)
    json.dump({"model": model, "related": rel}, open(RELATED, "w"), indent=1)
    print(f"related frameworks for {len(rel)} entries ({model}) -> {os.path.relpath(RELATED, ROOT)}")


def cmd_themes(a):
    sem = _semantic()
    entries, _ = load_entries()
    th, model = sem.themes(entries, k=a.k)
    json.dump({"model": model, "status": ["gap"], "themes": th}, open(THEMES, "w"), indent=1)
    print(f"{len(th)} work themes over {sum(t['size'] for t in th)} gap rows ({model}) -> {os.path.relpath(THEMES, ROOT)}")
    for t in th[:15]:
        print(f"  {t['id']} ({t['size']:3} rows, {len(t['frameworks'])} frameworks): {t['title']}")


RANK = {"n/a": -1, "gap": 0, "os": 1, "partial": 2, "parity": 3}


def cmd_apply(a):
    sem = _semantic()
    entries, _ = load_entries()
    by_slug = {e["slug"]: e for e in entries}
    by_slug.update({e["index_slug"]: e for e in entries if e.get("index_slug")})
    closures = json.load(open(a.file))
    changed, appended, unknown = set(), 0, []
    for c in closures:
        e = by_slug.get(c["slug"])
        if not e:
            unknown.append(c["slug"])
            continue
        path = os.path.join(DATA, e["slug"] + ".json")
        d = json.load(open(path))
        e["rows"] = [dict(r, status=norm(r.get("status"))) for r in d.get("features") or []]
        status = norm(c.get("status") or "partial")
        i, score = sem.match_row(e, c["feature"])
        note = c.get("note", "")
        if i is None:
            d.setdefault("features", []).append({"feature": c["feature"], "apple": c.get("apple", ""),
                                                 "bifrost": c.get("bifrost", ""), "status": status, "note": note})
            appended += 1
            print(f"+ {e['slug']}: new row {c['feature'][:70]!r} ({status})")
        else:
            r = d["features"][i]
            if RANK[status] > RANK[norm(r.get("status"))]:
                print(f"~ {e['slug']}: {r.get('feature', '')[:60]!r} {norm(r.get('status'))} -> {status} (match {score:.2f})")
                r["status"] = status
                r["bifrost"] = c.get("bifrost") or r.get("bifrost", "")
                r["note"] = (note + " " + (r.get("note") or "")).strip()
            else:
                print(f"= {e['slug']}: {r.get('feature', '')[:60]!r} already {norm(r.get('status'))} (match {score:.2f})")
                continue
        rows = d["features"]
        st = collections.Counter(norm(r.get("status")) for r in rows)
        if norm(d.get("status")) in ("gap", "n/a") and (st["partial"] or st["parity"]):
            d["status"] = "partial"
        changed.add(e["slug"])
        if not a.dry_run:
            json.dump(d, open(path, "w"), indent=2, ensure_ascii=False)
            open(path, "a").write("\n")
    print(f"{len(changed)} frameworks changed, {appended} rows appended, unknown slugs: {sorted(set(unknown))}"
          + (" (dry run)" if a.dry_run else ""))


def load_semantic_files():
    rel = json.load(open(RELATED))["related"] if os.path.exists(RELATED) else {}
    th = json.load(open(THEMES))["themes"] if os.path.exists(THEMES) else []
    return rel, th


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv[:1] == ["verify"]:
        import verify
        return verify.main(argv[1:])
    p = argparse.ArgumentParser(prog="bifrost pc parity", description=(__doc__ or "").split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("status")
    sub.add_parser("os-status")
    c = sub.add_parser("coverage")
    c.add_argument("--group")
    c.add_argument("--sort", choices=["coverage", "name", "group"], default="group")
    c.add_argument("--min", type=float)
    c.add_argument("--max", type=float)
    c.add_argument("--json", action="store_true")
    s = sub.add_parser("show")
    s.add_argument("name")
    g = sub.add_parser("gaps")
    g.add_argument("--group")
    g.add_argument("--status")
    sub.add_parser("check")
    b = sub.add_parser("build")
    b.add_argument("out", nargs="?")
    i = sub.add_parser("import")
    i.add_argument("dir")
    i.add_argument("--new-only", action="store_true", help="skip files that already exist in data/ (keep edited rows)")
    d = sub.add_parser("db-export")
    d.add_argument("dir")
    sub.add_parser("refresh-index")
    q = sub.add_parser("search")
    q.add_argument("query")
    q.add_argument("--top", type=int, default=10)
    q.add_argument("--status")
    r = sub.add_parser("related")
    r.add_argument("--k", type=int, default=5)
    ap = sub.add_parser("apply")
    ap.add_argument("file")
    ap.add_argument("--dry-run", action="store_true")
    t = sub.add_parser("themes")
    t.add_argument("--k", type=int)
    a = p.parse_args(argv)
    {"status": cmd_status, "os-status": cmd_os_status, "coverage": cmd_coverage, "show": cmd_show, "gaps": cmd_gaps, "check": cmd_check,
     "build": cmd_build, "import": cmd_import, "db-export": cmd_db_export, "refresh-index": cmd_refresh_index,
     "search": cmd_search, "related": cmd_related, "themes": cmd_themes, "apply": cmd_apply}[a.cmd](a)


if __name__ == "__main__":
    main()
