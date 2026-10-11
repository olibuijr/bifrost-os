#!/usr/bin/env bash
# Push the current installer build to the lab hosts that boot a --lab/--debug ISO: they fetch it at boot (bifrost-lab-sync),
# so no ISO rebuild is needed. usage: tools/lab-push.sh [--skip-cargo | --installer-only]
# --installer-only (midget.py reinstall): the installed system's shell comes from the package repo, so the live copy is
# only rebuilt when there is none; relinking it cost ~45 s after every installer change
set -euo pipefail
cd "$(dirname "$0")/.."
SYNC=out/lab/sync; mkdir -p "$SYNC"
BINS="--bin installer --bin shell"
[ "${1:-}" = --installer-only ] && [ -x springboard/target/fast/shell ] && BINS="--bin installer"
# shellcheck disable=SC2086
[ "${1:-}" = --skip-cargo ] || (cd springboard && cargo build --profile fast --features desktop-wayland $BINS)
# the guard (guard.rs and its modules) only when a source is newer than the last build
[ -f out/bifrost-update-guard-lab ] && [ -z "$(find packaging/arch/recovery -name '*.rs' -newer out/bifrost-update-guard-lab)" ] ||
  rustc --edition 2021 -O -C target-feature=+crt-static -C strip=symbols packaging/arch/recovery/guard.rs -o out/bifrost-update-guard-lab
install -m755 out/bifrost-update-guard-lab "$SYNC/bifrost-update-guard"
cp packaging/arch/recovery/splash.efi "$SYNC/splash.efi"
cp packaging/arch/recovery/bifrost-update-guard.service "$SYNC/bifrost-update-guard.service"
install -m755 springboard/target/fast/installer "$SYNC/bifrost-installer"
install -m755 springboard/target/fast/shell "$SYNC/springboard"
# stripped like the ISO's copies, so an unchanged build matches byte for byte and lab-sync does not restart the installer
strip --strip-debug "$SYNC/bifrost-installer" "$SYNC/springboard"
for f in session session-env apps environment autostart rc.xml open-url bifrost-browser.desktop mimeapps.list; do
  cp "packaging/arch/wayland/$f" "$SYNC/wayland-$f"
done
cp packaging/archiso/airootfs/usr/lib/systemd/system/bifrost-installer.service "$SYNC/bifrost-installer.service"
# the App Store's packages over the lab link: lab installs fetch the default apps here (checksums from the HTTPS catalogue)
STORE=${BIFROST_STORE_DIR:-$HOME/Projects/bifrost-site/data/store}
[ -d "$STORE" ] && ln -sfn "$STORE" "$SYNC/store"
D=packaging/archiso/airootfs/usr/share/bifrost
cp "$D/installer/logo.png" "$D/installer/bg.jpg" "$SYNC/"
cp "$D"/video/*.mp4 "$SYNC/" 2>/dev/null || true
cat > "$SYNC/manifest" <<M
bifrost-installer /usr/bin/bifrost-installer 755
springboard /usr/bin/springboard 755
splash.efi /usr/share/bifrost/splash.efi 644
bifrost-update-guard /usr/bin/bifrost-update-guard 755
bifrost-update-guard.service /usr/lib/systemd/system/bifrost-update-guard.service 644
wayland-open-url /usr/bin/bifrost-open-url 755
wayland-bifrost-browser.desktop /usr/share/applications/bifrost-browser.desktop 644
wayland-mimeapps.list /etc/xdg/mimeapps.list 644
wayland-session /usr/lib/bifrost/session 755
wayland-session-env /usr/lib/bifrost/session-env 755
wayland-apps /usr/bin/bifrost-apps 755
wayland-environment /usr/share/bifrost/wayland/environment 644
wayland-autostart /usr/share/bifrost/wayland/autostart 644
wayland-rc.xml /usr/share/bifrost/wayland/rc.xml 644
bifrost-installer.service /usr/lib/systemd/system/bifrost-installer.service 644
logo.png /usr/share/bifrost/installer/logo.png 644
bg.jpg /usr/share/bifrost/installer/bg.jpg 644
Godafoss_1080.mp4 /usr/share/bifrost/video/Godafoss_1080.mp4 644
Svartifoss_1080.mp4 /usr/share/bifrost/video/Svartifoss_1080.mp4 644
M
ss -ltn | grep -q ":8078 " || (cd "$SYNC" && setsid python3 -m http.server 8078 --bind 10.99.0.1 >/dev/null 2>&1 </dev/null &)
echo "pushed $(stat -c%s "$SYNC/bifrost-installer") bytes; lab hosts pick it up on their next boot (http://10.99.0.1:8078)"
