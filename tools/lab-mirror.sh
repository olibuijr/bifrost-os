#!/usr/bin/env bash
# Offline package mirror for lab installs, the approach of Omarchy's ISO (builder/build-iso.sh, pacman-offline.conf):
# every package the installer pacstraps (BASE_PACKAGES, runtime-packages.txt and every GPU package, from
# springboard/src/installer/plan.rs) with its whole dependency closure, downloaded once with an empty private pacman
# database and indexed as the repo "offline". The lab server serves it at http://10.99.0.1:8078/mirror/, and a live
# lab system pacstraps from it instead of the internet (tools/vm-cycle.py). Re-running fetches only what changed.
# usage: tools/lab-mirror.sh [--list | --refresh]
set -euo pipefail
cd "$(dirname "$0")/.."
MIRROR=out/lab/sync/mirror
STATE=${XDG_DATA_HOME:-$HOME/.local/share}/bifrost-lab
DB=$STATE/mirror-db
CONF=$STATE/pacman-mirror.conf

packages() {
  python3 - <<'PY'
import re
plan = open("springboard/src/installer/plan.rs").read()
base = re.search(r"BASE_PACKAGES: &\[&str\] = &\[(.*?)\];", plan, re.S).group(1)
gpu = re.search(r"pub fn gpu_packages\(.*?\n}\n", plan, re.S).group(0)
names = re.findall(r'"([a-z0-9][a-z0-9+._-]*)"', base) + open("packaging/arch/runtime-packages.txt").read().split()
names += [n for n in re.findall(r'"([a-z0-9][a-z0-9+._-]*)"', gpu) if not n.startswith(("[", "intel corp", "advanced", "nvidia"))]
names += ["nvidia-open", "nvidia-utils", "libva-nvidia-driver", "mkinitcpio", "grub"]
seen = []
for n in names:
    if n not in seen and " " not in n:
        seen.append(n)
print(" ".join(seen))
PY
}

if [ "${1:-}" = --list ]; then packages | tr ' ' '\n'; exit 0; fi

mkdir -p "$MIRROR" "$DB"
# pacman -Syw re-verifies the signature of every cached package (~35 s for the whole set) even when nothing changed, so
# the same package list within LAB_MIRROR_MAX_AGE (default 24 h) keeps the mirror as is; --refresh forces a sync
want=$(packages | sha256sum | cut -d' ' -f1)
if [ "${1:-}" != --refresh ] && [ -s "$MIRROR/offline.db.tar.gz" ] && [ "$(cat "$STATE/mirror.list" 2>/dev/null)" = "$want" ] \
   && [ -n "$(find "$STATE/mirror.list" -mmin -$((${LAB_MIRROR_MAX_AGE:-1440})) 2>/dev/null)" ]; then
  echo "lab mirror: fresh ($(ls "$MIRROR"/*.pkg.tar.zst | wc -l) packages, synced $(date -r "$STATE/mirror.list" +%H:%M)); --refresh to sync now"
  exit 0
fi
# only the Arch repositories, from titan's mirror list (titan's own extra repos must not leak into installs)
cat > "$CONF" <<EOF
[options]
Architecture = auto
ParallelDownloads = 8
SigLevel = Required DatabaseOptional
[core]
Include = /etc/pacman.d/mirrorlist
[extra]
Include = /etc/pacman.d/mirrorlist
EOF
# shellcheck disable=SC2046
sudo pacman --config "$CONF" --dbpath "$DB" --cachedir "$(realpath "$MIRROR")" --noconfirm -Syw $(packages)
sudo chown -R "$(id -u):$(id -g)" "$MIRROR" "$DB"
# the newest version of each package only, then the repo index; repo-add hashes every package (about a minute for the
# whole set), so an unchanged set keeps its index
command -v paccache >/dev/null && paccache -q -r -k1 -c "$MIRROR" || true
set_now=$(cd "$MIRROR" && ls *.pkg.tar.zst | sha256sum | cut -d' ' -f1)
if [ -s "$MIRROR/offline.db.tar.gz" ] && [ "$(cat "$MIRROR/.indexed" 2>/dev/null)" = "$set_now" ]; then
  echo "lab mirror: index unchanged"
else
  rm -f "$MIRROR"/offline.db* "$MIRROR"/offline.files*
  repo-add -q "$MIRROR/offline.db.tar.gz" "$MIRROR"/*.pkg.tar.zst
  echo "$set_now" > "$MIRROR/.indexed"
fi
echo "$want" > "$STATE/mirror.list"
echo "lab mirror: $(ls "$MIRROR"/*.pkg.tar.zst | wc -l) packages, $(du -sh "$MIRROR" | cut -f1), http://10.99.0.1:8078/mirror/"
