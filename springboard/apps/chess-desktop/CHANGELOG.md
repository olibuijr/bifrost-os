# Changelog

## 0.1.1 — 2026-10-14

- Add the generated app compliance harness.
- The current game and opponent setting are atomically saved and restored from the app data directory, allowing
  sudden termination only after a durable save. Board and move highlights use shared, appearance-aware colours.
## 0.1.0

- First version: a chess board with the full rules (castling, en passant, promotion, check, checkmate, stalemate,
  draw by threefold repetition, the fifty-move rule and insufficient material), a move list in standard algebraic
  notation, undo, and a computer opponent at four levels (Beginner, Easy, Medium, Hard) or two players at one board.
