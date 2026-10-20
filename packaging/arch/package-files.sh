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
  # lab machines only (the scriptlet copies it to /etc/bifrost/mcp.env next to the lab-install marker)
  install -Dm644 "$here/mcp-lab.env" "$pkgdir/usr/share/bifrost/mcp-lab.env"
  # socket-activated services (docs/boot-architecture.md "Services on demand"): the supervisor, the target it runs
  # under, and the trigger that starts the target on the first desktop frame
  install -Dm755 "$PC_BIFROSTD" "$pkgdir/usr/bin/bifrostd"
  install -Dm644 "$here/bifrostd.service" "$pkgdir/usr/lib/systemd/system/bifrostd.service"
  # The system services bifrostd supervises (docs/services.md "System services"): the audio (media player), network and
  # update services are part of the OS, not store apps. The package owns /usr/lib/bifrost/services/<id>/ (a path no earlier
  # install has a file at, so pacman finds no conflict) with the manifest from springboard/system/<dir> and the executable
  # the manifest names. bifrostd reads it read-only; a packaged id always wins, a store-installed copy never overrides it.
  # Sockets stay <root>/run/<id>.sock. PC_SVC_AUDIOD / PC_SVC_NETD / PC_SVC_UPDATED override the binary paths; they default
  # to the directory of the shell binary, where `cargo build -p audiod -p netd -p updated` puts them.
  local spec sdir sbin svar sbinpath sid sexec smanifest
  for spec in audio:audiod:PC_SVC_AUDIOD net:netd:PC_SVC_NETD update:updated:PC_SVC_UPDATED; do
    IFS=: read -r sdir sbin svar <<<"$spec"
    smanifest="$here/../../springboard/system/$sdir/manifest.toml"
    sbinpath=${!svar:-$(dirname "$binary")/$sbin}
    if [[ ! -x $sbinpath ]]; then
      echo "package-files.sh: system service binary $sbin is not an executable ($sbinpath); build it first (cargo build -p $sbin; tools/pc-publish.py does)" >&2
      return 1 2>/dev/null || exit 1
    fi
    sid=$(sed -n 's/^id[[:space:]]*=[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p' "$smanifest" | head -n1)
    sexec=$(sed -n 's/^exec[[:space:]]*=[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p' "$smanifest" | head -n1)
    if [[ -z $sid || $sexec != "$sbin" ]]; then
      echo "package-files.sh: $smanifest must name id and exec = \"$sbin\" (id='$sid', exec='$sexec')" >&2
      return 1 2>/dev/null || exit 1
    fi
    install -Dm644 "$smanifest" "$pkgdir/usr/lib/bifrost/services/$sid/manifest.toml"
    install -Dm755 "$sbinpath" "$pkgdir/usr/lib/bifrost/services/$sid/$sexec"
    if [[ -f $here/../../springboard/system/$sdir/icon.png ]]; then
      install -Dm644 "$here/../../springboard/system/$sdir/icon.png" "$pkgdir/usr/lib/bifrost/services/$sid/icon.png"
    fi
  done
  # zram swap at boot (memory pressure; bifrostd adds swap files under pressure once it runs)
  install -Dm644 "$here/bifrost-zram.service" "$pkgdir/usr/lib/systemd/system/bifrost-zram.service"
  # application firewall: the per-app inbound policies (bifrost-firewall, springboard/cmd) loaded as nftables rules
  install -Dm644 "$here/bifrost-firewall.service" "$pkgdir/usr/lib/systemd/system/bifrost-firewall.service"
  install -Dm644 "$here/bifrost-desktop-ready.service" "$pkgdir/usr/lib/systemd/system/bifrost-desktop-ready.service"
  install -Dm644 "$here/bifrost-after-desktop.target" "$pkgdir/usr/lib/systemd/system/bifrost-after-desktop.target"
  install -Dm644 "$here/dbus/org.freedesktop.portal.Desktop.service" "$pkgdir/usr/share/dbus-1/services/org.freedesktop.portal.Desktop.service"
  install -Dm644 "$here/springboard.service" "$pkgdir/usr/lib/systemd/system/springboard.service"
  # Printer Sharing uses the cups package own cups-lpd.socket and cups-lpd@.service (off until Sharing turns it on)
  # DHCPv6 for a shared segment: privd writes /run/bifrost/dhcp6/<interface>.conf and starts this unit (dnsmasq)
  install -Dm644 "$here/bifrost-dhcp6@.service" "$pkgdir/usr/lib/systemd/system/bifrost-dhcp6@.service"
  # login round: root helper, PAM service, privd socket group (the user-session migration is a later stage)
  # (PC_PRIVD overrides; otherwise bifrost-privd is looked up next to the shell binary)
  privd=${PC_PRIVD:-$(dirname "$binary")/bifrost-privd}
  if [[ -x $privd ]]; then
    install -Dm755 "$privd" "$pkgdir/usr/bin/bifrost-privd"
  fi
  install -Dm644 "$here/bifrost-privd.service" "$pkgdir/usr/lib/systemd/system/bifrost-privd.service"
  install -Dm644 "$here/pam-bifrost-login" "$pkgdir/etc/pam.d/bifrost-login"
  # Screen Sharing: wayvnc behind a login (PAM service "wayvnc", limited to the one account Settings names) and its
  # RSA-AES key; off until Settings > Sharing > Screen Sharing turns it on (privd writes the configuration first)
  install -Dm644 "$here/bifrost-wayvnc.service" "$pkgdir/usr/lib/systemd/system/bifrost-wayvnc.service"
  # /etc/pam.d/wayvnc belongs to the wayvnc package: ours ships here and the install script puts it in place
  install -Dm644 "$here/pam-bifrost-wayvnc" "$pkgdir/usr/share/bifrost/pam/wayvnc"
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
  # MIDI server (docs/audio/15-midi-server.md): apps connect to /run/bifrost/audio/is.olibuijr.midi.sock. The unit is
  # installed only with its binary, because post-install enables it.
  midid=${PC_MIDID:-$(dirname "$binary")/bifrost-midid}
  if [[ -x $midid ]]; then
    install -Dm755 "$midid" "$pkgdir/usr/bin/bifrost-midid"
    install -Dm644 "$here/bifrost-midid.service" "$pkgdir/usr/lib/systemd/system/bifrost-midid.service"
  fi
  install -Dm644 "$here/sysusers-bifrost-audio.conf" "$pkgdir/usr/lib/sysusers.d/bifrost-audio.conf"
  install -Dm644 "$here/tmpfiles-bifrost-audio.conf" "$pkgdir/usr/lib/tmpfiles.d/bifrost-audio.conf"
  # same-uid processes (Term, apps) must not be able to ptrace the shell, which holds the session token
  install -Dm644 "$here/sysctl-bifrost.conf" "$pkgdir/usr/lib/sysctl.d/60-bifrost.conf"
  install -Dm755 "$here/seed-data" "$pkgdir/usr/lib/springboard/seed-data"
  # System assets (UI fonts, app icons, wallpapers): the package owns /usr/share/springboard/system-assets, a path no
  # earlier install has a file at, so an update can change them and pacman finds no conflict. PC_SYSTEM_ASSETS is the
  # tree tools/stage-assets.py staged (only files packaging/assets-allowlist.txt names). The scriptlet removes the old
  # unowned copies in /usr/share/springboard/assets with the helper and the allowlist installed here.
  if [[ -n ${PC_SYSTEM_ASSETS:-} && -d ${PC_SYSTEM_ASSETS} ]]; then
    install -d -m755 "$pkgdir/usr/share/springboard/system-assets"
    cp -r --no-preserve=mode,ownership "$PC_SYSTEM_ASSETS"/. "$pkgdir/usr/share/springboard/system-assets"/
    find "$pkgdir/usr/share/springboard/system-assets" -type d -exec chmod 755 {} +
    find "$pkgdir/usr/share/springboard/system-assets" -type f -exec chmod 644 {} +
  fi
  # public key that signs this repository (packaging/arch/keys/README.md); the scriptlet imports it into pacman's keyring.
  # Absent until the key is generated: the package then ships none and the repository stays unsigned.
  if [[ -f $here/keys/bifrost-repo.gpg ]]; then
    install -Dm644 "$here/keys/bifrost-repo.gpg" "$pkgdir/usr/share/bifrost/keys/bifrost-repo.gpg"
  fi
  install -Dm755 "$here/prune-stale-assets" "$pkgdir/usr/lib/springboard/prune-stale-assets"
  install -Dm644 "$here/../assets-allowlist.txt" "$pkgdir/usr/lib/springboard/assets-allowlist.txt"
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
  install -Dm755 "$here/wayland/privileged-socket" "$pkgdir/usr/lib/bifrost/privileged-socket"
  # PID 1 status and log lines stay off the console (docs/boot-architecture.md, "Text-free on every path").
  install -Dm644 "$here/systemd/50-bifrost-console.conf" "$pkgdir/usr/lib/systemd/system.conf.d/50-bifrost-console.conf"
  # Bootweave owns the EFI-frame-to-desktop lifecycle; no text console is exposed.
  for unit in getty@tty1.service; do
    install -d "$pkgdir/etc/systemd/system"
    ln -sfn /dev/null "$pkgdir/etc/systemd/system/$unit"
  done
  # open equivalents for compat family names (Arial, Times New Roman, Courier New, Menlo, Symbol, ...)
  install -Dm644 "$here/fontconfig/60-bifrost-aliases.conf" "$pkgdir/etc/fonts/conf.d/60-bifrost-aliases.conf"
  # the Browser engine's last-resort family for CJK punctuation is answered by the installed Noto Sans CJK (a scan rule:
  # springboard.install rebuilds the font cache so it applies)
  install -Dm644 "$here/fontconfig/61-bifrost-cjk-fallback.conf" "$pkgdir/etc/fonts/conf.d/61-bifrost-cjk-fallback.conf"
  install -Dm755 "$here/wayland/apps" "$pkgdir/usr/bin/bifrost-apps"
  # bifrost-cmd (springboard/cmd/src/bin/*.rs): every tool of the crate, found by listing the sources, so a new tool
  # ships without editing this script. PC_CMD_DIR holds the release binaries of `cargo build -p bifrost-cmd --bins`.
  # Tools named below collide with a file an Arch package owns, so they go to /usr/lib/bifrost/cmd/bin, which
  # /etc/profile.d/bifrost-cmd.sh puts on PATH after /usr/bin (the Arch file keeps the name):
  local src tool dest colliding
  local cmd_colliding=(
    vis      # Arch extra/vis: the vis editor (/usr/bin/vis)
    units    # Arch units: the GNU unit converter (/usr/bin/units)
    lastcomm # Arch acct: process accounting (/usr/bin/lastcomm, with accton and sa)
  )
  # Checked and kept in /usr/bin: open, leave, and the other tool names. No package in runtime-packages.txt or
  # BASE_PACKAGES (springboard/src/installer/plan.rs) owns a file of that name.
  if [[ -z ${PC_CMD_DIR:-} || ! -d ${PC_CMD_DIR} ]]; then
    echo "package-files.sh: PC_CMD_DIR is unset or not a directory; build bifrost-cmd first (tools/pc-publish.py does)" >&2
    return 1 2>/dev/null || exit 1
  fi
  for src in "$here/../../springboard/cmd/src/bin"/*.rs; do
    if [[ ! -f $src ]]; then
      echo "package-files.sh: no bifrost-cmd tool sources under $here/../../springboard/cmd/src/bin" >&2
      return 1 2>/dev/null || exit 1
    fi
    tool=$(basename "$src" .rs)
    if [[ ! -x $PC_CMD_DIR/$tool ]]; then
      echo "package-files.sh: bifrost-cmd tool $tool is not built ($PC_CMD_DIR/$tool)" >&2
      return 1 2>/dev/null || exit 1
    fi
    dest=/usr/bin
    for colliding in "${cmd_colliding[@]}"; do
      if [[ $colliding == "$tool" ]]; then dest=/usr/lib/bifrost/cmd/bin; fi
    done
    install -Dm755 "$PC_CMD_DIR/$tool" "$pkgdir$dest/$tool"
  done
  install -Dm644 "$here/profile-bifrost-cmd.sh" "$pkgdir/etc/profile.d/bifrost-cmd.sh"
  # pico is the older name of nano; the nano package ships only nano, so the name is linked here
  install -d "$pkgdir/usr/bin"
  ln -sfn nano "$pkgdir/usr/bin/pico"
  # Nothing from the local reference folder ever enters a package: the system carries only BifrOSt's own and openly
  # licensed files (tools/check-package-licenses.py refuses a package with anything under /usr/share/bifrost/ref).
}
