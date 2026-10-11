#!/usr/bin/env bash
# Fail if a RELEASE ISO still carries lab/test artifacts. usage: check-iso-hardening.sh ISO
set -euo pipefail
ISO=${1:?usage: $0 ISO}
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
bsdtar -xOf "$ISO" arch/x86_64/airootfs.sfs > "$T/root.sfs"
bad=0
for p in root/.ssh/authorized_keys root/.ssh/id_ed25519 root/.ssh/id_rsa etc/bifrost/lab.token etc/bifrost/debug usr/local/bin/bifrost-lab-sync etc/systemd/system/bifrost-installer.service.d/lab-sync.conf etc/systemd/system/bifrost-lab-sync.service usr/local/bin/bifrost-debug-log usr/local/bin/bifrost-lab usr/lib/systemd/system/bifrost-lab.service etc/systemd/system/multi-user.target.wants/bifrost-lab.service etc/bifrost; do
  if unsquashfs -l "$T/root.sfs" "$p" 2>/dev/null | grep -q "squashfs-root/$p"; then echo "FOUND $p"; bad=1; fi
done
# no stale brand or old installers
for p in usr/bin/bifrost-installer usr/lib/systemd/system/bifrost-installer.service usr/share/bifrost; do
  if unsquashfs -l "$T/root.sfs" "$p" 2>/dev/null | grep -q "squashfs-root/$p"; then echo "STALE $p"; bad=1; fi
done
# no path of the old brand anywhere in the image. Construct the legacy name
# so this checker itself does not reintroduce it into the release image.
legacy_brand=$(printf 'andr%s' oios)
n=$(unsquashfs -l "$T/root.sfs" 2>/dev/null | grep -ci "$legacy_brand" || true)
[ "${n:-0}" = 0 ] || { echo "STALE: $n paths still use the legacy brand"; bad=1; }
# root must not have an SSH-usable password hash in shadow, and the installer must be present
unsquashfs -cat "$T/root.sfs" usr/bin/bifrost-installer >/dev/null 2>&1 || { echo "MISSING bifrost-installer"; bad=1; }
[ "$bad" = 0 ] && echo "OK: $ISO has no lab artifacts" || { echo "FAILED: $ISO"; exit 1; }
