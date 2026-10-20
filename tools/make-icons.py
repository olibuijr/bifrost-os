#!/usr/bin/env python3
"""Draws BifrOSt's own app icons (Python 3 + Pillow, nothing else).

usage: tools/make-icons.py [--size PX --out DIR] [--only NAME,...] [--sheet FILE]

With no arguments it writes every icon where the build expects it:
  springboard/res/app-icons/<bundle id>.png   the 13 shell built-in icons at 1024 px; springboard/tools/pack.py
                                              turns them into the icons/<bundle id>.raw the Dock and Launchpad load
  springboard/apps/<app>-desktop/icon.png     the desktop app icons that used to be copies of other makers' artwork,
                                              180 px RGBA like the other app icons
  springboard/res/settings-icons/<pane>.png   the 26 Settings pane tiles at 58 px RGBA (29 pt at 2x, 6.5 pt corners, as
                                              springboard/src/shell/icons.rs draws them); a second spec table, PANE_SPEC
--size and --out render every selected icon at one size into a directory instead (for previews); --only limits the
set by name; --sheet also writes a contact sheet (each icon at 180 px, and at 32 px on light and dark; the Settings
pane tiles at 64 px and at 20 px) for review.

Everything is drawn from the spec table at the bottom of this file: a full-bleed rounded-square tile with a vertical
gradient (the corner radius, 40 px at 180 px, and the flat top-lighter lighting match the other own icons) and a
flat white symbol made of circles, rounded rectangles, polygons, arcs and lines. Shapes are laid out on a 100 x 100
grid, so any size renders the same picture, and the output is deterministic: the same Pillow gives the same bytes.
No image is read, traced or sampled; the colours are the named values of springboard/render/src/ui.rs.
"""
import argparse
import math
import sys
from contextlib import contextmanager
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SHELL_DIR = ROOT / "springboard/res/app-icons"
APPS_DIR = ROOT / "springboard/apps"
SHELL_SIZE = 1024
DESKTOP_SIZE = 180
PANE_DIR = ROOT / "springboard/res/settings-icons"
PANE_SIZE = 58               # a Settings tile is 29 pt, stored at 2x
UNITS = 100.0                # the drawing grid: every coordinate below is 0..100
TILE_RADIUS = 40 / 180 * UNITS   # corner radius of the full-bleed tile, as in the other own icons
PANE_RADIUS = 6.5 / 29 * UNITS   # a Settings tile: 6.5 pt corners on 29 pt (TILE_RADIUS_PT / TILE_PT in shell/icons.rs)

# BifrOSt palette: RGB values copied from springboard/render/src/ui.rs (the constant name in each comment)
WHITE = (255, 255, 255)      # WHITE
BLACK = (0, 0, 0)            # BLACK
SYSTEM_RED = (255, 69, 58)   # SYSTEM_RED
SYSTEM_ORANGE = (255, 159, 10)   # SYSTEM_ORANGE
SYSTEM_YELLOW = (255, 204, 0)    # SYSTEM_YELLOW
SYSTEM_GREEN = (48, 209, 88)     # SYSTEM_GREEN
SYSTEM_MINT = (0, 199, 190)      # colors::SYSTEM_MINT, Light value
SYSTEM_TEAL = (100, 210, 255)    # SYSTEM_TEAL
SYSTEM_BLUE = (10, 132, 255)     # SYSTEM_BLUE (systemBlue, the tint)
SYSTEM_INDIGO = (94, 92, 230)    # colors::SYSTEM_INDIGO
SYSTEM_PURPLE = (191, 90, 242)   # SYSTEM_PURPLE
SYSTEM_PINK = (255, 55, 95)      # SYSTEM_PINK
SYSTEM_BROWN = (172, 142, 104)   # SYSTEM_BROWN
SYSTEM_GRAY = (142, 142, 147)    # SECONDARY (systemGray)
TRACK = (58, 58, 60)             # TRACK
CELL = (28, 28, 30)              # CELL


def mix(a, b, t):
    """The colour t of the way from a to b."""
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


class Grad:
    """A vertical gradient fill, c0 at grid row y0 to c1 at row y1."""

    def __init__(self, c0, c1, y0=0.0, y1=UNITS):
        self.c0, self.c1, self.y0, self.y1 = c0, c1, y0, y1

    def image(self, n):
        col = Image.new("RGB", (1, n))
        px = []
        for j in range(n):
            t = ((j + 0.5) / n * UNITS - self.y0) / (self.y1 - self.y0)
            px.append(mix(self.c0, self.c1, min(1.0, max(0.0, t))))
        col.putdata(px)
        return col.resize((n, n), Image.NEAREST)


