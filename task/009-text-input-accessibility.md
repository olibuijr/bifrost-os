# 009 — Phase 7: finish shared text input and accessibility

Status: planned. Depends on: 004, 005. Parent: `001`.

## Problem (verified)

- `springboard/ui/src/lib.rs` (about 360 lines) provides semantic nodes and a UTF-8-safe single-line editor.
- No IME, preedit, text-input protocol, or AT-SPI code exists in `ui`, `render`, `sdk`, or `proto` (searched).
- Semantic nodes are limited to 128 per packet (`proto/src/lib.rs:339`).

## Steps

1. Grapheme-aware editing in `bifrost-ui`: cursor movement and deletion by grapheme cluster, shift selection, undo and redo. Keep byte-offset and logical-text conversions explicit and tested.
2. Cover combining marks, emoji sequences, RTL text, multiline needs, and right-to-left selection.
3. IME: preedit, commit, and cancel. Report surrounding text and caret rectangle. Composition interrupted by focus change or configuration change is cancelled cleanly.
4. Integrate the compositor-supported text-input protocol (Wayland `text-input-v3`) through a documented compatibility adapter. Verify the protocol against the current upstream definitions before implementation.
5. Extend semantic nodes into an accessibility tree: stable IDs, parent relationships, role and state, focus, value and action support, and incremental events.
6. Handle trees larger than 128 nodes with bounded pagination or deltas. Document the design in `springboard/docs/apps.md`.
7. PC: an AT-SPI bridge with only the access it needs. Preserve application sandboxing (see 007).
8. Do not confuse appearance settings (Larger Text, contrast, Reduce Motion) with screen-reader support. Those appearance settings must still work.

## Acceptance

- Migrate Files search and one editor to the new editor.
- Keyboard selection and undo work.
- Real IME composition reaches committed text (verified on PC with a real input method, recorded in the exit report).
- An accessibility client can discover controls, follow focus, and activate enabled actions.
- Larger Text, contrast, Reduce Motion, and appearance remain correct.
