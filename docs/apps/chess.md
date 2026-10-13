# Chess

Chess (`springboard/apps/chess-desktop`, crate `bifrost-chess`, id `is.olibuijr.chess.desktop`, version 0.1.0) is a
full-size game window with the board on the left and a panel on the right: the status, the opponent choice, New
Game and Undo, and the move list in standard algebraic notation.

## Playing

- Tap a piece to select it; its legal destinations show as dots, and a tap on one moves the piece. Tapping another
  piece of the side to move selects that piece instead.
- A pawn that reaches the last rank asks for its new piece: Queen, Rook, Bishop or Knight, in the panel.
- The last move's two squares are tinted; the king of the side in check is tinted red.
- The opponent is one of four levels (Beginner, Easy, Medium, Hard), or Two Players at one board. The person plays
  White, and the computer answers each move at once. The default is Medium.
- Undo takes back the computer's answer and the person's move together, so it is the person's turn again. In Two
  Players it takes back one move. The Game menu offers New Game (cmd+n), Undo Move (cmd+z), Two Players and the
  levels.

## Rules

The rules are complete for a game that ends by the board: castling (both sides, not through or out of check, the rook
moves with the king), en passant, promotion to any of four pieces, check, checkmate and stalemate.

The game also ends in a draw by:

- threefold repetition: the same placement, side to move, castling rights and en passant possibility occur three
  times. This is applied automatically, without a claim;
- the fifty-move rule: 100 half-moves without a capture or a pawn move. Also applied automatically;
- insufficient material: king against king, or king and one knight or bishop against a king. Other mating material
  always counts as possible.

A mate on the move that makes the fifty-move count is still a mate.

## The computer

The computer is a negamax search with alpha-beta pruning, with move ordering (captures by victim and attacker, then
promotions) and a capture-only quiescence search at the horizon, so it does not stop in the middle of a trade.

The evaluation is material plus small positional terms: advanced and central pawns, central knights and bishops, a rook
on the seventh rank, and a king that stays on the back rank near the edge. Mate scores favour the faster mate.

| Level | Plies searched | Noise added to each root move |
|---|---|---|
| Beginner | 1 | up to 150 centipawns |
| Easy | 2 | up to 30 centipawns |
| Medium | 3 | none |
| Hard | 4 | none |

The noise makes the weak levels choose among near-equal moves and miss some tactics. The search does not know about
repetition or the fifty-move rule while it looks ahead, so it does not steer toward or away from a draw by those
rules.

Search runs on the UI thread when the computer moves. Hard can take a noticeable moment on a slow machine.

## Model and tests

The code is split so the rules and the search can be tested without a window:

- `board.rs`: squares, pieces, FEN, the attack test, the repetition key and the insufficient-material test;
- `moves.rs`: move generation, legality, and making a move;
- `game.rs`: the game history, undo, the status and draws, and standard notation;
- `ai.rs`: the evaluation, the search and the levels;
- `main.rs`: the window, the drawing and the taps.

Unit tests cover each module (`cargo test -p bifrost-chess`): start-position move count, pins, castling in every
blocked case, en passant, promotion, the fifty-move clock, fool's mate, stalemate, threefold repetition, notation with
disambiguation, check and promotion suffixes, and the search finding a mate in one and a hanging queen.

## Known limits

- The game is not saved. Quitting the app ends the game.
- Pieces are drawn as letters (K, Q, R, B, N, P) rather than chess symbols, because the system font does not carry the
  chess symbols.
- No takeback of a whole game, no clocks, no draw offers or resignation, and no board flip.
- Keyboard control is not implemented; the board is operated by pointer or touch.
- Only the automatic draw rules are applied. Claimed draws (a threefold claim before the third occurrence) are not
  offered.