class Pen:
    """Draws shapes (grid units) into an 8-bit mask. `erase()` switches to cutting shapes out of what is drawn."""

    def __init__(self, mask):
        self.d = ImageDraw.Draw(mask)
        self.s = mask.size[0] / UNITS
        self.ink = 255

    @contextmanager
    def erase(self):
        self.ink = 0
        try:
            yield self
        finally:
            self.ink = 255

    def _px(self, pts):
        return [(x * self.s, y * self.s) for x, y in pts]

    def rrect(self, x0, y0, x1, y1, r):
        s = self.s
        self.d.rounded_rectangle([x0 * s, y0 * s, x1 * s, y1 * s], radius=r * s, fill=self.ink)

    def circle(self, cx, cy, r):
        s = self.s
        self.d.ellipse([(cx - r) * s, (cy - r) * s, (cx + r) * s, (cy + r) * s], fill=self.ink)

    def ellipse(self, cx, cy, rx, ry, turn=0.0):
        """An ellipse turned by `turn` degrees (clockwise on screen)."""
        c, s = math.cos(math.radians(turn)), math.sin(math.radians(turn))
        pts = []
        for i in range(120):
            a = i * math.tau / 120
            x, y = rx * math.cos(a), ry * math.sin(a)
            pts.append((cx + x * c - y * s, cy + x * s + y * c))
        self.poly(pts)

    def poly(self, pts):
        self.d.polygon(self._px(pts), fill=self.ink)

    def line(self, pts, w):
        """A polyline `w` wide with round caps and joins."""
        r = w / 2
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            dx, dy = x1 - x0, y1 - y0
            n = math.hypot(dx, dy)
            if n:
                nx, ny = -dy / n * r, dx / n * r
                self.poly([(x0 + nx, y0 + ny), (x1 + nx, y1 + ny), (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)])
        for x, y in pts:
            self.circle(x, y, r)

    def rpoly(self, core, r):
        """`core` grown by `r` on every side: a polygon with rounded corners."""
        self.poly(core)
        self.line(list(core) + [core[0]], 2 * r)

    def arc(self, cx, cy, r, a0, a1, w, ry=None):
        """An arc stroke `w` wide, angles in degrees (0 = right, 90 = down); `ry` makes it elliptical."""
        ry = r if ry is None else ry
        n = max(8, int(abs(a1 - a0) / 3))
        pts = [(cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)),
                cy + ry * math.sin(math.radians(a0 + (a1 - a0) * i / n))) for i in range(n + 1)]
        self.line(pts, w)


def bezier(p0, p1, p2, p3, n=40):
    out = []
    for i in range(n + 1):
        t = i / n
        u = 1 - t
        out.append(tuple(u ** 3 * p0[k] + 3 * u * u * t * p1[k] + 3 * u * t * t * p2[k] + t ** 3 * p3[k] for k in range(2)))
    return out


def toward(cx, cy, deg, dist):
    """The point `dist` from (cx, cy) in direction `deg` clockwise from 12 o'clock."""
    a = math.radians(deg)
    return (cx + dist * math.sin(a), cy - dist * math.cos(a))


class Canvas:
    """One icon: the gradient tile, then flat shapes painted over it, then the rounded-square clip."""

    def __init__(self, size, top, bottom, radius=TILE_RADIUS):
        self.n = size
        self.radius = radius
        self.k = 4 if size <= 256 else 3       # supersampling for the shape masks (box-filtered down: exact coverage)
        self.img = Grad(top, bottom).image(size).convert("RGBA")

    def mask(self, fn):
        m = Image.new("L", (self.n * self.k,) * 2, 0)
        fn(Pen(m))
        return m.resize((self.n, self.n), Image.BOX)

    def paint(self, fn, fill=WHITE, alpha=1.0, clip=None, shadow=None):
        """Paints what `fn(pen)` draws in `fill` (a colour or Grad). `clip` is a drawing function limiting the shape;
        `shadow` is (dx, dy, blur, alpha) in grid units for a soft black shadow under it."""
        m = self.mask(fn)
        if clip:
            m = ImageChops.multiply(m, self.mask(clip))
        if shadow:
            dx, dy, blur, sa = shadow
            px = self.n / UNITS
            sm = ImageChops.offset(m, round(dx * px), round(dy * px)).filter(ImageFilter.GaussianBlur(blur * px))
            sm = sm.point(lambda v: round(v * sa))
            layer = Image.new("RGBA", (self.n, self.n), BLACK + (0,))
            layer.putalpha(sm)
            self.img.alpha_composite(layer)
        if alpha < 1:
            m = m.point(lambda v: round(v * alpha))
        src = fill.image(self.n) if isinstance(fill, Grad) else Image.new("RGB", (self.n, self.n), fill)
        layer = src.convert("RGBA")
        layer.putalpha(m)
        self.img.alpha_composite(layer)

    def finish(self):
        tile = self.mask(lambda p: p.rrect(0, 0, UNITS, UNITS, self.radius))
        self.img.putalpha(tile)
        return self.img


# ---- the symbols: each draws white (and translucent white) shapes on the 100 x 100 grid ----------------------------

def store(c):
    """A shopping bag with a plus cut into its front."""
    def bag(p):
        p.arc(50, 41, 12.5, 180, 360, 5)                                   # the handle
        p.rpoly([(33, 42), (67, 42), (70.5, 74), (29.5, 74)], 6)           # the body, a little wider at the foot
        with p.erase():
            p.line([(50, 51.5), (50, 67.5)], 5.2)
            p.line([(42, 59.5), (58, 59.5)], 5.2)
    c.paint(bag, shadow=(0, 1.8, 2.2, 0.16))


def browser(c):
    """A globe: a ring, a meridian, an axis and three latitudes bowed as if seen from above."""
    c.paint(lambda p: p.circle(50, 50, 30), alpha=0.2)

    def lines(p):
        p.arc(50, 50, 30, 0, 360, 3.6)
        p.arc(50, 50, 13.5, 0, 360, 3.2, ry=30)
        p.line([(50, 20), (50, 80)], 3.2)
        for dy in (-17, 0, 17):
            hw = math.sqrt(30 ** 2 - dy ** 2)
            p.arc(50, 50 + dy, hw, 0, 180, 3.2, ry=0.17 * hw)
    c.paint(lines)


def camera(c):
    """A camera body with a three-ring lens, a viewfinder hump and a flash."""
    body = mix(WHITE, SYSTEM_GRAY, 0.12)

    def shape(p):
        p.rrect(14, 32, 86, 78, 11)
        p.rrect(33, 23, 61, 40, 5)
    c.paint(shape, body, shadow=(0, 2, 2.5, 0.25))
    c.paint(lambda p: p.circle(50, 55, 19), CELL)
    c.paint(lambda p: p.circle(50, 55, 14.5), Grad(SYSTEM_BLUE, mix(SYSTEM_INDIGO, BLACK, 0.45), 40, 70))
    c.paint(lambda p: p.circle(50, 55, 7.5), mix(CELL, SYSTEM_INDIGO, 0.35))
    c.paint(lambda p: p.circle(44.5, 49.5, 3.8), alpha=0.85)
    c.paint(lambda p: p.circle(75, 41, 3), SYSTEM_ORANGE)


