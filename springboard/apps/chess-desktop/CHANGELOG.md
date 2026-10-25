# Changelog

## 0.1.4

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.1.3

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.2

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.1 — 2026-10-14

- Add the generated app compliance harness.
- The current game and opponent setting are atomically saved and restored from the app data directory, allowing
  sudden termination only after a durable save. Board and move highlights use shared, appearance-aware colours.
## 0.1.0

- First version: a chess board with the full rules (castling, en passant, promotion, check, checkmate, stalemate,
  draw by threefold repetition, the fifty-move rule and insufficient material), a move list in standard algebraic
  notation, undo, and a computer opponent at four levels (Beginner, Easy, Medium, Hard) or two players at one board.
