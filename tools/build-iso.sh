#!/usr/bin/env bash
# Build the BifrOSt installer ISO.
#   tools/build-iso.sh [--lab|--release] [--skip-cargo] [--deploy]
#     --debug    lab + full trace: graphical splash + journal diagnostics streamed to titan
#                (run tools/lab-log-sink.py), BIFROST_TRACE for the installer
#     --lab      (default) adds remote access for testing: root SSH key (out/lab), bifrost-lab token + API
#     --release  hardened: no SSH key, no lab token or service, no lab hooks
#     --deploy   copy the ISO into iVentoy's iso/ folder under a NEW name (never replaces a served file)
# Needs sudo (mkarchiso). Work dirs live in out/; they are unmounted before removal and removed with
# --one-file-system (a work dir can still hold bind mounts of /dev and /run after an interrupted build).
set -euo pipefail
cd "$(dirname "$0")/.."
MODE=lab; CARGO=1; DEPLOY=0; DEBUG=0
CPROF=fast
for a in "$@"; do case $a in --lab) MODE=lab ;; --debug) MODE=lab; DEBUG=1 ;; --release) MODE=release; CPROF=release ;; --skip-cargo) CARGO=0 ;; --deploy) DEPLOY=1 ;; *) echo "unknown option $a" >&2; exit 2 ;; esac; done

KEEP=3        # newest local ISO builds kept in out/
KEEP_WORK=1   # archiso work dirs kept (a successful build removes its own; only interrupted ones remain)
# unmount anything still mounted inside a work dir first, then remove it without crossing mounts
remove_work() {
  if findmnt -R "$1" >/dev/null 2>&1; then sudo umount -R -l "$1" || true; fi
  [ -z "$(findmnt -R -n "$1" 2>/dev/null)" ] && sudo rm -rf --one-file-system "$1"
  return 0
}
PROFILE=out/bifrost-iso; AIROOT=$PROFILE/airootfs; PKG=packaging/archiso/airootfs; STAMP=$(date +%Y%m%d-%H%M%S)
WORK=out/archiso-work-$STAMP; OUT=out/iso-$MODE-$STAMP

PD=$CPROF
if [ "$CARGO" = 1 ]; then
  (cd springboard && cargo build --profile "$CPROF" --features desktop-wayland --bin shell --bin installer --bin bootweave)
fi
tools/build-efi-boot.sh "$CPROF"
python3 tools/apply-archiso-profile.py "$PROFILE"
# The live installer and installed shell need these generated assets; never inherit a stale profile copy. Preparing the
# assets (including any font fetches) is a separate build-time step, not an ISO-build/network fallback.
ASSET_SRC=${BIFROST_ISO_ASSETS_DIR:-springboard/assets}
for file in SFUI.ttf SFUIMono.ttf Symbols.ttf wallpaper.raw wallpapers/index.ini \
            icons/bifrost.settings.raw icons/bifrost.appstore.raw icons/bifrost.files.raw icons/bifrost.browser.raw; do
  [ -s "$ASSET_SRC/$file" ] || { echo "required ISO asset missing: $ASSET_SRC/$file (prepare assets before building)" >&2; exit 1; }
done
ASSET_DEST="$AIROOT/usr/share/springboard/assets"
mkdir -p "$ASSET_DEST"
rsync -a --delete "$ASSET_SRC/" "$ASSET_DEST/"
# Installer footage is a required live-media input. The canonical source is the archiso overlay; lab builds can point
# at the already-published payload explicitly without depending on an old AIROOT tree.
VIDEO_SRC=${BIFROST_ISO_VIDEO_DIR:-packaging/archiso/airootfs/usr/share/bifrost/video}
for file in Godafoss_1080.mp4 Svartifoss_1080.mp4; do
  [ -s "$VIDEO_SRC/$file" ] || { echo "required installer video missing: $VIDEO_SRC/$file (set BIFROST_ISO_VIDEO_DIR to a prepared source directory)" >&2; exit 1; }