def clock(c):
    """A dial with tick marks, an hour hand, a minute hand and a red second hand, at ten past ten."""
    c.paint(lambda p: p.circle(50, 50, 34), shadow=(0, 1.8, 2.4, 0.2))

    def ticks(major):
        def f(p):
            for i in range(12):
                if (i % 3 == 0) == major:
                    a = i * 30
                    p.line([toward(50, 50, a, 30 - (7 if major else 3.5)), toward(50, 50, a, 30)], 3.8 if major else 2.2)
        return f
    c.paint(ticks(False), SYSTEM_GRAY)
    c.paint(ticks(True), CELL)

    def hands(p):
        p.line([(50, 50), toward(50, 50, 305, 16)], 5.2)
        p.line([(50, 50), toward(50, 50, 60, 22)], 4.0)
    c.paint(hands, CELL)
    c.paint(lambda p: p.line([toward(50, 50, 330, 8), toward(50, 50, 150, 22.5)], 1.9), SYSTEM_RED)
    c.paint(lambda p: p.circle(50, 50, 3.8), CELL)
    c.paint(lambda p: p.circle(50, 50, 1.9), SYSTEM_RED)


def files(c):
    """A folder: a darker back with a tab, a light front panel."""
    def back(p):
        p.rrect(18, 22, 47, 42, 5)
        p.rrect(18, 31, 82, 76, 7)
    c.paint(back, SYSTEM_ORANGE, shadow=(0, 2, 2.5, 0.18))
    c.paint(lambda p: p.rrect(18, 41, 82, 78, 7), Grad(mix(SYSTEM_YELLOW, WHITE, 0.4), SYSTEM_YELLOW, 41, 78),
            shadow=(0, -1.2, 1.4, 0.2))


def messages(c):
    """Two speech bubbles: a translucent one behind, a white one in front carrying three dots."""
    def front_shape(p):
        p.rrect(14, 22, 66, 56, 12)
        p.poly([(22, 51), (19, 71), (41, 53)])

    def back(p):
        p.rrect(42, 40, 87, 70, 11)
        p.poly([(76, 66), (80, 81), (62, 68)])
        with p.erase():                                  # a gap around the front bubble
            p.rrect(11.5, 19.5, 68.5, 58.5, 14.5)
            p.rpoly([(22, 51), (19, 71), (41, 53)], 2.6)
    c.paint(back, alpha=0.55)

    def front(p):
        front_shape(p)
        with p.erase():
            for x in (28, 40, 52):
                p.circle(x, 39, 3.3)
    c.paint(front)


def music(c):
    """A note (eighth note with a flag) on a faint record disc."""
    c.paint(lambda p: p.circle(50, 50, 35), alpha=0.16)

    def note(p):
        p.ellipse(39.5, 67.5, 12.5, 9.2, turn=-24)
        p.line([(51, 64), (51, 24)], 6.2)
        p.line(bezier((51, 24), (57, 32), (74, 36), (68, 54)), 6.2)
    c.paint(note, shadow=(0, 1.8, 2.2, 0.16))


def news(c):
    """A page with a folded corner, a heavy headline bar and lines of text."""
    def page(p):
        p.rpoly([(27.5, 20.5), (57, 20.5), (72.5, 36), (72.5, 79.5), (27.5, 79.5)], 3.5)
        with p.erase():
            p.rrect(31, 41, 69, 49.5, 2.5)
            p.rrect(31, 56, 69, 60.5, 2.2)
            p.rrect(31, 65, 69, 69.5, 2.2)
            p.rrect(31, 74, 54, 78.5, 2.2)
    c.paint(page, shadow=(0, 1.8, 2.2, 0.18))
    c.paint(lambda p: p.rpoly([(58, 18.5), (58, 34), (73.5, 34)], 1.0), mix(WHITE, SYSTEM_RED, 0.45))


def people(c):
    """A head-and-shoulders silhouette on a round plate."""
    plate = lambda p: p.circle(50, 50, 36)
    c.paint(plate, alpha=0.22)

    def figure(p):
        p.ellipse(50, 90, 30, 33)                  # the shoulders
        with p.erase():
            p.circle(50, 38, 18)                   # a gap around the head
        p.circle(50, 38, 13.5)
    c.paint(figure, clip=plate)


def photos(c):
    """A picture in a white frame: sky, sun, two ridges and a green hill."""
    c.paint(lambda p: p.rrect(16, 20, 84, 80, 10), shadow=(0, 2, 3, 0.2))
    inner = lambda p: p.rrect(21.5, 25.5, 78.5, 74.5, 6.5)
    c.paint(inner, Grad(SYSTEM_TEAL, mix(SYSTEM_TEAL, WHITE, 0.55), 25, 75))
    c.paint(lambda p: p.circle(63, 40, 6.5), SYSTEM_YELLOW, clip=inner)

    def ridges(p):
        p.poly([(28, 76), (47, 45), (66, 76)])
        p.poly([(52, 76), (67, 54), (86, 76)])
    c.paint(ridges, SYSTEM_BLUE, clip=inner)
    c.paint(lambda p: p.ellipse(33, 86, 25, 26), mix(SYSTEM_GREEN, BLACK, 0.12), clip=inner)
    c.paint(lambda p: p.ellipse(74, 91, 26, 21), SYSTEM_GREEN, clip=inner)


