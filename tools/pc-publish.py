#!/usr/bin/env python3
"""Build and publish a BifrOSt PC system update.

usage: tools/pc-publish.py [--release] [--skip-build] [--no-serve] (also: bifrost pc publish)

The lab repo is served from out/pc-repo/bifrost/x86_64 at
http://192.168.1.12:8079/bifrost/$arch (override with BIFROST_REPO_BIND=host:port).
Packages and the pacman database are also exported to ~/Projects/bifrost-site/site/arch/x86_64,
served at https://bifrost.olibuijr.com/arch/$arch (override the site root with BIFROST_PUBLIC_SITE).
Installed systems use that repository from /etc/pacman.conf. Version = Cargo version + epoch seconds,
so every publish is newer than the last.
Signing (docs/installer-and-boot.md, "Package signing"): with BIFROST_REPO_SIGN_KEY set (a key id or fingerprint in the GnuPG
home BIFROST_REPO_GNUPGHOME, default out/lab/gnupg, which is never in git) every package and the repository database are
signed (makepkg --sign, repo-add --sign) and the .sig files are published next to them. Without a key the publish still
works, unsigned, with a loud warning; a configured key that cannot sign stops the publish.
The package carries only BifrOSt's own and openly licensed files: tools/check-package-licenses.py inspects it
before it enters the repo, and a failing package is deleted and the publish stops.
The system assets (UI fonts, app icons, wallpapers) ship in it too, at /usr/share/springboard/system-assets: they are
staged from springboard/assets by tools/stage-assets.py (only what packaging/assets-allowlist.txt names).
"""
import argparse, glob, hashlib, os, re, shutil, subprocess, sys, tempfile, time

import pc_mirror

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
POOL = os.path.join(REPO, "out/pc-repo/bifrost/x86_64")
PIDF = os.path.join(REPO, "out/pc-repo/server.pid")
PC_CATALOG = "https://bifrost.olibuijr.com/api/v1/pc/apps.json"
APPS_DIR = os.path.join(REPO, "springboard/apps")


def version_key(v):
    """A version as a comparable list: numeric parts compare as numbers ('0.16.2' > '0.9.10')."""
    return [int(x) if x.isdigit() else 0 for x in re.split(r"[.\-+]", str(v))]


def outdated_apps(store, manifests):
    """App directory names whose manifest version is newer than the store's (or that the store lacks). `store` maps id
    to the store's newest version; `manifests` is [(dir name, id, version, architecture)]. Only x86_64-linux apps."""
    return [d for d, i, v, arch in manifests if arch == "x86_64-linux" and (i not in store or version_key(v) > version_key(store[i]))]


def app_manifests(apps_dir=APPS_DIR):
    """(dir name, id, version, architecture) of every app manifest under springboard/apps."""
    out = []
    for m in sorted(glob.glob(os.path.join(apps_dir, "*", "manifest.toml"))):
        t = open(m, encoding="utf-8").read()
        f = {k: re.search(rf'^{k}\s*=\s*"([^"]+)"', t, re.M) for k in ("id", "version", "architecture")}
        if f["id"] and f["version"]:
            out.append((os.path.basename(os.path.dirname(m)), f["id"].group(1), f["version"].group(1), f["architecture"].group(1) if f["architecture"] else ""))
    return out


def store_versions(url=PC_CATALOG):
    """The PC store's newest version of each app id, as devices see it."""
    import json, urllib.request
    with urllib.request.urlopen(url, timeout=30) as r:
        apps = json.load(r)["apps"]
    best = {}
    for a in apps:
        if a["id"] not in best or version_key(a["version"]) > version_key(best[a["id"]]):
            best[a["id"]] = a["version"]
    return best


def publish_outdated_apps():
    """Publishes every app whose repo version is newer than the store's, so a system release also brings the apps:
    devices install store updates in the background (`shell::ota`, Automatic Updates), right after the update reboot.
    A failing app is reported and the others still go out."""
    todo = outdated_apps(store_versions(), app_manifests())
    failed = []
    for name in todo:
        print(f"pc-publish: publishing app {name} (newer than the store)", flush=True)
        if subprocess.run([sys.executable, os.path.join(REPO, "tools/pc-app.py"), "publish", name]).returncode:
            failed.append(name)
    print(f"pc-publish: {len(todo) - len(failed)} of {len(todo)} newer apps published" + (f"; FAILED: {', '.join(failed)}" if failed else ""))
    return failed


