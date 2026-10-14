# Changelog

## 0.5.1

- Exported the app icon at the required 180 x 180 pixel size.

## 0.5.0

- Changed: the aperture is always odd (1 to 31 pixels). An even square had no centre pixel, so its mean was offset from the pointer; an even size now rounds up to the odd size above it. The slider, the assistive steps and the View menu step between odd sizes (two pixels at a time).
- Changed: the matrix citations for the Color Meter rows point at the current file and line of each symbol, and the aperture note describes the centred square.

## 0.4.0

- Changed: the aperture model that was kept beside the window (an odd-size aperture type, its bounds and an update gate) is removed. The window already reads the aperture, the loupe outline and Update Continuously through its own tested state, so the unused copy only duplicated them.
- Changed: the matrix notes for the aperture, Update Continuously and the menu rows describe what the window does now. The aperture slider, the loupe outline and the View menu are wired; the checks on Midget remain.

## 0.3.0

- New: View and Color menus. View has Update Continuously, Display In (Native, sRGB, Display P3, Adobe RGB, Generic RGB), Display Values (hexadecimal, 8-bit and 16-bit decimal, percent), Magnification, Show Mouse Location, Lock Position (cmd+L), Lock X Coordinate (cmd+X), Lock Y Coordinate (cmd+Y) and aperture size steps. Color has Copy Color as Text (shift+cmd+C) and Copy Color as Image (alt+cmd+C). The menus and the toolbar share the same actions.
- New: aperture size. A slider under the swatch (1 to 31 pixels) sets the square around the pointer whose mean colour is read; the loupe outlines it. It can also be set through accessibility.
- New: Adobe RGB, and Native values (the delivered pixel unconverted).
- New: a 16-bit (0 to 65535) value format, listed in the readout and available to Copy.
- New: Lock X and Lock Y freeze one axis while the other follows the pointer; the Pointer row says which axis is locked.
- New: Update Continuously. Off, the loupe is sampled again only when the readout's pixel, the loupe size or the aperture changes.
- New: Show Mouse Location hides the Position row. The loupe has two more sizes (7 and 41 cells), and the readout names the format Copy uses.

## 0.2.0

- New: the readout reads real pixels. The shell's `screen` service answers the display size, the pointer and a square of pixels around it (Screen permission); the Color Meter asks on a 50 ms refresh loop, keeps one request of each kind in flight, and reads the loupe and the pixel under the pointer from the last square that came back. The lock, the zoom and copy as text or swatch now act on those pixels.
- Changed: "not available" shows only until the shell answers or when the permission is refused.

## 0.1.0

- New: Digital Color Meter. A magnified view around the pointer, the pixel value in sRGB, Display P3 and Generic RGB as hex, decimal and percent, a locked readout with arrow-key nudging, and copy as text or as a swatch. The pure conversions and the magnifier geometry are tested. Screen pixels are not yet delivered to apps by the shell, so the readout reports "not available" until that path exists.