def podcasts(c):
    """A microphone with sound waves to both sides."""
    def mic(p):
        p.rrect(42.5, 22, 57.5, 54, 7.5)
        p.arc(50, 49, 14.5, 0, 180, 4)             # the cradle
        p.line([(50, 63.5), (50, 74)], 4)
        p.line([(40, 75), (60, 75)], 4)
        with p.erase():
            p.line([(44, 33), (56, 33)], 2.2)
            p.line([(44, 40), (56, 40)], 2.2)
            p.line([(44, 47), (56, 47)], 2.2)
    c.paint(mic, shadow=(0, 1.8, 2.2, 0.16))

    def waves(r, w):
        def f(p):
            p.arc(50, 39, r, -38, 38, w)
            p.arc(50, 39, r, 142, 218, w)
        return f
    c.paint(waves(24, 4))
    c.paint(waves(33, 4), alpha=0.6)


def settings(c):
    """A cog with eight teeth and a round hole."""
    def cog(p):
        p.circle(50, 50, 24.5)
        for i in range(8):
            a = i * 45
            u = (math.sin(math.radians(a)), -math.cos(math.radians(a)))
            v = (-u[1], u[0])
            core = [(50 + u[0] * ra + v[0] * w, 50 + u[1] * ra + v[1] * w) for ra, w in ((20, -4), (30, -4), (30, 4), (20, 4))]
            p.rpoly(core, 1.6)
        with p.erase():
            p.circle(50, 50, 11.5)
    c.paint(cog, shadow=(0, 1.8, 2.2, 0.2))


def voicememos(c):
    """A waveform of nine bars in cool colours."""
    heights = (12, 26, 44, 30, 58, 38, 22, 32, 14)
    pitch = 8.2
    x0 = 50 - pitch * (len(heights) - 1) / 2

    def bars(p):
        for i, h in enumerate(heights):
            x = x0 + i * pitch
            p.line([(x, 50 - h / 2 + 2.6), (x, 50 + h / 2 - 2.6)], 5.2)
    c.paint(bars, Grad(SYSTEM_TEAL, SYSTEM_MINT, 20, 80))


def games(c):
    """A game controller: body, two grips, a direction cross, two buttons, two shoulder bars."""
    def body(p):
        p.rrect(16, 36, 84, 64, 14)
        p.circle(30, 62, 12.5)
        p.circle(70, 62, 12.5)
        with p.erase():
            p.rrect(32, 40.5, 36.6, 55.5, 2.2)     # the direction cross, cut out
            p.rrect(26.7, 45.9, 41.9, 50.5, 2.2)
            p.circle(66.5, 52, 3.4)
            p.circle(74.5, 45.6, 3.4)
    c.paint(body, shadow=(0, 1.8, 2.2, 0.2))

    def bumpers(p):
        p.rrect(24, 27, 40, 32.5, 2.7)
        p.rrect(60, 27, 76, 32.5, 2.7)
    c.paint(bumpers, alpha=0.7)


# ---- the Settings pane symbols: 16 tiles of 58 px, so strokes stay at least 5.5 grid units thick ------------------

PANE_SHADOW = (0, 1.5, 2.0, 0.2)     # the soft shadow under every pane symbol


def turned(pts, deg, cx=50.0, cy=50.0):
    """`pts` turned `deg` degrees clockwise about (cx, cy)."""
    c, s = math.cos(math.radians(deg)), math.sin(math.radians(deg))
    return [(cx + (x - cx) * c - (y - cy) * s, cy + (x - cx) * s + (y - cy) * c) for x, y in pts]


def mirrored(pts, cx=50.0):
    return [(2 * cx - x, y) for x, y in pts]


def sparkle(cx, cy, big, small):
    """A four-pointed star with concave sides: tips `big` from the centre, waist `small`."""
    return [(cx + (big if i % 2 == 0 else small) * math.sin(i * math.pi / 4),
             cy - (big if i % 2 == 0 else small) * math.cos(i * math.pi / 4)) for i in range(8)]


def wifi(c):
    """Three nested arcs fanning up from a dot."""
    def fan(p):
        for r in (15, 26, 37):
            p.arc(50, 67, r, -139, -41, 6.6)
        p.circle(50, 67, 6.4)
    c.paint(fan, shadow=PANE_SHADOW)


def airplane(c):
    """A plane seen from above, swept wings and tailplane, flying up and to the right."""
    k, dx, dy = 0.92, 5.7, -5.7        # scaled up and shifted so its extent is centred on the tile

    def place(pts):
        return [(x + dx, y + dy) for x, y in turned([(50 + x * k, 50 + y * k) for x, y in pts], 45)]
    nose = bezier((5, -18), (5, -30), (2.4, -37), (0, -40), 12)          # a rounded nose, the right half
    fuselage = nose + mirrored(nose, 0)[::-1] + [(-5, 24), (-2.6, 35), (2.6, 35), (5, 24)]
    wing = [(4, -9), (38, 13), (38, 19.5), (4, 13)]
    tail = [(3, 23), (17, 32.5), (17, 38), (3, 35)]

    def shape(p):
        p.poly(place(fuselage))
        for part in (wing, tail):
            p.rpoly(place(part), 1.2)
            p.rpoly(place(mirrored(part, 0)), 1.2)
    c.paint(shape, shadow=PANE_SHADOW)


def bluetooth(c):
    """A rune of a stem with two angular lobes on its right."""
    def rune(p):
        p.line([(37, 22), (37, 78)], 7)
        p.line([(37, 22), (61, 36), (37, 50)], 7)
        p.line([(37, 50), (66, 64), (37, 78)], 7)
    c.paint(rune, shadow=PANE_SHADOW)