done
VIDEO_DEST="$AIROOT/usr/share/bifrost/video"
mkdir -p "$VIDEO_DEST"
install -m644 "$VIDEO_SRC/Godafoss_1080.mp4" "$VIDEO_DEST/Godafoss_1080.mp4"
install -m644 "$VIDEO_SRC/Svartifoss_1080.mp4" "$VIDEO_DEST/Svartifoss_1080.mp4"
rustc --edition 2021 -O -C target-feature=+crt-static -C strip=symbols packaging/arch/recovery/guard.rs -o out/bifrost-update-guard-iso
install -Dm755 out/bifrost-update-guard-iso "$AIROOT/usr/bin/bifrost-update-guard"
install -Dm644 packaging/arch/recovery/bifrost-update-guard.service "$AIROOT/usr/lib/systemd/system/bifrost-update-guard.service"
install -m755 springboard/target/$PD/shell "$AIROOT/usr/bin/springboard"
install -m755 springboard/target/$PD/installer "$AIROOT/usr/bin/bifrost-installer"
install -m755 springboard/target/$PD/bootweave "$AIROOT/usr/bin/bifrost-bootweave"
# debug info only: the symbol table stays, so a crash backtrace still names its functions
strip --strip-debug "$AIROOT/usr/bin/springboard" "$AIROOT/usr/bin/bifrost-installer"
install -Dm644 packaging/arch/springboard.service "$AIROOT/usr/lib/systemd/system/springboard.service"
install -Dm755 packaging/arch/wayland/session "$AIROOT/usr/lib/bifrost/session"
install -Dm755 packaging/arch/wayland/session-env "$AIROOT/usr/lib/bifrost/session-env"
install -Dm755 packaging/arch/wayland/open-url "$AIROOT/usr/bin/bifrost-open-url"
install -Dm644 packaging/arch/wayland/bifrost-browser.desktop "$AIROOT/usr/share/applications/bifrost-browser.desktop"
install -Dm644 packaging/arch/wayland/mimeapps.list "$AIROOT/etc/xdg/mimeapps.list"
install -Dm755 packaging/arch/wayland/apps "$AIROOT/usr/bin/bifrost-apps"
W="$AIROOT/etc/systemd/system"
mkdir -p "$W/graphical.target.wants" "$W/multi-user.target.wants"

if [ "$MODE" = lab ]; then
  mkdir -p "$AIROOT/root/.ssh" "$AIROOT/etc/bifrost"
  cp out/lab/id_ed25519.pub "$AIROOT/root/.ssh/authorized_keys"
  cp out/lab/token "$AIROOT/etc/bifrost/lab.token"
  ln -sfn /usr/lib/systemd/system/bifrost-lab.service "$W/multi-user.target.wants/bifrost-lab.service"
else
  rm -rf "$AIROOT/root/.ssh" "$AIROOT/etc/bifrost/lab.token" "$W/multi-user.target.wants/bifrost-lab.service" "$AIROOT/usr/local/bin/bifrost-lab" "$AIROOT/usr/lib/systemd/system/bifrost-lab.service"
fi
# debug trace (lab only)
DBG="quiet loglevel=3 log_buf_len=16M systemd.log_level=debug systemd.log_target=journal systemd.show_status=0 rd.systemd.show_status=0 rd.udev.log_level=3 vt.global_cursor_default=0 i915.enable_fbc=0 i915.enable_psr=0 threadirqs"
if [ "$DEBUG" = 1 ]; then
  mkdir -p "$AIROOT/etc/bifrost" "$AIROOT/etc/systemd/system/bifrost-installer.service.d"
  : > "$AIROOT/etc/bifrost/debug"
  mkdir -p "$W/sysinit.target.wants"; ln -sfn /usr/lib/systemd/system/bifrost-debug-log.service "$W/sysinit.target.wants/bifrost-debug-log.service"
  printf '[Service]\nEnvironment=BIFROST_TRACE=1 RUST_BACKTRACE=1\n' > "$AIROOT/etc/systemd/system/bifrost-installer.service.d/debug.conf"
  echo "$DBG" > "$PROFILE/bifrost-cmdline"
else
  rm -rf "$AIROOT/etc/bifrost/debug" "$AIROOT/etc/systemd/system/bifrost-installer.service.d" "$W/multi-user.target.wants/bifrost-debug-log.service" "$W/sysinit.target.wants/bifrost-debug-log.service" "$PROFILE/bifrost-cmdline"
