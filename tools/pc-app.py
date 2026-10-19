#!/usr/bin/env python3
"""Build and publish independent Linux desktop apps to the BifrOSt App Store."""
import argparse
import hashlib
import json
import os
import shutil
import tarfile
import urllib.request
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def run(argv, cwd=ROOT):
    subprocess.run([str(a) for a in argv], cwd=cwd, check=True)


def fetch_upstream(app, up):
    """Download the pinned upstream tarball (cached, sha256 checked) and return the binary.
    Licence files named by `licenses` go to <app>/assets/licenses (ignored by version control)."""
    cache = ROOT / 'out/upstream' / f"{app.name}-{up['sha256'][:12]}.tar.gz"
    ok = lambda: cache.exists() and hashlib.sha256(cache.read_bytes()).hexdigest() == up['sha256']
    if not ok():
        cache.parent.mkdir(parents=True, exist_ok=True)
        print(f"Fetching {up['url']}", flush=True)
        with urllib.request.urlopen(up['url'], timeout=120) as src, cache.open('wb') as dst:
            shutil.copyfileobj(src, dst)
        if not ok():
            sys.exit('upstream download checksum mismatch')
    out = ROOT / 'out/pc-apps/bin'
    out.mkdir(parents=True, exist_ok=True)
    exe = out / Path(up['binary']).name
    lic = app / 'assets/licenses'
    shutil.rmtree(lic, ignore_errors=True)
    prefix = up.get('licenses', '').rstrip('/') + '/'
    with tarfile.open(cache) as tf:
        for m in tf.getmembers():
            if m.isfile() and m.name == up['binary']:
                exe.write_bytes(tf.extractfile(m).read())
                exe.chmod(0o755)
            elif m.isfile() and up.get('licenses') and m.name.startswith(prefix) and '/' not in m.name[len(prefix):]:
                lic.mkdir(parents=True, exist_ok=True)
                (lic / Path(m.name).name).write_bytes(tf.extractfile(m).read())
    if not exe.exists():
        sys.exit(f"{up['binary']} not found in the upstream tarball")
    return exe


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['build', 'pack', 'publish', 'info'])
    parser.add_argument('app', help='App directory name under springboard/apps, e.g. files')
    parser.add_argument('--skip-build', action='store_true', help='Publish the already built executable')
    args = parser.parse_args()
    if Path(args.app).name != args.app or args.app in ('.', '..'):
        parser.error('Expected one app directory name')
    crate = ROOT / 'springboard/apps' / args.app
    # a system package (kind = "system", e.g. the keyboard) keeps its manifest, icon and assets under springboard/system/<name>
    app = ROOT / 'springboard/system' / args.app if (ROOT / 'springboard/system' / args.app / 'manifest.toml').exists() else crate
    manifest = tomllib.loads((app / 'manifest.toml').read_text())
    upstream = tomllib.loads((app / 'upstream.toml').read_text()) if (app / 'upstream.toml').exists() else None
    cargo = None if upstream else tomllib.loads((crate / 'Cargo.toml').read_text())
    if manifest.get('architecture') != 'x86_64-linux':
        parser.error('This command requires an architecture = "x86_64-linux" app manifest')
    if cargo and manifest['version'] != cargo['package']['version']:
        parser.error('Cargo and app manifest versions must match')
    binary = ROOT / 'springboard/target/fast' / manifest['exec']
    if args.action == 'info':
        print(f"{manifest['id']} {manifest['version']} ({manifest['architecture']})\n{binary}")
        return
    engine_file = app / 'engine.json'
    if engine_file.exists():
        engine = json.loads(engine_file.read_text())
        cache = ROOT / 'out/browser-engine' / f"servo-{engine['version']}.tar.gz"
        bundled = app / 'assets/servo.tar.gz'
        if not bundled.exists() or hashlib.sha256(bundled.read_bytes()).hexdigest() != engine['sha256']:
            cache.parent.mkdir(parents=True, exist_ok=True)
            if not cache.exists() or hashlib.sha256(cache.read_bytes()).hexdigest() != engine['sha256']:
                print(f"Fetching pinned Servo {engine['version']}", flush=True)
                with urllib.request.urlopen(engine['url'], timeout=120) as src, cache.open('wb') as dst:
                    shutil.copyfileobj(src, dst)
            if hashlib.sha256(cache.read_bytes()).hexdigest() != engine['sha256']:
                parser.error('Servo download checksum mismatch')
            bundled.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(cache, bundled)
        shutil.copyfile(engine_file, app / 'assets/engine.json')
    if upstream:
        if args.action != 'info':
            binary = fetch_upstream(app, upstream)
    elif not args.skip_build:
        features = ['--features', 'springboard/desktop-wayland'] if 'springboard' in cargo.get('dependencies', {}) else []
        run(['cargo', 'build', '--offline', '--profile', 'fast', '-p', cargo['package']['name'],
             '--bin', manifest['exec'], *features], ROOT / 'springboard')
    if args.action == 'build':
        print(binary)
        return
    out = ROOT / 'out/pc-apps'
    stripped = out / 'bin' / manifest['exec']
    stripped.parent.mkdir(parents=True, exist_ok=True)
    if binary != stripped:
        shutil.copy2(binary, stripped)
    run(['strip', '--strip-unneeded', stripped])
    run([sys.executable, ROOT / 'springboard/tools/aap.py', 'pack', app, '--bin', stripped, '--out', out])
    package = out / f"{manifest['id']}-{manifest['version']}.aap"
    # licence guard: an app package carries only BifrOSt's own and openly licensed files. A failing package is deleted,
    # so neither a publish nor the ISO bundler (tools/bundle-iso-offline.py reads out/pc-apps) can pick it up.
    if subprocess.run([sys.executable, str(ROOT / 'tools/check-package-licenses.py'), str(package)]).returncode:
        package.unlink(missing_ok=True)
        sys.exit(f"refusing {package.name}: the licence guard failed (see above); the package was deleted")
    if args.action == 'pack':
        # a local package for lab testing (`springboard --install-package`), not published
        print(package)
        return
    key = os.environ.get('BIFROST_PUBLISHER_KEY')
    if key:
        # detached ML-DSA-65 signature next to the package (`springboard --sign-package`); the shell checks it when a
        # publisher key is pinned under /etc/bifrost/publisher-keys
        run([os.environ.get('BIFROST_SPRINGBOARD', 'springboard'), '--sign-package', key, package])
    run([Path.home() / 'Projects/bifrost-site/tools/publish-app.sh', package])
    print('Published to the PC App Store. Install/update from App Store on Midget.')


if __name__ == '__main__':
    main()
