#!/usr/bin/env python3
"""BifrOSt app packager (docs/apps.md, "Package").

  tools/aap.py pack apps/calculator [--bin PATH] [--out DIR]
  tools/aap.py info <file.aap>

An .aap is an uncompressed POSIX (ustar) tar holding manifest.toml, the
executable named by `exec`, the 180x180 `icon` PNG and an optional assets/ tree.
The app dir holds manifest.toml (+ icon.png, assets/); the binary comes from
--bin or target/aarch64-linux-android/release/<exec>. A missing icon is taken
from bifrost-ref/icons/<id>.png when it exists.

System packages (`kind = "system"`, e.g. system/keyboard) may be data-only: no
`exec`, no binary, just manifest, icon and assets/. For `provides = ["keyboard"]`
pack adds the word lists of apps/keyboard/src/words/*.txt as assets/words/<lang>.txt and
requires assets/NotoColorEmoji.ttf (CBDT; not committed, see system/keyboard/README.md).
"""
import argparse, hashlib, io, os, re, sys, tarfile

REQUIRED = ["id", "name", "version", "exec", "icon", "min_shell", "summary", "category"]
KINDS = ("app", "system-app", "system", "service")  # springboard/src/shell/kinds.rs is the shell's copy
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORDS = os.path.join(ROOT, "apps", "keyboard", "src", "words")

sys.path.insert(0, os.path.join(ROOT, "..", "tools"))
from _util import changelog_has  # noqa: E402


def parse_manifest(text):
    m = {}
    for line in text.splitlines():
        line = line.strip()
        # top-level keys only: `[[intent]]` and other tables follow them
        if line.startswith("["):
            break
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = (p.strip() for p in line.split("=", 1))
        if v.startswith("["):
            m[k] = [x.strip().strip('"') for x in v.strip("[]").split(",") if x.strip()]
        else:
            m[k] = v.strip('"')
    return m


def is_system(m):
    return m.get("kind") == "system"


def check(m):
    if m.get("kind", "app") not in KINDS:
        sys.exit(f"manifest.toml: kind must be one of {KINDS}")
    for k in REQUIRED:
        # only a system package may be data-only
        if k == "exec" and is_system(m):
            continue
        if not m.get(k):
            sys.exit(f"manifest.toml: missing `{k}`")
    if not re.fullmatch(r"[A-Za-z0-9._-]+", m["id"]) or "." not in m["id"]:
        sys.exit("manifest.toml: id must be reverse-DNS (letters, digits, . - _)")
    if not re.fullmatch(r"\d+\.\d+\.\d+", m["version"]):
        sys.exit("manifest.toml: version must be semver x.y.z")
    if m.get("provides") and not is_system(m):
        sys.exit("manifest.toml: only a system package may declare `provides`")
    for k in ("exec", "icon"):
        if m.get(k) and (m[k].startswith("/") or ".." in m[k].split("/")):
            sys.exit(f"manifest.toml: `{k}` must be a relative path inside the package")
    for p in m.get("permissions", []):
        if p not in ("network", "audio", "camera", "storage", "photos", "notifications", "documents", "keychain", "remote-desktop"):
            sys.exit(f"manifest.toml: unknown permission {p!r}")


def check_location(app, m):
    """Apps live under apps/, system packages and services under system/ (docs/apps.md, "Package kinds")."""
    where = os.path.basename(os.path.dirname(os.path.abspath(app)))
    kind = m.get("kind", "app")
    if kind in ("app", "system-app") and where != "apps":
        sys.exit(f"{app}: kind {kind!r} belongs under apps/, not {where}/")
    if kind in ("system", "service") and where != "system":
        sys.exit(f"{app}: kind {kind!r} belongs under system/, not {where}/")