fi
# lab hosts fetch the newest installer from titan at boot (tools/lab-push.sh)
if [ "$MODE" = lab ]; then
  # the sync runs beside the installer (which starts at once) and restarts it only when a fetched file changed
  rm -f "$AIROOT/etc/systemd/system/bifrost-installer.service.d/lab-sync.conf"
  printf '[Unit]\nDescription=BifrOSt lab installer sync\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart=-/usr/local/bin/bifrost-lab-sync\n\n[Install]\nWantedBy=multi-user.target\n' > "$AIROOT/etc/systemd/system/bifrost-lab-sync.service"
  mkdir -p "$W/multi-user.target.wants"; ln -sfn /etc/systemd/system/bifrost-lab-sync.service "$W/multi-user.target.wants/bifrost-lab-sync.service"
else
  rm -f "$AIROOT/etc/systemd/system/bifrost-installer.service.d/lab-sync.conf" "$AIROOT/usr/local/bin/bifrost-lab-sync" "$AIROOT/etc/systemd/system/bifrost-lab-sync.service" "$W/multi-user.target.wants/bifrost-lab-sync.service"
fi
if [ "$MODE" = release ]; then rm -f "$AIROOT/usr/local/bin/bifrost-debug-log" "$AIROOT/usr/lib/systemd/system/bifrost-debug-log.service"; fi
python3 - "$PROFILE/profiledef.sh" "$MODE" <<'PY'
import sys
p, mode = sys.argv[1:]
s = open(p).read()
perm = {
    "/usr/bin/bifrost-installer": "0:0:755",
    "/usr/bin/springboard": "0:0:755",
    "/usr/lib/bifrost/session": "0:0:755",
    "/usr/lib/bifrost/session-env": "0:0:755",
    "/usr/bin/bifrost-bootweave": "0:0:755",
    "/usr/bin/bifrost-apps": "0:0:755",
    "/usr/bin/bifrost-open-url": "0:0:755",
}
if mode == "lab":
    perm.update({"/root/.ssh": "0:0:700", "/root/.ssh/authorized_keys": "0:0:600", "/usr/local/bin/bifrost-lab": "0:0:755", "/usr/local/bin/bifrost-debug-log": "0:0:755", "/usr/local/bin/bifrost-lab-sync": "0:0:755", "/etc/bifrost/lab.token": "0:0:600"})
else:
    for k in ("/root/.ssh", "/root/.ssh/authorized_keys", "/usr/local/bin/bifrost-lab", "/etc/bifrost/lab.token"):
        s = "".join(l for l in s.splitlines(True) if f'["{k}"]' not in l)
add = "".join(f'  ["{k}"]="{v}"\n' for k, v in perm.items() if f'["{k}"]' not in s)
open(p, "w").write(s.replace("file_permissions=(\n", "file_permissions=(\n" + add, 1))
PY

mkdir -p "$OUT"
sudo python3 tools/prepare-mkarchiso.py -v -w "$WORK" -o "$OUT" -m iso -L BIFROST "$PROFILE"
sudo chown -R "$USER" "$OUT"
ISO=$(ls "$OUT"/*.iso | head -1)
remove_work "$WORK"
echo "ISO ($MODE): $ISO"
# keep at most KEEP local ISO builds and KEEP_WORK work dirs left behind by interrupted builds
{ ls -dt out/iso-lab-* out/iso-release-* 2>/dev/null || true; } | tail -n +$((KEEP + 1)) | while read -r d; do rm -rf --one-file-system "$d"; done
{ ls -dt out/archiso-work-* 2>/dev/null || true; } | tail -n +$((KEEP_WORK + 1)) | while read -r d; do remove_work "$d"; done
if [ "$DEPLOY" = 1 ]; then
  D=out/iventoy/iventoy-1.0.44/iso; NAME=bifrost-$MODE$([ "$DEBUG" = 1 ] && echo -debug)-$STAMP.iso
  cp --reflink=auto "$ISO" "$D/$NAME"
  curl -s http://127.0.0.1:26000/iventoy/json -d '{"method":"refresh_img_list"}' >/dev/null || true
  echo "deployed to iVentoy as $NAME"
  tools/iventoy-prune.sh 1
fi
