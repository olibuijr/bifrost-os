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
  SFUI.ttf, SFUIMono.ttf, SFUISymbols-Regular.otf, Symbols.ttf

The wallpapers are the royalty-free photos in springboard/wallpapers (see CREDITS.md); the fonts and
UI sounds still come from <ref>, which holds the licensed reference assets and is not in the repo.
"""
import os
import shutil
import struct
import subprocess
import sys

WALLPAPERS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "wallpapers")
DEFAULT_WALLPAPER = "waterfall_idyllic.jpg"
ICON_PT = 64
bifrost_WIDTH_PT = 768
THUMB = (320, 180)
SOUNDS = {"lock": "lock.caf", "tock": "Tock.caf", "key_press_click": "key_press_click.caf", "key_press_delete": "key_press_delete.caf", "key_press_modifier": "key_press_modifier.caf"}
FONTS = ("SFUI.ttf", "SFUIMono.ttf", "SFUISymbols-Regular.otf", "Symbols.ttf")


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
    for font in FONTS:
        if os.path.exists(f"{ref}/fonts/{font}"):
            shutil.copy(f"{ref}/fonts/{font}", f"{out}/{font}")
    stills(out, w, h)
    sounds(ref, out)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]))
