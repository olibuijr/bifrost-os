#!/usr/bin/env python3
"""Render the BifrOSt mark (packaging/brand/bifrost-mark.svg) into the assets that carry it:
the installer logo and the splash loader's embedded pixels.
usage: bifrost_brand.py   (run from anywhere; writes into packaging/ and springboard/bootweave/efi)
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / "springboard/assets/SFUI.ttf"
S = 4  # supersample


def squircle_mask(n, r=0.225):
    m = Image.new("L", (n * S, n * S), 0)
    ImageDraw.Draw(m).rounded_rectangle((0, 0, n * S - 1, n * S - 1), radius=int(n * S * r), fill=255)
    return m.resize((n, n), Image.LANCZOS)


def gradient(n, top, bot):
    g = Image.new("RGB", (1, n))
    for y in range(n):
        t = y / (n - 1)
        g.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(top, bot)))
    return g.resize((n, n))


def mark(n, letter="B"):
    """Blue gradient squircle with a white letter, a soft shadow and a top highlight, padded like a Golden Gate icon."""
    canvas = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    pad = int(n * 0.1)
    k = n - 2 * pad
    body = Image.new("RGBA", (k, k), (0, 0, 0, 0))
    body.paste(gradient(k, (64, 156, 255), (10, 92, 220)), (0, 0), squircle_mask(k))
    hi = Image.new("RGBA", (k, k), (0, 0, 0, 0))
    ImageDraw.Draw(hi).ellipse((-k * 0.2, -k * 0.9, k * 1.2, k * 0.45), fill=(255, 255, 255, 46))
    body.alpha_composite(Image.composite(hi, Image.new("RGBA", (k, k), (0, 0, 0, 0)), squircle_mask(k)))
    font = ImageFont.truetype(str(FONT), int(k * 0.62))
    try:
        font.set_variation_by_name("Bold")
    except Exception:
        pass
    ImageDraw.Draw(body).text((k / 2, k / 2 + k * 0.01), letter, font=font, fill=(255, 255, 255, 255), anchor="mm")
    shadow = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    shadow.paste((0, 0, 0, 120), (pad, pad + int(n * 0.03)), squircle_mask(k))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(n * 0.03)))
    canvas.alpha_composite(body, (pad, pad))
    return canvas


def svg_mark(n):
    """The Icelandic-viking mark (packaging/brand/bifrost-mark.svg) rendered to n x n RGBA with rsvg-convert."""
    import io, subprocess
    png = subprocess.check_output(["rsvg-convert", "-w", str(n), "-h", str(n), str(ROOT / "packaging/brand/bifrost-mark.svg")])
    return Image.open(io.BytesIO(png)).convert("RGBA")


def main():
    pk = ROOT / "packaging/archiso/airootfs/usr/share"
    svg_mark(512).save(pk / "bifrost/installer/logo.png")
    on_black = Image.new("RGBA", (160, 160), (0, 0, 0, 255))
    on_black.alpha_composite(svg_mark(160))
    raw = bytearray()
    for r, g, b in on_black.convert("RGB").getdata():
        raw += bytes((b, g, r, 0))
    (ROOT / "springboard/bootweave/efi/logo.bgrx").write_bytes(raw)
    print("wrote installer logo and bootweave-efi/logo.bgrx")


if __name__ == "__main__":
    main()
