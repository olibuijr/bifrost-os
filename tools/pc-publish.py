#!/usr/bin/env python3
"""Build and publish a BifrOSt PC system update.

usage: tools/pc-publish.py [--release] [--skip-build] [--no-serve] (also: bifrost pc publish)

The lab repo is served from out/pc-repo/bifrost/x86_64 at
http://192.168.1.12:8079/bifrost/$arch (override with BIFROST_REPO_BIND=host:port).
Packages and the pacman database are also exported to ~/Projects/bifrost-site/site/arch/x86_64,
served at https://bifrost.olibuijr.com/arch/$arch (override the site root with BIFROST_PUBLIC_SITE).
Installed systems use that repository from /etc/pacman.conf. Version = Cargo version + epoch seconds,
so every publish is newer than the last.
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
PUBLIC_SITE = os.path.abspath(os.path.expanduser(os.environ.get(
    "BIFROST_PUBLIC_SITE", "~/Projects/bifrost-site/site")))
ASSETS_SRC = os.path.join(REPO, "springboard/assets")
_h, _, _p = os.environ.get("BIFROST_REPO_BIND", "192.168.1.12:8079").partition(":")
BIND = (_h, int(_p or 8079))


def run(cmd, **kw):
    r = subprocess.run(cmd, **kw)
    if r.returncode:
        sys.exit(f"failed: {' '.join(map(str, cmd))}")
    return r


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
    # Keep the compressed target available before replacing pacman's ordinary database URL.
    for name, source in (("bifrost.db.tar.zst", db_tar), ("bifrost.db", db)):
        source = _contained_path(source_root, source)
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
    a = ap.parse_args()
    prof = "release" if a.release else "fast"
    t0 = time.time()
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
    env = {**os.environ, **assets_env, "PC_VER": ver, "PC_PKGSRC": src, "PC_BIN": binpath, "PC_BOOTWEAVE": bootweave, "PC_PORTAL": portal, "PC_PRIVD": privd, "PC_MCP": mcp, "PC_BIFROSTD": bifrostd, "PC_AUDIOD": audiod, "PC_SVC_AUDIOD": svc_audiod, "PC_SVC_NETD": svc_netd, "PC_SVC_UPDATED": svc_updated, "PC_AUDIO_BENCH": audio_bench, "PC_MIDID": midid, "PC_ALSA_PLUGIN": alsa_plugin, "PC_CMD_DIR": tdir, **wm_env, "PC_GUARD": os.path.join(REPO,"out/bifrost-update-guard-publish"), "PKGDEST": POOL, "BUILDDIR": work + "/b", "SRCDEST": work + "/s", "PKGEXT": ".pkg.tar.zst"}
    run(["makepkg", "-f", "--nodeps", "--skipinteg", "--nocheck"], cwd=work, env=env, stdout=subprocess.DEVNULL)
    pkg = sorted(glob.glob(os.path.join(POOL, f"springboard-shell-{ver}-1-x86_64.pkg.tar.zst")))[-1]
    # licence guard: the package may carry only BifrOSt's own and openly licensed files. A failing package is deleted
    # before it reaches the repo database or the public export.
    if license_status(pkg):
        os.unlink(pkg)
        sys.exit(f"refusing to publish {os.path.basename(pkg)}: the licence guard failed (see above); the package was deleted")
    # packages built before the guard existed may hold such files too: drop them, so the public export cannot offer them again
    for old in sorted(glob.glob(os.path.join(POOL, "springboard-shell-*.pkg.tar.zst"))):
        if old != pkg and license_status(old) == 1:
            print(f"removed {os.path.basename(old)}: it holds files BifrOSt may not redistribute", file=sys.stderr)
            os.unlink(old)
    # keep the last 3 packages
    olds = sorted(glob.glob(os.path.join(POOL, "springboard-shell-*.pkg.tar.zst")), key=os.path.getmtime)[:-3]
    for o in olds:
        os.unlink(o)
    run(["repo-add", "-q", "-R", os.path.join(POOL, "bifrost.db.tar.zst"), pkg], stdout=subprocess.DEVNULL)
    public_dir, public_files = publish_public_repo(POOL)
    pid = None if a.no_serve else serve()
    print(f"build {built_at - t0:.1f}s; package/publish {time.time() - built_at:.1f}s")
    print(f"published springboard-shell {ver} ({os.path.getsize(pkg) // 1024} KiB, {prof}) in {time.time() - t0:.0f}s"
          + (f"; repo served at http://{BIND[0]}:{BIND[1]}/bifrost/x86_64 (pid {pid})" if pid else "")
          + f"; public repo exported to {public_dir} ({len(public_files)} files)")


if __name__ == "__main__":
    main()