PUBLIC_SITE = os.path.abspath(os.path.expanduser(os.environ.get(
    "BIFROST_PUBLIC_SITE", "~/Projects/bifrost-site/site")))
ASSETS_SRC = os.path.join(REPO, "springboard/assets")
_h, _, _p = os.environ.get("BIFROST_REPO_BIND", "192.168.1.12:8079").partition(":")
BIND = (_h, int(_p or 8079))
DEFAULT_GNUPGHOME = os.path.join(REPO, "out/lab/gnupg")


def run(cmd, **kw):
    r = subprocess.run(cmd, **kw)
    if r.returncode:
        sys.exit(f"failed: {' '.join(map(str, cmd))}")
    return r


def signing_config(env=None):
    """The repository signing setup from the environment, or None when no key is configured:
    {"key": id, "gnupghome": dir}. The key lives in a GnuPG home under out/lab (BIFROST_REPO_GNUPGHOME overrides), never in git."""
    env = os.environ if env is None else env
    key = env.get("BIFROST_REPO_SIGN_KEY", "").strip()
    if not key:
        return None
    home = os.path.abspath(os.path.expanduser(env.get("BIFROST_REPO_GNUPGHOME", "") or DEFAULT_GNUPGHOME))
    return {"key": key, "gnupghome": home}


def signing_env(cfg):
    """Environment additions that make gpg (and so makepkg and repo-add) use the signing key's home."""
    return {"GNUPGHOME": cfg["gnupghome"]} if cfg else {}


def signing_args(cfg):
    """Extra arguments for makepkg and repo-add (both take --sign --key); empty when unsigned."""
    return ["--sign", "--key", cfg["key"]] if cfg else []


