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

status, coverage, show, gaps, check and refresh-index take --platform macos|ipados (default macos). The tablet platform
lives apart in docs/parity-matrix/ipados/ (index.json, data/<slug>.json, same row format). A framework that is in the
tablet index and has a macOS assessment reuses those rows by reference; its own data file replaces them, or extends
them when it carries "reuse": "macos" (own rows override same-named rows, new rows are added). Tablet-only
frameworks have their own files.

  build [OUT.md]                 regenerate docs/apple-parity-matrix.md
  import DIR [--new-only]        copy agent output files DIR/*.json into data/ (validated; --new-only keeps existing files)
  db-export DIR                  one JSON file per database document for the published artifact
                                 (frameworks/<slug>.json, meta/summary.json) plus batches.json (50 per batch)
  refresh-index                  re-read the official documentation index and rewrite index.json (--platform ipados:
                                 records ipados_since, and macos_since when the framework is on both)
  search QUERY [--top N] [--status gap,partial]
                                 semantic search over feature rows (Titan embeddings, then rerank)
  related                        nearest frameworks per framework by embedding -> related.json
  themes [--k N]                 cluster Gap rows across frameworks into work themes -> themes.json
  verify os-cli plan|run|closures
                                 assess the os/cli rows and verify each tool on Midget (tools/parity/os_cli.py)
  os-verify plan|run|promote     run the checks of the OS rows marked ready (file, cmd, unit, test) and promote the
                                 passing rows to parity (tools/parity/os_verify.py); `check` rejects malformed ready/checks
  apps [--gaps APP] [--json]     per-app feature inventories (docs/parity-matrix/apps/<slug>.json): one line per app sorted by
                                 coverage ascending (app, BifrOSt app, features, parity/partial/gap/n/a counts, coverage %);
                                 --gaps APP lists that app's gap and partial rows with their wire text (what to implement
                                 and where); --json prints the same data as JSON. `check` validates these files too
                                 (statuses, required keys, wire text on gap and partial rows, cited paths exist)
  apply FILE.json [--dry-run]    apply closures [{slug, feature, status, bifrost, note}]: each updates the row it
                                 matches (exact feature, else rerank) or is appended as a new row
  verify plan|run|closures       promote partial rows to parity from Midget test evidence (tools/parity/verify.py):
                                 plan [--group G] [--slug S] [--out plan.json] classifies rows logic/ui/hardware/untested;
                                 run PLAN --version X runs the logic rows' tests on Midget -> results.json;
                                 closures RESULTS --version X emits apply closures + needs-manual.md

App coverage (apps): rows that apply (status not n/a) score parity 1, partial 0.5, gap 0.

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

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import os_verify  # noqa: E402  (schema of the OS rows' ready and checks fields; also the os-verify subcommand)

ROOT =os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BASE = os.path.join(ROOT, "docs", "parity-matrix")
DATA = os.path.join(BASE, "data")
INDEX = os.path.join(BASE, "index.json")
IPADOS = os.path.join(BASE, "ipados")  # the tablet platform: index.json and data/<slug>.json
MATRIX_MD = os.path.join(ROOT, "docs", "apple-parity-matrix.md")
RELATED = os.path.join(BASE, "related.json")
THEMES = os.path.join(BASE, "themes.json")
APPS = os.path.join(BASE, "apps")
APP_STATUSES = ["parity", "partial", "gap", "n/a"]
APP_WEIGHT = {"parity": 1.0, "partial": 0.5, "gap": 0.0}
APP_KEYS = ["feature", "area", "apple", "status", "bifrost", "note", "wire"]
STATUSES = ["parity", "partial", "os", "gap", "n/a"]
LABEL = {"parity": "Parity", "partial": "Partial", "os": "OS only", "gap": "Gap", "n/a": "n/a"}
WEIGHT = {"parity": 1.0, "partial": 0.5, "os": 0.5, "gap": 0.0}
PARTS = {  # large frameworks split by official topic sections; slug prefix -> framework title
    "appkit-": "AppKit", "foundation-": "Foundation", "swiftui-": "SwiftUI", "avfoundation-": "AVFoundation",
    "uikit-": "UIKit",  # the tablet platform's UIKit topic parts (ipados/data/uikit-*.json)
}
PLATFORMS = ("macos", "ipados")
PLATFORM_NAME = {"macos": "macOS", "ipados": "iPadOS"}  # as the documentation metadata spells them
SINCE = {"macos": "macos_since", "ipados": "ipados_since"}
DOCS_URL = "https://developer.apple.com/tutorials/data/documentation/"
PATH_RE = re.compile(r"(?<![\w/~.:-])((?:springboard|docs|tools|packaging)/[A-Za-z0-9_./-]+)")  # this repo only


def norm(s):
    s = (s or "").strip().lower()
    return {"na": "n/a", "none": "gap", "os only": "os", "missing": "gap"}.get(s, s if s in STATUSES else "gap")


def key(s):
    """Loose match for names and slugs: lower case, letters and digits only."""
    return re.sub(r"[^a-z0-9]", "", (s or "").lower())


def platform_paths(platform):
    """(data directory, index.json) of a reference platform."""
    if platform == "macos":
        return DATA, INDEX
    if platform == "ipados":
        return os.path.join(IPADOS, "data"), os.path.join(IPADOS, "index.json")
    raise ValueError(f"unknown platform {platform!r}; expected one of {', '.join(PLATFORMS)}")


def load_index(platform="macos"):
    return json.load(open(platform_paths(platform)[1]))


def row_stats(rows):
    """Counts per status, applicable rows, score and coverage percent (None when no row applies)."""
    c = collections.Counter(r["status"] for r in rows)
    applicable = sum(c[s] for s in WEIGHT)
    score = sum(WEIGHT[s] * c[s] for s in WEIGHT)
    return c, applicable, score, (round(100 * score / applicable, 1) if applicable else None)


def read_entries(data_dir, index, since_key):
    """Entries from every <data_dir>/*.json, matched against the index rows; unreadable files come back as messages."""
    idx = {r["slug"]: r for r in index}
    by_title = {r["title"]: r for r in idx.values()}
    by_key = {key(r["slug"]): r for r in idx.values()} | {key(r["title"]): r for r in idx.values()}
    out, bad = [], []
    for f in sorted(os.listdir(data_dir)) if os.path.isdir(data_dir) else []:
        if not f.endswith(".json"):
            continue
        slug = f[:-5]
        try:
            d = json.load(open(os.path.join(data_dir, f)))
        except Exception as e:  # noqa: BLE001 - report and continue
            bad.append(f"{slug}: {e}")
            continue
        meta = idx.get(slug) or by_key.get(key(slug)) or by_key.get(key(d.get("framework", "")))
        if not meta:
            parent = next((t for p, t in PARTS.items() if slug.startswith(p)), None)
            meta = by_title.get(parent, {})
        rows = [dict(r, status=norm(r.get("status"))) for r in d.get("features") or []]
        c, applicable, score, coverage = row_stats(rows)
        out.append(dict(
            slug=slug, index_slug=meta.get("slug"), name=d.get("framework") or meta.get("title", slug), group=meta.get("group", "Other"),
            parent=meta.get("title", d.get("framework", slug)), since=meta.get(since_key, ""),
            deprecated=bool(meta.get("deprecated")), status=norm(d.get("status")), doc=d.get("doc") or meta.get("doc", ""),
            bifrost=d.get("bifrost", ""), summary=d.get("summary", ""), rea=d.get("rea_evidence", ""),
            gaps=[g for g in d.get("gaps") or [] if g], rows=rows, counts=c, applicable=applicable, score=score,
            coverage=coverage, reuse=d.get("reuse"), given=set(d),
        ))
    return out, bad


def parse_reuse(spec, default_slug):
    """The (platform, slug) a data file's "reuse" key points to: "macos", "macos:<slug>" or {"platform", "slug"}."""
    if isinstance(spec, dict):
        platform, slug = spec.get("platform", "macos"), spec.get("slug") or default_slug
    elif isinstance(spec, str):
        platform, _, slug = spec.partition(":")
        slug = slug or default_slug
    else:
        raise ValueError(f"reuse must be a string or an object, got {spec!r}")
    if platform != "macos":
        raise ValueError(f"reuse platform {platform!r} is not supported; only macos can be reused")
    return platform, slug


def derive_status(counts):
    """Framework status of a merged row set: parity when every applicable row is parity, gap when none is built."""
    applicable = sum(counts[s] for s in WEIGHT)
    if not applicable:
        return "n/a"
    if counts["parity"] == applicable:
        return "parity"
    return "partial" if counts["parity"] or counts["partial"] or counts["os"] else "gap"


def merge_reused(own, bases):
    """One entry from a data file that extends macOS assessments: the base rows, then own rows replacing rows with the
    same feature name or appended. Fields the file states win; the rest come from the base."""
    rows = [dict(r) for b in bases for r in b["rows"]]
    at = {r.get("feature"): i for i, r in enumerate(rows)}
    for r in own["rows"]:
        i = at.get(r.get("feature"))
        if i is None:
            at[r.get("feature")] = len(rows)
            rows.append(r)
        else:
            rows[i] = r
    c, applicable, score, coverage = row_stats(rows)
    first, given = bases[0], own["given"]
    return dict(
        own, rows=rows, counts=c, applicable=applicable, score=score, coverage=coverage, source="macos+own",
        status=own["status"] if "status" in given else (first["status"] if len(bases) == 1 and not own["rows"] else derive_status(c)),
        bifrost=own["bifrost"] if "bifrost" in given else first["bifrost"],
        summary=own["summary"] if "summary" in given else first["summary"],
        rea=own["rea"] if "rea_evidence" in given else first["rea"], doc=own["doc"] or first["doc"],
        gaps=own["gaps"] if "gaps" in given else [g for b in bases for g in b["gaps"]],
    )


def tablet_entries(own, bad, index, macos):
    """Tablet entries: own files (replacing the macOS rows, or extending them when the file says "reuse"), then, for
    every other framework in the tablet index that has a macOS assessment, that assessment by reference."""
    by_index = collections.defaultdict(list)
    for e in macos:
        if e.get("index_slug"):
            by_index[e["index_slug"]].append(e)
    out, bad = [], list(bad)
    replaced = {e["slug"] for e in own}  # macOS entries an own file replaces (same file name)
    replaced_frameworks = {e["index_slug"] for e in own if e.get("index_slug") == e["slug"]}  # a whole split framework
    for e in own:
        e["source"] = "own"
        if e.get("reuse") is not None:
            try:
                same_name = any(b["slug"] == e["slug"] for b in macos)  # a part file extends that part
                _, target = parse_reuse(e["reuse"], e["slug"] if same_name else e.get("index_slug") or e["slug"])
            except ValueError as err:
                bad.append(f"{e['slug']}: {err}")
            else:
                bases = by_index.get(target) or [b for b in macos if b["slug"] == target]  # a framework or one of its parts
                if bases:
                    replaced |= {b["slug"] for b in bases}
                    e = merge_reused(e, bases)
                else:
                    bad.append(f"{e['slug']}: reuse target {target!r} has no macOS assessment")
        out.append(e)
    for r in index:
        if r["slug"] in replaced_frameworks:
            continue
        for b in by_index.get(r["slug"], []):
            if b["slug"] in replaced:
                continue
            out.append(dict(b, since=r.get(SINCE["ipados"], ""), group=r.get("group", b["group"]), parent=r["title"],
                            deprecated=bool(r.get("deprecated")), source="macos", reuse=None, given=set()))
    return out, bad


def load_entries(platform="macos"):
    data_dir, _ = platform_paths(platform)
    index = load_index(platform)
    out, bad = read_entries(data_dir, index, SINCE[platform])
    if platform == "macos":
        for e in out:
            e["source"] = "own"
    else:
        out, bad = tablet_entries(out, bad, index, load_entries("macos")[0])
    for e in out:
        e["platform"] = platform
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


def missing_slugs(entries, platform="macos"):
    have = {e["slug"] for e in entries} | {s for e in entries for s in (e.get("index_slug"),) if s}
    parts = {p for p in PARTS if any(s.startswith(p) for s in have)}
    out = []
    for r in load_index(platform):
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
            problems += [f"os/{sh['category']}/{sh['shard']}: {p}" for p in os_verify.validate_entry(e)]
            for part in re.split(r"[;,]\s*", e.get("bifrost", "") or ""):
                m = re.match(r"(springboard/\S+|tools/\S+|packaging/\S+|docs/\S+)", part.strip())
                if m and not os.path.exists(os.path.join(root, m.group(1).split(":")[0].rstrip(")."))):
                    problems.append(f"os/{sh['category']}/{sh['shard']}: {e['id']}: cited path not found: {m.group(1)}")
    return problems


def plat(a):
    return getattr(a, "platform", None) or "macos"


def source_counts(entries, index):
    """Where a platform's assessments come from: index frameworks that are also on macOS (and how many of those
    reuse macOS rows or have their own), and the tablet-only ones."""
    shared = {r["slug"] for r in index if r.get("macos_since")}
    own = {e["index_slug"] for e in entries if e["source"] != "macos" and e.get("index_slug")}
    reused = {e["index_slug"] for e in entries if e["source"] == "macos" and e.get("index_slug")}
    return dict(frameworks=len(index), shared=len(shared), reused=len(reused), shared_own=len(shared & own),
                tablet_only=len(index) - len(shared), tablet_only_own=len(own - shared))


def cmd_status(a):
    platform = plat(a)
    entries, bad = load_entries(platform)
    fw, rows, n, score = totals(entries)
    built = fw["parity"] + fw["partial"] + fw["os"]
    if platform != "macos":
        sc = source_counts(entries, load_index(platform))
        print(f"Platform {platform}: {sc['frameworks']} frameworks in the index; {sc['shared']} also on macOS "
              f"({sc['reused']} reuse the macOS rows, {sc['shared_own']} have their own file); "
              f"{sc['tablet_only']} tablet-only ({sc['tablet_only_own']} assessed)")
    print(f"Assessed: {len(entries)} frameworks/parts; not yet assessed: {len(missing_slugs(entries, platform))}; unreadable: {len(bad)}")
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
    entries, _ = load_entries(plat(a))
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
    entries, _ = load_entries(plat(a))
    hits = find(entries, a.name)
    if not hits:
        sys.exit(f"no framework matches {a.name!r}")
    for e in hits:
        cov = "n/a" if e["coverage"] is None else f"{e['coverage']:.1f}%"
        src = {"own": "", "macos": ", rows from macOS", "macos+own": ", macOS rows plus own"}[e["source"]]
        print(f"# {e['name']}  [{LABEL[e['status']]}, coverage {cov}, since {e['since']}{src}]  {e['doc']}")
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
    entries, _ = load_entries(plat(a))
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


def load_apps():
    """Every docs/parity-matrix/apps/*.json as (slug, data or None, error or None), sorted by slug."""
    out = []
    if not os.path.isdir(APPS):
        return out
    for fn in sorted(os.listdir(APPS)):
        if not fn.endswith(".json"):
            continue
        try:
            with open(os.path.join(APPS, fn)) as fh:
                out.append((fn[:-5], json.load(fh), None))
        except (OSError, ValueError) as e:
            out.append((fn[:-5], None, str(e)))
    return out


def app_counts(data):
    """Counts per status for an app inventory, and its coverage percent (None when no row applies)."""
    c = collections.Counter(r.get("status") for r in data.get("features", []))
    n = sum(c[k] for k in APP_WEIGHT)
    score = sum(APP_WEIGHT[k] * c[k] for k in APP_WEIGHT)
    return {k: c[k] for k in APP_STATUSES}, (round(100.0 * score / n, 1) if n else None)


def app_summaries():
    rows = []
    for slug, data, err in load_apps():
        if data is None:
            continue
        counts, cov = app_counts(data)
        rows.append({"slug": slug, "app": data.get("app", slug), "bifrost_app": data.get("bifrost_app", ""),
                     "features": len(data.get("features", [])), "counts": counts, "coverage": cov})
    return sorted(rows, key=lambda r: (-1 if r["coverage"] is None else r["coverage"], r["app"].lower()))


def find_app(name):
    """The one app summary that matches a slug or name (exact, else a unique substring), or None."""
    k = key(name)
    rows = app_summaries()
    exact = [r for r in rows if k in (key(r["slug"]), key(r["app"]))]
    if exact:
        return exact[0]
    hits = [r for r in rows if k and (k in key(r["slug"]) or k in key(r["app"]))]
    return hits[0] if len(hits) == 1 else None


def app_gap_rows(slug):
    data = next(d for s, d, _ in load_apps() if s == slug)
    return [r for r in data["features"] if r.get("status") in ("gap", "partial")]


def check_apps():
    """Problems in docs/parity-matrix/apps/*.json: unreadable files, missing keys, unknown statuses, gap and partial rows
    without wire text, a summary that disagrees with the rows, cited repo paths that do not exist."""
    problems = []
    for slug, data, err in load_apps():
        where = f"apps/{slug}.json"
        if data is None:
            problems.append(f"{where}: unreadable ({err})")
            continue
        for k in ("app", "bifrost_app", "evidence", "features"):
            if k not in data:
                problems.append(f"{where}: missing key {k}")
        for i, r in enumerate(data.get("features", [])):
            name = r.get("feature") or f"row {i}"
            for k in APP_KEYS:
                if k not in r:
                    problems.append(f"{where}: {name}: missing key {k}")
            st = r.get("status")
            if st not in APP_STATUSES:
                problems.append(f"{where}: {name}: status {st!r} is not one of {', '.join(APP_STATUSES)}")
            elif st in ("gap", "partial") and not str(r.get("wire") or "").strip():
                problems.append(f"{where}: {name}: a {st} row needs wire text")
            for m in PATH_RE.finditer(str(r.get("bifrost") or "")):
                path = m.group(1).rstrip(".,;:)").split(":")[0]
                if not os.path.exists(os.path.join(ROOT, path)):
                    problems.append(f"{where}: {name}: cited path not found: {path}")
        counts, cov = app_counts(data)
        sm = data.get("summary")
        if sm and (sm.get("features") != len(data.get("features", [])) or any(sm.get(k) != counts[k] for k in APP_STATUSES if k in sm)):
            problems.append(f"{where}: summary does not match the rows")
    return problems


def cmd_apps(a):
    if a.gaps:
        hit = find_app(a.gaps)
        if not hit:
            sys.exit(f"no single app matches {a.gaps!r}; run `apps` for the list")
        rows = app_gap_rows(hit["slug"])
        if a.json:
            print(json.dumps({"app": hit["app"], "bifrost_app": hit["bifrost_app"], "rows": rows}, indent=2, ensure_ascii=False))
            return
        print(f"{hit['app']} ({hit['bifrost_app']}): {len(rows)} gap or partial rows, coverage {hit['coverage']}%")
        for r in sorted(rows, key=lambda r: (r["status"] != "gap", r.get("area", ""))):
            print(f"- [{r['status']}] {r['feature']} ({r.get('area', '')}): {r.get('note', '')}")
            print(f"    wire: {r.get('wire', '')}")
        return
    rows = app_summaries()
    if a.json:
        print(json.dumps(rows, indent=2, ensure_ascii=False))
        return
    print(f"{'app':<22} {'BifrOSt app':<30} {'feat':>4}  {'parity':>6} {'partial':>7} {'gap':>4} {'n/a':>4}  {'cover':>6}")
    for r in rows:
        c = r["counts"]
        cov = "n/a" if r["coverage"] is None else f"{r['coverage']:.1f}%"
        print(f"{r['app']:<22} {r['bifrost_app'][:30]:<30} {r['features']:>4}  {c['parity']:>6} {c['partial']:>7} {c['gap']:>4} {c['n/a']:>4}  {cov:>6}")


def cmd_check(a):
    platform = plat(a)
    entries, bad = load_entries(platform)
    problems = list(bad)
    for e in entries:
        if not e["rows"]:
            problems.append(f"{e['slug']}: no feature rows")
        if not e["doc"].startswith("https://developer.apple.com/"):
            problems.append(f"{e['slug']}: doc link is not the official documentation")
        if platform != "macos" and not e.get("index_slug"):
            problems.append(f"{e['slug']}: not in the {platform} index (run refresh-index --platform {platform} or fix the slug)")
    ok, missing = cited_paths([e for e in entries if e["source"] != "macos"])  # reused rows are checked with macOS
    problems += [f"{s}: cited path not found: {p}" for s, p in missing]
    miss = missing_slugs(entries, platform)
    if platform == "macos":
        problems += os_check()
        problems += check_apps()
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


def docc_get(path):
    with urllib.request.urlopen(DOCS_URL + path + ".json", timeout=30) as r:
        return json.load(r)


def abstract_text(page):
    return "".join(x.get("text", "") for x in page.get("abstract") or []).strip()


def index_rows(get, platform, old=None, workers=8):
    """Index rows for a platform from the documentation index: every technology whose page lists the platform.
    get(path) returns the DocC JSON of a page. macOS rows carry macos_since; tablet rows carry ipados_since and, when the
    framework is also on macOS, macos_since."""
    from concurrent.futures import ThreadPoolExecutor
    old = old or {}
    want = PLATFORM_NAME[platform]
    techs = []
    for s in get("technologies")["sections"]:
        if s.get("kind") != "technologies":
            continue
        for g in s["groups"]:
            for t in g["technologies"]:
                ident = t["destination"]["identifier"]
                if "/documentation/" in ident:
                    techs.append((g["name"], t["title"], ident.split("/documentation/", 1)[1].lower()))

    def page(path):
        try:
            return get(path)
        except Exception:  # noqa: BLE001 - pages without data are tool or overview pages
            return None
    with ThreadPoolExecutor(max_workers=workers) as pool:
        pages = list(pool.map(page, [t[2] for t in techs]))
    rows, seen = [], set()
    for (group, title, path), pg in zip(techs, pages):
        if pg is None:
            continue
        plats = {p.get("name"): p for p in pg.get("metadata", {}).get("platforms", [])}
        mine = plats.get(want)
        slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
        if not mine or slug in seen:
            continue
        seen.add(slug)
        row = {"title": title, "slug": slug, "path": path, "group": group, "doc": "https://developer.apple.com/documentation/" + path}
        if platform == "macos":
            row["macos_since"] = mine.get("introducedAt", "")
        else:
            # a page can omit the tablet version while listing the phone one, which the tablet shares
            row[SINCE[platform]] = mine.get("introducedAt") or plats.get("iOS", {}).get("introducedAt", "")
            if "macOS" in plats:
                row["macos_since"] = plats["macOS"].get("introducedAt", "")
        row["deprecated"] = mine.get("deprecatedAt") if mine.get("deprecated") else None
        row["abstract"] = old.get(slug, {}).get("abstract") or abstract_text(pg)
        rows.append(row)
    rows.sort(key=lambda r: (r["group"], r["title"].lower()))
    return rows


def cmd_refresh_index(a):
    platform = plat(a)
    index_path = platform_paths(platform)[1]
    old = {r["slug"]: r for r in load_index(platform)} if os.path.exists(index_path) else {}
    rows = index_rows(docc_get, platform, old)
    os.makedirs(os.path.dirname(index_path), exist_ok=True)
    with open(index_path, "w") as fh:
        json.dump(rows, fh, indent=1)
    both = sum(1 for r in rows if r.get("macos_since")) if platform != "macos" else len(rows)
    print(f"{len(rows)} frameworks with {PLATFORM_NAME[platform]} availability written to {os.path.relpath(index_path, ROOT)}"
          + (f" ({both} also on macOS)" if platform != "macos" else ""))


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
    if argv[:2] == ["verify", "os-cli"]:
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        import os_cli
        return os_cli.main(argv[2:])
    if argv[:1] == ["verify"]:
        import verify
        return verify.main(argv[1:])
    if argv[:1] == ["os-verify"]:
        sys.exit(os_verify.main(argv[1:]))  # exit status matters: a failing check run must fail the caller
    p = argparse.ArgumentParser(prog="bifrost pc parity", description=(__doc__ or "").split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    def platform_arg(sp):
        sp.add_argument("--platform", choices=PLATFORMS, default="macos", help="reference platform (default macos)")
        return sp
    platform_arg(sub.add_parser("status"))
    sub.add_parser("os-status")
    c = platform_arg(sub.add_parser("coverage"))
    c.add_argument("--group")
    c.add_argument("--sort", choices=["coverage", "name", "group"], default="group")
    c.add_argument("--min", type=float)
    c.add_argument("--max", type=float)
    c.add_argument("--json", action="store_true")
    s = platform_arg(sub.add_parser("show"))
    s.add_argument("name")
    g = platform_arg(sub.add_parser("gaps"))
    g.add_argument("--group")
    g.add_argument("--status")
    platform_arg(sub.add_parser("check"))
    ps = sub.add_parser("apps")
    ps.add_argument("--gaps", metavar="APP")
    ps.add_argument("--json", action="store_true")
    b = sub.add_parser("build")
    b.add_argument("out", nargs="?")
    i = sub.add_parser("import")
    i.add_argument("dir")
    i.add_argument("--new-only", action="store_true", help="skip files that already exist in data/ (keep edited rows)")
    d = sub.add_parser("db-export")
    d.add_argument("dir")
    platform_arg(sub.add_parser("refresh-index"))
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
    {"status": cmd_status, "os-status": cmd_os_status, "coverage": cmd_coverage, "show": cmd_show, "gaps": cmd_gaps, "apps": cmd_apps, "check": cmd_check,
     "build": cmd_build, "import": cmd_import, "db-export": cmd_db_export, "refresh-index": cmd_refresh_index,
     "search": cmd_search, "related": cmd_related, "themes": cmd_themes, "apply": cmd_apply}[a.cmd](a)


if __name__ == "__main__":
    main()
