#!/usr/bin/env python3
"""Packs the bifrost assets into raw RIMG files the launcher maps straight from
the APK or /data/local/springboard (no image decoding at runtime).

usage: tools/pack.py <bifrost-ref dir> <out assets dir> <screen w> <screen h>

Output layout:
  wallpaper.raw              default wallpaper (the lock background)
  wallpapers/<id>.raw        every photo in springboard/wallpapers at screen size (centre-cropped)
  wallpapers/<id>.thumb.raw  320 x 180 (16:9) thumbnail for Settings > Wallpaper
  wallpapers/index.ini       id=Display Name, one per line
  icons/<bundle id>.raw      app icons, premultiplied
  sounds/<name>.pcm          UI sounds as raw 48 kHz stereo signed 16-bit LE PCM
  SFUI.ttf, SFUIMono.ttf, Symbols.ttf   the default UI fonts: open-source lookalikes of the reference desktop fonts
  fonts/<file>.ttf           the font library Settings > Fonts offers as overrides

The wallpapers are the royalty-free photos in springboard/wallpapers (see CREDITS.md). The fonts are open
source (SIL OFL 1.1), fetched from one pinned google/fonts commit, checked by SHA-256 and cached in
~/.cache/bifrost/fonts. Icons and UI sounds still come from <ref>, which is not in the repo.
"""
import os
import shutil
import hashlib
import struct
import subprocess
import sys
import urllib.parse
import urllib.request

WALLPAPERS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "wallpapers")
DEFAULT_WALLPAPER = "waterfall_idyllic.jpg"
ICON_PT = 64
bifrost_WIDTH_PT = 768
THUMB = (320, 180)
SOUNDS = {"lock": "lock.caf", "tock": "Tock.caf", "key_press_click": "key_press_click.caf", "key_press_delete": "key_press_delete.caf", "key_press_modifier": "key_press_modifier.caf"}
GOOGLE_FONTS = "https://raw.githubusercontent.com/google/fonts/bd8f81ddb5c74d5c8897b36ad88b440266245103/ofl/"
FONT_CACHE = os.path.expanduser("~/.cache/bifrost/fonts")
# The defaults, under the asset names the code loads: the closest open lookalikes of the reference system fonts
DEFAULT_FONTS = {
    "SFUI.ttf": ("inter/Inter[opsz,wght].ttf", "29160a80ff49ddcab2c97711247e08b1fab27a484a329ce8b813d820dc559031"),
    "SFUIMono.ttf": ("jetbrainsmono/JetBrainsMono[wght].ttf", "48715a42ec242c21e9f02692891e147d022299a52e48d5e413e1a942193ffeda"),
    "Symbols.ttf": ("notosanssymbols2/NotoSansSymbols2-Regular.ttf", "7d5fb73b7ca67a6798101741f5d280a3d016a56a197afcd4199dbb57b4b82a21"),
}
# fonts/: other open-source fonts Settings > Fonts offers in place of the defaults (variable builds, every weight);
# file names avoid brackets so the lab sync (curl, URL globbing on) can fetch them
LIBRARY_FONTS = {
    "Geist-Variable.ttf": ("geist/Geist[wght].ttf", "73894e0448cae90a92b6c2f8732b7bb9acb7b94c418bff559dad4a18e1de9659"),
    "IBMPlexSans-Variable.ttf": ("ibmplexsans/IBMPlexSans[wdth,wght].ttf", "3b031aa4216174205bd8471f88a49b91f093169e9e87bd5262242bc5967fe2e3"),
    "SourceSans3-Variable.ttf": ("sourcesans3/SourceSans3[wght].ttf", "042fe2cc0b933e328410d7acbd0aa6a1873dca5aef81875f4bc214b08825c7b9"),
    "GeistMono-Variable.ttf": ("geistmono/GeistMono[wght].ttf", "d00e590b8eb3a59acc329b2d044fd143ae935090b7da33199ebee27cc7de8196"),
    "FiraCode-Variable.ttf": ("firacode/FiraCode[wght].ttf", "9335b082b3c7850d98a64b584f3417f65355f3471278bb5eeb8c6c0e8657aeeb"),
    "SourceCodePro-Variable.ttf": ("sourcecodepro/SourceCodePro[wght].ttf", "b400fc584e10aff25d0e775ce181b4fc1c5ea1b5dc37b81aeb2084375b945790"),
}


