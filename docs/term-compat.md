# Term compatibility

Status by reading code paths (emulator: the `iosterm` crate, imported as `termcore` from `../iOSTerm`, read only; BifrOSt wrapper: `springboard/src/term/mod.rs`; Terminal UI: `springboard/src/shell/terminal/`). Nothing here was run; "Midget" marks items that need a check on the device. Re-read 2026-10-13: the escape-sequence, selection and program rows below still match the code (`report_mouse` still has no caller from the desktop pointer path).

## Terminal features around the emulator

Not compatibility items, but they exist and are easy to miss: tabs and split panes (`split.rs`), saved window groups (`groups.rs`), colour profiles (`profiles.rs`, `profile_table.rs`), the Find bar (`find.rs`), SSH connections from a config file with import (`ssh.rs`), prompt marks (OSC 133 `A`, used by Cmd+Up and Cmd+Down to jump between prompts), opening the first web link in a selection from the More menu (`links.rs`; this is not OSC 8), and the `term` tool in the MCP server that reads and drives tabs ([bifrost-mcp.md](bifrost-mcp.md), "Term tool").

## Control keys

Every Ctrl chord (hardware keyboard, the on-screen Ctrl key, `term send_keys`) sends its control byte to the pty, and Alt sends ESC plus the key, so readline editing and job control all work: Ctrl+A/E line start/end, Ctrl+B/F/N/P move and history, Ctrl+D/H delete, Ctrl+K/U/W kill, Ctrl+Y yank, Ctrl+L clear, Ctrl+R/S search, Ctrl+T transpose, Ctrl+C/Z interrupt/suspend, Alt+B/F/D and Alt+Backspace word edits, Alt+Left/Right word jumps, and Home/End/Delete/PageUp/PageDown. One mapping serves all three: the `terminal_keys` SDK kit (`bifrost_ui::terminal_keys`, `CONTROL_BINDINGS` documents every byte). Kept for the shell: Ctrl+Space (input source), any Tab chord (app switcher), Ctrl+Alt chords (screen reader), and Cmd chords (copy/paste, tabs, find); an open panel keeps pty input out. Midget: confirm Ctrl+C interrupts and Ctrl+R searches in a shell.

## Escape sequences

| Feature | Status | Where | Notes |
|---|---|---|---|
| Alternate screen 47/1047/1049 | OK | `Term::set_alt` | 1049 saves the cursor and clears; scrollback off on alt. Midget: vim/less exit restores the screen. |
| Bracketed paste 2004 | OK | `paste_bytes`, test in `term/mod.rs` | ESC stripped so the paste cannot end early. |
| Mouse 1000/1002/1003, SGR 1006 | Wheel and taps wired | `Terminal::scroll`, `Terminal::report_mouse`, `TermTouch::tap` | Emulator tracks modes. Wheel goes to the program as reports (was arrow keys), and a tap is a press and release at its cell. More > Allow mouse reporting turns both off for every tab. Drags from touch and the desktop pointer path do not call `report_mouse` yet: follow-up. Midget: htop, vim `mouse=a`, tmux `mouse on`. |
| OSC 0/1/2 title | OK | `Term::osc`, `Terminal::title` | Capped; tab caption falls back to the job name. |
| OSC 7 cwd | Not parsed, not needed locally | `pty.cwd()` reads `/proc/PID/cwd` | New tab in the same dir works for local shells; ssh sessions would need OSC 7 (gap). |
| OSC 8 hyperlinks | Ignored | `Term::osc` drops unknown codes | Text shows, link not clickable. Needs a cell link table in iosterm: gap. |
| OSC 52 clipboard | Write only, no ask | `take_clipboard`, `Shell::pump` | Reads (`?`) are ignored, good. Writes go straight to the system clipboard with no prompt: gap, needs a consent sheet like Terminal.app. |
| OSC 10/11 colour query | OK | `Term::osc` | |
| OSC 133 prompt marks | `A` only | `term/marks.rs`, `Terminal::advance_marked` | The core ignores OSC 133; the Terminal scans the output for `ESC ] 133 ; A` (also across two reads) and marks the caret line. Cmd+Up and Cmd+Down jump between marks, Cmd+U marks a line. Marks are kept until the scrollback is full, because the core does not say how many lines it dropped. `B`, `C` and `D` are not used. |
| 256 colour, truecolor | OK | `Term::sgr`, tests near term.rs line 1300 | Malformed params ignored. |
| Wide and combining | Partial | `iosterm::width::char_width` | CJK and emoji blocks 1F300-1F64F, 1F900-1F9FF are 2; ZWJ is 0. Gaps: 1F680-1F6FF, 2600-27BF emoji presentation, regional indicators, VS16 widening. Wide pairs stay consistent on edit and resize (tested). |

## Selection

| Feature | Status |
|---|---|
| Double click word, triple click line | OK (`termtouch.rs`, `select_word`, `select_line`); Midget: confirm with a mouse. |
| Rectangular with Option | Missing: `Selection` is linear. Needs a block flag on a BifrOSt-side selection type. |
| Copy trims trailing blanks, skips wide spacers | OK (`select::extract`) |

## Programs

| Program | Status | Basis |
|---|---|---|
| vim | Expected OK; Midget | alt screen, app cursor, bracketed paste, colours; taps are clicks, drags pending |
| htop | Expected OK; Midget | alt screen, 256/truecolor; wheel ok; taps are clicks |
| tmux | Expected OK; Midget | alt screen, wheel ok, taps are clicks, title via OSC 2 |
| less | OK | alt screen; wheel becomes arrows unless less enabled mouse |
| git log | OK | runs in less |
| ls --color | OK | SGR 16/256 |
| fzf | Expected OK; Midget | cursor save/restore, clicks pending |