def png_size(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit("icon is not a PNG")
    return int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")


def add(tf, name, data, mode):
    ti = tarfile.TarInfo(name)
    ti.size, ti.mode, ti.mtime = len(data), mode, 0
    ti.uname = ti.gname = ""
    tf.addfile(ti, io.BytesIO(data))


def pack(args):
    app = args.dir.rstrip("/")
    text = open(os.path.join(app, "manifest.toml")).read()
    m = parse_manifest(text)
    check(m)
    check_location(app, m)
    cl_path = os.path.join(app, "CHANGELOG.md")
    if not os.path.isfile(cl_path) or not changelog_has(open(cl_path).read(), m["version"]):
        sys.exit(f"{cl_path}: needs a '## {m['version']}' section with notes (every package version has a changelog entry)")
    binary = None
    if m.get("exec"):
        binary = args.bin or os.path.join(ROOT, "target/aarch64-linux-android/release", os.path.basename(m["exec"]))
        if not os.path.isfile(binary):
            sys.exit(f"binary not found: {binary} (cargo build --release -p {os.path.basename(app)} --target aarch64-linux-android)")
    extra = {}  # archive name -> bytes, generated at pack time
    if is_system(m) and "keyboard" in m.get("provides", []):
        font = os.path.join(app, "assets", "NotoColorEmoji.ttf")
        if not os.path.isfile(font):
            sys.exit(f"{font} missing: copy the CBDT Noto Color Emoji (cp /usr/share/fonts/noto/NotoColorEmoji.ttf {font}); see system/keyboard/README.md")
        if not os.path.isfile(os.path.join(app, "assets", "OFL.txt")):
            print("warning: assets/OFL.txt (font license notice) missing", file=sys.stderr)
        for lang in sorted(os.listdir(WORDS)):
            if lang.endswith(".txt"):
                extra["assets/words/" + lang] = open(os.path.join(WORDS, lang), "rb").read()
    icon = os.path.join(app, m["icon"])
    if not os.path.isfile(icon):
        icon = os.path.join(ROOT, "..", "bifrost-ref", "icons", m["id"] + ".png")
    if not os.path.isfile(icon):
        sys.exit(f"icon not found: {os.path.join(app, m['icon'])}")
    icon_data = open(icon, "rb").read()
    if png_size(icon_data) != (180, 180):
        sys.exit("icon must be 180x180")
    out = os.path.join(args.out, f"{m['id']}-{m['version']}.aap")
    os.makedirs(args.out, exist_ok=True)
    with tarfile.open(out, "w", format=tarfile.USTAR_FORMAT) as tf:
        add(tf, "manifest.toml", text.encode(), 0o644)
        add(tf, "CHANGELOG.md", open(cl_path, "rb").read(), 0o644)
        if binary:
            add(tf, m["exec"], open(binary, "rb").read(), 0o755)
        add(tf, m["icon"], icon_data, 0o644)
        assets = os.path.join(app, "assets")
        for d, _, files in sorted(os.walk(assets)):
            for f in sorted(files):
                p = os.path.join(d, f)
                name = os.path.relpath(p, app)
                if name in extra:  # the generated copy (word lists) wins
                    continue
                add(tf, name, open(p, "rb").read(), 0o644)
        for name, data in sorted(extra.items()):
            add(tf, name, data, 0o644)
    data = open(out, "rb").read()
    print(f"{out}\n  size {len(data)}\n  sha256 {hashlib.sha256(data).hexdigest()}")


def info(args):
    with tarfile.open(args.file) as tf:
        names = tf.getnames()
        m = parse_manifest(tf.extractfile("manifest.toml").read().decode())
    check(m)
    for need in (m.get("exec"), m["icon"]):
        if need and need not in names:
            sys.exit(f"package lacks {need}")
    print(f"{m['id']} {m['version']} ({m['name']}): {', '.join(names)}")


ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
sub = ap.add_subparsers(dest="cmd", required=True)
p = sub.add_parser("pack")
p.add_argument("dir")
p.add_argument("--bin")
p.add_argument("--out", default=os.path.join(ROOT, "target/aap"))
p.set_defaults(fn=pack)
i = sub.add_parser("info")
i.add_argument("file")
i.set_defaults(fn=info)
a = ap.parse_args()
a.fn(a)
