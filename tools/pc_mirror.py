#!/usr/bin/env python3
"""The pacman mirror titan serves on :8079 (out/pc-repo/mirror/$repo/os/$arch): core and extra dbs plus every package file
BifrOSt systems install, so a guarded `pacman -Su` never meets a 404 (release 0.7.25 added fonts the mirror lacked).

The package set is `tools/lab-mirror.sh --list` (base, runtime-packages.txt, GPU sets); its dependency closure is resolved
from the db itself, so the files are exactly the versions the served db lists. Incremental and idempotent: only missing
files are fetched, and a db is replaced only after every file it needs is present.

usage: tools/pc_mirror.py [--sync] [--check]     (default: both; pc-publish.py runs both)
env:   BIFROST_UPSTREAM_MIRROR  Arch mirror base, e.g. https://geo.mirror.pkgbuild.com ($repo/os/$arch is appended);
       default: https://geo.mirror.pkgbuild.com
"""
import io, os, re, subprocess, sys, tarfile, urllib.error, urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ROOT = os.path.join(REPO, "out/pc-repo/mirror")
REPOS = ("core", "extra")
ARCH = "x86_64"


def parse_db(data):
    """{name: {version, filename, depends, provides}} from the bytes of a pacman sync db (tar, any compression)."""
    pkgs = {}
    with tarfile.open(fileobj=io.BytesIO(data)) as tf:
        for m in tf.getmembers():
            if not m.isfile() or not m.name.endswith("/desc"):
                continue
            fields, key = {}, None
            for line in tf.extractfile(m).read().decode().splitlines():
                if line.startswith("%") and line.endswith("%"):
                    key = line.strip("%")
                    fields[key] = []
                elif line and key:
                    fields[key].append(line)
            if "NAME" in fields:
                pkgs[fields["NAME"][0]] = {"version": fields["VERSION"][0], "filename": fields["FILENAME"][0],
                                           "depends": fields.get("DEPENDS", []), "provides": fields.get("PROVIDES", [])}
    return pkgs


def dep_name(dep):
    """'glibc>=2.38' -> 'glibc'; 'libfoo.so=1-64' -> 'libfoo.so'."""
    return re.split(r"[<>=]", dep, maxsplit=1)[0].strip()


def provider_index(pkgs):
    """{provided name: [package names]}, a package providing itself."""
    idx = {}
    for n, p in pkgs.items():
        for prov in [n] + [dep_name(x) for x in p["provides"]]:
            idx.setdefault(prov, []).append(n)
    return idx


def closure(wanted, pkgs):
    """(set of package names to mirror, sorted list of unresolved names). A dependency already in the set satisfies a later
    one; otherwise the package of that exact name, else the first provider, the way pacman resolves a non-interactive -S."""
    idx, out, missing, todo = provider_index(pkgs), set(), set(), list(wanted)
    while todo:
        want = dep_name(todo.pop())
        cands = idx.get(want)
        if not cands:
            missing.add(want)
            continue
        if any(c in out for c in cands):
            continue
        pick = want if want in cands else cands[0]
        out.add(pick)
        todo.extend(pkgs[pick]["depends"])
    return out, sorted(missing)


def filename_matches(name, version, filename):
    """True when filename is the package file of exactly name-version (version includes the pkgrel)."""
    return re.fullmatch(re.escape(f"{name}-{version}") + r"-(any|x86_64)\.pkg\.tar\.(zst|xz|gz)", filename) is not None


def missing_files(names, pkgs, have):
    """Filenames the db lists for `names` that are not in `have` (the files present in the repo dir)."""
    have = set(have)
    return sorted(pkgs[n]["filename"] for n in names if pkgs[n]["filename"] not in have)


def stale_files(pkgs, have):
    """Package files present whose name matches a db package but not its db version (informational, safe to prune)."""
    out = []
    for f in sorted(have):
        m = re.fullmatch(r"(.+)-[^-]+-[^-]+-(?:any|x86_64)\.pkg\.tar\.\w+", f)
        if m and m.group(1) in pkgs and not filename_matches(m.group(1), pkgs[m.group(1)]["version"], f):
            out.append(f)
    return out