def notifications(c):
    """A bell: knob, dome, flared skirt, lip and clapper."""
    def bell(p):
        p.circle(50, 21, 4.3)
        p.circle(50, 41.5, 18)
        p.rpoly([(34, 41.5), (66, 41.5), (72, 61), (28, 61)], 2)
        p.rrect(22, 59.5, 78, 67.5, 4)
        p.circle(50, 77.5, 5.5)
    c.paint(bell, shadow=PANE_SHADOW)


def sounds(c):
    """A speaker cone and two wave marks."""
    def cone(p):
        p.rpoly([(24, 43.5), (32, 43.5), (45, 30.5), (45, 69.5), (32, 56.5), (24, 56.5)], 3)
    c.paint(cone, shadow=PANE_SHADOW)

    def waves(r, w):
        return lambda p: p.arc(50, 50, r, -42, 42, w)
    c.paint(waves(14, 6.3), shadow=PANE_SHADOW)
    c.paint(waves(26.5, 6.3), shadow=PANE_SHADOW)


def focus(c):
    """A fat crescent and two sparkles."""
    def moon(p):
        p.circle(44, 54, 27)
        with p.erase():
            p.circle(58, 43, 23)
    c.paint(moon, shadow=PANE_SHADOW)
    c.paint(lambda p: p.rpoly(sparkle(63, 41, 8.5, 2.6), 1.2), shadow=PANE_SHADOW)
    c.paint(lambda p: p.rpoly(sparkle(77, 59, 4.8, 1.4), 0.8), shadow=PANE_SHADOW)


def screentime(c):
    """An hourglass from two triangles, with sand."""
    top = [(31, 25), (69, 25), (50, 50)]
    foot = [(50, 50), (31, 75), (69, 75)]
    c.paint(lambda p: (p.rpoly(top, 1.2), p.rpoly(foot, 1.2)), alpha=0.4)
    c.paint(lambda p: p.rpoly(top, 1.2), clip=lambda p: p.rrect(0, 37, 100, 100, 0))
    c.paint(lambda p: p.rpoly(foot, 1.2), clip=lambda p: p.rrect(0, 62, 100, 100, 0))
    c.paint(lambda p: p.line([(50, 49), (50, 62)], 2.6))

    def plates(p):
        p.rrect(26, 18, 74, 26.5, 4)
        p.rrect(26, 73.5, 74, 82, 4)
    c.paint(plates, shadow=PANE_SHADOW)


def general(c):
    """A cog with six chunky teeth and a round hole."""
    def cog(p):
        p.circle(50, 50, 21.5)
        for i in range(6):
            u = (math.sin(math.radians(i * 60)), -math.cos(math.radians(i * 60)))
            v = (-u[1], u[0])
            core = [(50 + u[0] * ra + v[0] * w, 50 + u[1] * ra + v[1] * w) for ra, w in ((19, -7), (28, -4.6), (28, 4.6), (19, 7))]
            p.rpoly(core, 1.9)
        with p.erase():
            p.circle(50, 50, 9.5)
    c.paint(cog, shadow=PANE_SHADOW)


def controlcentre(c):
    """Two slider pills, one filled to the right and one to the left, each with a round knob."""
    rows = ((32, 62), (68, 38))
    c.paint(lambda p: [p.rrect(18, y - 7, 82, y + 7, 7) for y, _ in rows], alpha=0.3)
    c.paint(lambda p: [p.rrect(18, y - 7, kx, y + 7, 7) for y, kx in rows], alpha=0.62)
    c.paint(lambda p: [p.circle(kx, y, 11) for y, kx in rows], shadow=(0, 1.5, 2.4, 0.3))


def display(c):
    """A sun: a disc with eight rays, the four straight ones longer."""
    def sun(p):
        p.circle(50, 50, 12)
        for i in range(8):
            p.line([toward(50, 50, i * 45, 20), toward(50, 50, i * 45, 28.5 if i % 2 == 0 else 25.5)], 6)
    c.paint(sun, shadow=PANE_SHADOW)


def homescreen(c):
    """A two by two grid of rounded squares, the last one faint."""
    cells = ((20, 20), (54, 20), (20, 54), (54, 54))
    c.paint(lambda p: [p.rrect(x, y, x + 26, y + 26, 7.5) for x, y in cells[:3]], shadow=PANE_SHADOW)
    c.paint(lambda p: p.rrect(54, 54, 80, 80, 7.5), alpha=0.5)


def accessibility(c):
    """A white disc with a person cut out of it, arms spread."""
    def disc(p):
        p.circle(50, 50, 34)
        with p.erase():
            p.circle(50, 30, 6.2)
            p.line([(28, 41), (50, 45), (72, 41)], 6.4)
            p.line([(50, 45), (50, 57)], 7)
            p.line([(50, 56), (41, 71)], 6.4)
            p.line([(50, 56), (59, 71)], 6.4)
    c.paint(disc, shadow=PANE_SHADOW)


def battery(c):
    """A battery outline with a terminal, two cells full and one faint."""
    def body(p):
        p.rrect(16, 31, 72, 69, 9)
        with p.erase():
            p.rrect(21, 36, 67, 64, 5)
        p.rrect(74, 43.5, 80.5, 56.5, 3)
    c.paint(body, shadow=PANE_SHADOW)
    bars = ((24.5, 35.5), (38.5, 49.5), (52.5, 63.5))
    c.paint(lambda p: [p.rrect(a, 40, b, 60, 2.5) for a, b in bars[:2]])
    c.paint(lambda p: p.rrect(bars[2][0], 40, bars[2][1], 60, 2.5), alpha=0.4)


