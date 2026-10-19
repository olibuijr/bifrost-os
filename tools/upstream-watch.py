#!/usr/bin/env python3
"""Follow upstream releases for prebuilt apps (springboard/apps/*/upstream.toml).

For each app whose latest GitHub release is newer than manifest.toml: pin the new tarball (url, sha256),
bump the manifest, prepend a CHANGELOG entry, publish with tools/pc-app.py and commit the app directory.
"""
import argparse
import hashlib
import json
import re
import subprocess
import sys
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TIMEOUT = 60


def get(url, limit=None):
    req = urllib.request.Request(url, headers={'User-Agent': 'bifrost-upstream-watch', 'Accept': 'application/vnd.github+json'})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
        return r.read()


def ver(s):
    return tuple(int(x) for x in s.lstrip('v').split('.'))


def repo_of(up):
    if up.get('repo'):
        return up['repo']
    m = re.match(r'https://github\.com/([^/]+/[^/]+)/', up['url'])
    if not m:
        raise ValueError('no repo in upstream.toml and url is not a GitHub release')
    return m.group(1)


def update(app, args):
    up = tomllib.loads((app / 'upstream.toml').read_text())
    manifest = (app / 'manifest.toml').read_text()
    current = re.search(r'^version\s*=\s*"([^"]+)"', manifest, re.M).group(1)
    repo = repo_of(up)
    rel = json.loads(get(f'https://api.github.com/repos/{repo}/releases/latest'))
    latest = rel['tag_name'].lstrip('v')
    # the packaged upstream release (the url's tag); the package version may run ahead of it after a packaging fix
    packaged = re.search(r'/download/v?([0-9][^/]*)/', up['url']).group(1)
    if ver(latest) <= ver(packaged):
        return f'up to date ({packaged})'
    if ver(latest) <= ver(current):
        bumped = current.split('.')
        latest_pkg = '.'.join(bumped[:2] + [str(int(bumped[2]) + 1)])
    else:
        latest_pkg = latest
    asset = next((a for a in rel.get('assets', []) if a['name'].endswith('-linux-x86_64.tar.gz')), None)
    if not asset:
        return f'{latest} has no linux-x86_64 tarball'
    if args.dry_run:
        return f'update available {current} -> {latest}'
    data = get(asset['browser_download_url'])
    sha = hashlib.sha256(data).hexdigest()
    name = up['binary'].split('/')[-1]
    old_base = up['binary'].split('/')[0]
    new_base = asset['name'][:-len('.tar.gz')]
    upstream_text = (app / 'upstream.toml').read_text()
    # the url first: replacing the base name first changes the url so it no longer matches (the tag stays stale)
    upstream_text = upstream_text.replace(up['url'], asset['browser_download_url']).replace(old_base, new_base)
    upstream_text = upstream_text.replace(up['sha256'], sha)
    (app / 'upstream.toml').write_text(upstream_text)
    (app / 'manifest.toml').write_text(re.sub(r'^version\s*=\s*"[^"]+"', f'version = "{latest_pkg}"', manifest, count=1, flags=re.M))
    notes = [l.rstrip() for l in (rel.get('body') or '').splitlines() if l.strip()][:5]
    entry = f'## {latest_pkg}\n\n- Upstream {name} v{latest}.\n' + ''.join(f'  {l}\n' for l in notes)
    log = app / 'CHANGELOG.md'
    text = log.read_text()
    head, _, rest = text.partition('\n## ')
    log.write_text(f'{head}\n{entry}\n## {rest}' if rest else f'{text.rstrip()}\n\n{entry}')
    subprocess.run([sys.executable, ROOT / 'tools/pc-app.py', 'publish', app.name], cwd=ROOT, check=True)
    subprocess.run(['git', 'add', str(app)], cwd=ROOT, check=True)
    subprocess.run(['git', 'commit', '-m', f'feat({name}): upstream {name} v{latest}', '--', str(app)], cwd=ROOT, check=True)
    return f'updated {current} -> {latest} and published'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dry-run', action='store_true', help='report only')
    parser.add_argument('--app', help='only this app, e.g. photocraft or photocraft-desktop')
    args = parser.parse_args()
    failed = 0
    for up in sorted((ROOT / 'springboard/apps').glob('*/upstream.toml')):
        app = up.parent
        if args.app and args.app not in (app.name, app.name.removesuffix('-desktop')):
            continue
        try:
            msg = update(app, args)
        except Exception as e:  # one app failing must not stop the others
            failed += 1
            msg = f'FAILED: {e}'
        print(f'{app.name}: {msg}', flush=True)
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