def wanted_names():
    out = subprocess.run([os.path.join(REPO, "tools/lab-mirror.sh"), "--list"], capture_output=True, text=True, check=True).stdout
    return out.split()


def upstream():
    # the official Arch mirror by default: Titan's own mirrorlist (Omarchy's mirror) answers 403 to urllib
    base = os.environ.get("BIFROST_UPSTREAM_MIRROR") or "https://geo.mirror.pkgbuild.com"
    return re.sub(r"/\$repo/os/\$arch/?$", "", base).rstrip("/")


def fetch(url, dest, optional=False):
    tmp = dest + ".part"
    try:
        with urllib.request.urlopen(url, timeout=60) as r, open(tmp, "wb") as f:
            while chunk := r.read(1 << 20):
                f.write(chunk)
    except urllib.error.HTTPError as e:
        if os.path.exists(tmp):
            os.unlink(tmp)
        if optional and e.code == 404:
            return False
        raise
    os.replace(tmp, dest)
    return True


def repo_dir(repo):
    return os.path.join(ROOT, repo, "os", ARCH)


def load_all(fresh_dbs=None):
    """{name: pkg + 'repo'} from the dbs in `fresh_dbs` {repo: bytes} or the ones on disk. core wins over extra."""
    pkgs = {}
    for repo in reversed(REPOS):
        data = (fresh_dbs or {}).get(repo)
        if data is None:
            data = open(os.path.join(repo_dir(repo), f"{repo}.db"), "rb").read()
        for n, p in parse_db(data).items():
            pkgs[n] = {**p, "repo": repo}
    return pkgs


def check(names=None):
    """Exit non-zero, naming every package file Midget could not download, when the served dbs need a file the mirror lacks."""
    pkgs = load_all()
    need, unresolved = closure(names or wanted_names(), pkgs)
    bad = []
    for repo in REPOS:
        have = os.listdir(repo_dir(repo))
        bad += [f"{repo}/{f}" for f in missing_files([n for n in need if pkgs[n]["repo"] == repo], pkgs, have)]
    if bad or unresolved:
        sys.exit("pc-mirror: the 8079 mirror cannot serve a full update, so this release would roll back on Midget.\n"
                 + "".join(f"  missing file: {b}\n" for b in bad) + "".join(f"  unresolved dependency: {u}\n" for u in unresolved)
                 + "run: tools/pc_mirror.py --sync")
    print(f"pc-mirror: ok, {len(need)} packages in the closure, all files present and matching the served db versions")


def sync(names=None):
    base = upstream()
    dbs = {}
    for repo in REPOS:
        os.makedirs(repo_dir(repo), exist_ok=True)
        with urllib.request.urlopen(f"{base}/{repo}/os/{ARCH}/{repo}.db", timeout=60) as r:
            dbs[repo] = r.read()
    pkgs = load_all(dbs)
    need, unresolved = closure(names or wanted_names(), pkgs)
    if unresolved:
        sys.exit("pc-mirror: unresolved in core/extra: " + " ".join(unresolved))
    got = 0
    for n in sorted(need):
        p = pkgs[n]
        url = f"{base}/{p['repo']}/os/{ARCH}/{p['filename']}"
        dest = os.path.join(repo_dir(p["repo"]), p["filename"])
        if not os.path.exists(dest):
            fetch(url, dest)
            got += 1
        if not os.path.exists(dest + ".sig"):
            fetch(url + ".sig", dest + ".sig", optional=True)
    # dbs last: a client never sees a db whose files are not yet there
    for repo, data in dbs.items():
        dest = os.path.join(repo_dir(repo), f"{repo}.db")
        with open(dest + ".part", "wb") as f:
            f.write(data)
        os.replace(dest + ".part", dest)
    print(f"pc-mirror: synced {len(need)} packages from {base} ({got} downloaded)")


def main(argv):
    do_sync = "--sync" in argv or "--check" not in argv
    do_check = "--check" in argv or "--sync" not in argv
    if do_sync:
        sync()
    if do_check:
        check()


if __name__ == "__main__":
    main(sys.argv[1:])