def privacy(c):
    """A shield with a check cut into it."""
    def shield(p):
        pts = bezier((50, 17), (58, 24), (68, 27), (79, 27), 20)
        pts += bezier((79, 27), (79, 36), (79, 44), (79, 50), 4)
        pts += bezier((79, 50), (79, 68), (64, 77), (50, 84), 24)
        pts += bezier((50, 84), (36, 77), (21, 68), (21, 50), 24)
        pts += bezier((21, 50), (21, 44), (21, 36), (21, 27), 4)
        pts += bezier((21, 27), (32, 27), (42, 24), (50, 17), 20)
        p.poly(pts)
        with p.erase():
            p.line([(36.5, 50), (46.5, 60), (64, 40)], 7)
    c.paint(shield, shadow=PANE_SHADOW)


def passcode(c):
    """A padlock whose body carries three dots."""
    def lock(p):
        p.arc(50, 41, 16, 180, 360, 7)
        p.line([(34, 41), (34, 50)], 7)
        p.line([(66, 41), (66, 50)], 7)
        p.rrect(24, 47, 76, 82, 9)
        with p.erase():
            for x in (36.5, 50, 63.5):
                p.circle(x, 65, 4.3)
    c.paint(lock, shadow=PANE_SHADOW)


def wallpaper(c):
    """A flower: six petals round a ring, with a yellow heart."""
    def petals(p):
        for i in range(6):
            cx, cy = toward(50, 50, i * 60, 15.5)
            p.ellipse(cx, cy, 8.8, 15.5, turn=i * 60)
        with p.erase():
            p.circle(50, 50, 10.5)
    c.paint(petals, shadow=PANE_SHADOW)
    c.paint(lambda p: p.circle(50, 50, 7), SYSTEM_YELLOW)


def printer(c):
    """A printer: a paper tray on top, a body with a status light, a sheet coming out below."""
    def body(p):
        p.rrect(30, 20, 70, 42, 3.5)
        p.rrect(16, 38, 84, 70, 9)
        with p.erase():
            p.rrect(27, 58, 73, 86, 3.5)
            p.circle(73, 48, 3)
    c.paint(body, shadow=PANE_SHADOW)
    c.paint(lambda p: p.rrect(32, 62, 68, 82, 2.5), shadow=PANE_SHADOW)
    c.paint(lambda p: [p.line([(38, y), (62, y)], 2.4) for y in (69, 75)], fill=SLATE_DARK, alpha=0.5)


def keyboard(c):
    """A keyboard seen from above: two rows of keys and a space bar cut out of the body."""
    def body(p):
        p.rrect(13, 31, 87, 69, 7)
        with p.erase():
            for row_y in (37.5, 47.5):
                for i in range(5):
                    x = 21 + 12 * i
                    p.rrect(x, row_y, x + 8, row_y + 6.5, 1.6)
            p.rrect(30, 58, 70, 63.5, 2)
    c.paint(body, shadow=PANE_SHADOW)


def assistant(c):
    """A ring with a four-pointed spark at its heart."""
    def orb(p):
        p.circle(50, 50, 30)
        with p.erase():
            p.circle(50, 50, 23.5)
    c.paint(orb, shadow=PANE_SHADOW)
    c.paint(lambda p: p.rpoly(sparkle(50, 50, 15, 4.2), 1.4), shadow=PANE_SHADOW)


def trackpad(c):
    """A trackpad outline with its click edge along the foot."""
    def pad(p):
        p.rrect(17, 24, 83, 76, 10)
        with p.erase():
            p.rrect(22.5, 29.5, 77.5, 70.5, 5.5)
        p.line([(22.5, 62), (77.5, 62)], 3.2)
    c.paint(pad, shadow=PANE_SHADOW)
    c.paint(lambda p: p.circle(50, 45, 5.5), alpha=0.55)


def mouse(c):
    """A mouse: a rounded body split into two buttons, with a scroll wheel."""
    def body(p):
        p.rrect(32, 17, 68, 83, 18)
        with p.erase():
            p.line([(50, 20), (50, 44)], 3.2)
            p.line([(34, 45), (66, 45)], 3.2)
    c.paint(body, shadow=PANE_SHADOW)
    c.paint(lambda p: p.rrect(47, 27, 53, 38, 3), alpha=0.5)


def search(c):
    """A magnifying glass: a ring and a short handle."""
    def glass(p):
        p.circle(44, 44, 23)
        with p.erase():
            p.circle(44, 44, 16)
        p.line([(61, 61), (78, 78)], 8.4)
    c.paint(glass, shadow=PANE_SHADOW)


def ethernet(c):
    """A wired network: one box on top branching to three below."""
    def tree(p):
        p.rrect(38, 18, 62, 38, 4)
        p.line([(50, 38), (50, 54)], 4.4)
        p.line([(27, 54), (73, 54)], 4.4)
        for x in (27, 50, 73):
            p.line([(x, 54), (x, 62)], 4.4)
        for x in (18, 41, 64):
            p.rrect(x, 62, x + 18, 82, 3.5)
    c.paint(tree, shadow=PANE_SHADOW)


def globe_shape(p, cx, cy, r, w):
    """A globe outline: the ring, a meridian and the equator."""
    p.circle(cx, cy, r)
    with p.erase():
        p.circle(cx, cy, r - w)
    p.ellipse(cx, cy, r * 0.46 + w / 2, r, 0)
    with p.erase():
        p.ellipse(cx, cy, r * 0.46 - w / 2, r - w, 0)
    p.line([(cx - r + w / 2, cy), (cx + r - w / 2, cy)], w * 0.9)
    p.line([(cx, cy - r + w / 2), (cx, cy + r - w / 2)], w * 0.9)


def network(c):
    """A globe."""
    c.paint(lambda p: globe_shape(p, 50, 50, 31, 5.4), shadow=PANE_SHADOW)


