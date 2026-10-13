# Changelog

## 0.2.0

- New: the readout reads real pixels. The shell's `screen` service answers the display size, the pointer and a square of pixels around it (Screen permission); the Color Meter asks on a 50 ms refresh loop, keeps one request of each kind in flight, and reads the loupe and the pixel under the pointer from the last square that came back. The lock, the zoom and copy as text or swatch now act on those pixels.
- Changed: "not available" shows only until the shell answers or when the permission is refused.

## 0.1.0

- New: Digital Color Meter. A magnified view around the pointer, the pixel value in sRGB, Display P3 and Generic RGB as hex, decimal and percent, a locked readout with arrow-key nudging, and copy as text or as a swatch. The pure conversions and the magnifier geometry are tested. Screen pixels are not yet delivered to apps by the shell, so the readout reports "not available" until that path exists.
