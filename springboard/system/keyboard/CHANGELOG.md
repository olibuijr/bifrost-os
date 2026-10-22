# Changelog

## 1.1.5

- The keyboard shows again after the terminal row comes or goes (a field without it after the terminal, or back): it was rebuilt hidden and the old picture stayed on screen.
- A keyboard that has just started draws at rest: its first picture could sit mid-slide, low or off its area, until the next touch.

## 1.1.4

- The Ctrl key uses the shared terminal-keys SDK kit: Ctrl+Space sends NUL, and keys Ctrl does not combine with are dropped instead of sending a shifted code.

## 1.1.0 — 2026-10-09
- The keyboard is now a program (`bifrost-keyboard`), not code inside the shell: layout, prediction bar, autocorrect, emoji, split and floating keyboards, the space-bar trackpad. The shell composites its frame and forwards what it types.
- It takes the input languages, caps lock and "." shortcut from Settings through the shell, and works inside the app sandbox.
- Needs BifrOSt 0.4.77.

## 1.0.0 — 2026-10-05
- First release: color emoji (Noto Color Emoji) on the emoji keyboard and in all text.
- Icelandic and English word suggestions.
