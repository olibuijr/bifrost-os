# Terminal finish (round 75)

Slug `terminal-finish`. Builds on `terminal-polish` (wave 2). Terminal lives in `springboard/src/shell/terminal/`
(shell code, not an SDK app), so no app version bump.

## Reference evidence

Read directly with `grep -a` from the extract (names only, nothing copied into the repo):

- `~/Projects/macos-ref/extract/ref27/System/Applications/Utilities/Terminal.app/Contents/Resources/Base.lproj/MainMenu.nib`:
  `Split Pane`, `Close Split Pane`, `Secure Keyboard Entry`, `Open Window Group`.
- `.../Base.lproj/TTRunCommand.nib`: `New Command`, `Run command inside a shell` (the panel that runs a typed
  line in a new tab).
- `.../Base.lproj/TTAppPreferences.nib`: `Cursor`, `CursorType`, `CursorBlink`, `Blink cursor`, `Window Groups`,
  `Save Windows as Group`, `Open Window Group` (help text: "A window group is a configuration of one or more
  Terminal windows"), `Remove Window Group`.

REA (`open_binary`, `decode_interface_builder`) was not used: the same gap as `terminal-polish` (no program file in
the extract, nib format not supported). Re-measure once the extract has the program.

## Rows

| Feature | State before | State after | BifrOSt path |
|---|---|---|---|
| Split panes | Missing | Two sessions side by side (one split at a time) | `shell/terminal/split.rs`, `term/sessions.rs` (`split`, `pair`), `shell/mod.rs` (`draw_terminal`, `terminal_touch`) |
| Window groups (save and open) | Missing | Save the folder of each tab; open one tab per saved folder (one group kept) | `shell/terminal/groups.rs`, prefs `terminal.window_group` |
| New Command | Missing | More menu row opens a command line; it runs in `/bin/sh -c` in a new tab that closes when the command ends | `shell/terminal/command.rs`, `shell/mod.rs` (`spawn_terminal_tab`) |
| Secure Keyboard Entry | Missing | While Terminal is in front, the shell's shortcuts (switcher, search, input source, Home, Back, window keys) do not take its keys; Term and Paste keys still run | `shell/hwkeys.rs` (`hw_shortcut`), `shell/terminal/profiles.rs` (`terminal_toggle_secure`), prefs `terminal.secure_input` |
| Profile cursor style and blink drawing | Profile fields existed, not drawn | Caret style (Block, Underline, Bar) replaces the default block; a program's own DECSCUSR shape still wins. Blink toggles on the shell clock (500 ms half period) | `term/mod.rs` (`set_cursor`, `set_blink_phase`, `cursor_state`, `blink_phase`, `next_blink`), `shell/terminal/profile_table.rs` (`CursorStyle::shape`, `key`, `from_key`, `next`), `shell/mod.rs` (tick and next-wake) |
| Tap on a link | Only the More menu opened a link | A single tap on an http(s) link in the grid opens it; the keyboard still comes back | `shell/termtouch.rs` (`link_at`, `take_link`), `term/mod.rs` (`row_text`), `shell/mod.rs` (`terminal_touch`) |

## Where the More menu rows are

Rows 16 to 23 are appended, so the existing indices (and the tests that assert them) stay put:

| Row | Label | Action |
|---|---|---|
| 16 | Cursor style | cycles Block, Underline, Bar; saved |
| 17 | Blink cursor | on or off; saved |
| 18 | Secure keyboard entry | on or off; saved |
| 19 | New command | opens the command panel |
| 20 | Split pane | splits the active tab |
| 21 | Close split pane | closes the active pane of the shown pair |
| 22 | Save window group | saves the tab folders |
| 23 | Open window group | opens the saved folders as tabs |

## Shared-state changes

- `term::Sessions` gained `split` (the pair), `pair()` (shown while the active session is in it), `set_split`, and
  `close` keeps the pair's indexes in step with the removal. `clear` resets it.
- Every place that resized the terminals now calls `Shell::resize_terminals` (set_keyboard, close panel, the More
  menu actions, rotation, the desktop window resize, the chrome tap). It gives each session its pane and skips
  sessions already at their size, because a resize clears the selection and repaints the grid.
- `Shell::new_terminal_tab` now delegates to `spawn_terminal_tab(argv, dir)`, shared with New command.
- `Terminal` keeps the profile caret and blink phase; `Terminal::draw` and `repaint` read them through `cursor_state`.

## Tests

- `termtouch::tests::a_tap_on_a_link_takes_its_url_once`
- `term::tests::caret_blinks_on_half_periods_and_reports_phase_changes`
- `shell::terminal::profile_table::tests::cursor_styles_cycle_and_round_trip_their_keys`
- `shell::terminal::split::tests::halves_share_the_area_with_a_one_pixel_divider`
- `shell::terminal::groups::tests` (folder round trip, empty and blank folders)
- `shell::terminal::tests::{new_command_runs_in_its_own_tab_and_an_empty_line_stays_open,
  secure_keyboard_entry_keeps_shell_shortcuts_out_of_the_terminal, window_group_saves_tab_folders_and_opens_them_as_tabs,
  split_pane_sits_beside_the_active_tab_and_a_tap_picks_the_pane, cursor_style_and_blink_persist_and_reload}`

None of these were built or run in this round (code-only rule). The lead builds and runs them.

## Not done, and why

- **Split panes are two panes.** One split at a time, side by side only (no horizontal stacking, no nested splits).
  The tab strip still lists both panes as tabs. Terminal.app shows one tab with panes; folding the pair into one tab
  needs the header to draw pane groups, which is left for a follow-up.
- **Window groups keep one group.** Terminal.app keeps named groups and a startup group. Here "Save" replaces the
  saved group, and "Open" adds one tab per folder. Only folders are saved (not commands or profiles).
- **Secure Keyboard Entry is a shell-only grab.** Terminal.app's version stops other processes from seeing the keys.
  On BifrOSt the shell is the only consumer of the keyboard, so the grab only stops the shell's own shortcuts. No
  lock indicator is shown in the menu bar yet; the message after toggling says it is on.
- **New command has no keep-open.** The tab closes when the command ends, as Terminal's run-command window does by
  default. It runs through `/bin/sh -c`, not the login shell.
- **Cursor and blink are global.** They apply to every tab from the More menu, saved in prefs. Per-profile cursor
  settings wait for the profile table merge (`profile_table::PROFILES` is still dead code, as in `terminal-polish`).
  The blink period is fixed at 500 ms per half.
- **A double tap on a link opens the link on its first tap.** The first tap is a plain tap, so the link opens; the
  second tap then selects the word. Holding the link to select it first would need a long-press change, not done.

## Midget checks still needed

- More menu: rows 16 to 23 fit, scroll, and read correctly at phone and desktop sizes.
- Cursor style: pick Underline and Bar, then type; the caret shape changes and survives an app restart.
- Blink: turn it on and watch the caret toggle about twice a second; turn it off and it holds still.
- Secure keyboard entry: turn it on, press the switcher and search keys in Terminal; nothing else should open.
  Turn it off; the switcher works again.
- New command: type `uname -a`, the tab shows the output; it closes when the command exits.
- Split pane: split a tab, tap each half, type in each; close one half and the other fills the screen.
- Window groups: open two folders in tabs, save the group, close the tabs, open the group; two tabs return.
- Tap on a link: tap a `https://` link in output; the browser opens and the keyboard comes back.
