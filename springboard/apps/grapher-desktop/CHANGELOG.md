# Changelog

## 0.1.9

- Rebuilt for 0.10.44: every touch says whether a mouse or a finger made it, so a mouse drag no longer scrolls lists on the desktop (the wheel, trackpad and scroll bar still do).

## 0.1.8

- The layout uses points instead of pixels on scaled displays.

## 0.1.7

- A new equation from + takes the typing; existing equations are left alone.

## 0.1.6

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.1.5

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.1.4

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.3

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.2

- Fix: the toolbar starts after the traffic lights instead of under them (the SDK toolbar now keeps clear of the window controls).

## 0.1.1 — 2026-10-14

- Add the store icon asset.

## 0.1.0 — 2026-10-13

- First version. Plots equations written as `y = f(x)`, a bare `f(x)`, parametric pairs `x = X(t), y = Y(t)` and polar
  curves `r = R(t)`, with an optional `; t in [a, b]` range.
- Several equations at once, each in its own colour, listed in the sidebar. Add, edit and remove them.
- Pan by dragging the plot; zoom with the toolbar buttons; reset the view.
- Grid, axes with tick values, and axis names.
- Equations are kept between launches in the app's data folder.
- Export the plot as a PNG into the app's `exports` folder.