def rgba(path, w, h, fill=False):
    geo = f"{w}x{h}^" if fill else f"{w}x{h}!"
    args = ["magick", path, "-resize", geo]
    if fill:
        args += ["-gravity", "center", "-extent", f"{w}x{h}"]
    out = subprocess.run(args + ["-depth", "8", "rgba:-"], check=True, capture_output=True).stdout
    assert len(out) == w * h * 4, path
    return out


def premultiply(px):
    out = bytearray(px)
    for i in range(0, len(out), 4):
        a = out[i + 3]
        if a != 255:
            for c in range(3):
                out[i + c] = (out[i + c] * a + 127) // 255
    return bytes(out)


def write_raw(path, w, h, px):
    """Writes atomically (temp file + rename) so a running launcher never maps a partial file."""
    with open(path + ".tmp", "wb") as f:
        f.write(b"RIMG" + struct.pack("<II", w, h) + px)
    os.replace(path + ".tmp", path)




def fetch_font(rel, sha):
    """The font at `rel` under GOOGLE_FONTS, from the cache or downloaded, checked against `sha`."""
    path = os.path.join(FONT_CACHE, sha)
    if not os.path.exists(path):
        data = urllib.request.urlopen(GOOGLE_FONTS + urllib.parse.quote(rel)).read()
        if hashlib.sha256(data).hexdigest() != sha:
            sys.exit(f"pack.py: {rel}: checksum mismatch")
        os.makedirs(FONT_CACHE, exist_ok=True)
        with open(path + ".tmp", "wb") as f:
            f.write(data)
        os.replace(path + ".tmp", path)
    return path


def fonts(out):
    """The default UI fonts under their asset names, and the override library in fonts/."""
    for name, (rel, sha) in DEFAULT_FONTS.items():
        shutil.copy(fetch_font(rel, sha), f"{out}/{name}")
    os.makedirs(f"{out}/fonts", exist_ok=True)
    for name, (rel, sha) in LIBRARY_FONTS.items():
        shutil.copy(fetch_font(rel, sha), f"{out}/fonts/{name}")


def stills(out, w, h):
    """Every photo in springboard/wallpapers, plus thumbnails and an index (id = file stem)."""
    os.makedirs(f"{out}/wallpapers", exist_ok=True)
    index = {}
    for name in sorted(os.listdir(WALLPAPERS)):
        if not name.endswith((".jpg", ".png")):
            continue
        wp_id = os.path.splitext(name)[0]
        path = os.path.join(WALLPAPERS, name)
        write_raw(f"{out}/wallpapers/{wp_id}.raw", w, h, rgba(path, w, h, fill=True))
        write_raw(f"{out}/wallpapers/{wp_id}.thumb.raw", *THUMB, rgba(path, *THUMB, fill=True))
        index[wp_id] = wp_id.replace("_", " ").title()
    with open(f"{out}/wallpapers/index.ini", "w") as f:
        f.writelines(f"{k}={v}\n" for k, v in sorted(index.items()))


def sounds(ref, out):
    """UI sounds as raw PCM: 48 kHz, 2 channels, s16le (what the ALSA path plays)."""
    src = f"{ref}/sounds"
    if not os.path.isdir(src) or shutil.which("ffmpeg") is None:
        return
    os.makedirs(f"{out}/sounds", exist_ok=True)
    for name, caf in SOUNDS.items():
        if os.path.exists(f"{src}/{caf}"):
            subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", f"{src}/{caf}", "-f", "s16le", "-ac", "2", "-ar", "48000", f"{out}/sounds/{name}.pcm"], check=True)


def main(ref, out, w, h):
    icon = round(ICON_PT * w / bifrost_WIDTH_PT)
    os.makedirs(f"{out}/icons", exist_ok=True)
    write_raw(f"{out}/wallpaper.raw", w, h, rgba(os.path.join(WALLPAPERS, DEFAULT_WALLPAPER), w, h, fill=True))
    for name in os.listdir(f"{ref}/icons"):
        if name.endswith(".png"):
            px = premultiply(rgba(f"{ref}/icons/{name}", icon, icon))
            write_raw(f"{out}/icons/{name[:-4]}.raw", icon, icon, px)
    fonts(out)
    stills(out, w, h)
    sounds(ref, out)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]))