def vpn(c):
    """A globe with a padlock at its lower right."""
    def shape(p):
        globe_shape(p, 45, 45, 26, 4.8)
        with p.erase():
            p.rrect(52, 52, 90, 90, 12)
    c.paint(shape, shadow=PANE_SHADOW)

    def lock(p):
        p.arc(71, 66, 7.5, 180, 360, 4.2)
        p.line([(63.5, 66), (63.5, 70)], 4.2)
        p.line([(78.5, 66), (78.5, 70)], 4.2)
        p.rrect(58, 69, 84, 85, 4.5)
    c.paint(lock, shadow=PANE_SHADOW)


def account(c):
    """A person: a head over rounded shoulders."""
    c.paint(lambda p: p.circle(50, 36, 14.5), shadow=PANE_SHADOW)
    c.paint(lambda p: p.ellipse(50, 84, 29, 26), shadow=PANE_SHADOW, clip=lambda p: p.rrect(0, 0, 100, 80, 0))


# ---- the spec table: name, shell bundle id (or None), desktop app folder (or None), tile top, tile bottom, symbol ----

def lit(base, up=0.28, down=0.0):
    """A tile gradient: `base` lightened by `up` at the top, darkened by `down` at the foot."""
    return mix(base, WHITE, up), mix(base, BLACK, down)


SPEC = [
    ("appstore",   "bifrost.appstore",   None,                  mix(SYSTEM_BLUE, WHITE, 0.2), SYSTEM_INDIGO,    store),
    ("browser",    "bifrost.browser",    None,                  SYSTEM_TEAL,                  SYSTEM_BLUE,      browser),
    ("camera",     "bifrost.camera",     None,                  mix(TRACK, WHITE, 0.14),      CELL,             camera),
    ("clock",      "bifrost.clock",      "clock-desktop",       *lit(SYSTEM_ORANGE, 0.28, 0.02),                clock),
    ("files",      "bifrost.files",      None,                  *lit(SYSTEM_BLUE, 0.28, 0.14),                  files),
    ("messages",   "bifrost.messages",   None,                  *lit(SYSTEM_MINT, 0.3, 0.1),                    messages),
    ("music",      "bifrost.music",      "music-desktop",       mix(SYSTEM_TEAL, WHITE, 0.1),  mix(SYSTEM_INDIGO, SYSTEM_BLUE, 0.2), music),
    ("news",       "bifrost.news",       "news-desktop",        *lit(SYSTEM_RED, 0.2, 0.22),                    news),
    ("people",     "bifrost.people",     None,                  *lit(SYSTEM_BROWN, 0.2, 0.22),                  people),
    ("photos",     "bifrost.photos",     None,                  mix(SYSTEM_YELLOW, WHITE, 0.3), mix(SYSTEM_YELLOW, SYSTEM_ORANGE, 0.5), photos),
    ("podcasts",   "bifrost.podcasts",   "podcasts-desktop",    mix(SYSTEM_ORANGE, WHITE, 0.1), mix(SYSTEM_PINK, SYSTEM_RED, 0.4), podcasts),
    ("settings",   "bifrost.settings",   None,                  mix(mix(SYSTEM_GRAY, SYSTEM_BLUE, 0.22), WHITE, 0.14), mix(mix(SYSTEM_GRAY, SYSTEM_BLUE, 0.22), BLACK, 0.34), settings),
    ("voicememos", "bifrost.voicememos", "voicememos-desktop",  mix(CELL, SYSTEM_INDIGO, 0.38), mix(CELL, SYSTEM_INDIGO, 0.1), voicememos),
    ("games",      None,                 "games-desktop",       *lit(SYSTEM_GREEN, 0.25, 0.12),                 games),
]

# ---- the Settings pane table: name (the file springboard/res/settings-icons/<name>.png), tile top, tile bottom, symbol.
# The hues spread round the wheel so neighbouring rows of the Settings sidebar stay apart. ----------------------------

SLATE = mix(SYSTEM_GRAY, SYSTEM_BLUE, 0.3)
NIGHT = mix(SYSTEM_INDIGO, CELL, 0.55)
SLATE_DARK = mix(SLATE, BLACK, 0.55)
GRAPHITE = mix(TRACK, WHITE, 0.2)

