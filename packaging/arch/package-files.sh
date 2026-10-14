# Shared system payload. Caller supplies package root, binary and static guard.
package_bifrost_files() {
  local here=$1 binary=$2 guard=$3
  install -Dm755 "$here/wayland/open-url" "$pkgdir/usr/bin/bifrost-open-url"
  install -Dm644 "$here/wayland/bifrost-browser.desktop" "$pkgdir/usr/share/applications/bifrost-browser.desktop"
  install -Dm644 "$here/wayland/mimeapps.list" "$pkgdir/etc/xdg/mimeapps.list"
  # refuse a loader older than its source (callers build it with tools/build-efi-boot.sh first)
  "$here/../../tools/verify-efi-loader.sh" "$here/recovery/bootweave.efi" || return 1
  install -Dm644 "$here/recovery/bootweave.efi" "$pkgdir/usr/share/bifrost/bootweave.efi"
  install -Dm755 "$guard" "$pkgdir/usr/bin/bifrost-update-guard"
  install -Dm644 "$here/recovery/bifrost-update-guard.service" "$pkgdir/usr/lib/systemd/system/bifrost-update-guard.service"
  install -Dm644 "$here/recovery/bifrost-rollback.service" "$pkgdir/usr/lib/systemd/system/bifrost-rollback.service"
  # unified kernel image after each kernel install (docs/boot-architecture.md, "Prepared, not probed")
  install -Dm644 "$here/recovery/95-bifrost-uki.hook" "$pkgdir/usr/share/libalpm/hooks/95-bifrost-uki.hook"
  install -Dm644 "$here/recovery/bifrost-modules-record.service" "$pkgdir/usr/lib/systemd/system/bifrost-modules-record.service"
  install -Dm755 "$binary" "$pkgdir/usr/bin/springboard"
  if [[ -n ${PC_BOOTWEAVE:-} && -x ${PC_BOOTWEAVE} ]]; then
    install -Dm755 "$PC_BOOTWEAVE" "$pkgdir/usr/bin/bifrost-bootweave"
  fi
  # Settings portal: native apps (GTK, Qt, Firefox, winit) follow the shell's Light/Dark and accent.
  if [[ -n ${PC_PORTAL:-} && -x ${PC_PORTAL} ]]; then
    install -Dm755 "$PC_PORTAL" "$pkgdir/usr/bin/bifrost-portal"
  fi
  # MCP server for agents (docs/bifrost-mcp.md): run over ssh as a stdio server.
  # the units below are enabled on every install: a package without their binaries leaves them failing at exec
  # (0.6.0/0.6.1 shipped no bifrostd, so nothing behind bifrost-after-desktop.target ever started), so refuse to build one
  PC_AUDIOD=${PC_AUDIOD:-$(dirname "$binary")/bifrost-audiod}
  for need in PC_MCP PC_BIFROSTD PC_AUDIOD; do
    if [[ -z ${!need:-} || ! -x ${!need} ]]; then
      echo "package-files.sh: $need is not an executable (${!need:-unset}); build it first (tools/pc-publish.py does)" >&2
      return 1 2>/dev/null || exit 1
    fi
  done
  install -Dm755 "$PC_MCP" "$pkgdir/usr/bin/bifrost-mcp"
  # the window server (docs/window-server.md) and its chrome clients. /etc/bifrost/compositor = bifrost-wm selects them;
  # bifrost-wm is the only window server, so PC_WM is mandatory.
  # The package must carry the chrome it launches, or the session would start a bare window server.
  for need in PC_WM PC_MENUBAR PC_DOCK PC_PANELS; do
    if [[ -z ${!need:-} || ! -x ${!need} ]]; then
      echo "package-files.sh: $need is not an executable (${!need:-unset}); build it first (tools/pc-publish.py does)" >&2
      return 1 2>/dev/null || exit 1
    fi
  done
  install -Dm755 "$PC_WM" "$pkgdir/usr/bin/bifrost-wm"
  install -Dm755 "$PC_MENUBAR" "$pkgdir/usr/bin/bifrost-menubar"
  install -Dm755 "$PC_DOCK" "$pkgdir/usr/bin/bifrost-dock"
  install -Dm755 "$PC_PANELS" "$pkgdir/usr/bin/bifrost-panels"
  if [[ -n ${PC_LOG:-} && -x ${PC_LOG} ]]; then
    install -Dm755 "$PC_LOG" "$pkgdir/usr/bin/bifrost-log"
  fi
  install -Dm644 "$here/bifrost-mcp.service" "$pkgdir/usr/lib/systemd/system/bifrost-mcp.service"
  # socket-activated services (docs/boot-architecture.md "Services on demand"): the supervisor, the target it runs
  # under, and the trigger that starts the target on the first desktop frame
  install -Dm755 "$PC_BIFROSTD" "$pkgdir/usr/bin/bifrostd"
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
  # ALSA compatibility plug-in for third-party apps (audio task 08): the default PCM goes to bifrost-audiod
  if [[ -n ${PC_ALSA_PLUGIN:-} && -f ${PC_ALSA_PLUGIN} ]]; then
    install -Dm755 "$PC_ALSA_PLUGIN" "$pkgdir/usr/lib/alsa-lib/libasound_module_pcm_bifrost.so"
    install -Dm644 "$here/alsa/50-bifrost.conf" "$pkgdir/etc/alsa/conf.d/50-bifrost.conf"
  fi
  install -Dm644 "$here/sysusers-bifrost.conf" "$pkgdir/usr/lib/sysusers.d/bifrost.conf"
  # audio server (docs/audio-architecture.md): own `audio` user, /run/bifrost/audio, CAP_SYS_NICE unit
  install -Dm755 "$PC_AUDIOD" "$pkgdir/usr/bin/bifrost-audiod"
  # latency benches (tools/audio-bench.py midget runs this over ssh)
  audiobench=${PC_AUDIO_BENCH:-$(dirname "$binary")/bifrost-audio-bench}
  if [[ -x $audiobench ]]; then
    install -Dm755 "$audiobench" "$pkgdir/usr/bin/bifrost-audio-bench"
  fi
  install -Dm644 "$here/bifrost-audiod.service" "$pkgdir/usr/lib/systemd/system/bifrost-audiod.service"
  install -Dm644 "$here/sysusers-bifrost-audio.conf" "$pkgdir/usr/lib/sysusers.d/bifrost-audio.conf"
  install -Dm644 "$here/tmpfiles-bifrost-audio.conf" "$pkgdir/usr/lib/tmpfiles.d/bifrost-audio.conf"
  # same-uid processes (Term, apps) must not be able to ptrace the shell, which holds the session token
  install -Dm644 "$here/sysctl-bifrost.conf" "$pkgdir/usr/lib/sysctl.d/60-bifrost.conf"
  install -Dm755 "$here/seed-data" "$pkgdir/usr/lib/springboard/seed-data"
  # boot speed: the initrd mounts the fstab subvolumes, and seatd starts without waiting for local-fs.target
  install -Dm644 "$here/initcpio/install/bifrost-mounts" "$pkgdir/usr/lib/initcpio/install/bifrost-mounts"
  install -Dm644 "$here/initcpio/hooks/bifrost-mounts" "$pkgdir/usr/lib/initcpio/hooks/bifrost-mounts"
  install -Dm644 "$here/initcpio/install/bifrost-firmware" "$pkgdir/usr/lib/initcpio/install/bifrost-firmware"
  # "unlock is login": opens an encrypted root from the loader's one-time key (HOOKS order: before filesystems)
  install -Dm644 "$here/initcpio/install/bifrost-unlock" "$pkgdir/usr/lib/initcpio/install/bifrost-unlock"
  install -Dm644 "$here/initcpio/hooks/bifrost-unlock" "$pkgdir/usr/lib/initcpio/hooks/bifrost-unlock"
  install -Dm644 "$here/seatd-early.conf" "$pkgdir/usr/lib/systemd/system/seatd.service.d/bifrost-early.conf"
  for file in session session-env session-policy; do
    install -Dm755 "$here/wayland/$file" "$pkgdir/usr/lib/bifrost/$file"
  done
  install -Dm755 "$here/wayland/video-arg" "$pkgdir/usr/lib/bifrost/video-arg"
  install -Dm755 "$here/wayland/display-refresh" "$pkgdir/usr/lib/bifrost/display-refresh"
  # PID 1 status and log lines stay off the console (docs/boot-architecture.md, "Text-free on every path").
  install -Dm644 "$here/systemd/50-bifrost-console.conf" "$pkgdir/usr/lib/systemd/system.conf.d/50-bifrost-console.conf"
  # Bootweave owns the EFI-frame-to-desktop lifecycle; no text console is exposed.
  for unit in getty@tty1.service; do
    install -d "$pkgdir/etc/systemd/system"
    ln -sfn /dev/null "$pkgdir/etc/systemd/system/$unit"
  done
  # open equivalents for compat family names (Arial, Times New Roman, Courier New, Menlo, Symbol, ...)
  install -Dm644 "$here/fontconfig/60-bifrost-aliases.conf" "$pkgdir/etc/fonts/conf.d/60-bifrost-aliases.conf"
  install -Dm755 "$here/wayland/apps" "$pkgdir/usr/bin/bifrost-apps"
  # pico is the older name of nano; the nano package ships only nano, so the name is linked here
  install -d "$pkgdir/usr/bin"
  ln -sfn nano "$pkgdir/usr/bin/pico"
  # Personal-use reference assets (gitignored bifrost-ref/assets, already converted to wav/png): skipped when absent.
  local refsrc="${PC_REFASSETS:-$here/../../bifrost-ref/assets}"
  if [ -d "$refsrc" ]; then
    (cd "$refsrc" && find INDEX.tsv sounds cursors wallpapers icons symbols fonts -type f 2>/dev/null) | while IFS= read -r f; do
      install -Dm644 "$refsrc/$f" "$pkgdir/usr/share/bifrost/ref/$f"
    done
  fi
}
