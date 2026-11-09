# Changelog

## 1.1.6

- Rebuilt with the SDK fixes from the review wave: a document saved in place on quit reaches the shell, a semaphore signal releases one waiter, and queue waits no longer hold the pool.

## 1.1.4

- The Ctrl key uses the shared terminal-keys SDK kit: Ctrl+Space sends NUL, and keys Ctrl does not combine with are dropped (consuming the Ctrl tap) instead of sending a shifted code.

## 1.1.3

- The floating keyboard's grabber follows the appearance, and the keyboard enables sudden termination because its state is supplied by the shell.

## 1.1.2 — 2026-10-14

- Keyboard shades, accents and shadows use shared named colours; labels use the desktop text scale.
