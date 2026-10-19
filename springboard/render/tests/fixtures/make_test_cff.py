#!/usr/bin/env python3
"""Builds `test-cff.otf`, the CFF-flavoured OpenType fixture of the renderer tests.

The font is BifrOSt's own work: three plain shapes (a box, a disc made of cubic curves and a ring) drawn here, with
no outline taken from any other font. The renderer tests use it to cover the CFF path of `Font` (`font.rs`) and `Cff`
(`cff.rs`), because every packed font is TrueType and no openly licensed CFF face ships with BifrOSt.

usage: python3 make_test_cff.py   (needs fontTools; writes test-cff.otf beside this script)
"""
import os

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.t2CharStringPen import T2CharStringPen

UPEM = 1000
ADVANCE = 800
# a circle of radius r from four cubic arcs
K = 0.5522847498


def box(pen):
    pen.moveTo((100, 0))
    pen.lineTo((700, 0))
    pen.lineTo((700, 700))
    pen.lineTo((100, 700))
    pen.closePath()


def circle(pen, cx, cy, r, ccw):
    k = r * K
    segs = [
        ((cx + r, cy), (cx + r, cy + k), (cx + k, cy + r), (cx, cy + r)),
        ((cx, cy + r), (cx - k, cy + r), (cx - r, cy + k), (cx - r, cy)),
        ((cx - r, cy), (cx - r, cy - k), (cx - k, cy - r), (cx, cy - r)),
        ((cx, cy - r), (cx + k, cy - r), (cx + r, cy - k), (cx + r, cy)),
    ]
    if not ccw:
        segs = [(end, c2, c1, start) for start, c1, c2, end in reversed(segs)]
    pen.moveTo(segs[0][0])
    for _, c1, c2, end in segs:
        pen.curveTo(c1, c2, end)
    pen.closePath()


def disc(pen):
    circle(pen, 400, 350, 350, True)


def ring(pen):
    circle(pen, 400, 350, 350, True)
    circle(pen, 400, 350, 200, False)


def empty(pen):
    pass


GLYPHS = [(".notdef", empty), ("box", box), ("disc", disc), ("ring", ring)]
CMAP = {0x25A0: "box", 0x25CF: "disc", 0x25EF: "ring"}


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    order = [name for name, _ in GLYPHS]
    fb = FontBuilder(UPEM, isTTF=False)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(CMAP)
    charstrings = {}
    for name, draw in GLYPHS:
        pen = T2CharStringPen(ADVANCE, None)
        draw(pen)
        charstrings[name] = pen.getCharString()
    fb.setupCFF("BifrostTestCff-Regular", {"FullName": "Bifrost Test Cff"}, charstrings, {})
    fb.setupHorizontalMetrics({name: (ADVANCE, 0) for name in order})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "Bifrost Test Cff", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    fb.setupPost()
    fb.font.save(os.path.join(here, "test-cff.otf"))


if __name__ == "__main__":
    main()
