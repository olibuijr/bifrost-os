# Shared system payload. Caller supplies package root, binary and static guard.
package_bifrost_files() {
  local here=$1 binary=$2 guard=$3
  install -Dm755 "$here/wayland/open-url" "$pkgdir/usr/bin/bifrost-open-url"
  install -Dm644 "$here/wayland/bifrost-browser.desktop" "$pkgdir/usr/share/applications/bifrost-browser.desktop"
  install -Dm644 "$here/wayland/mimeapps.list" "$pkgdir/etc/xdg/mimeapps.list"
  install -Dm644 "$here/recovery/bootweave.efi" "$pkgdir/usr/share/bifrost/bootweave.efi"
  install -Dm755 "$guard" "$pkgdir/usr/bin/bifrost-update-guard"
  install -Dm644 "$here/recovery/bifrost-update-guard.service" "$pkgdir/usr/lib/systemd/system/bifrost-update-guard.service"
  install -Dm644 "$here/recovery/bifrost-rollback.service" "$pkgdir/usr/lib/systemd/system/bifrost-rollback.service"
  install -Dm755 "$binary" "$pkgdir/usr/bin/springboard"
  if [[ -n ${PC_BOOTWEAVE:-} && -x ${PC_BOOTWEAVE} ]]; then
    install -Dm755 "$PC_BOOTWEAVE" "$pkgdir/usr/bin/bifrost-bootweave"
  fi
  # Settings portal: native apps (GTK, Qt, Firefox, winit) follow the shell's Light/Dark and accent.
  if [[ -n ${PC_PORTAL:-} && -x ${PC_PORTAL} ]]; then
    install -Dm755 "$PC_PORTAL" "$pkgdir/usr/bin/bifrost-portal"
  fi
  # MCP server for agents (docs/bifrost-mcp.md): run over ssh as a stdio server.
  if [[ -n ${PC_MCP:-} && -x ${PC_MCP} ]]; then
    install -Dm755 "$PC_MCP" "$pkgdir/usr/bin/bifrost-mcp"
  fi
  install -Dm644 "$here/bifrost-mcp.service" "$pkgdir/usr/lib/systemd/system/bifrost-mcp.service"
  # socket-activated services (docs/boot-architecture.md "Services on demand"): the supervisor, the target it runs
  # under, and the trigger that starts the target on the first desktop frame
  if [[ -n ${PC_BIFROSTD:-} && -x ${PC_BIFROSTD} ]]; then
    install -Dm755 "$PC_BIFROSTD" "$pkgdir/usr/bin/bifrostd"
  fi
  install -Dm644 "$here/bifrostd.service" "$pkgdir/usr/lib/systemd/system/bifrostd.service"
  install -Dm644 "$here/bifrost-desktop-ready.service" "$pkgdir/usr/lib/systemd/system/bifrost-desktop-ready.service"
  install -Dm644 "$here/bifrost-after-desktop.target" "$pkgdir/usr/lib/systemd/system/bifrost-after-desktop.target"
  install -Dm644 "$here/dbus/org.freedesktop.portal.Desktop.service" "$pkgdir/usr/share/dbus-1/services/org.freedesktop.portal.Desktop.service"
  install -Dm644 "$here/springboard.service" "$pkgdir/usr/lib/systemd/system/springboard.service"
  # login round: root helper, PAM service, privd socket group (the user-session migration is a later stage)
  # (PC_PRIVD overrides; otherwise bifrost-privd is looked up next to the shell binary)
  privd=${PC_PRIVD:-$(dirname "$binary")/bifrost-privd}
  if [[ -x $privd ]]; then
    install -Dm755 "$privd" "$pkgdir/usr/bin/bifrost-privd"
  fi
  install -Dm644 "$here/bifrost-privd.service" "$pkgdir/usr/lib/systemd/system/bifrost-privd.service"
  install -Dm644 "$here/pam-bifrost-login" "$pkgdir/etc/pam.d/bifrost-login"
  install -Dm644 "$here/sysusers-bifrost.conf" "$pkgdir/usr/lib/sysusers.d/bifrost.conf"
  # same-uid processes (Term, apps) must not be able to ptrace the shell, which holds the session token
  install -Dm644 "$here/sysctl-bifrost.conf" "$pkgdir/usr/lib/sysctl.d/60-bifrost.conf"
  install -Dm755 "$here/seed-data" "$pkgdir/usr/lib/springboard/seed-data"
  # boot speed: the initrd mounts the fstab subvolumes, and seatd starts without waiting for local-fs.target
  install -Dm644 "$here/initcpio/install/bifrost-mounts" "$pkgdir/usr/lib/initcpio/install/bifrost-mounts"
  install -Dm644 "$here/initcpio/hooks/bifrost-mounts" "$pkgdir/usr/lib/initcpio/hooks/bifrost-mounts"
  install -Dm644 "$here/seatd-early.conf" "$pkgdir/usr/lib/systemd/system/seatd.service.d/bifrost-early.conf"
  for file in session session-env; do
    install -Dm755 "$here/wayland/$file" "$pkgdir/usr/lib/bifrost/$file"
  done
  install -Dm755 "$here/wayland/video-arg" "$pkgdir/usr/lib/bifrost/video-arg"
  # Bootweave owns the EFI-frame-to-desktop lifecycle; no text console is exposed.
  for unit in getty@tty1.service; do
    install -d "$pkgdir/etc/systemd/system"
    ln -sfn /dev/null "$pkgdir/etc/systemd/system/$unit"
  done
  install -Dm755 "$here/wayland/apps" "$pkgdir/usr/bin/bifrost-apps"
  for file in environment autostart rc.xml; do
    install -Dm644 "$here/wayland/$file" "$pkgdir/usr/share/bifrost/wayland/$file"
  done
  # labwc title-bar theme: both variants for the shell, Light as the system default.
  for variant in light dark; do
    for f in "$here/wayland/themes/$variant/labwc/"*; do
      install -Dm644 "$f" "$pkgdir/usr/share/bifrost/wayland/themes/$variant/labwc/${f##*/}"
    done
  done
  for f in "$here/wayland/themes/light/labwc/"*; do
    install -Dm644 "$f" "$pkgdir/usr/share/themes/BifrOSt/labwc/${f##*/}"
  done
}
