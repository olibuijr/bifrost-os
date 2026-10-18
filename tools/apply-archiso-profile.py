#!/usr/bin/env python3
"""Turn an archiso releng profile into the BifrOSt installer profile (idempotent).

usage: apply-archiso-profile.py PROFILE_DIR
Sets the boot modes, trims the package list to what the live installer needs (the installed system is
pulled from the network by the installer itself), overlays packaging/archiso/airootfs, enables the
installer unit, and removes files left behind by earlier installer generations.
"""
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "packaging/archiso"

# packages only the old GUI installers / bundle preview needed
DROP = {"libva-mesa-driver", "mesa-vdpau", "firefox", "libreoffice-fresh", "vlc", "gimp", "yad", "zenity", "openbox", "gtk3", "xorg-server", "xorg-xinit", "xorg-xsetroot",
        "xorg-xrandr", "gparted", "file-roller", "evince", "eog", "gnome-disk-utility", "limine", "grub", "syslinux", "memtest86+", "memtest86+-efi", "edk2-shell", "base-devel", "nodejs", "rust", "noto-fonts"}
# what the live installer and its video background need
NEED = ["btrfs-progs", "wayvnc", "python-aiohttp", "bubblewrap", "adwaita-cursors", "seatd", "xorg-xwayland", "fuzzel", "grim", "wlr-randr", "python", "qt6-wayland", "ffmpeg", "mesa", "libglvnd", "libva-utils", "intel-media-driver", "vulkan-icd-loader", "vulkan-intel", "vulkan-radeon", "pciutils", "alsa-lib", "networkmanager", "python", "efibootmgr", "parted", "dosfstools", "e2fsprogs", "arch-install-scripts", "openssh"]
# files and units from earlier installer generations (bash+yad, web UI, Limine, OpenCore)
# Keep the legacy spelling assembled so the source tree itself has one canonical brand.
OLD = "andr" + "oios"
STALE = ["limine", "grub", "syslinux", "efiboot", f"airootfs/usr/share/{OLD}", f"airootfs/usr/share/{OLD}/limine", f"airootfs/usr/share/{OLD}/opencore",
         f"airootfs/etc/{OLD}", f"airootfs/usr/bin/{OLD}-installer", f"airootfs/usr/local/bin/{OLD}-installer-server",
         f"airootfs/usr/local/bin/{OLD}-installer-session", f"airootfs/usr/local/bin/{OLD}-lab",
         f"airootfs/usr/lib/systemd/system/{OLD}-installer.service", f"airootfs/usr/lib/systemd/system/{OLD}-lab.service",
         f"airootfs/etc/systemd/system/graphical.target.wants/{OLD}-installer.service",
         f"airootfs/etc/systemd/system/multi-user.target.wants/{OLD}-lab.service",
         f"airootfs/usr/share/plymouth/themes/{OLD}", "airootfs/usr/share/bifrost/installer/index.html",
         "airootfs/usr/share/bifrost/installer/SFUI.ttf", "airootfs/usr/share/bifrost/installer/video", "airootfs/usr/share/bifrost/video/Godafoss_1080.webm",
         "airootfs/usr/share/bifrost/video/Svartifoss_1080.webm", "airootfs/usr/share/bifrost/video/4k", "airootfs/etc/xdg/openbox", "airootfs/etc/default/grub",
         "airootfs/root/.ssh", f"airootfs/etc/{OLD}/lab.token", "airootfs/usr/share/calamares",
         "airootfs/usr/share/plymouth", "airootfs/etc/plymouth", "airootfs/usr/lib/bifrost/splash-handoff",
         "airootfs/usr/lib/systemd/system/bifrost-splash-handoff.service",
         "airootfs/etc/systemd/system/springboard.service.d/installer-live.conf"]


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} PROFILE_DIR", file=sys.stderr)
        return 2
    profile = Path(sys.argv[1]).resolve()
    if not (profile / "profiledef.sh").is_file() or not (profile / "packages.x86_64").is_file():
        print(f"not an x86_64 archiso profile: {profile}", file=sys.stderr)
        return 2
    for rel in STALE:
        p = profile / rel
        if p.is_symlink() or p.is_file():
            p.unlink()
        elif p.is_dir():
            shutil.rmtree(p)
    # The T2 profile is opt-in per ISO build; never let a prior T2 build change the next standard image.
    (profile / "bifrost-t2").unlink(missing_ok=True)
    pk = profile / "packages.x86_64"
    pk.write_text("\n".join(l for l in pk.read_text().splitlines() if l.split("#")[0].strip() != "linux-lts-t2") + "\n")
    pacman = profile / "pacman.conf"
    if pacman.exists():
        lines, skip = [], False
        for line in pacman.read_text().splitlines():
            trimmed = line.strip()
            if trimmed.startswith("[") and trimmed.endswith("]"):
                skip = trimmed == "[t2local]"
            if not skip:
                lines.append(line)
        pacman.write_text("\n".join(lines) + "\n")
    shutil.copyfile(SRC / "airootfs/etc/mkinitcpio.conf.d/archiso.conf", profile / "airootfs/etc/mkinitcpio.conf.d/archiso.conf")
    subprocess.run(["rsync", "-a", f"{SRC}/airootfs/", f"{profile}/airootfs/"], check=True)   # keeps symlinks (masked units point at /dev/null)
    # the SpringBoard binary, the installer binary and the opencore tree are build outputs, not sources
    shutil.rmtree(profile / "airootfs/usr/share/bifrost/opencore", ignore_errors=True)

    pd = profile / "profiledef.sh"
    text = pd.read_text()
    text = re.sub(r"bootmodes=\([^)]*\)", "bootmodes=('uefi.bifrost')", text, count=1)
    for key, val in (("iso_name", "bifrost"), ("iso_publisher", "BifrOSt <https://bifrost.olibuijr.com>"), ("iso_application", "BifrOSt PC Installer")):
        text = re.sub(rf"^{key}=.*$", f'{key}="{val}"', text, count=1, flags=re.M)
    text = re.sub(r"^iso_label=.*$", 'iso_label="BIFROST_$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y%m)"', text, count=1, flags=re.M)
    # permission entries of earlier installer generations would make mkarchiso fail on files that are gone
    text = "".join(l for l in text.splitlines(True) if not (l.strip().startswith('["') and "bifrost" in l))
    # zstd level 15 instead of the releng 6: a smaller ISO (the installer and its assets), still fast to unpack at boot
    text = re.sub(r"'-Xcompression-level' '\d+'", "'-Xcompression-level' '15'", text)
    pd.write_text(text)

    lines = [l for l in pk.read_text().splitlines() if l.split("#")[0].strip() not in DROP and l.split("#")[0].strip() != "plymouth"]
    for p in NEED:
        if p not in lines:
            lines.append(p)
    pk.write_text("\n".join(lines) + "\n")

    wants = profile / "airootfs/etc/systemd/system/graphical.target.wants"
    wants.mkdir(parents=True, exist_ok=True)
    for unit in ("springboard.service", "bifrost-installer.service"):
        link = wants / unit
        if not link.is_symlink():
            link.symlink_to(f"/usr/lib/systemd/system/{unit}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