def check_signing_key(cfg, runner=subprocess.run):
    """Stop early when a configured key is not usable, instead of publishing unsigned by accident."""
    r = runner(["gpg", "--batch", "--list-secret-keys", cfg["key"]], env={**os.environ, **signing_env(cfg)},
               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if r.returncode:
        sys.exit(f"pc-publish: BIFROST_REPO_SIGN_KEY={cfg['key']} has no secret key in {cfg['gnupghome']}; "
                 "create it (packaging/arch/keys/README.md) or unset BIFROST_REPO_SIGN_KEY to publish unsigned")


def unsigned_warning():
    return ("pc-publish: WARNING: BIFROST_REPO_SIGN_KEY is not set; the packages and the repository database are published "
            "UNSIGNED. Installed systems only trust a signed repository once the key is generated and shipped "
            "(packaging/arch/keys/README.md).")


def license_status(package):
    """Exit status of tools/check-package-licenses.py on `package`: 0 clean, 1 it holds reference assets or a font
    without an open licence (the paths go to stderr), 2 it could not be read. Packages carry only BifrOSt's own and
    openly licensed files."""
    return subprocess.run([sys.executable, os.path.join(REPO, "tools/check-package-licenses.py"), package]).returncode


def _contained_path(root, path):
    """Resolve a path and refuse symlinks or traversal that escape its configured root."""
    root = os.path.realpath(root)
    path = os.path.realpath(path)
    if os.path.commonpath((root, path)) != root:
        raise ValueError(f"path escapes its root: {path}")
    return path


def _file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.digest()


def _atomic_repo_copy(src, dst, immutable=False):
    """Publish one file by atomic rename; immutable package names may only be reused byte-for-byte."""
    if os.path.lexists(dst) and immutable:
        if not os.path.isfile(dst) or os.path.islink(dst) or _file_sha256(src) != _file_sha256(dst):
            raise FileExistsError(f"immutable public package already differs: {dst}")
        return False
    fd, tmp = tempfile.mkstemp(prefix=f".{os.path.basename(dst)}.", suffix=".tmp", dir=os.path.dirname(dst))
    try:
        with os.fdopen(fd, "wb") as out, open(src, "rb") as inp:
            shutil.copyfileobj(inp, out)
            out.flush()
            os.fsync(out.fileno())
        os.chmod(tmp, 0o644)
        if immutable:
            # A hard link is an atomic create-if-absent on this same-directory temporary file.
            try:
                os.link(tmp, dst)
            except FileExistsError:
                if not os.path.isfile(dst) or os.path.islink(dst) or _file_sha256(src) != _file_sha256(dst):
                    raise FileExistsError(f"immutable public package already differs: {dst}")
                return False
            return True
        os.replace(tmp, dst)
        dirfd = os.open(os.path.dirname(dst), os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(dirfd)
        finally:
            os.close(dirfd)
        return True
    finally:
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass


def publish_public_repo(source_dir, site_dir=PUBLIC_SITE):
    """Export the system repo under the live site's /arch/x86_64 path, publishing the database last."""
    source_root = os.path.realpath(source_dir)
    if not os.path.isdir(source_root):
        raise FileNotFoundError(f"package repository not found: {source_dir}")
    site_root = os.path.realpath(site_dir)
    repo_dir = _contained_path(site_root, os.path.join(site_root, "arch", "x86_64"))
    packages = sorted(glob.glob(os.path.join(source_root, "springboard-shell-*-1-x86_64.pkg.tar.zst")))
    packages = [_contained_path(source_root, p) for p in packages]
    packages = [p for p in packages if os.path.isfile(p)]
    if not packages:
        raise FileNotFoundError(f"no springboard-shell packages in {source_root}")
    db_tar = _contained_path(source_root, os.path.join(source_root, "bifrost.db.tar.zst"))
    db = _contained_path(source_root, os.path.join(source_root, "bifrost.db"))
    if not os.path.isfile(db_tar) or not os.path.isfile(db):
        raise FileNotFoundError(f"pacman database files missing from {source_root}")

    os.makedirs(repo_dir, exist_ok=True)
    repo_dir = _contained_path(site_root, repo_dir)
    published = []
    for package in packages:
        name = os.path.basename(package)
        _atomic_repo_copy(package, os.path.join(repo_dir, name), immutable=True)
        published.append(name)
        sig = package + ".sig"
        if os.path.lexists(sig):
            _atomic_repo_copy(_contained_path(source_root, sig), os.path.join(repo_dir, name + ".sig"), immutable=True)
            published.append(name + ".sig")
    # Keep the compressed target available before replacing pacman's ordinary database URL. The database signatures go
    # just before their database; a stale one from an earlier signed publish is removed when this database is unsigned.
    db_sig = db_tar + ".sig"
    signed_db = os.path.lexists(db_sig)
    for name, source, sig_name in (("bifrost.db.tar.zst", db_tar, "bifrost.db.tar.zst.sig"), ("bifrost.db", db, "bifrost.db.sig")):
        source = _contained_path(source_root, source)
        dest_sig = os.path.join(repo_dir, sig_name)
        if signed_db:
            _atomic_repo_copy(_contained_path(source_root, db_sig), dest_sig)
            published.append(sig_name)
        elif os.path.lexists(dest_sig):
            os.unlink(dest_sig)
        _atomic_repo_copy(source, os.path.join(repo_dir, name))
        published.append(name)
    return repo_dir, published


def serve():
    if subprocess.run(["systemctl", "is-active", "--quiet", "bifrost-repo"]).returncode == 0:
        return "bifrost-repo.service"
    try:
        pid = int(open(PIDF).read())
        if os.path.exists(f"/proc/{pid}") and b"http.server" in open(f"/proc/{pid}/cmdline", "rb").read():
            return pid
    except (OSError, ValueError):
        pass
    p = subprocess.Popen([sys.executable, "-m", "http.server", str(BIND[1]), "--bind", BIND[0], "--directory", os.path.join(REPO, "out/pc-repo")],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, start_new_session=True)
    open(PIDF, "w").write(str(p.pid))
    return p.pid


def stage_system_assets(work, assets=ASSETS_SRC):
    """Stage the packed assets (springboard/assets, from tools/pack.py) into `work`/system-assets with
    tools/stage-assets.py, so only what packaging/assets-allowlist.txt names can ship, and return the environment
    for the package build ({"PC_SYSTEM_ASSETS": dir}). The package installs the tree at the package-owned
    /usr/share/springboard/system-assets (packaging/arch/package-files.sh). With no assets directory the package ships
    none and a machine keeps the ones it has; the licence guard then runs on the built package as for any other file."""
    if not os.path.isdir(assets):
        print(f"pc-publish: no packed assets at {assets} (run springboard/tools/pack.py); the package ships no system-assets", file=sys.stderr)
        return {}
    dest = os.path.join(work, "system-assets")
    run([sys.executable, os.path.join(REPO, "tools/stage-assets.py"), "--replace", assets, dest])
    return {"PC_SYSTEM_ASSETS": dest}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--release", action="store_true")
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--no-serve", action="store_true")
    ap.add_argument("--skip-mirror-sync", action="store_true", help="only check the 8079 mirror, do not download")
    ap.add_argument("--skip-apps", action="store_true", help="do not publish the apps that are newer than the store")
    a = ap.parse_args()
    prof = "release" if a.release else "fast"
    t0 = time.time()
    sign = signing_config()
    if sign:
        check_signing_key(sign)
    else:
        print(unsigned_warning(), file=sys.stderr)
    # the 8079 mirror must hold every package the new springboard-shell depends on, or Midget's update 404s and rolls back
    if not a.skip_mirror_sync:
        try:
            pc_mirror.sync()
        except Exception as e:  # offline titan: the check below still decides
            print(f"pc-mirror: sync failed ({e}); checking what is there", file=sys.stderr)
    pc_mirror.check()
    if not a.skip_build:
        run(["cargo", "build", "--profile", prof, "--bin", "shell", "--bin", "bootweave", "--bin", "bifrost-portal", "--bin", "bifrost-privd", "--features", "desktop-wayland", "--timings"], cwd=os.path.join(REPO, "springboard"))
        run(["cargo", "build", "--profile", prof, "-p", "bifrost-mcp", "-p", "bifrostd", "-p", "bifrost-audiod", "-p", "bifrost-audio-bench", "-p", "bifrost-midid", "-p", "bifrost-alsa-plugin"], cwd=os.path.join(REPO, "springboard"))
        # the system services bifrostd supervises (audio, network, update); package-files.sh installs them under
        # /usr/lib/bifrost/services/<id>/ with the manifests of springboard/system
        run(["cargo", "build", "--profile", prof, "-p", "audiod", "-p", "netd", "-p", "updated"], cwd=os.path.join(REPO, "springboard"))
        # the window server, its chrome clients and bifrost-log (the PKGBUILD builds the same set)
        run(["cargo", "build", "--profile", prof, "-p", "bifrost-wm", "--bin", "bifrost-wm", "-p", "bifrost-chrome", "--bins", "-p", "bifrost-app", "--bin", "bifrost-log"], cwd=os.path.join(REPO, "springboard"))
        # the command-line tools (springboard/cmd/src/bin); packaging/arch/package-files.sh installs every one of them
        run(["cargo", "build", "--profile", prof, "-p", "bifrost-cmd", "--bins"], cwd=os.path.join(REPO, "springboard"))
    # the EFI loader is always built from source (even with --skip-build) and checked against its sources; run() exits on failure
    run([os.path.join(REPO, "tools/build-efi-boot.sh"), "release"])
    run([os.path.join(REPO, "tools/verify-efi-loader.sh"), os.path.join(REPO, "packaging/arch/recovery/bootweave.efi")])
    run(["rustc", "--edition", "2021", "-O", "-C", "target-feature=+crt-static", "-C", "strip=symbols", os.path.join(REPO,"packaging/arch/recovery/guard.rs"), "-o", os.path.join(REPO,"out/bifrost-update-guard-publish")])
    built_at = time.time()
    binpath = os.path.join(REPO, f"springboard/target/{prof}/shell")
    bootweave = os.path.join(REPO, f"springboard/target/{prof}/bootweave")
    portal = os.path.join(REPO, f"springboard/target/{prof}/bifrost-portal")
    privd = os.path.join(REPO, f"springboard/target/{prof}/bifrost-privd")
    mcp = os.path.join(REPO, f"springboard/target/{prof}/bifrost-mcp")
    bifrostd = os.path.join(REPO, f"springboard/target/{prof}/bifrostd")
    audiod = os.path.join(REPO, f"springboard/target/{prof}/bifrost-audiod")
    svc_audiod = os.path.join(REPO, f"springboard/target/{prof}/audiod")
    svc_netd = os.path.join(REPO, f"springboard/target/{prof}/netd")
    svc_updated = os.path.join(REPO, f"springboard/target/{prof}/updated")
    audio_bench = os.path.join(REPO, f"springboard/target/{prof}/bifrost-audio-bench")
    midid = os.path.join(REPO, f"springboard/target/{prof}/bifrost-midid")
    alsa_plugin = os.path.join(REPO, f"springboard/target/{prof}/libasound_module_pcm_bifrost.so")
    tdir = os.path.join(REPO, f"springboard/target/{prof}")
    wm_bins = {"PC_WM": "bifrost-wm", "PC_MENUBAR": "bifrost-menubar", "PC_DOCK": "bifrost-dock", "PC_PANELS": "bifrost-panels", "PC_LOG": "bifrost-log"}
    wm_env = {k: os.path.join(tdir, v) for k, v in wm_bins.items()}
    base = re.search(r'^version\s*=\s*"([^"]+)"', open(os.path.join(REPO, "springboard/Cargo.toml")).read(), re.M).group(1)
    ver = f"{base}.{int(time.time())}"
    work = os.path.join(REPO, "out/pc-repo/build")
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    os.makedirs(POOL, exist_ok=True)
    src = os.path.join(REPO, "packaging/arch")
    for f in ("fast/PKGBUILD", "fast/springboard.install"):
        shutil.copy(os.path.join(src, f), work)
    assets_env = stage_system_assets(work)
    env = {**os.environ, **assets_env, "PC_VER": ver, "PC_PKGSRC": src, "PC_BIN": binpath, "PC_BOOTWEAVE": bootweave, "PC_PORTAL": portal, "PC_PRIVD": privd, "PC_MCP": mcp, "PC_BIFROSTD": bifrostd, "PC_AUDIOD": audiod, "PC_SVC_AUDIOD": svc_audiod, "PC_SVC_NETD": svc_netd, "PC_SVC_UPDATED": svc_updated, "PC_AUDIO_BENCH": audio_bench, "PC_MIDID": midid, "PC_ALSA_PLUGIN": alsa_plugin, "PC_CMD_DIR": tdir, **wm_env, "PC_GUARD": os.path.join(REPO,"out/bifrost-update-guard-publish"), "PKGDEST": POOL, "BUILDDIR": work + "/b", "SRCDEST": work + "/s", "PKGEXT": ".pkg.tar.zst", **signing_env(sign)}
    run(["makepkg", "-f", "--nodeps", "--skipinteg", "--nocheck", *signing_args(sign)], cwd=work, env=env, stdout=subprocess.DEVNULL)
    pkg = sorted(glob.glob(os.path.join(POOL, f"springboard-shell-{ver}-1-x86_64.pkg.tar.zst")))[-1]
    # licence guard: the package may carry only BifrOSt's own and openly licensed files. A failing package is deleted
    # before it reaches the repo database or the public export.
    if license_status(pkg):
        os.unlink(pkg)
        if os.path.exists(pkg + ".sig"):
            os.unlink(pkg + ".sig")
        sys.exit(f"refusing to publish {os.path.basename(pkg)}: the licence guard failed (see above); the package was deleted")
    # packages built before the guard existed may hold such files too: drop them, so the public export cannot offer them again
    for old in sorted(glob.glob(os.path.join(POOL, "springboard-shell-*.pkg.tar.zst"))):
        if old != pkg and license_status(old) == 1:
            print(f"removed {os.path.basename(old)}: it holds files BifrOSt may not redistribute", file=sys.stderr)
            os.unlink(old)
            if os.path.exists(old + ".sig"):
                os.unlink(old + ".sig")
    # keep the last 3 packages
    olds = sorted(glob.glob(os.path.join(POOL, "springboard-shell-*.pkg.tar.zst")), key=os.path.getmtime)[:-3]
    for o in olds:
        os.unlink(o)
        if os.path.exists(o + ".sig"):
            os.unlink(o + ".sig")
    # an unsigned run must not leave the database signature of an older run next to the new database
    if not sign:
        for stale in ("bifrost.db.tar.zst.sig", "bifrost.db.sig"):
            if os.path.lexists(os.path.join(POOL, stale)):
                os.unlink(os.path.join(POOL, stale))
    run(["repo-add", "-q", *signing_args(sign), "-R", os.path.join(POOL, "bifrost.db.tar.zst"), pkg],
        env={**os.environ, **signing_env(sign)}, stdout=subprocess.DEVNULL)
    public_dir, public_files = publish_public_repo(POOL)
    pid = None if a.no_serve else serve()
    print(f"build {built_at - t0:.1f}s; package/publish {time.time() - built_at:.1f}s")
    print(f"published springboard-shell {ver} ({os.path.getsize(pkg) // 1024} KiB, {prof}) in {time.time() - t0:.0f}s"
          + (f"; repo served at http://{BIND[0]}:{BIND[1]}/bifrost/x86_64 (pid {pid})" if pid else "")
          + f"; public repo exported to {public_dir} ({len(public_files)} files)"
          + (f"; signed with {sign['key']}" if sign else "; UNSIGNED"))
    # a system release also brings the apps: the ones bumped in the repo but not in the store yet
    if not a.skip_apps and publish_outdated_apps():
        sys.exit("pc-publish: the system is published, some apps failed (see above)")


if __name__ == "__main__":
    main()