PANE_SPEC = [
    ("accessibility", *lit(SYSTEM_BLUE, 0.22, 0.1),                                      accessibility),
    ("airplane",      mix(SYSTEM_TEAL, SYSTEM_BLUE, 0.25), mix(SYSTEM_TEAL, SYSTEM_BLUE, 0.85), airplane),
    ("battery",       *lit(SYSTEM_GREEN, 0.22, 0.14),                                    battery),
    ("bluetooth",     *lit(mix(SYSTEM_BLUE, SYSTEM_INDIGO, 0.5), 0.2, 0.14),             bluetooth),
    ("controlcentre", mix(TRACK, WHITE, 0.2), mix(TRACK, BLACK, 0.2),                    controlcentre),
    ("display",       mix(SYSTEM_YELLOW, SYSTEM_ORANGE, 0.5), mix(SYSTEM_ORANGE, SYSTEM_RED, 0.15), display),
    ("focus",         mix(NIGHT, WHITE, 0.14), mix(NIGHT, BLACK, 0.3),                   focus),
    ("general",       *lit(SLATE, 0.2, 0.22),                                            general),
    ("homescreen",    *lit(mix(SYSTEM_ORANGE, SYSTEM_RED, 0.3), 0.2, 0.1),               homescreen),
    ("notifications", *lit(SYSTEM_RED, 0.22, 0.1),                                       notifications),
    ("passcode",      *lit(SYSTEM_BROWN, 0.18, 0.28),                                    passcode),
    ("privacy",       *lit(mix(SYSTEM_BLUE, CELL, 0.4), 0.2, 0.22),                      privacy),
    ("screentime",    *lit(SYSTEM_PURPLE, 0.2, 0.18),                                    screentime),
    ("sounds",        *lit(SYSTEM_PINK, 0.2, 0.12),                                      sounds),
    ("wallpaper",     mix(SYSTEM_PURPLE, SYSTEM_PINK, 0.4), mix(SYSTEM_INDIGO, SYSTEM_PURPLE, 0.5), wallpaper),
    ("wifi",          *lit(SYSTEM_MINT, 0.12, 0.26),                                     wifi),
    ("account",       mix(SYSTEM_TEAL, SYSTEM_BLUE, 0.35), mix(SYSTEM_TEAL, SYSTEM_BLUE, 0.9), account),
    ("assistant",     *lit(mix(CELL, SYSTEM_PURPLE, 0.4), 0.2, 0.3),                     assistant),
    ("ethernet",      *lit(mix(SYSTEM_BLUE, SYSTEM_INDIGO, 0.3), 0.2, 0.14),             ethernet),
    ("keyboard",      mix(GRAPHITE, WHITE, 0.1), mix(GRAPHITE, BLACK, 0.3),              keyboard),
    ("mouse",         mix(GRAPHITE, WHITE, 0.1), mix(GRAPHITE, BLACK, 0.3),              mouse),
    ("network",       *lit(SYSTEM_BLUE, 0.22, 0.12),                                     network),
    ("printer",       *lit(SYSTEM_GRAY, 0.2, 0.18),                                      printer),
    ("search",        *lit(SYSTEM_GRAY, 0.24, 0.16),                                     search),
    ("trackpad",      mix(GRAPHITE, WHITE, 0.1), mix(GRAPHITE, BLACK, 0.3),              trackpad),
    ("vpn",           *lit(mix(SYSTEM_BLUE, SYSTEM_INDIGO, 0.5), 0.2, 0.16),             vpn),
]
NAMES = [row[0] for row in SPEC] + [row[0] for row in PANE_SPEC]


def render(row, size):
    """One icon at `size` px. A Settings pane row (name, top, bottom, symbol) gets the shell's 6.5 pt tile corners."""
    if len(row) == 4:
        (_name, top, bottom, draw), radius = row, PANE_RADIUS
    else:
        _name, _bundle, _desktop, top, bottom, draw = row
        radius = TILE_RADIUS
    c = Canvas(size, top, bottom, radius)
    draw(c)
    return c.finish()


def save(img, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path, optimize=True)
    print(f"wrote {path.relative_to(ROOT) if path.is_relative_to(ROOT) else path}  {img.size[0]}x{img.size[1]} {img.mode}")


def sheet(rows, path):
    """A contact sheet: each icon big, then small on a light and on a dark ground. App icons are 180 px and 32 px;
    Settings pane tiles are 64 px and 20 px, the sizes they are judged at."""
    pad = 16
    blocks = [(g, cell, small, cols) for g, cell, small, cols in
              (([r for r in rows if len(r) != 4], 180, 32, 7), ([r for r in rows if len(r) == 4], 64, 20, 8)) if g]
    width = max(cols * (cell + pad) + pad for _g, cell, _s, cols in blocks)
    heights = [((len(g) + cols - 1) // cols) * (cell + small + 3 * pad + pad) for g, cell, small, cols in blocks]
    out = Image.new("RGBA", (width, sum(heights) + pad), (0, 0, 0, 255))
    dark, light = (30, 30, 30, 255), (240, 240, 240, 255)
    y0 = pad
    for (g, cell, small, cols), height in zip(blocks, heights):
        h = cell + small + 3 * pad
        for i, row in enumerate(g):
            x, y = pad + (i % cols) * (cell + pad), y0 + (i // cols) * (h + pad)
            out.paste(dark, (x - pad // 2, y - pad // 2, x + cell + pad // 2, y + h - pad // 2))
            out.alpha_composite(render(row, cell), (x, y))
            tiny = render(row, small * 4).resize((small, small), Image.LANCZOS)
            out.paste(light, (x, y + cell + pad, x + cell // 2, y + cell + pad + small + 8))
            out.alpha_composite(tiny, (x + 4, y + cell + pad + 4))
            out.alpha_composite(tiny, (x + cell // 2 + 8, y + cell + pad + 4))
        y0 += height
    path.parent.mkdir(parents=True, exist_ok=True)
    out.convert("RGB").save(path)
    print(f"wrote {path}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--size", type=int, help="render every selected icon at this size into --out")
    ap.add_argument("--out", type=Path, help="directory for --size output")
    ap.add_argument("--only", help="comma separated names (" + ", ".join(NAMES) + ")")
    ap.add_argument("--sheet", type=Path, help="also write a contact sheet to this file")
    a = ap.parse_args()
    only = set(a.only.split(",")) if a.only else set(NAMES)
    if only - set(NAMES):
        sys.exit(f"make-icons.py: unknown name(s): {', '.join(sorted(only - set(NAMES)))}")
    if bool(a.size) != bool(a.out):
        sys.exit("make-icons.py: --size and --out go together")
    rows = [r for r in SPEC + PANE_SPEC if r[0] in only]
    if a.size:
        for r in rows:
            save(render(r, a.size), a.out / f"{r[0]}.png")
    else:
        for r in rows:
            if len(r) == 4:
                save(render(r, PANE_SIZE), PANE_DIR / f"{r[0]}.png")
                continue
            if r[1]:
                save(render(r, SHELL_SIZE), SHELL_DIR / f"{r[1]}.png")
            if r[2]:
                save(render(r, DESKTOP_SIZE), APPS_DIR / r[2] / "icon.png")
    if a.sheet:
        sheet(rows, a.sheet)


if __name__ == "__main__":
    main()
