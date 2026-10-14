#!/usr/bin/env python3
"""Stage the exact installer package closure and default apps into an ISO root.

Usage: tools/bundle-iso-offline.py --dest AIROOT/usr/share/bifrost/offline [--sync]

Without --sync this is deliberately offline and fails closed on an incomplete mirror.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

import pc_mirror

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
POOL = os.path.join(ROOT, "out/pc-repo/bifrost/x86_64")
APP_OUT = os.path.join(ROOT, "out/pc-apps")
SITE_STORE = os.path.expanduser("~/Projects/bifrost-site/data/store")
MARKER = ".bifrost-offline-bundle"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def committed_version(path):
    with open(path, "rb") as f:
        return tomllib.load(f)["package"]["version"]


def shell_package():
    base = committed_version(os.path.join(ROOT, "springboard/Cargo.toml"))
    dbpath = os.path.join(POOL, "bifrost.db")
    if not os.path.isfile(dbpath):
        raise RuntimeError(f"missing local BifrOSt repository database: {dbpath}")
    records = pc_mirror.parse_db(open(dbpath, "rb").read())
    record = records.get("springboard-shell")
    if not record or not re.fullmatch(re.escape(base) + r"\.\d+-[A-Za-z0-9._+]+", record["version"]):
        raise RuntimeError(f"repository shell version must match committed Cargo version {base}.<epoch>-<pkgrel>")
    filename = record["filename"]
    if not pc_mirror.filename_matches("springboard-shell", record["version"], filename):
        raise RuntimeError(f"repository shell filename/version mismatch: {filename}")
    path = os.path.join(POOL, filename)
    if not os.path.isfile(path):
        raise RuntimeError(f"repository database references missing shell package: {path}")
    return path


def app_package(app_id):
    app_dir = os.path.join(ROOT, "springboard/apps", "browser" if app_id.endswith("browser.desktop") else "files")
    with open(os.path.join(app_dir, "manifest.toml"), "rb") as f:
        manifest = tomllib.load(f)
    if manifest.get("id") != app_id:
        raise RuntimeError(f"default app source id mismatch in {app_dir}/manifest.toml")
    version = manifest["version"]
    cargo_version = committed_version(os.path.join(app_dir, "Cargo.toml"))
    if version != cargo_version:
        raise RuntimeError(f"{app_id} manifest/Cargo versions differ ({version} vs {cargo_version})")
    name = f"{app_id}-{version}.aap"
    candidates = [os.path.join(APP_OUT, name), os.path.join(SITE_STORE, app_id, name)]
    path = next((p for p in candidates if os.path.isfile(p)), None)
    if not path:
        raise RuntimeError(f"missing exact default app package {name}; expected one of: " + ", ".join(candidates))
    spec = importlib.util.spec_from_file_location("aap_packager", os.path.join(ROOT, "springboard/tools/aap.py"))
    aap = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(aap)
    with open(path, "rb") as f, __import__("tarfile").open(fileobj=f) as tf:
        member = tf.extractfile("manifest.toml")
        if member is None:
            raise RuntimeError(f"{path}: missing manifest.toml")
        packaged = aap.parse_manifest(member.read().decode())
    if packaged.get("id") != app_id or packaged.get("version") != version:
        raise RuntimeError(f"{path}: packaged app id/version does not match committed source {app_id} {version}")
    return app_id, version, path


def package_closure():
    wanted = pc_mirror.wanted_names()
    pkgs = pc_mirror.load_all()
    names, unresolved = pc_mirror.closure(wanted, pkgs)
    if unresolved:
        raise RuntimeError("unresolved Arch dependencies: " + ", ".join(unresolved))
    selected = []
    for name in sorted(names):
        pkg = pkgs[name]
        if not pc_mirror.filename_matches(name, pkg["version"], pkg["filename"]):
            raise RuntimeError(f"Arch database filename/version mismatch for {name}: {pkg['filename']}")
        source = os.path.join(pc_mirror.repo_dir(pkg["repo"]), pkg["filename"])
        if not os.path.isfile(source):
            raise RuntimeError(f"Arch mirror is incomplete; missing {pkg['repo']}/{pkg['filename']} (run with --sync)")
        selected.append((name, pkg["filename"], source))
    return selected


def checked_destination(dest):
    dest = os.path.abspath(dest)
    out_root = os.path.realpath(os.path.join(ROOT, "out"))
    parent = os.path.realpath(os.path.dirname(dest))
    resolved = os.path.realpath(dest)
    if os.path.islink(dest) or (parent != out_root and not parent.startswith(out_root + os.sep)) \
            or (os.path.lexists(dest) and resolved != dest):
        raise RuntimeError(f"bundle destination must be inside generated out/: {dest}")
    return dest


def build(dest, sync=False):
    dest = checked_destination(dest)
    if sync:
        pc_mirror.sync()
    selected = package_closure()
    shell = shell_package()
    apps = [app_package("is.olibuijr.browser.desktop"), app_package("is.olibuijr.files.desktop")]
    parent = os.path.dirname(dest)
    os.makedirs(parent, exist_ok=True)
    stage = tempfile.mkdtemp(prefix=".bifrost-offline-", dir=parent)
    try:
        repo = os.path.join(stage, "repo")
        os.makedirs(repo)
        package_rows = []
        copied_repo = []
        for _, filename, source in selected:
            shutil.copyfile(source, os.path.join(repo, filename))
            package_rows.append({"file": filename, "sha256": sha256(source)})
            copied_repo.append(os.path.join(repo, filename))
        database = os.path.join(repo, "offline.db")
        subprocess.run(["repo-add", "-q", database, *copied_repo], check=True)
        if not os.path.isfile(database):
            raise RuntimeError("repo-add did not create repo/offline.db")
        shutil.copyfile(shell, os.path.join(stage, "springboard-shell.pkg.tar.zst"))
        shell_row = {"file": "springboard-shell.pkg.tar.zst", "sha256": sha256(shell)}
        os.makedirs(os.path.join(stage, "apps"))
        app_rows = []
        for app_id, version, source in apps:
            rel = f"apps/{app_id}-{version}.aap"
            shutil.copyfile(source, os.path.join(stage, rel))
            app_rows.append({"id": app_id, "version": version, "file": rel, "sha256": sha256(source)})
        manifest = {"schema_version": 1, "packages": package_rows,
                    "repo": {"path": "repo", "database": "offline.db", "sha256": sha256(database)},
                    "shell": shell_row, "apps": app_rows}
        with open(os.path.join(stage, "manifest.json"), "w") as f:
            json.dump(manifest, f, indent=2, sort_keys=True)
            f.write("\n")
        with open(os.path.join(stage, MARKER), "w") as f:
            f.write("schema=1\n")
        if os.path.lexists(dest):
            marker = os.path.join(dest, MARKER)
            if os.path.islink(dest) or not os.path.isdir(dest) or not os.path.isfile(marker) or open(marker).read() != "schema=1\n":
                raise RuntimeError(f"refusing to replace unmanaged destination: {dest}")
            shutil.rmtree(dest)
        os.replace(stage, dest)
        stage = None
    finally:
        if stage and os.path.isdir(stage):
            shutil.rmtree(stage)
    print(f"offline ISO bundle: {len(selected)} Arch packages, shell, and {len(apps)} default apps -> {dest}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dest", required=True, help="generated AIROOT destination, e.g. .../usr/share/bifrost/offline")
    parser.add_argument("--sync", action="store_true", help="explicitly refresh the Arch package mirror before bundling")
    args = parser.parse_args()
    try:
        build(args.dest, args.sync)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as exc:
        sys.exit(f"offline bundle: {exc}")


if __name__ == "__main__":
    main()
