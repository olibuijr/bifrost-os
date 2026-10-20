# Review of `integration`, 2026-10-20 (10 Sonnet reviewers, report only)

Branch `integration` at `d1eccb8e6`, the work since `main` (2026-10-13). Ten read-only reviewers, one area each, read the
changes and the code around them. Each finding was confirmed by reading code, not by running it. Severity: **P0** security
or data loss, **P1** broken core behaviour or crash, **P2** wrong behaviour in a normal case, **P3** minor. IDs are `R<area>-<n>`.

| Area | P0 | P1 | P2 | P3 |
|---|---|---|---|---|
| Window server security and capture | 2 | 3 | 4 | 5 |
| Keyboard routing and the text engine | 0 | 0 | 5 | 5 |
| Clipboard and pasteboard | 0 | 3 | 6 | 5 |
| Desktop windows, overlays and Overview | 0 | 0 | 5 | 4 |
| App host, launch policy and permissions | 4 | 2 | 3 | 6 |
| Packaging, updates, recovery and system services | 2 | 3 | 7 | 2 |
| Audio server and media service | 0 | 1 | 7 | 3 |
| Mail app and network code | 0 | 2 | 5 | 8 |
| SDK runtime, widgets and rendering | 0 | 1 | 2 | 6 |
| Settings and the system apps changed this week | 0 | 0 | 4 | 11 |
| **Total** | **8** | **15** | **48** | **55** |

## R1. Window server security and capture

The capture side is well built. screencopy, ext-image-copy-capture and bifrost_window_capture_v1 are visible only to privileged clients (the root-owned socket), and the shell control socket is 0600 and checks SO_PEERCRED. A sandboxed app cannot take screen pixels or reach the privileged requests. Admission is not enforced consistently elsewhere: the mount-namespace check guards only data-control. Lock, input-method, virtual-keyboard, layer-shell, foreign-toplevel, output-management and output-power globals are open to every sandboxed app, which allows lock-screen bypass, keystroke injection and sniffing, control of other apps' windows, and a window-server crash.

### R1-1 P0 (high): Any client can lock and unlock the session (ext-session-lock is open to everyone and the lock handler trusts every caller)

`springboard/wm/src/state.rs:221`

- **Failure:** SessionLockManagerState::new(&dh, |_| true) lists ext_session_lock_manager_v1 to every client, including sandboxed apps on the raw Wayland socket. lock.rs lock() unconditionally sets locked=true, drops the existing lock_surface and confirms, with no check that a lock is already held. The app's own lock object then gets lock_status=true. While the shell's lock screen is up, an app binds the manager, sends lock, waits for locked, then sends unlock_and_destroy. Smithay accepts it because that object's own status is true, and calls WmState::unlock(). That sets locked=false, so the real lock screen disappears and all app windows are drawn and take input. Alternatively the app keeps the lock and shows its own lock surface (a password phishing screen). Or it locks and exits, leaving a permanent black, input-dead session, because a lock whose client dies stays locked.
- **Fix:** Make the global privileged-only: SessionLockManagerState::new(&dh, |c| is_privileged(c)). In lock(), if self.locked, return without confirming (dropping the SessionLocker sends finished). Remember which lock object holds the session and ignore unlock from any other.

### R1-2 P0 (high): Input method and virtual keyboard are offered to sandboxed apps (same-uid check without the mount-namespace check)

`springboard/wm/src/text_input.rs:75`

- **Failure:** client_filter uses only Trust::SameUser, which compares peer uid with the compositor's uid. Native packages run in bwrap as the same account on the raw socket, which data_control.rs itself documents. A sandboxed app binds zwp_virtual_keyboard_manager_v1 and types into any focused window (a Terminal, the lock screen, a sudo or Log Out card). It can also bind zwp_input_method_manager_v2, grab the keyboard and read surrounding text from every text field, which is a system-wide keylogger. The sandbox is bypassed completely.
- **Fix:** Admit only privileged clients, or at least reuse data_control::admits (privileged, or same uid and shares_mount_ns). The on-screen keyboard process should connect through the privileged socket. Add a host test that a same-uid client with shares_mount_ns=false is refused.

### R1-3 P1 (high): Layer-shell Exclusive keyboard interactivity steals focus from the lock screen and every app, and is open to every client

`springboard/wm/src/handlers.rs:179`

- **Failure:** layer_commit gives keyboard focus to any layer surface that commits keyboard_interactivity=Exclusive. There is no self.locked check and no privilege check, and zwlr_layer_shell_v1 is unfiltered. A sandboxed app maps a transparent Overlay layer surface with Exclusive interactivity. While the screen is locked, the password the user types goes to the app instead of the lock surface (focus() returns early when locked, but this path calls keyboard.set_focus directly). Outside the lock, every keystroke goes to the app and it can phish with an overlay.
- **Fix:** In layer_commit return early when self.locked. Refuse Exclusive interactivity and the Overlay and Top layers to non-privileged clients (downgrade to OnDemand or close the surface). Better, gate the layer-shell global to privileged clients.

### R1-4 P1 (high): An app's layer surface can make the usable area negative and crash the window server in keep_on_screen (i32::clamp panic)

`springboard/wm/src/wm.rs:340`

- **Failure:** refresh_area (state.rs:258-270) feeds Smithay's non_exclusive_zone to Wm::set_area with no clamping. Smithay subtracts the exclusive amount without limits (for example size.w -= amount for a RIGHT-anchored surface). A sandboxed client creates a layer surface anchored right with exclusive_zone 100000 and commits. The area width goes hugely negative and set_area calls keep_on_screen for every non-tiled window. r.x.clamp(a.x - r.w + 48, a.x + a.w - 48) has min > max, so std's clamp asserts and panics. The panic happens inside the commit handler in the main event loop (no catch_unwind anywhere in wm), so bifrost-wm exits and the session dies. Anchor::all() with an exclusive zone gives a 0x0 area and the same panic for windows narrower than 96. The same call is reachable through map_at via set_origin.
- **Fix:** Clamp the usable area in refresh_area to a sane minimum (width and height at least one, and at least a fraction of the output). In keep_on_screen compute hi = (a.x + a.w - KEEP_VISIBLE).max(lo) as the y line already does. Reject the exclusive zones of non-privileged layer surfaces.

### R1-5 P1 (high): zwlr_foreign_toplevel_manager is unfiltered: any app can list titles and close, minimize, activate, maximize or fullscreen other apps' windows

`springboard/wm/src/session_protocols.rs:329`

- **Failure:** The global is created with () data and no can_view. A sandboxed app binds it, receives title and app_id for every toplevel (privacy leak: document names, URLs in browser titles) and sends close, set_minimized, activate, set_maximized or set_fullscreen on any handle. The Dispatch (lines 721-770) performs these with no ownership or privilege check, so one app can kill, hide or force-resize the others' windows or steal focus.
- **Fix:** Make it privileged-only (PrivilegedOnly with can_view, as for screencopy) or at least hide handles that belong to other clients' toplevels from non-privileged managers and ignore their requests. The shell already connects privileged.

### R1-6 P2 (high): Output management and output power are open to every client (rotate, change mode, blank the display)

`springboard/wm/src/session_protocols.rs:331`

- **Failure:** zwlr_output_manager_v1 and zwlr_output_power_manager_v1 are created with no filter. A sandboxed app creates a configuration and applies set_transform or set_mode. run_configuration then calls rotate_output and the DRM set_mode hook (a real modeset), so it can rotate the desktop, flip modes in a loop, or send zwlr_output_power_v1.set_mode(off) to blank the screen (hook set_power). create_configuration ignores the serial, so there is no stale-state check either.
- **Fix:** Create both globals as privileged-only. The shell already uses them through its privileged connection.

### R1-7 P2 (high): xdg-activation accepts any client-supplied serial, so any app can raise and focus itself at will

`springboard/wm/src/session_protocols.rs:208`

- **Failure:** activation_allowed only checks has_serial (data.serial.is_some()) and that the token is recent. The serial is never validated against a real input event on that seat. Any app calls get_activation_token, set_serial(1, seat), commit, then activate(token, its surface). activate_surface restores, raises and focuses the window. An app can steal keyboard focus repeatedly, for example while the user types a password into another app.
- **Fix:** Validate the serial against the seat's last input serial and require the requesting client to have had keyboard focus (Smithay's token data holds both). Otherwise only mark the window as urgent.

### R1-8 P2 (high): Compositor key chords run while the session is locked (Alt+F4 closes apps behind the lock screen)

`springboard/wm/src/keybindings.rs:142`

- **Failure:** input.rs key() intercepts chords before any client and calls run_action without checking self.locked. With the screen locked, Alt+F4 sends close to the focused toplevel (wm.focused() is still set). Ctrl+Super+arrows and Ctrl+Super+F re-tile or fullscreen windows, and Alt+Tab changes wm state. Someone at a locked machine can close every window one by one, causing unsaved-data loss in apps that do not prompt. The shell gates the control-socket actions (Force Quit, Log Out, Overview) correctly, but the window-server-local actions are not gated.
- **Fix:** In WmState::run_action return immediately when self.locked.

### R1-9 P2 (medium): bifrost_window_level_v1 (set_level, set_origin, tile) is open to every client: any app can pin itself above all windows

`springboard/wm/src/window_level.rs:153`

- **Failure:** The global is created for everyone, and the docs say it is the shell's. Any app calls set_level(Floating) on its own surface and wm keeps it above all Normal windows permanently. Combined with set_origin and tile it can cover the menu bar, Dock or other apps' permission dialogs for spoofing or clickjacking. Arguments are only checked against own-surface scope, and this stays within the client's own surfaces, so there is no cross-app move. Any client can also bind it repeatedly (each bind adds a Reporter and runs report_frames), an O(n squared) CPU cost.
- **Fix:** Restrict Floating level to privileged clients or to window types that need it, and ignore set_origin and tile from unprivileged clients. Cap the number of managers per client.

### R1-10 P3 (medium): Mount-namespace admission is a single-point proxy: pid reuse and a TOCTOU between accept and /proc read

`springboard/wm/src/state.rs:56`

- **Failure:** shares_mount_ns reads /proc/<pid>/ns/mnt after accept, using the SO_PEERCRED pid taken from connect time. If the sandboxed connector exits before the compositor accepts and its pid is recycled by a host-namespace process, the check compares the wrong process. A dead pid fails closed, but a recycled one admits. This needs pid-space wrap, which is hard to force, so the risk is low. Reading is also permission-dependent: a non-dumpable same-uid host program (a legitimate wl-paste in some setups) is refused. The soundness answer is: sandboxed apps (own mnt ns, pid 0 or user-ns translation fail closed) are correctly refused, and fork does not matter because admission is per connection, but a host program that hands its connected fd to a sandboxed child lends it the globals.
- **Fix:** Prefer pidfd_open or SO_PEERPIDFD (Linux 6.5) taken at accept and read the namespace through the pidfd, or verify the process start time is unchanged after the read. Apply the same check to every same-uid-gated global (see the input-method finding).

### R1-11 P3 (medium): Privileged listening descriptor leaks to child processes (no CLOEXEC)

`springboard/wm/src/privileged.rs:78`

- **Failure:** The helper sets the fd inheritable, and listener() adopts it with from_raw_fd without setting FD_CLOEXEC. Children the window server spawns (spawn() in lib.rs and the XWayland process) inherit the root-owned listening socket. A compromised X client or child running as the account can accept() on it and steal or impersonate connections from the root shell. BIFROST_WM_PRIVILEGED_FD also stays in their environment.
- **Fix:** After taking over the descriptor, set FD_CLOEXEC (fcntl F_SETFD) and remove the env var. Also verify with fstat/getsockopt that the fd is a listening unix socket.

### R1-12 P3 (high): Shell main loop blocks on the control socket: a stalled client freezes the desktop

`springboard/src/bare/control.rs:413`

- **Failure:** accept() runs inside the shell's frame loop (src/bin/shell.rs:787). After an allowed peer connects it does a blocking read_line with a 2 s read timeout, up to 65536 bytes. A same-uid unsandboxed process that connects and sends nothing, or dribbles a byte every 1.9 s, stalls rendering and input for 2 s per connection, or effectively indefinitely. The sandbox hides the path, so only unsandboxed account processes can do this.
- **Fix:** Read the request line from a non-blocking socket across frames, or hand each accepted connection to a short-lived thread with a hard total deadline of about 200 ms.

### R1-13 P3 (medium): Control socket serves screenshots, raw input and shell control to any unsandboxed process of the account, which bypasses the privileged-socket capture protection

`springboard/src/bare/control.rs:58`

- **Failure:** allowed_uids includes the account uid, the same uid every unsandboxed user program has. privileged.rs argues a uid cannot tell the shell from an app, so capture is root-socket only. But shot, tap, swipe, key, open-url, term and image-attach on this socket give such a process screen contents, input injection and terminal control. This is consistent with the data-control trust model (unsandboxed same-user programs are trusted), but it makes the capture-global privilege boundary moot for them, and the documentation does not say so. Sandboxed apps are correctly excluded because a fresh /run hides the path.
- **Fix:** Document it as an accepted trust boundary, or restrict shot/term/image-attach to root and require a per-session token for the account uid.

### R1-14 P3 (medium): Privileged accept loop spins on persistent accept errors (level-triggered source)

`springboard/wm/src/privileged.rs:113`

- **Failure:** On an error other than WouldBlock (for example EMFILE or ENFILE under fd exhaustion, which any client flood can cause) the handler logs, breaks and returns Continue with Mode::Level. The pending connection stays in the queue, so the source fires again immediately, giving 100% CPU and a log flood until descriptors are freed.
- **Fix:** On an accept error other than WouldBlock or Interrupted, rate-limit the log and pause the source briefly (remove and re-arm a short timer), or switch to edge triggering.

## R2. Keyboard routing and the text engine

The routing and the grapheme-aware text model in ui/src/text.rs are sound: the char-boundary handling, undo grouping (including replace_ranges) and the Term, Search and lock-screen guards all held up under reading, and I found no slicing panic on UTF-8 input. The defects are gaps around the new chord paths: secure fields still copy and cut to the clipboard, TextEdit's Cmd+V/X/A/Z ignore a focused Find field or open sheet, several modal layers are missing from shell_takes_typing, and a few shell shortcuts and the key repeat bypass the alert and Secure Keyboard Entry guards.

### R2-1 P2 (high): Secure (password) shell fields copy and cut their text to the clipboard on Cmd/Ctrl+A then Cmd/Ctrl+C or X

`springboard/src/shell/textfield.rs:251`

- **Failure:** In a secure TextField (Wi-Fi password field in wifi_page.rs:209, the app-authentication and volume-unlock sheets), press Cmd+A then Cmd+C. hw_keys maps these to Shortcut::SelectAll and Shortcut::Copy, and hw_type forwards them as Out::Named("Copy") to TextField::edit. That calls edit::apply(Action::Copy), which runs clipboard::set_text(selected(target)) with the real password text. Cut does the same at line 252. The touch edit menu hides Cut and Copy for secure fields (edit::actions), but the keyboard path has no secure check. The cleartext password then sits on the shell clipboard, where any app or clipboard history can read it. The Lock screen is not affected because hw_shortcut returns early there.
- **Fix:** In TextField::edit, make Out::Named("Copy") and Out::Named("Cut") do nothing when self.secure. Better, enforce it in edit::apply so every EditTarget with a secure flag is covered. Add a test next to secure_fields_only_paste that presses the Copy and Cut keys.

### R2-2 P2 (high): TextEdit: Cmd+V/X/A/Z act on the document while the Find or Replace field or a sheet has focus

`springboard/apps/textedit-desktop/src/main.rs:2302`

- **Failure:** The shell now sends Cmd+A/C/X/V/Z to the SDK app. The chord branch calls chord_edit_action(...) and perform(...) before page_has_keys() or any focus check. perform() redirects only Paste to a panel, and only for link, props, line and spacing_sheet (line 410); it ignores a focused Find or Replace field. Cmd+V with the Find field focused therefore pastes into the document body, because Action::Paste runs doc.text.insert, and the find text stays empty. Cmd+X cuts the document's selection. Cmd+A selects the whole document instead of the field text. Cmd+Z undoes document edits behind the bar. With a props, spacing, line or link sheet open, Cmd+X, Cmd+A and Cmd+Z still hit the document behind the sheet; only Paste is redirected. This silently modifies the user's file.
- **Fix:** In the chord branch, check focus and open sheets first. When find_open && focus != Body, or any sheet or panel is open, route Cut, Copy, Paste, SelectAll and Undo to that field's buffer. Otherwise swallow them and leave the document untouched. Reuse the same condition as page_has_keys.

### R2-3 P2 (medium): shell_takes_typing omits several modal layers, so chords and nav keys reach the app behind them

`springboard/src/shell/hwkeys.rs:831`

- **Failure:** modal_up() (mod.rs:4710) treats power_menu, pick_sheet, trust_sheet, photo_pick_sheet, keyboard_viewer, a11y_panel, switch_panel and print as modal. shell_takes_typing does not list them. hw_chord, hw_nav and the field_edit gate in shell.rs:1084 rely on it. With the Shut Down or Restart menu or a pick or trust sheet up over an SDK app, Cmd+Backspace, Delete, Page Up/Down and Cmd+arrows go to the app behind as chords. For example, Cmd+Backspace in Files or TextEdit deletes an item or text. hw_back also has no branch for these layers: Escape is typed to the external app, or goes home for a built-in app, instead of closing the layer. hw_type's modal_layer guard only stops plain typing, and it is Some only on the desktop after a draw.
- **Fix:** Add power_menu, pick_sheet, trust_sheet, photo_pick_sheet, keyboard_viewer, a11y_panel, switch_panel_shown() and print_showing() to shell_takes_typing. Better, derive it from modal_up() so the two lists cannot drift. Give hw_back an Escape branch for them.

### R2-4 P2 (medium): Ctrl+A and Ctrl+Y no longer Select All or Redo in any SDK app, even with no text field focused

`springboard/src/bin/shell.rs:1011`

- **Failure:** The raw_sc filter drops Shortcut::SelectAll for Ctrl+A, and Shortcut::Redo for Ctrl+Y, whenever sdk_app_front() && !shell_takes_typing(). It does not check ext_text_focused(). In a list or grid app with no text field (Files, Mail list, Photos), Ctrl+A now falls through to hw_chord and arrives as chord(30, CTRL). Only text fields give that chord a meaning (line start). The app gets no SelectAll command, so Windows-style Ctrl+A Select All does nothing.
- **Fix:** Add && shell.ext_text_focused() to the filter, so Ctrl+A is the line start only while a text field is focused and stays Select All elsewhere.

### R2-5 P2 (medium): A held Return that was already auto-repeating when a card alert appears answers the alert

`springboard/src/bin/shell.rs:1128`

- **Failure:** key_repeat is cancelled only after a key has answered an alert (take_key_repeat_cancel), not when an alert appears. If the user is holding Return and a crash alert, low-battery alert or launch alert pops up, key_repeat.due() calls hw_type(Out::Enter). card_alert_up() then makes hw_type call card_alert_key(false), which presses the bold button: Reopen, Low Power Mode, or Open for a downloaded program. This is the same stray-Return hazard the permission and notification sheets guard against with 'a key never grants'.
- **Fix:** In the key loop, cancel the repeat when card_alert_up() is true, for example if shell.card_alert_up() { key_repeat.cancel(); } before the due() loop. Alternatively drop repeated Enter in hw_type while an alert is up.

### R2-6 P3 (medium): Return on the launch-confirm alert runs a downloaded program and grants it permanently

`springboard/src/shell/launch_alert.rs:71`

- **Failure:** key_button(cancel=false) returns Button::Main. For Kind::Confirm that is Open, and launch_alert_touch then calls launch_gate::grant, save_overrides and run_downloaded. The alert appears right after the user opens the file in Files, so a second Return press, or the repeat in the finding above, executes a quarantined binary and remembers the grant. The malware alert avoids this by making a key always take Cancel, and the permission sheets by making Return do nothing.
- **Fix:** Treat Confirm like the permission sheets: key_button returns None for Return (cancel=false), or maps Return to Dismiss. Only a click or tap should say Open.

### R2-7 P3 (medium): Session event taps still see keystrokes under Secure Keyboard Entry and in password sheets

`springboard/src/shell/services/event_services.rs:326`

- **Failure:** tap_key bypasses the taps only for reserved keys, the bypass flag and the lock screen. With Terminal secure=true (Secure Keyboard Entry), or while auth_showing(), volume_showing(), net_auth_showing() or a secure TextField is focused, every key still goes through input_taps.process. An app with the events permission that registered a tap can read, drop or replace keystrokes, including password characters. hw_shortcut says that under Secure Keyboard Entry only the Terminal's own keys run, but the taps run before that gate in the key loop.
- **Fix:** In tap_key, return Some(code) unchanged when self.terminal_ui.secure is set and the Terminal is in front, and when a secure sheet or field has the keyboard.

### R2-8 P3 (medium): Some shell shortcuts and Overview keys bypass the card-alert and Secure Keyboard Entry gates

`springboard/src/shell/hwkeys.rs:1159`

- **Failure:** hw_shortcut blocks only Shortcut::Term and Shortcut::Paste while a card alert is up. Cmd+Q, Cmd+W, Cmd+M, Cmd+, Cmd+Space, Cmd+Tab and the capture and zoom chords still run, so Cmd+Q quits the app that raised a permission or crash alert, which then outlives it. hw_overview_key (shell.rs:1022) runs before hw_shortcut and has no card_alert_up, secure-Terminal or shell_takes_typing check. F3, F4 and Ctrl+Up therefore toggle Overview or the Apps grid over an alert, or over a Terminal that has Secure Keyboard Entry on.
- **Fix:** Early-return in hw_shortcut and hw_overview_key when card_alert_up(). Return false from hw_overview_key when the Terminal is secure and in front.

### R2-9 P3 (medium): line_end, kill_line and select_line_at split a CRLF pair

`springboard/ui/src/text.rs:193`

- **Failure:** line_end returns the byte index of '\n', which for CRLF text lies after '\r'. End, Cmd+Right and Ctrl+E put the caret between '\r' and '\n', and typing there yields "abc\rX\n" and corrupts the line ending. at_column and kill_line leave a lone '\r', and select_line_at selects a trailing '\r' that Copy then includes. next_boundary treats \r\n as one cluster, so the line helpers are inconsistent with the rest of the module.
- **Fix:** Make line_end step back over a '\r' that precedes the '\n' (and the end of text), and let kill_line and delete_to_line_start treat \r\n as one break. Or normalise line endings on load.

### R2-10 P3 (medium): TextEdit moves and deletes by char cell, not by grapheme

`springboard/apps/textedit-desktop/src/select.rs:177`

- **Failure:** Document text is a Vec<Cell> of single chars. Left and Right use head-1 and head+1, and DeleteKey::Back and Forward remove one cell. Backspace over a flag emoji, a ZWJ family or a skin-tone emoji, or over an 'e' followed by a combining acute, deletes only the last code point. That leaves an orphan variation selector, skin tone or half a flag, and one visible character takes several key presses. The shell and SDK fields do this correctly through text::prev_boundary and next_boundary.
- **Fix:** Use bifrost_ui::text::next_boundary and prev_boundary over a String view of the cells, or port the cluster rules to Cell, for the arrows and the single-character deletes.

## R3. Clipboard and pasteboard

The shell now owns the Wayland selection through a data-control client thread (pasteboard_wl.rs) that mirrors an in-process pasteboard, and the import and export directions are guarded against echoing each other. The failure paths are weak: the bridge thread cannot die without the shell's main poll loop spinning, an app can kill it with a malformed type, reads and writes use a hard 16 KiB IPC packet that silently truncates or drops payloads, and copied secrets reach every app, the Wayland selection and a file on disk with no concealed or expiry handling. Clipboard contents are never logged (confirmed by reading the code).

### R3-1 P1 (high): Dead clipboard thread makes the shell busy-loop (notify socket at EOF stays readable)

`springboard/src/shell/pasteboard_wl.rs:67`

- **Failure:** The 'clipboard' thread returns on ANY error (no data-control global because BIFROST_WM_DATA_CONTROL=off or a non-Bifrost compositor, connect failure, protocol error, window-server disconnect, a panic). Its `notify_thread` socket is dropped, so `notify_shell` (the fd registered by `pasteboard_sync::poll_fd()` in Shell::poll_fds) is at EOF and readable forever. `Wl::incoming` reads Ok(0) and stops, but the fd never stops being readable, so `kms::wait_precise` (POLLIN|POLLHUP) returns immediately every iteration and `shell.pump()` runs in a tight loop: 100% CPU and no idle. The module doc claims a missing global 'only ends the thread'; in fact it starts a spin from the first tick.
- **Fix:** On Ok(0) from the notify socket (or when run() exits) set NOTIFY_FD to -1 / mark the Wl dead so poll_fd() returns -1; better, keep the notify write end alive in the Wl (not the thread) so EOF cannot happen, and replace the connect-once thread with a reconnect loop with backoff.

### R3-2 P1 (high): Any app can panic the clipboard thread with a NUL in a pasteboard type

`springboard/src/shell/pasteboard_sync.rs:66`

- **Failure:** ToShell::PasteboardSet items decode (proto items()) checks only UTF-8 and length <=128, so a type like "\0/x" is accepted. `pasteboard::mime_for` passes unknown ids through, `offers()` keeps anything containing '/', and `Client::set` calls `source.offer(mime)`. The wayland-scanner generated request does `CString::new(mime).unwrap()` (wayland-scanner-0.31.11/src/common.rs:543), which panics on an interior NUL. The clipboard thread dies, the Wayland clipboard stays dead for the session, and with the previous finding the shell then spins. Needs only one app message, no focus.
- **Fix:** Validate offered MIME strings in offers() (reject NUL/control chars, bound length, require token chars `type/subtype`), and validate UTIs at decode in proto items(). Also wrap the thread body in catch_unwind or restart the thread.

### R3-3 P1 (high): Password copied by the Passwords app is broadcast, offered to all Wayland programs and persisted to disk, never concealed or cleared

`springboard/apps/passwords-desktop/src/main.rs:314`

- **Failure:** 'Copy' (and Pending::Copy at line 375) call `clipboard::set_text(password)`. The app's runtime sends SetClipboard; the shell stores it in clipboard.txt (render/src/clipboard.rs save(), 0600 but plaintext and surviving reboot), pushes ToApp::Pasteboard/Clipboard to every running app, and pasteboard_sync offers it to the window server as text/plain etc., readable by wl-paste, any unsandboxed same-user program and any focused app. There is no concealed/transient marker in the pasteboard model (grep for concealed/transient/expire finds nothing) and no auto-clear timer, so the password stays on the clipboard and on disk indefinitely.
- **Fix:** Add a concealed flag (a type such as org.nspasteboard.ConcealedType in PasteboardSet) that the shell honours: do not persist to clipboard.txt, do not forward to other apps or offer to the window server's selection until pasted, and clear after ~30-45 s if unchanged. Make the Passwords app arm the clear.

### R3-4 P2 (high): Items over about 12 KB are silently dropped on encode, so an image copy empties the clipboard

`springboard/proto/src/lib.rs:1139`

- **Failure:** items_() drops every item larger than (16384-4096)/n bytes, for both ToShell::PasteboardSet and ToApp::Pasteboard. An app copying a 100 KB PNG sends PasteboardSet with zero items; the shell runs pasteboard::set(vec![]) (apphost.rs:2295), bumps the count, empties the board and, in pasteboard_sync, calls device.set_selection(None) because offers() is empty. The user's previous clipboard is destroyed and nothing is copied, with no error. Conversely, a 32 MB image imported from a browser is stored in the shell but never reaches any app (dropped on encode), while ToApp::Clipboard{text:""} clears the apps' text clipboard.
- **Fix:** Carry large payloads out of band (memfd/pipe via SCM_RIGHTS, as send_file does) or in chunks; at minimum make the sender report failure instead of sending an empty set, and have the shell ignore a PasteboardSet that arrives empty because items were dropped.

### R3-5 P2 (high): Text over about 16 KB is truncated and the truncated copy overwrites the originating app's clipboard

`springboard/proto/src/lib.rs:1155`

- **Failure:** str_() caps strings at MAX_PACKET-64 bytes. An app copies 40 KB of text: runtime.rs:1071 sends SetClipboard (truncated to ~16 KB). The shell sets clipboard text to the truncated string, pasteboard_sync replaces the pasteboard with it (set_plain), and offers the truncated text to wl-paste and all programs. Because SetClipboard does not set pb_from, ext_sync_pasteboard echoes ToApp::Clipboard{truncated} back to the originating app, whose kit clipboard (which held the full text) is overwritten. Pasting anywhere, including in the same app, yields silently shortened text.
- **Fix:** Same transport fix as above (fd or chunked transfer for clipboard text). Meanwhile do not echo to the writing app (treat SetClipboard like PasteboardSet for the `written` slot) and refuse or flag oversize copies instead of truncating.

### R3-6 P2 (medium): Every connected app (and any app, at any time) can read and overwrite the clipboard with no paste or focus gate

`springboard/src/shell/apphost.rs:2294`

- **Failure:** Read side: ext_sync_pasteboard pushes ToApp::Pasteboard and ToApp::Clipboard with the full typed items to every running app (including background and store-installed, bwrap-sandboxed ones) the moment anything is copied, and again on first contact. This defeats the point of keeping sandboxed apps off the data-control globals (wm/src/data_control.rs): they simply read the control socket. A background app can log every copy (passwords, tokens). Write side: `ToShell::SetClipboard` and `PasteboardSet` are accepted from any app regardless of `fg == Some(slot)` (ContextMenu, WantsKeyboard etc. are gated, these are not), so a background app can replace the clipboard at will (address swapping) or flood it, and each write re-offers the Wayland selection.
- **Fix:** Deliver the board lazily: send it to an app only when it is foreground (or on an explicit paste request/permission), and accept SetClipboard/PasteboardSet only from the foreground app or after user input; rate-limit writes per app.

### R3-7 P2 (medium): Unbounded 'clipboard-send' threads and fds from the shell's selection source

`springboard/src/shell/pasteboard_wl.rs:162`

- **Failure:** Every `Send` event spawns a thread that does a blocking write_all to the reader's pipe. Because the shell's source is the seat selection, any focused client (including a sandboxed app via wl_data_device) can issue `receive` repeatedly with large types (e.g. image/png of megabytes), hold the read ends open without reading, and each request leaves a thread blocked in write plus an fd. Thousands of requests exhaust threads/fds; `spawn_named` uses expect(), so a failed spawn panics the clipboard thread (then the busy-loop finding applies) and fd exhaustion hurts the whole shell. A reader that set O_NONBLOCK on its pipe gets a truncated payload on WouldBlock with no retry.
- **Fix:** Cap concurrent senders (e.g. 8) and drop beyond that, write with poll and a deadline (a few seconds) on a single worker, handle WouldBlock/EAGAIN, and use a non-panicking spawn.

### R3-8 P2 (medium): An external program clearing its copy (password-manager timeout) does not clear BifrOSt's copy; secret hints ignored

`springboard/src/shell/pasteboard_wl.rs:244`

- **Failure:** settle() returns on `Selection(None)` and items_from() drops empty data, so when a password manager sets then clears the clipboard after its timeout, the shell keeps the pasted secret in the pasteboard, in clipboard.txt, and in every app. The import also ignores the x-kde-passwordManagerHint / ConcealedType MIME hints and reads and persists the secret, forwarding it to all apps.
- **Fix:** On a program-set clear or empty text, clear the pasteboard and shell clipboard (only if the current contents came from that program). Skip import, persistence and forwarding for sources that carry the password-manager hint.

### R3-9 P2 (medium): Large imported selections are cloned per app before the size filter

`springboard/src/shell/apphost.rs:804`

- **Failure:** A browser copy offering image/png (30 MB) plus jpeg, html and others is imported whole (limit 32 MB per type, up to six types, so around 190 MB on the clipboard thread). tick() then calls ext_sync_pasteboard, which does snapshot() (clone) and sync_pasteboard calls `items.to_vec()` per running app and encodes it, only for items_() to drop everything over 12 KB. With N apps that is N x the payload of allocation and memcpy on the shell thread per copy, a visible stall, and 190 MB stays resident in the STATE mutex.
- **Fix:** Filter items by wire limit (or pass Arc'd data) before cloning for apps; lower the import cap for rich types or do not import types that cannot be delivered.

### R3-10 P3 (medium): Bridge connects once; a window-server restart or transient failure disables it for the session

`springboard/src/shell/pasteboard_wl.rs:299`

- **Failure:** run() calls Client::connect() once and returns on any dispatch/flush/read error. After the window server restarts (or if it is not yet listening at first tick) nothing reconnects and nothing re-offers the current pasteboard; Wl::set silently drops commands (channel receiver gone) while Board.offered claims the board was offered. The user then has no external clipboard in either direction and only a stderr line.
- **Fix:** Loop with backoff: reconnect, rebind, and reset `offered` so the current pasteboard is re-offered; expose a 'connected' state so incoming/set can report it.

### R3-11 P3 (medium): Selection reads are serial and block the bridge for up to ~9 s; no throttle on selection churn

`springboard/src/shell/pasteboard_wl.rs:270`

- **Failure:** settle() reads every wanted type one after another, 1.5 s deadline each (up to six types). While reading, the thread dispatches nothing and applies no shell commands, so the shell's own copies are not offered and Send events for its source are not served. A program (or a clipboard manager) that replaces the selection every few hundred ms keeps the thread permanently busy reading, with no rate limit.
- **Fix:** Read the text type first and deliver it, then fetch rich types lazily or concurrently with a shared deadline; coalesce and throttle selection changes.

### R3-12 P3 (medium): Import early return loses a copy made in the same pass; clearContents / image-only writes leave stale text

`springboard/src/shell/pasteboard_sync.rs:152`

- **Failure:** (a) pass() returns right after an import, so a shell-field or app SetClipboard that changed the shell clipboard in the same tick is overwritten by clipboard::set_text(imported) and seen_rev jumps past it: the local copy is lost. (b) An empty PasteboardSet (clearContents) or an image-only set returns None from pasteboard::set/set_items, so the old plain clipboard text stays in the shell (and clipboard.txt) and remains pasteable in shell fields even though the typed pasteboard is empty and the apps were told Clipboard{""}.
- **Fix:** Handle local revision changes before honouring an import (or compare timestamps), and clear clipboard text when the pasteboard is replaced by a set with no text.

### R3-13 P3 (medium): Superseded selection offers are dropped without destroy

`springboard/src/shell/pasteboard_wl.rs:129`

- **Failure:** `st.selection = Some(...)` overwrites an earlier pending offer proxy when two selection events arrive between passes (or while read_offer blocks), and `st.mimes.clear()` drops announced-but-unselected offers. Dropping a wayland-client proxy does not send destroy, so those offer objects (and their MIME lists) stay alive in the window server until the connection closes; a clipboard manager that rewrites the selection often leaks steadily.
- **Fix:** Call destroy() on the previous pending offer before overwriting it, and on offers cleared from `mimes`.

### R3-14 P3 (medium): pbcopy named boards are world-readable and unbounded

`springboard/cmd/src/pasteboard.rs:156`

- **Failure:** copy_to_board writes the board file and .type with std::fs::write (default umask, typically 0644) under ~/.local/share/bifrost/pasteboards. Content copied with `pbcopy -pboard find` (search strings, fonts, ruler data) is readable by other local users if the home or XDG dir is traversable, and there is no size cap on stdin.
- **Fix:** Create the directory 0700 and the files 0600 (OpenOptions mode), and cap input size.

## R4. Desktop windows, overlays and Overview

The chrome-surface scheme (ids 0-15, one layer surface per overlay, dropped the frame it closes) is mostly sound. Ids are unique, every exported rectangle is clamped to the screen, and sheets, the grid and Overview are removed cleanly. The real defects are around it: a left or right Dock is placed with a bottom-only anchor, window events for an app's extra windows are dropped, the green button stops being clickable above roughly 125% scale, and any open menu, panel or Overview keeps the shell redrawing every frame while idle.

### R4-1 P2 (high): Dock set to Left or Right is anchored to the bottom edge only, so it lands in the wrong place and reserves a huge bottom zone

`springboard/src/bare/wayland.rs:625`

- **Failure:** The Dock menu offers Bottom, Left and Right (desktop_bar.rs Menu::Dock rows). For Left or Right, desk.rs:1763 builds a tall narrow rect at x=margin or x=w-bar.h-margin. place_chrome gives chrome id 2 only Anchor::Bottom plus a bottom margin, with exclusive_zone = frame.height + bottom. Under layer-shell a surface anchored to one edge is centred along the other axis, so the vertical Dock appears horizontally centred and x is ignored. The pointer offset adds frame.x, so hit-testing disagrees with the drawing. The exclusive zone is the Dock's full height (several hundred px) taken from the bottom, so maximized and snapped windows shrink.
- **Fix:** Choose the anchor from the Dock position. Left: Anchor::Left|Top|Bottom with margin(y,0,0,x) and exclusive_zone = width + x. Right: Anchor::Right|Top|Bottom. Keep Bottom|Left|Right only for the bottom Dock. Pass the position into ChromeFrame, or infer it from the frame aspect.

### R4-2 P2 (high): Compositor events for an app's extra windows (Close, Resize, Maximized, Focus) are ignored

`springboard/src/shell/desk.rs:1259`

- **Failure:** Extra windows use flagged ids (win_id: WINDOW_ID_FLAG | slot<<16 | id, flag 0x0100_0000). wayland_window_event maps only Frame and Fullscreen through frame_app. Every other event uses apps().get(id.saturating_sub(1)), which is out of range for a flagged id, so it returns early. A window-server close on such a window does nothing. A compositor edge-resize never updates the shell's w.r or the app's size, so the window is drawn at its old size. Maximize state and click-focus raise are lost. Only commit 44e260440 fixed this, and only for Frame and Fullscreen.
- **Fix:** Resolve every event variant with frame_app(id), so the Focus, Close, Resize and Maximized arms receive ExternalWindow apps.

### R4-3 P2 (medium): Press on the green button is swallowed by a hard-coded 80 px window-move threshold

`springboard/src/bare/wayland.rs:1097`

- **Failure:** The move starts when s.pressed && drags && s.xy.0 > 80.0. For titled (non-unified) windows drag_zone returns (m.title, no no_drag rects), so the lights are not excluded. The green light spans 59..73 pt, so its centre passes 80 px at about 1.2x scale (pt=1.5625 puts it near x=103 px). A press on the green button then starts xdg move and returns before Event::Down is queued, so zoom and the arrange menu cannot be clicked. The Up is never sent and s.pressed stays true. A title-bar double-click zoom (desk.rs:2294) never gets its Down either. This also breaks the rule against fixed pixel positions.
- **Fix:** Remove the 80 px test. Add the traffic-light rectangle (w.light_rect for lights 0..2, inflated by a few px) to no_drag in drag_zone for non-unified windows too. After _move, clear s.pressed or forward a synthetic Up.

### R4-4 P2 (medium): Any open menu, panel, Search, Overview, Quick Look, on-screen keyboard or AssistiveTouch keeps the shell redrawing every frame while idle

`springboard/src/shell/mod.rs:1411`

- **Failure:** desk_animating() returns true for state flags rather than motion: cc_active, center_active, quicksearch.active(), desktop_menu.is_some(), overlay_visible() (AssistiveTouch or motion cues), quick_look.is_visible(), overview_active() and keyboard.shown(). needs_frame() ORs desk_animating(0.0) and draw() skips its early return while it holds, so a menu left open, an idle Control Center, Overview or the Apps grid, or AssistiveTouch enabled all force a full desktop draw at vsync forever. Each presented frame also runs wayland_windows and desktop_chrome_frames, which copies the chrome regions (the whole veil for Overview or the grid) every frame. The idle bench only covers the settled path.
- **Fix:** Use these flags only to decide that a frame may be drawn when dirty. Request continuous frames only for real motion (animating(), blink or next_wake timers, fx, dock slide). Redraw an open overlay on input or a change, as the Cmd+Tab strip does.

### R4-5 P2 (medium): An installed app can open an extra window narrower than 1 px, producing a zero-width toplevel

`springboard/src/shell/desk.rs:2107`

- **Failure:** app_window_table accepts any finite positive size (good_size), so w=0.1 pt passes. desk_sync_extra_windows computes size.0 = (w*k) as i32 = 0, and Desk::open clamps only the maximum (min(a.w)), so the window is Rect{w:0}. Extra windows' Resize events are dropped (finding 2), so min_w is never enforced. The backend then calls xdg set_window_geometry(margin,margin,0,h) (wayland.rs:508), which xdg-shell treats as invalid_size, a protocol error that disconnects the shell. The 8192 guard at wayland.rs:497 also turns an oversize into Err and exits the shell.
- **Fix:** Clamp extra-window sizes to size_range/m.min_w and m.min_h at open and resize. In the backend, skip or clamp windows with a zero or oversize dimension instead of returning Err.

### R4-6 P3 (high): full_from_zoom is never cleaned when a window closes, so a later full-screen exit restores the wrong frame

`springboard/src/shell/desk.rs:743`

- **Failure:** set_full_screen pushes the app into full_from_zoom when a zoomed window goes full screen. Closing the window (Cmd+W) while it is full screen never removes the entry; only the leaving branch does. When the app is reopened, entered full screen without zoom, then left, position() finds the stale entry. The window becomes maximized (area) instead of returning to w.restore. Repeated cycles add duplicates.
- **Fix:** Remove the app's entries from full_from_zoom in Desk::close and when w.full is cleared elsewhere. Or store a flag on Win instead of a side Vec.

### R4-7 P3 (medium): Every xdg configure raises and focuses that window

`springboard/src/bare/wayland.rs:1185`

- **Failure:** xdg_surface::Configure always pushes WindowEvent::Focus(id), and the shell handler (desk.rs:1265) does desk.raise(i) and sets screen=App(app). Any compositor configure on a non-front window (a tile placed on the back window, a resize or scale change, a maximize or fullscreen answer) lifts it over the window the user is working in and moves the screen focus. After Overview or a desktop switch, the order is right only because toplevels are recreated and configured in z-order.
- **Fix:** Send Focus only for the first configure of a new toplevel, or when the xdg_toplevel Activated state is set. Do not raise on size and state configures.

### R4-8 P3 (medium): Overview or the Apps grid can open on top of Search and the assistant, giving two exclusive-keyboard surfaces

`springboard/src/shell/overview.rs:1098`

- **Failure:** overview_toggle and appgrid_toggle hide only the Control Center. open_quicksearch_hw (quicksearch.rs:2460) closes neither Overview nor the grid, and hot corners can trigger either. Search (id 3), assistant (6), grid (8) and Overview (9) are all KeyboardInteractivity::Exclusive, so with two open the compositor chooses the key owner. Esc closes only Overview or the grid first, via overview_escape.
- **Fix:** Opening any of these dismisses the others (quicksearch.hide, assistant_hide, grid/overview close). Or make open_quicksearch_hw a no-op while overview_active().

### R4-9 P3 (medium): Overlay mask stride is derived from the clipped height, so a card that runs off the bottom gets a scrambled mask

`springboard/src/shell/desktop_bar.rs:1073`

- **Failure:** For ids 3 and 6, desk_overlay returns a mask of ow*oh from the unclipped size. desktop_chrome_frames then clips h to the screen and computes full = mask.len()/h, which is wrong whenever h was clipped (full != ow). Rows are mis-indexed, giving a sheared or empty rounded mask. This happens on a short screen or a tall assistant answer, since the assistant body is not capped like Search (max_bottom).
- **Fix:** Return the unclipped width with the rect, or clip the mask and the rect together (crop rows), and use the original ow as the stride.

## R5. App host, launch policy and permissions

The sandbox launch path (bwrap, cgroup, dropped identity, entitlements limited to first-party ids) is carefully built, but the consent layer around it is not: several powerful capabilities are self-grantable from a store manifest or exposed to every app through unguarded services, and the new root-run shell features (screenshot writer, downloaded-program launcher) trust user-writable paths. Four P0 issues were found, each confirmed by reading the code: the accessibility event service, the DocGrantPath home-file grant, the root screenshot write, and root execution of downloaded programs.

### R5-1 P0 (high): `accessibility` is self-grantable with no consent and lets any app click the permission sheets

`springboard/src/bare/app_policy.rs:229`

- **Failure:** A store app lists permissions = ["accessibility"]. from_permissions accepts it for any id. events::call (services/event_services.rs:358) only checks the raw manifest string, with no sheet and no Settings consent. The app then calls tap_create (listen tap on key events) and logs every keystroke typed into other apps. It also calls `post` with a pointer event at the centre of the Allow button of the permission sheet, Open Anyway, USB, screen-recording or notification sheet. Posted::Pointer goes to shell.event_untapped (bin/shell.rs:930) into the normal modal path, which acts on Event::Up. Every other consent in this area is bypassed.
- **Fix:** Treat `accessibility` like the other consented permissions: refuse it in from_permissions for non-first-party ids, or require a per-app grant (a sheet and a Settings switch) in events::call and tap_create. Also reject synthetic pointer or key events while any system card or sheet is up, and mark posted events so modal_event ignores them.

### R5-2 P0 (high): DocGrantPath hands any `documents` app a read descriptor for any file or folder under the home, with no user action

`springboard/src/shell/docpanel.rs:877`

- **Failure:** An app with only the self-grantable `documents` permission sends ToShell::DocGrantPath{path:"/home/u/.ssh/id_ed25519"}. drag_grant strips the home prefix and calls opener.open_any(rel, Read), which opens any regular file or, failing that, a directory. It records a persistent grant and returns the fd by SCM_RIGHTS. Nothing checks that a drag is in progress or that the user chose the path. A folder fd lets the app openat(fd, "..") up to the whole home, so the picker and file-portal model is defeated.
- **Fix:** Accept DocGrantPath only for the app that owns an active dnd session (or only for first-party Files) and only for paths in that session's item list. Refuse directories. Make the grant expire when the drag ends.

### R5-3 P0 (high): Root screenshot writer follows symlinks in a user-writable folder (arbitrary root file overwrite and chown)

`springboard/src/shell/screenshot.rs:73`

- **Failure:** The shell runs as root (springboard.service). save_capture spawns write_as(dir = the account's Desktop/Pictures, name = Screenshot-YYYYMMDD-HHMMSS.png, owner = account). write_as runs std::fs::write on `<name>.tmp`, then chown(tmp, uid, gid), then rename. Any process with the user's home (a sandboxed app with `storage`) can pre-create ~/Desktop/Screenshot-<next seconds>.tmp as a symlink to /etc/shadow, /etc/sudoers.d/x or another file. The next screenshot truncates the target with PNG bytes and chowns it to the user, so the user can then rewrite /etc/shadow. A dangling symlink creates files in arbitrary directories. Making Desktop itself a symlink also redirects the rename.
- **Fix:** Do the write as the account, not root. Use the Opener from bare/docbroker.rs (setfsuid plus openat2 with RESOLVE_BENEATH and O_NOFOLLOW) or open the tmp file with O_CREAT|O_EXCL|O_NOFOLLOW in a dir fd and fchown the fd. Never chown by path.

### R5-4 P0 (high): Downloaded programs opened from Files run as root, unsandboxed, in the shell's environment

`springboard/src/shell/launch_gate.rs:103`

- **Failure:** Files sends Request::Launch for any executable (files/mod.rs:963). launch_program calls hooks.run, which is run_program: Command::new(path).spawn() with no identity drop, no bwrap and no cgroup. Because springboard.service runs as root, a double-click on a downloaded file, whether Allow, Confirm then Open, or Open Anyway (and Anywhere makes this routine), executes attacker code with full root and the shell's environment. The child is never waited on, so zombies also accumulate.
- **Fix:** Launch through the same path as apps. Drop to the installed Account (setgroups, setgid, setuid) with no_new_privs and close_range, preferably inside the app_policy sandbox with storage only. Reap or double-fork the child.

### R5-5 P1 (high): app_firewall, wifi and bluetooth services are callable by every app with no permission or entitlement

`springboard/src/shell/services/app_firewall.rs:228`

- **Failure:** AppFirewall::call ignores `_app_id` and forwards to handle. Any sandboxed app, even without `network`, calls app_firewall.set{app_id, policy:"allow", ports:["tcp/22"]} or `block` on any port. The shell loads the result through root `nft -f -`, so it can open inbound ports for itself or block DNS, ssh or update traffic system-wide. wifi.call (services/wifi.rs:215) lets any app power the radio off, join an attacker SSID with a chosen PSK (rogue AP) or forget networks, and `networks` exposes nearby APs. bluetooth_service.rs:62 lets any app pair, connect or remove devices.
- **Fix:** Add an entitlement check (app_has_entitlement, first-party system-app only) at the top of each of these calls, or limit mutating methods to the Settings or System app ids. Reads like wifi.networks should sit behind the location consent.

### R5-6 P1 (medium): Shared-memory size check happens before the shrink seal, so an app can make the shell SIGBUS

`springboard/src/shell/apphost.rs:1112`

- **Failure:** For ToShell::Buffers (and window_buffers at 1307) the code does `shm_len(f) >= bytes && seal_shrink(f)`. The comment says seal first, but the length is read before F_ADD_SEALS(SHRINK). A malicious app runs a thread that ftruncates the memfd between 0 and the full size and sends Buffers repeatedly. When the truncate lands between lseek and fcntl, the file is sealed at the short size but still mapped for `bytes`. The first blit (copy_from_slice on pixels()) touches pages past EOF and the shell dies with SIGBUS, taking the root desktop session with it.
- **Fix:** Seal first (seal_shrink, plus F_SEAL_GROW if sizes are fixed), then read shm_len, then map. Re-check shm_len after mmap.

### R5-7 P2 (high): Permission and notification sheets act on a bare mouse-up, so an app can arrange a click-through grant

`springboard/src/shell/mod.rs:1983`

- **Failure:** The permission sheet (also usernotif.rs:686, the guardian sheet and others) handles only Event::Up and has no check that the matching Down happened on the button after the sheet appeared, and no minimum on-screen time. An app that gets a pointer Down (or a double-click) calls privacy.request{media:screen} or a notifications request. The sheet appears within a frame, and the user's Up lands on Allow if the app placed its own button where the centred card's Allow will be. Screen-recording, camera, contacts and similar grants are given without a deliberate click.
- **Fix:** Track the press: require Down and Up both inside the same button after the sheet was shown. Ignore pointer input for about 500 ms after a sheet appears. Cancel any button press that began before the sheet existed.

### R5-8 P2 (high): Every app receives every pasteboard change, and any app, in the background, can overwrite it

`springboard/src/shell/apphost.rs:1932`

- **Failure:** ext_sync_pasteboard sends the full pasteboard snapshot (all typed items) to every running host on each change, regardless of focus or permission. A background store app with no permissions therefore sees passwords copied from a password manager. SetClipboard and PasteboardSet (2294-2295) are accepted from any slot, foreground or not, so a background app can replace a copied address or command.
- **Fix:** Deliver the pasteboard only to the foreground app when it is activated or pastes (pull model), or gate background reads behind a `pasteboard` grant. Accept writes only from the focused app or after a user gesture. Skip items marked concealed.

### R5-9 P2 (high): `disks` service has no entitlement: any app reads volumes, labels and mount points and can claim disks forever

`springboard/src/shell/services/disks.rs:185`

- **Failure:** The new `volumes` call returns every disk and partition with fs type, label and mount point (for example /home/u or /media/<label>), exactly the information the sandbox hides (no /sys, no /proc/mounts). `list`, `describe` and the broadcast disk_appeared and description_changed events (app: None) leak it as well. Any app can also `claim` a disk, and claims are not released when the app exits, so Disk Utility's plan_unmount and plan_eject then fail with 'in use by <app>' until the shell restarts.
- **Fix:** Gate the service on a `disks` entitlement or on first-party system apps (Disk Utility), send events only to holders, and release claims in the app-gone path.

### R5-10 P3 (high): A second USB request is silently dropped and its device is never asked about again

`springboard/src/shell/services/usb_access.rs:656`

- **Failure:** request inserts (app, key) into ASKED, then queue_prompt de-duplicates on (app_id, kind) only. Kind::Usb equals Kind::Usb for different devices, and the queue also drops at 8 entries. If the app asks for device A and then, while A's sheet is queued or showing, device B, B is marked ASKED but no prompt is queued, and it returns asked:true. B can never be requested again until the app quits or the device is replugged.
- **Fix:** Make the de-dup key (app, kind, purpose). Insert into ASKED only after queue_prompt accepts the prompt (return a bool from queue_prompt).

### R5-11 P3 (medium): Rich notification requests have no rate limit and the held queues are unbounded

`springboard/src/shell/usernotif.rs:281`

- **Failure:** The legacy ToShell::Notify is limited to one per second (apphost.rs:2312). NotifyRequest, NotifySchedule and NotifyAuthorize have no limit. A permitted app floods banners and sounds, and `delivered` grows until a prune. While the first-time alert is unanswered (no timeout) hold, hold_schedule and hold_authorization push without bound, one up-to-packet-size message per request, which grows the root shell's memory.
- **Fix:** Cap held and delivered entries per app (about 64), reuse the one-per-second throttle for immediate deliveries, and drop the oldest held request first.

### R5-12 P3 (medium): Any app, in the background, can open web pages and bring the Browser forward

`springboard/src/shell/apphost.rs:2349`

- **Failure:** ToShell::OpenUrl is accepted from every slot with no foreground check or user-gesture requirement, no permission and no rate limit. desktop_open_url only checks the http or https scheme, then desk_open()s the Browser and sends the URL. A background app navigates the user to a phishing page and steals focus repeatedly.
- **Fix:** Accept OpenUrl only from the foreground app (as Share and ContextMenu are) and rate-limit it. Require a `network` or launch permission.

### R5-13 P3 (medium): Force Quit does not kill a hung native or window-server app, it only asks its windows to close

`springboard/src/shell/utility_window.rs:407`

- **Failure:** For a Foreign (Linux) row, or an installed app with no host of ours, utility_force_quit emits Effect::ForeignWindowAction(id,false) per window, which is a close request. A hung app ignores it and is never killed, and the row stays in the list. Only apps with an AppHost get SIGKILL (and AppHost::sigkill kills the whole id cgroup, so it does reach an SDK app's children).
- **Fix:** Resolve the toplevel's pid (xdg/foreign-toplevel pid or the cgroup/bwrap child for native apps) and SIGKILL it after a short grace period. Keep the window-close effect as the first step.

### R5-14 P3 (medium): Downloaded-program approval hashes the file at click time, not the file that was assessed

`springboard/src/shell/launch_alert.rs:138`

- **Failure:** The Confirm alert is raised for path P. On Open, identity_of(&alert.path) is recomputed and recorded with grant(), then the file is run. Between the alert and the click (unbounded), another app with `storage` can replace P. The approval is then remembered for, and run on, content the user was never shown. open_anyway (launch_gate.rs:163) has the same shape.
- **Fix:** Store the Identity computed when the alert was raised in the Alert and grant/run against it. Verify the hash again immediately before exec using an fd (fexecve or the /proc/self/fd path).

### R5-15 P3 (medium): The app registry never drops uninstalled packages and silently ignores apps beyond 200

`springboard/src/shell/apphost.rs:370`

- **Failure:** merge_installed starts from `old` and only replaces or appends scanned apps, and refuses to add past 200. An uninstalled app keeps its manifest in installed(), so app_has_permission and entitlement checks for that id still pass until restart (calendar change broadcast and similar). A 201st package is never listed or launchable, and nothing tells the user.
- **Fix:** Mark entries whose package disappeared as removed (keep the slot, drop permissions and make ext_open refuse). Raise the cap or report when it is hit.

## R6. Packaging, updates, recovery and system services

The btrfs snapshot and rollback design is well thought out (separate @home, @state and @recovery subvolumes, a pending marker, a heartbeat-gated 30 s probation, a pinned-inode check). Around it, though, trust is thin and several flows are fragile: the update channel and the store accept unsigned content and run it as root, a root daemon deletes through a user-controlled symlink, and the rollback generation can be overwritten or wedged by the recovery tooling itself. I read the code but ran no builds, tests or lab commands; every finding comes from reading it.

### R6-1 P0 (high): bifrostd low-space purge follows a user-controlled ~/.cache symlink and deletes as root

`springboard/bifrostd/src/purge.rs:144`

- **Failure:** plan() calls entries(&home.join(".cache")), which is read_dir and follows a symlink at .cache. The is_real_dir check only looks at the entries under it. A local user (or any process running as that uid) runs `ln -s / ~/.cache`. When free space drops under 10 GiB or 5 % (a user can fill /home to cause that) or Free Up Space is pressed, the 60 s tick lists /usr, /etc, /var and so on as AppCache candidates, largest first. execute() then runs remove_dir_all on them as root, so the OS is wiped. ~/.RecentlyDeleted (line 158) has the same pattern when the trash policy is on.
- **Fix:** Open each home and .cache with O_NOFOLLOW (or symlink_metadata and require a real directory on .cache itself). Check that the directory owner matches the home owner, and do the deletion relative to that directory fd with openat/unlinkat. Do the same for .RecentlyDeleted. Alternatively drop privileges to the home's owner before deleting.

### R6-2 P0 (high): Installed systems trust an unsigned package repo, and pc-publish never signs

`springboard/src/installer/plan.rs:914`

- **Failure:** The installer writes `[bifrost] SigLevel = Optional TrustAll` into the target's /etc/pacman.conf. The lab variant uses http://192.168.1.12:8079. tools/pc-publish.py builds with makepkg --skipinteg and runs repo-add with no signing, so nothing in the packages or the database is signed. bifrost-update-guard install then runs `pacman -Sy` and `-Su --noconfirm` as root. Anyone who can serve or alter the repo (compromised host, MITM on the lab http mirror, bad CDN cache) gets root plus a rewritten boot image on every installed machine. TLS is the only authentication for the public repo. The OTA path in shell/update.rs also states there is no client-side manifest signature.
- **Fix:** Create a release signing key. Have pc-publish run makepkg --sign and repo-add --sign. Ship the public key in the installed keyring (pacman-key --add and --lsign-key) and set `SigLevel = Required DatabaseRequired` for [bifrost]. Make the lab mirror use a lab key rather than TrustAll, or at least refuse http outside lab.token installs.

### R6-3 P1 (high): A store-installed service overrides a packaged system service by id and version, runs as root by default, and no signature is enforced

`springboard/bifrostd/src/lib.rs:158`

- **Failure:** scan_with() lets a store copy replace a system service whenever its manifest version is newer than the packaged one. Manifest.user defaults to root. Store validate() has no reserved-id check, so a catalogue entry (or a compromised catalogue) with id `is.olibuijr.update` or `is.olibuijr.net`, kind=service, version 9.9.9 and no `user` gets its binary exec'd as root by bifrostd. The only integrity check is the SHA-256 that comes from the same catalogue, because enforce_signature_policy returns Ok when /etc/bifrost/publisher-keys is empty, which is its state on every image (nothing in packaging pins a key). Any other store `service` package also runs as root unless it names a user.
- **Fix:** Never let the store copy win for an id that exists under /usr/lib/bifrost/services. Refuse such ids in install_archive/validate. Make store services default to `session` unless the package is in the system dir. Ship and pin a publisher key so unsigned packages are refused.

### R6-4 P1 (high): bifrost-mcp (remote root exec, file and shell tools) is enabled on every install and listens on 0.0.0.0 over plain HTTP

`springboard/mcp/src/serve.rs:81`

- **Failure:** springboard.install enables and restarts bifrost-mcp.service unconditionally. The unit has no User=, hardening or firewall hook, and the server binds 0.0.0.0:7420. Auth is a single static bearer token sent in clear text over HTTP. The tools include exec, file and term as root. Anyone on the same LAN or Wi-Fi who sniffs one request, or reads the token (a backup, a screenshot of a client config), takes over the machine. There is no rate limit on failed tokens, and it stays on for ordinary consumer installs. The unix socket is also root/0660 for the desktop account's group, so that account gets root through exec.
- **Fix:** Ship it disabled; let Settings (via privd) enable it explicitly. Default BIFROST_MCP_LISTEN to 127.0.0.1 or the lab NIC. Add TLS or require ssh tunnelling. Add hardening to the unit and failed-auth throttling, and rotate the token when re-enabled.

### R6-5 P1 (medium): record-modules can overwrite bifrost-previous.efi (the rollback generation) while the new boot is still on probation

`packaging/arch/recovery/system.rs:547`

- **Failure:** build_uki() copies the current bifrost.efi over bifrost-previous.efi whenever the rebuilt image differs. It never checks the update state. After an update the first boot is `system-probation`. bifrost-modules-record.service starts after desktop-ready, and on that boot `fw_changed` is typically true (the firmware list file is new or the kernel requests different names), so mkinitcpio and ukify rebuild the image. The previous image is replaced by the new, unaccepted kernel. If probation then fails (90 s timeout, crash), rollback.txt boots `bifrost-previous.efi` (now the new kernel) with the old @rollback root, so the old kernel is never restored. restore() also copies that image back to bifrost.efi.
- **Fix:** In record_modules/build_uki, do nothing to bifrost-previous.efi (and ideally skip the rebuild) while state is system-prepared, system-probation or system-restoring, or while ESP/pending exists. Rebuild after accept.

### R6-6 P2 (high): A failed or interrupted pacman -Su (even a plain download failure) forces an immediate hard reboot into recovery

`packaging/arch/recovery/guard.rs:305`

- **Failure:** In full (btrfs) mode, any non-zero exit of `pacman -Su ...` calls system::fallback_reset(). That writes pending=1, then runs systemctl reboot and, after 3 s, a raw reboot(RB_AUTOBOOT). A 404 for one dependency (pc-publish itself warns about mirror 404s), a dropped Wi-Fi link or a full cache disk has not touched the system yet. The user's machine reboots with no prompt, boots the fallback line (bifrost.recovery=1) and runs a full @system swap for nothing. Unsaved work is lost.
- **Fix:** Download first (`pacman -Sw --noconfirm` or `-Su --downloadonly`), and only call prepare and pacman -Su once all downloads succeeded. On failure before the transaction commits, just clear the prepared state (phase back to accepted or rolled-back, remove pending) and return the error. Reboot only if the transaction actually started.

### R6-7 P2 (medium): restore() swaps @system before steps that can fail, then loops every 5 s creating @failed-* subvolumes

`packaging/arch/recovery/system.rs:174`

- **Failure:** restore() renames @system to @failed-<ns> and renames @restored to @system, and only then copies the kernel files, the previous image and boot.cfg. kernel_files() chooses the name from what exists in /boot now. If an update introduced linux-rt (the 2026-10-12 migration from stock `linux`), ESP/previous holds only vmlinuz-linux, so the copy of vmlinuz-linux-rt fails with NotFound. watch() returns Err and the process exits 1. Restart=on-failure re-runs it every 5 s (state is system-restoring, fallback_boot is true). Each run snapshots @rollback again and renames the live @system to a new @failed-<nanos>, so snapshots accumulate without bound and the machine never reboots into a consistent state. Old @failed-* are also never cleaned after a normal rollback.
- **Fix:** Make restore idempotent and order it safely: copy and validate the ESP files first (take the kernel names from the saved previous-cmdline/previous dir, not from the current /boot). Do the subvolume swap last, skip it if @system is already the restored snapshot (track with a marker), and bound retries. Prune old @failed-* subvolumes.

### R6-8 P2 (medium): A prepared update that never reaches pacman leaves the state stuck and blocks all future updates

`packaging/arch/recovery/guard.rs:74`

- **Failure:** prepare() refuses to run when state is prepared, probation or restoring. install() leaves state=prepared if anything fails between prepare() and the pacman transaction (for example `systemctl start bifrost-update-guard.service` fails, the guard or shell is killed, or power is lost before pacman replaces /usr/bin/springboard). watch() in the `prepared` state only moves on when the inode of /usr/bin/springboard changed, so it polls forever (every 2 s) and never clears. Every later `install` then fails with 'A previous update has not finished probation', so the machine can never update again until someone edits /var/lib/bifrost-recovery/state by hand. The non-full install path has no timeout or reset.
- **Fix:** Treat `prepared` older than a bound (or whose installed version equals previous-version with no pacman lock) as abandoned and reset it in prepare(). Make install() reset the phase on every early error, and check the prepared timestamp.

### R6-9 P2 (medium): seed-data failure keeps the desktop from starting

`packaging/arch/seed-data:14`

- **Failure:** springboard.service has `ExecStartPre=/usr/lib/springboard/seed-data` with no `-`, and the script runs under `set -eu`. If /var/lib/springboard is full (ENOSPC on cp, the stamp write or `touch .seeded`), or a path under it is a directory where a file is expected (`rm -f` fails), the script exits non-zero. springboard.service fails every 2 s and no desktop ever appears, so the user cannot free space from the UI. The same exit happens for any one unreadable asset file. The stamp is only written after a full successful copy, so a partial copy repeats forever.
- **Fix:** Prefix the unit line with `-` (best-effort seeding) or make the script tolerate failures (`|| true`, log and exit 0) and keep going per file; never let asset seeding gate session start.

### R6-10 P2 (medium): The pager's swap-file creation can never succeed on the installer's btrfs layout and retries every tick under memory pressure

`springboard/bifrostd/src/pager.rs:231`

- **Failure:** Swap files are made in /var/lib/bifrost/swap, which lives in @system (compress=zstd:3, COW) per BTRFS_LAYOUT in installer/plan.rs. create_swap_file does fallocate -l 1G, chmod, mkswap and swapon without `chattr +C`. btrfs refuses swapon on compressed or COW files, so swapon fails. The pager removes the file and forgets it, and decide() has no back-off, so every 2 s tick under pressure fallocates and formats 1 GiB again, adding disk I/O exactly when the machine is already thrashing. It also never checks free disk space. If it did succeed, an active swapfile in @system would make `btrfs subvolume snapshot @system @rollback` in system::prepare fail, blocking updates.
- **Fix:** Use `btrfs filesystem mkswapfile` (or create empty, chattr +C, then fallocate) in a dedicated NOCOW subvolume outside the snapshotted @system, for example under /var/lib/springboard or its own @swap. Add an exponential back-off after a failure and a free-space check.

### R6-11 P2 (medium): bifrost-firmware initcpio hook prunes initramfs firmware to last boot's list, so renamed firmware after an update is missing on the probation boot

`packaging/arch/initcpio/install/bifrost-firmware:5`

- **Failure:** The list in /var/lib/bifrost-recovery/firmware is the names requested during the previous boot, taken from dmesg. A linux-firmware or kernel update in the same transaction as the UKI hook can change versioned names (for example i915 GuC/DMC *_ver*.bin). The initramfs mkinitcpio builds keeps only the old names, so the first boot after the update loads the early GPU driver without its firmware. The display and power management run degraded, or fail to come up. The first boot after the update is exactly the probation boot, so a healthy update can be rolled back, and the corrected list is only recorded after that boot.
- **Fix:** Do not prune with a stale list across package upgrades: keep the full firmware in the image for the first boot after a kernel or linux-firmware change, or add the new package's matching files by pattern (the same directory as each recorded file). Alternatively regenerate the list from modinfo -F firmware of the boot modules instead of dmesg.

### R6-12 P2 (medium): Shell-only (non-btrfs) rollback restores the pacman db entry but leaves the new package's other files, so later updates hit file conflicts

`packaging/arch/recovery/guard.rs:132`

- **Failure:** restore() copies back only the nine FILES and replaces /var/lib/pacman/local/springboard-shell-* with the old entry. Files new in the failed package (new /usr/bin tools, /usr/lib/bifrost/services/*, bifrost-wm, units) stay on disk but are no longer owned by any package in the restored db. The next `pacman -Su ... springboard-shell` carries a narrow --overwrite list, so it aborts with 'exists in filesystem' for those files and the guard restores again. Also the old shell then runs against the new bifrostd, wm, privd and service binaries (protocol skew). This is limited to `!full` installs (not the installer's btrfs layout).
- **Fix:** Snapshot the whole package file list from the old db entry and remove files that were added by the new package (diff the two `files` lists) during restore, or pass `--overwrite '*'` for owned-by-springboard-shell paths. Restore the remaining binaries too, or refuse the non-snapshot path for packages that add files.

### R6-13 P3 (medium): Package scriptlet restarts bifrost-privd, which would kill an update started through privd

`packaging/arch/springboard.install:45`

- **Failure:** privd's update.apply runs `bifrost-update-guard install` as a direct child (shell/pacman_update.rs run_guard), so it sits in bifrost-privd.service's cgroup. post_upgrade runs `systemctl restart bifrost-privd.service` mid-transaction. KillMode is the default control-group, so the restart kills privd, the guard and pacman together. That leaves db.lck behind and ESP/pending at 1, and the update never becomes staged. Today the shell runs as root and takes the local path, so it is latent, but it breaks as soon as the user-session stage (SPRINGBOARD_PRIVD, non-root shell) lands.
- **Fix:** Run the guard install in its own transient unit (`systemd-run --unit=bifrost-update` or a oneshot service, as rollback-start already does), and use try-restart with a delayed or after-the-transaction restart for privd.

### R6-14 P3 (medium): Screen Sharing (wayvnc) is a LAN-facing network service running as root

`packaging/arch/bifrost-wayvnc.service:20`

- **Failure:** The unit has no User=. wayvnc parses network input from any viewer on the local network as root, only partly mitigated by ProtectHome, PrivateTmp and NoNewPrivileges. A memory-safety bug in the pre-authentication RFB handshake gives root. The PAM stack authenticates a single account, which unix_chkpwd can verify for a non-root process.
- **Fix:** Run it as the session account (User= and the runtime dir already shared), keep PAM via unix_chkpwd, and add CapabilityBoundingSet=, ProtectSystem=strict and RestrictAddressFamilies= to the unit.

## R7. Audio server and media service

The audio server (audio/server, hal, shm, alsa) has a careful real-time design: no allocation or blocking lock on the IO thread, a retired-snapshot scheme, DSP mailbox, level tap and stop watchdog. It is weak on trust boundaries and lifecycle. Any local client can hang the SCHED_FIFO IO thread through the shared ring header, reroute the default devices, and exhaust pinned memory. Device removal is never acted on. The media service (audiod) works but holds its core lock across blocking audio-server calls, trusts client-supplied file paths, and wakes about 100 times a second while idle.

### R7-1 P1 (high): A client-writable ring header can spin the SCHED_FIFO IO thread forever

`springboard/audio/server/src/io.rs:674`

- **Failure:** The stale-quantum drain `while ring.readable() as usize >= 2 * q { ring.read(...) }` runs on the IO thread at priority 88. `readable()` is `write.wrapping_sub(read).min(cap)`, and both indices sit in the memfd the client maps read-write. A client that sets the output ring's `write` field to `read + 2^40` makes `readable()` return `cap` after every read, because each read advances `read` by only q. The loop never exits, there is no syscall in it, and the stop flag is never checked. The IO thread pins a core at RT priority 88. The unit has `LimitRTTIME=infinity`, so nothing kills it. Every client of that device goes silent. `stop_device` times out and parks the handle in `stopping`, so the device can never be reopened until the service is restarted. The socket is 0666, so any local process or sandboxed app can do this.
- **Fix:** Bound the drain. Compute `excess = readable - q` once and skip at most that many frames, or cap the loop at `cap / q` iterations. If the raw `write - read` exceeds `cap`, treat the header as corrupt: mark the slot misbehaving, reset both indices server-side, and drop or deny the client. Audit the other loops that read client-writable indices for the same unbounded pattern.

### R7-2 P2 (high): Any client can set the default output, input and sound-effects devices to arbitrary ids

`springboard/audio/server/src/apps.rs:465`

- **Failure:** In `AppState::set`, only Process-class objects check `policy` or ownership. Every other class goes straight to `self.tree.set(...)`, with no policy test and no check that the value is a device. The System object's `DEFAULT_OUTPUT`, `DEFAULT_INPUT` and `DEFAULT_SOUND_EFFECTS` are marked settable in `props.rs`. Any unprivileged Hello'd client (policy=false, uid anything, socket 0666) can send `Set{System, DEFAULT_OUTPUT, Object(ObjectId(0))}` or `Object(999999)`, or point the input default at the loopback device. The `pinned` flag then freezes the choice for good. `default_device()` in the HAL returns `UnknownObject` or a dead id. The music player, system sounds and the recorder all fail with no sound until the daemon restarts. `control.rs:310` computes `policy` and passes it, but the non-process branch ignores it.
- **Fix:** Require `policy` (or uid 0/server uid) for any Set on the System object. Validate that the value is an existing Device in `self.devices` with channels in the matching direction. Do not pin until the set succeeds.

### R7-3 P2 (high): Device removal is ignored: defaults stay on an unplugged device and the IO thread keeps cycling

`springboard/audio/server/src/control.rs:674`

- **Failure:** `tick()` handles only `DriverEvent::DeviceAdded`. `DeviceRemoved`, `JackChanged`, `RateChanged` and `ControlChanged` fall into `other => log(...)`. The ALSA driver does emit `DeviceRemoved` after its rescan, and `AlsaIo::recover` sets `gone` on ENODEV. The server never sets `IS_ALIVE` false, never stops the device, never notifies listeners, and never re-runs `choose_defaults`. `choose_defaults` runs only from `add_device`, in `apps.rs:271`. Walk through unplugging a USB DAC that is the default: the IO thread keeps running at RT priority and returns an Xrun every cycle, which raises the quantum through the fallback. Client rings keep draining, so the media player believes it is playing. The default stays on the dead device, so output is silent. A newly plugged headset is not preferred either, because equal-rank devices pick the lowest index. If the user had pinned the default, it never moves.
- **Fix:** On `DeviceRemoved`, stop the host, set `IS_ALIVE=false`, fail or notify its slots, and recompute any unpinned or dangling default. Clear the pin when its device disappears. Consider preferring a newly added USB or Bluetooth output in `pick_default`.

### R7-4 P2 (high): Idle CPU: the media service wakes about 100 times a second with nothing playing, and the audio server wakes once a second

`springboard/src/media/audioplayer.rs:618`

- **Failure:** The player engine thread sleeps 10 ms and takes the shared mutex in a loop whenever it is paused, has no decoder, or has drained (lines 618, 631, 637). The initial state, with playing=false and paused=false, flips to paused at once, so the loop runs for the whole life of the service. The shell subscribes at boot, so the service stays resident. On top of that, the audiod ticker (`audiod/src/main.rs:237`) wakes at 5 Hz and clones a snapshot while any client is subscribed. Each `SpeakerOut` has a 1 Hz idle thread (`speaker.rs`). `serve()` in the audio server polls with `HOUSEKEEPING_MS=1000`, and each wake runs `tick()` and an ALSA card rescan. That contradicts the module doc, which says the loop blocks with no wakeups when idle. Together this is roughly 100 or more wakeups a second at idle, which defeats deep C-states.
- **Fix:** Park the engine thread on a Condvar, signalled by `restart`, `play`, `pause`, `quit` and the rate and volume setters, instead of sleeping 10 ms. Park the ticker on the Condvar while no song plays and not recording. For the server, compute the next deadline (hot-plug scan interval only when needed, or an inotify/udev fd for /dev/snd) and block on poll with no timeout when idle.

### R7-5 P2 (medium): Sustained-xrun fallback raises the quantum above a client's buffer size and permanently breaks that client

`springboard/audio/server/src/io.rs:894`

- **Failure:** The fallback (`FallbackPolicy`, ceiling 512) triggers on any 8 device xruns or late clients in a 1 s window. It sets `fallback_floor` and `quantum` to 256x2=512. The server then expects q=512 frames per cycle. Each client's trigger interval is `every = ceil(buffer_frames/q).max(1)`, which is 1 for a 256-frame client, and `shm::reset` puts q into the activation record. The HAL IO thread ignores the activation's frames. It always runs `cycle_once` with its fixed `IoArgs.frames` (256), so it writes 256 frames per cycle while the server needs 512 readable. Every other cycle is counted late and skipped. The player's ring drains at half speed, so music stutters at half rate until the stream is closed. The floor lasts until the device stops after the 2 s idle timeout. Only 'late client' needs to trigger it, so a single stalled client also raises latency for everyone, with no way back down while any stream runs.
- **Fix:** Make the client follow the activation's frame count, or make the server's `every` and the ring requirement use `max(q, buffer_frames)` consistently. Do not count a lone late client as a device overload. Let the floor decay when xruns stop.

### R7-6 P2 (medium): World-connectable socket with unbounded slots, connections and PromoteThread gives any local process RT priority and pinned-memory exhaustion

`springboard/audio/server/src/control.rs:879`

- **Failure:** The socket is mode 0666 and Hello takes any name. `PromoteThread` (line 419) only checks that `/proc/<peer pid>/task/<tid>` exists, then grants SCHED_FIFO 80 with util_min 1024. The daemon holds CAP_SYS_NICE and `LimitRTPRIO=99`. Any unprivileged process or sandboxed app can therefore get real-time scheduling for as many threads as it likes. `add_io_proc` (line 444) has no cap on slots per client or in total, and nothing enforces the `MAX_CLIENTS=64` per-device limit. `io.rs:512` silently ignores slot 65 and above, so those clients are never triggered. Each slot is an 8 MB-class memfd, with 32 channels at 8192 buffer frames and ring_frames x4. `mlockall(MCL_CURRENT|MCL_FUTURE)` plus `LimitMEMLOCK=infinity` means those pages are locked. The number of connections is also unbounded. One local client can pin all RAM or use up the descriptors. Hello's app name is client-claimed, so a process can claim another app's id and share its session through `declared` and the arbiter, which are keyed by app name. The name is also logged unsanitised.
- **Fix:** Cap slots per client (for example 8) and per device (64, refused at AddIoProc). Cap connections. Require the peer uid to match the session user, or use a group-restricted socket. Accept PromoteThread only for a hardened set of clients, or ratelimit and cap RT threads per client. Escape the Hello name when logging, and bind the app id to the peer's executable or cgroup instead of trusting the string.

### R7-7 P2 (medium): Media service holds its core mutex across blocking audio-server round trips

`springboard/audiod/src/main.rs:185`

- **Failure:** `serve()` takes `lock(&core)` and keeps it while `apply()` runs. For `RecordStart`, `Recorder::start()` (line 146) calls `System::connect` (a blocking connect with a 5 s Hello wait), then `default_device`, `add_io_proc` and `start`. Each can wait 5 s, and `start` waits up to 10 s for the thread-ready handshake. `RecordStop` joins the capture thread and does two requests in `halt()`. Replies to subscribers are also written under the lock (500 ms write timeout), and so are the ticker's `send_all` broadcasts. While the audio server is wedged or slow, one Voice Memos record click freezes every client of the service: transport, volume, state updates and the ticker. A slow subscriber stalls everyone for up to 500 ms.
- **Fix:** Do the connect and device work on a worker thread without the core lock, then publish the Recorder into `Core` when it is ready. Never write to client sockets while holding the lock: take a snapshot of the subscriber list, or give each client an outbound queue.

### R7-8 P2 (medium): Media service executes client-supplied paths without validating them (arbitrary file write, microphone use)

`springboard/audiod/src/main.rs:159`

- **Failure:** The path checks (absolute, .wav) exist only in the SDK client (`sdk/src/system_audio.rs`). The service accepts any `Cmd`. `Cmd::RecordStart{path}` starts mic capture with no permission or session check. The service connects as a plain client, so the audio server's per-app policy never sees the real caller. `RecordStop` then does `std::fs::write(&path, wav)` on any path. A hostile or buggy connected process can overwrite any file the service user can write with a WAV payload, record the microphone, and make the service decode arbitrary local files (`track_in` -> `Track::file`). Subscribers also receive other apps' track paths and URLs. The socket is 0600, but every app of the user can reach it unless the sandbox hides it. That is unconfirmed here, because the sandbox policy is outside this area.
- **Fix:** Validate on the service side: absolute path under the caller's data directory, `.wav`, create_new or O_NOFOLLOW, and refuse relative paths. Gate RecordStart on the same microphone permission prompt as the audio server. Authenticate the caller by SO_PEERCRED and the app sandbox identity.

### R7-9 P3 (medium): Device start reports success even when the IO thread fails to start the hardware

`springboard/audio/server/src/io.rs:846`

- **Failure:** `start_device` spawns the IO thread and marks the device `running=true`. `runner.start()` (the ALSA prepare, prefill and start) happens later, inside the spawned thread. If it fails, the thread logs 'start failed', sets the stop flag and exits. The control thread has already replied Done to StartIoProc and set IS_RUNNING. Later StartIoProc calls return Ok at once (`running` is still true). Clients write into a ring nobody drains until the speaker's 3 s write stall releases the stream. This repeats until the idle timer finally calls `stop_device`.
- **Fix:** Start the hardware before spawning the thread, or hand the start result back over a channel so `start_device` returns the error and does not set `running`.

### R7-10 P3 (medium): Recording is lost if the WAV write fails, and RecordStop can end someone else's take

`springboard/audiod/src/main.rs:159`

- **Failure:** `RecordStop` calls `r.stop()`, which consumes and moves out the whole take, then `fs::write`. On ENOSPC, EACCES or a missing directory, the samples are dropped and only an error event goes out. The file is written non-atomically, so a crash leaves a truncated WAV. Samples also grow in memory without bound while recording (about 345 MB per hour at 48 kHz mono). Ownership tracking is per connection: if connection A starts, B sends RecordStop and then B starts a new take, A's disconnect handler (`recording` still true) stops B's recording.
- **Fix:** Write to a temp file and rename. Keep the take available for retry or emit the error before discarding. Stream to disk or cap duration. Make the recording owner a connection id stored in Core, and stop only if it still matches.

### R7-11 P3 (medium): HAL request/reply pairing can attribute a late reply to the next request

`springboard/audio/hal/src/conn.rs:203`

- **Failure:** `request_within` drains stale replies (`while rx.try_recv().is_ok()`) before sending, then waits with a timeout. If request A times out (5 s, or 100 ms for workers) and its reply arrives after the drain but before request B's own reply, B receives A's reply. For example, a late `Reply::Done` for Remove makes a following `Start` or `Get` return the wrong Reply variant (Protocol or a wrong Value). Requests carry no sequence id. The drain only helps when the late reply lands before the next request is sent.
- **Fix:** Add a request id to the control protocol and match it, or reconnect (mark the connection dead) after a timeout.

## R8. Mail app and network code

The network layer is mostly sound. TLS uses the webpki roots with no bypass, STARTTLS failure is a hard error with no plaintext fallback, IMAP literals are capped, autoconfig is https-only, DNS and MIME slicing are on char boundaries, and the entities table is sorted. Real defects remain in the places that parse hostile input (HTML tag parser, nested MIME recursion), in the trust model of autoconfig redirects, in header and SMTP data hygiene (Subject injection, bare CR), and in the IDLE watcher lifecycle (25 s read timeout, no cancellation). All findings come from reading the code; nothing was run, as instructed.

### R8-1 P1 (high): HTML tag parser is quadratic: a hostile message freezes the UI thread

`springboard/apps/mail-desktop/src/html.rs:250`

- **Failure:** parse_tag consumes every following token as an attribute until it finds '>' and returns None at end of input (line 270). sanitize() and to_text() then emit '<' and resume at lt+1, so the next '<' rescans the whole remainder. An HTML part such as '<a <a <a <a ...' with no '>' makes n starts each scan O(n) tokens and allocate one String per attribute. 100 KB takes seconds, 1 MB effectively never finishes (the IMAP literal cap is 64 MB). It runs on the UI thread: body::readable -> to_text from draw.rs ensure_laid (~1125) and window.rs viewer_snapshot (~1384), and sanitize from draw.rs:981. Opening, or merely selecting, the message hangs Mail.
- **Fix:** Bound parse_tag: give up at the first '<' inside an unquoted attribute, or cap a tag at about 4 KB and fall back to treating '<' as text without re-scanning. Add a total input cap for to_text and sanitize. Run body decoding off the UI thread.

### R8-2 P1 (medium): Unbounded recursion on nested multipart: stack overflow and O(n*depth) hang

`springboard/apps/mail-desktop/src/body.rs:90`

- **Failure:** text_part (body.rs:90), collect_attachments (body.rs:40) and mime.rs strip_entity (mime.rs:63) recurse once per nested multipart/* part with no depth limit. A message with tens of thousands of nested 'Content-Type: multipart/mixed; boundary=bN' parts (a few MB, well under the 64 MB literal cap) overflows the stack, which aborts the process. Before that, each level's split_parts scans to the closing delimiter at the end of the message, which is O(n*depth). body_arrived (state.rs:1178) calls attachments() on the UI thread as soon as the body is fetched, so selecting the message crashes Mail.
- **Fix:** Pass a depth counter and treat anything deeper than about 12 levels as a non-text part. Cap the total number of parts per message.

### R8-3 P2 (high): Autoconfig result is trusted unconditionally, including after a cross-domain https redirect

`springboard/apps/mail-desktop/src/discover.rs:105`

- **Failure:** Servers from the autoconfig file are marked trusted: true without checking in_domain(host, domain). HttpFetch (line 166) uses require_https, which only forbids http: it follows a 30x from autoconfig.example.net to https://anywhere.example.org/config.xml. That file's hostname is accepted as trusted, so window.rs:2557 (discovered) submits the form at once and the account password goes to the named server with no confirmation. The doc comment promises an off-domain server is shown for confirmation, but that only holds for the SRV path. Any wildcard or hijacked autoconfig subdomain, or a redirect on a shared host, can capture credentials.
- **Fix:** Take the trust decision for autoconfig servers with the same in_domain test as SRV. Off-domain hosts must go through the confirm path, except those from an explicit provider allowlist. Also make the fetch client refuse redirects that leave the domain.

### R8-4 P2 (high): Header injection through Subject when a non-ASCII word follows a newline

`springboard/apps/mail-desktop/src/message.rs:576`

- **Failure:** encode_subject copies subject[..at] raw when the first non-ASCII word is not the first word. Subject 'x\nBcc: evil@example.com y é' gives at>0, and the prefix 'x\nBcc: evil@example.com y ' is written into the header block unchanged. encode_word only encodes the remainder (the all-ASCII path is safe, since encode_word checks for CR/LF). Form::type_text (state.rs:261) does no filtering, and mailto subject= is percent-decoded (%0A) in compose_handoff.rs, so a web page or another app can pre-fill the Subject of a compose sheet. Draft::render emits the string at message.rs:240, so the injected line adds headers such as Bcc or a spoofed Reply-To. Reply and Forward subjects come from the received header and are exposed the same way if they hold a bare CR.
- **Fix:** In render (or encode_subject), replace every CR and LF (and other control characters) in the subject with a space before encoding. Apply the same to In-Reply-To and References tokens.

### R8-5 P2 (high): IMAP IDLE can never idle: the 25 s socket read timeout kills it, so each watcher reconnects about every 85 s

`springboard/apps/mail-desktop/src/session.rs:201`

- **Failure:** Tls::connect sets read and write timeouts of 25 s (http.rs:166, READ_TIMEOUT). idle() blocks in Lines::read_line on that socket, and servers send keep-alives only every few minutes (RFC 2177 allows 29 min). After 25 s of silence read returns TimedOut, idle() returns Err, and watch_inbox returns WatchEnd::Retry. The watcher then sleeps 60 s (sync.rs WATCH_RETRY) and does a full TLS handshake, LOGIN, SELECT and CAPABILITY. New mail waits up to 60 s deaf, and each account makes about 1000 logins a day, which can trip server rate limits or fail2ban. Activity-log lines ('waiting' / 'stopped waiting') flap with the same period. The connection is dropped without DONE or LOGOUT.
- **Fix:** For IDLE, treat TimedOut/WouldBlock as 'no news': keep reading until about 25 min, then send DONE, a NOOP and IDLE again. Or raise the read timeout on the watcher connection. Do not sleep 60 s after a clean timeout.

### R8-6 P2 (high): Inbox watcher thread cannot be stopped: Remove Account and Account Settings leave it running with the old credentials

`springboard/apps/mail-desktop/src/sync.rs:322`

- **Failure:** watch() loops forever (Retry means sleep 60 s and reconnect). Only the server ending the dialogue gives Stop. submit_remove_account (window.rs ~2721) just removes the id from self.watching, but the thread keeps the cloned password and server and keeps logging in to a removed account's server and sending events for an unknown account. Re-adding the same account starts a second watcher. After Account Settings changes the password or server, replace_account does not clear watching, so the old watcher fails login (Stop) and the id stays in watching: no watcher is started for the new credentials until restart. Each watcher also occupies a serial dispatch thread.
- **Fix:** Give watch() an Arc<AtomicBool> cancel flag (or a channel) stored per account. Set it on remove and on edit, and have the loop check it before each reconnect. Restart the watcher with the new credentials after an edit.

### R8-7 P2 (medium): Passwords items keyed by (IMAP host, user) are shared: removing or editing one account deletes or overwrites another's

`springboard/apps/mail-desktop/src/window.rs:2718`

- **Failure:** The key is Account::password_site(), the IMAP host (or the DAV host for other kinds), plus the username. Two accounts with the same host and login (an alias, a second identity, or a CalDAV/CardDAV account on the same host and user) share one item. Remove Account (2718) and the move branch of account_saved (1819) call passwords::remove without checking other accounts, so the surviving account later reports 'No password is saved ... Add the account again'. In Pending::Add (~726), DuplicateItem silently falls through to passwords::update, so adding an account overwrites an existing, possibly different, saved password for that host and user with no prompt.
- **Fix:** Only remove the item when no other account in the shared list uses the same site and username. On DuplicateItem for a brand-new account, ask before replacing. Better, include the account id in the key.

### R8-8 P3 (medium): Security::Plain connects and logs in in cleartext with no guard

`springboard/apps/mail-desktop/src/session.rs:280`

- **Failure:** The shared account list accepts security = plain (ui/src/internet_accounts.rs:131, documented as 'only for a trusted local network'). session::connect, submit::greet and transport::open treat Plain as: connect without TLS, then LOGIN or AUTH PLAIN with the password. Nothing checks that the host is on a private network or asks the user, and the background check and the IDLE watcher use the same path. An account file edited by hand, or written by another app or sync, silently makes Mail send the password over the internet in the clear. The Mail UI never creates Plain, and autoconfig and SRV discovery never produce it.
- **Fix:** Refuse Plain unless the resolved address is loopback, private or link-local, or require an explicit per-account confirmation. Never send credentials before the TLS upgrade completes.

### R8-9 P3 (medium): SMTP data payload keeps bare CR, which allows SMTP smuggling against lenient servers

`springboard/apps/mail-desktop/src/smtp.rs:84`

- **Failure:** data_payload and message.rs crlf_lines (302) split only on '\n' and strip a trailing '\r', so a lone CR inside a line survives. Body text reaches the draft from received mail: forward_draft pastes readable(raw) unprefixed, and IMAP literals can contain any bytes. A received body 'x\r.\r\nMAIL FROM:<a@victim>...' is forwarded as the bytes '\r.\r\n'. A submission server that accepts <CR>.<CR><LF> as end of data (older Postfix, Exim and Sendmail; also nonconforming MTAs) ends the message early and runs the rest as commands in the user's authenticated session, sending mail as the user.
- **Fix:** In data_payload (and crlf_lines) normalise all CR and LF sequences first: replace lone CR or LF with CRLF (export::crlf already does this) before dot-stuffing.

### R8-10 P3 (medium): 25 s read timeout after the DATA terminator turns slow acceptance into duplicate sends

`springboard/apps/mail-desktop/src/submit.rs:64`

- **Failure:** After the closing '.', converse() waits for 250 under the transport's 25 s read timeout. A server that scans a large attachment or greylists for longer returns TimedOut. The message has been accepted but is reported as failed, so the Send Later and undo-send queue retries it every 300 s up to 5 times (schedule.rs MAX_ATTEMPTS), and the recipient gets duplicates. A restart while a message is in flight also re-sends it (the documented trade-off).
- **Fix:** Use a longer timeout (several minutes) after DATA, or treat a timeout at that point as 'unknown' and do not auto-retry. Reject oversize messages before DATA using the SIZE extension.

### R8-11 P3 (medium): Memory use is unbounded in practice: 64 cached bodies of up to 64 MB each, about 6 copies per fetch, uncapped export

`springboard/apps/mail-desktop/src/state.rs:31`

- **Failure:** MAX_CACHED_BODIES is 64 and each raw body can be up to 64 MB (MAX_LITERAL), plus a decoded copy of every attachment (body_arrived -> attachments()). A single FETCH copies the data bytes -> String -> quoted literal -> line -> Value clone -> text(), so about 400 MB is transient for one big message. sync::fetch_all (sync.rs:279) keeps up to 5000 whole messages in memory for Export Mailbox with no total cap.
- **Fix:** Cap the cache by bytes, not count. Stream attachments to disk instead of keeping them decoded. Stream export to the output file. Avoid the extra copies in literal folding.

### R8-12 P3 (medium): Remote-image count sanitises the entire raw RFC 822 text on the UI thread

`springboard/apps/mail-desktop/src/draw.rs:981`

- **Failure:** crate::html::sanitize(raw, false).remote_images_blocked is run on the whole raw message (headers, MIME boundaries, base64 attachments) once per opened message, on the draw thread. It does not decode quoted-printable or base64 first, so for the usual encodings of HTML mail the count is 0 and the 'Remote images are blocked' note never appears. The reader draws plain text only, so Load Remote Content has no effect. It also feeds large inputs to the quadratic parser (see the first finding).
- **Fix:** Count images in the decoded HTML part only (body::text_part). Memoise it off the UI thread, or drop the banner if the reader never loads remote content.

### R8-13 P3 (medium): New-mail baseline ignores UIDVALIDITY and never starts on an empty inbox

`springboard/apps/mail-desktop/src/background.rs:74`

- **Failure:** Baseline::apply keeps the highest UID per account. If the server changes UIDVALIDITY (mailbox rebuilt or migrated), UIDs restart low and every later message has uid <= baseline, so new mail is never announced. If the inbox is empty on the first check, no baseline is stored (listed.iter().max() is None). The first message that then arrives is taken as the 'first listing' and silently becomes the baseline, so it raises no notification.
- **Fix:** Record UIDVALIDITY from SELECT with the baseline and reset when it changes. Store baseline 0 for an empty inbox.

### R8-14 P3 (medium): DNS client: queries go to public resolvers as a fallback; one failure disables all later lookups

`springboard/apps/mail-desktop/src/dns.rs:183`

- **Failure:** Resolver::system() appends the shell's public resolvers after resolv.conf, so the user's mail domain (and SRV/MX/A names) is sent to a third party when the local resolver fails. lookup() sets the permanent 'unreachable' flag when every server returns SERVFAIL, REFUSED or a bad reply, not only on timeouts. A resolver that refuses SRV or MX queries therefore makes the later MX and A checks fail fast for the rest of the process. Transaction ids are derived from the clock, pid and a counter (about 16 bits of guessable state), and truncated (TC) replies are not retried over TCP.
- **Fix:** Use only configured resolvers unless the user opts in. Set 'unreachable' only on timeouts. Use getrandom for ids and handle TC.

### R8-15 P3 (medium): Damaged outbox file is silently read as empty and then overwritten; duplicate attachment names can overwrite each other

`springboard/apps/mail-desktop/src/schedule.rs:218`

- **Failure:** schedule::load maps any read or parse failure to Queue::default(), and the next save_queue_file writes the empty queue over the file, so queued Send Later mail and reminders are lost without a warning. Separately, attachments::save_all renames a duplicate to '{index+1}-{name}' without re-checking it against already-used names. Files [a, 3-a, a] write the third file over '3-a'.
- **Fix:** Keep the unreadable file (rename to .bad) and show a status line. In save_all, loop until the candidate name is unused.

## R9. SDK runtime, widgets and rendering

The SDK runtime and widget code is mostly sound. I found no panic on zero-size layout, no integer overflow on large surfaces, no unbounded emoji or text cache, and BIFROST_TEST_PIN only reads an env var. The window-open ordering fix in `draw_windows` is correct, and gfx.rs and motion.rs are unchanged since main. The real defects are protocol-level silent truncation of large clipboard payloads, a partial-redraw bug that drops window materials, the shell pushing clipboard contents to every running app, and several per-frame or per-turn allocation hot spots.

### R9-1 P1 (medium): Clipboard contents are pushed to every running app, with no concealed or transient exemption

`springboard/src/shell/apphost.rs:1921`

- **Failure:** A password manager or any app copies a secret. `ext_sync_pasteboard` hands the pasteboard to every running app, including background and third-party apps that run under their own accounts, as `ToApp::Pasteboard` plus `ToApp::Clipboard`. The SDK runtime applies `ToApp::Clipboard` unconditionally (`sdk/src/runtime.rs`, the `ToApp::Clipboard` arm). No concealed or transient pasteboard type exists anywhere in `pasteboard*.rs` or `apphost.rs`, so any app that merely runs sees the secret and can read it with `Ctx::pasteboard_string`.
- **Fix:** Deliver the pasteboard to an app only when it is focused, or when it asks, through a `PasteboardRead` request. Also add a concealed or transient item type that the sync skips, and set it in the passwords and keychain apps.

### R9-2 P2 (medium): Clipboard and pasteboard payloads over about 16 KB are silently truncated or dropped

`springboard/proto/src/lib.rs:1156`

- **Failure:** MAX_PACKET is 16 KB. `str_` cuts `ToShell::SetClipboard` and `ToApp::Clipboard` text at 16 KB minus 64 bytes with no signal, so copying a 100 KB document in TextEdit and pasting it in another app yields the first 16 KB. `items_` (line 1139) drops any pasteboard item above about 12 KB whole. A PNG copied from Preview or Photobooth therefore never reaches the other app. The runtime flush (`sdk/src/runtime.rs`, the `SetClipboard` send near line 1068) ignores the loss. I did not find a separate fd-based path for large payloads.
- **Fix:** Carry large pasteboard items by memfd or fd (as documents are), or chunk them. At minimum, refuse the copy and tell the user instead of sending a shortened value under the full type.

### R9-3 P2 (medium): A partial redraw replaces the whole material list with only the regions it redrew

`springboard/sdk/src/runtime.rs:1025`

- **Failure:** A view-based app with damage tracking (`draw_dirty`) redraws only the sidebar view after a row click. That frame declares only the sidebar material, so `drawn` is non-empty and `materials = drawn` runs. The runtime then sends `ToShell::Materials([sidebar])`, which the shell treats as the full replacement list (`apphost.rs:1264`). The toolbar and title-bar band materials are dropped. The app's tint pixels stay translucent, but the shell stops blurring those regions until the next full redraw. The existing guard only covers the case where a frame declares no materials. Materials that extra windows declare in `draw_window` are pushed to the same thread-local list but are never sent, because `Materials` carries no window id.
- **Fix:** Merge by region, as `glass::merge` does. Keep stored materials that lie outside this frame's damage and replace only those inside it. Alternatively, keep the last full-frame list until the next full redraw. Give extra windows their own window-scoped Materials message.

### R9-4 P3 (high): Every received packet triggers the full accessibility, menu and clipboard publish, and the clipboard text is cloned each turn

`springboard/sdk/src/runtime.rs:1043`

- **Failure:** `busy` is set to true for every packet, including each coalesced pointer Move. On each such turn the loop rebuilds and compares `app.controls`, `merge_controls`, `controls_tree`, `ax_text`, `ax_meta`, `menus_for`, `commands` and `undo_titles`, each of which allocates. At line 1068 it also calls `kit::clipboard::text()`, which clones the whole clipboard String (up to 16 KB) on every loop turn just to compare it with `clip_seen`. An app with a large tree, such as a file list or Mail, does O(tree) allocation per pointer event.
- **Fix:** Set `busy` only when an event can change published state, for example when the handler returns true or an app model flagged dirty. Compare a clipboard change counter, or a length plus hash, instead of cloning the text.

### R9-5 P3 (high): `cached_text` clones the full pixel buffer on every cache hit and on every insert

`springboard/render/src/ui.rs:378`

- **Failure:** `cached_text` returns `(*hit).clone()` of an `Arc<Image>`, which copies the whole Vec<u32>. On a miss it also does `img.clone()` into the Arc. Widgets that redraw every frame (Control Center, Settings rows, the desktop) allocate and memcpy one image per string per frame. It also builds a `TextKey` with `text.to_string()` on every lookup.
- **Fix:** Return `Arc<Image>`, or a borrowed `ImageRef` that `blit_colored` can take, from the cache. Look up with a borrowed key, or hash the text first.

### R9-6 P3 (high): The emoji pre-scan ignores `has_glyph`, so ordinary UI symbols load a 10.7 MB private copy of the emoji font

`springboard/render/src/emoji.rs:365`

- **Failure:** The cheap pre-scan is `text.chars().any(is_emoji_char && > 0x7F) || !available()`. `is_emoji_char` covers ⌥ ⌫ ⌦ ⏎ ™ ✓ and similar, which the UI font already draws. The first menu or label containing one of them makes every process (the shell and each app) call `faces()`, which runs `std::fs::read` on `/usr/share/fonts/noto/NotoColorEmoji.ttf` (10.7 MB here) into a private Vec held for the life of the process, on the UI thread. System fonts use the shared mmap (`fontprefs::map_font`) instead. The `NotoColorEmoji` path search itself is cached after the first call, so it does not hit the disk every frame.
- **Fix:** Run the pre-scan with `!has_glyph(c)`. Load the font with mmap, or `Arc<[u8]>` over a mapped file, so processes share the page cache.

### R9-7 P3 (medium): AlertCard can grow taller than the screen and does not break long words

`springboard/render/src/ui.rs:590`

- **Failure:** The title and message are wrapped with `Ui::wrap`, which never breaks a single long word. The card height is `title_height + 17pt * message.len()` plus the button row, with no cap. Strings that come from outside, such as USB product strings, crash reasons or app names, can be very long or contain a long unbroken token. The result is a card taller than the screen, centred with a negative y. Its buttons (Allow or Don't Allow) are then off-screen and unclickable, and an over-wide word is clipped. This matters most for permission and USB prompts.
- **Fix:** Truncate each wrapped line with `ui.fit`, and break words longer than the line width. Cap the title and message at a maximum number of lines with an ellipsis, so the card stays within the screen.

### R9-8 P3 (medium): The traffic-light inset is applied to any toolbar that overlaps the band, regardless of window style or form factor, and the overflow button can leave the bar

`springboard/sdk/src/widgets.rs:452`

- **Failure:** `clear_of_traffic_lights` shifts any bar with x < ~95 pt that overlaps y 19..33 pt. It does not check that the window is full-size-content or that the lights exist. A bar at x=0 in a classic-titlebar window, a sheet or a panel, or on the tablet form factor (`bifrost_ui::form_factor`; I found no check in this code), loses ~95 pt for no reason. A toolbar drawn in pane coordinates through plain `toolbar()` instead of `toolbar_in` is also shifted. When the shrunken bounds are narrower than the overflow width, `nav.rs:340` places the `»` button at `bounds.right() - overflow_w`, which is left of `bounds.x` and back over the lights.
- **Fix:** Apply the inset only when the window is full-size-content and the form factor draws lights, for example through a flag in `Ctx` or `layout`. Clamp the overflow button's x to `bounds.x`.

### R9-9 P3 (medium): BIFROST_TEST_PIN is honoured in production builds and re-read from the environment on every call

`springboard/core/src/pin.rs:13`

- **Failure:** `pinned()` is compiled in all builds and calls `std::env::var` (a lock plus String allocation) on each use, including the shell clock and every `testing::unix_secs`, `system_time` and `seed` call. If `BIFROST_TEST_PIN=1` leaks into a login session or a launcher environment, the shell clock freezes at 2025-10-10, apps run offline (maps, stocks and weather use `Offline`), and `seed()` becomes constant. The constant seed is a weak-randomness hazard for any app that uses `testing::seed()` for non-test purposes.
- **Fix:** Gate the pin behind `cfg(any(test, feature = "testkit"))` or a debug-only build flag, or require a second harness-only variable. Cache the result in a `OnceLock` where the process never changes it.

## R10. Settings and the system apps changed this week

The parsers I read are mostly defensive: IPP decode, DMI/cpuinfo/SMBIOS, GPX, links, weather JSON and the journal export all use bounds-checked reads, and Prefs writes are atomic and fsynced. The real defects are blocking I/O on UI threads (Print & Scan edits, Disk Utility's df/smartctl), a crafted-font memory blow-up in Font Book's cmap reader, a Maps tile queue that grows duplicates and refetches loaded tiles, and several unbounded or never-pruned resources (tile cache, CUPS replies, stuck scanimage). I found no platform-name violations in user-visible strings in this area. I read only part of settings.rs and pages.rs, so the rest of those files is not covered.

### R10-1 P2 (high): Print & Scan edits run blocking IPP on the shell UI thread

`springboard/src/shell/settings.rs:3091`

- **Failure:** Action::PrintScan calls print_scan_page::apply (settings.rs:3091) and the address sheet calls print_scan_page::finish (settings.rs:2107) synchronously inside activate/touch. apply does Client::local().set_default; finish does cups::add_device, which makes about 4 IPP exchanges (default, printers, classes, then Add-Modify-Printer with ppd-name=everywhere). cupsd probes the printer for that last one, and cups.rs:22 allows a 10 s read timeout per read, so a slow printer or a cold socket-activated cupsd freezes the whole shell for 10 s or more. A Tcp CUPS_SERVER also uses TcpStream::connect with no connect timeout (cups.rs:156). When the read times out the UI shows an error even though cupsd may still create the queue, so a retry makes a '-2' duplicate. Unlike View::load, neither call is wrapped in catch_unwind or run through Loaded::start.
- **Fix:** Run apply and finish on a worker (Loaded::start or spawn_named) and show a pending state. Use a longer or no read timeout for Add-Modify-Printer and re-list afterwards instead of reporting a timeout as failure. Use connect_timeout for Tcp endpoints.

### R10-2 P2 (high): Font Book cmap format-4 reader can allocate gigabytes from a ~260 KB font

`springboard/apps/fontbook-desktop/src/validate.rs:226`

- **Failure:** format4 loops start..=end for each of up to 32767 segments and pushes one (c,c) tuple per covered code point. A crafted font with 32767 segments of start=0, end=0xFFFF, delta=1, idRangeOffset=0 is only about 260 KB. It makes roughly 2 billion pushes (about 17 GB) before normalize runs, so the process is OOM-killed or hangs. validate::check runs on the UI thread when a family is selected (main.rs:358), on Validate File, and on install preview, so a hostile font already in ~/.local/share/fonts crashes Font Book every time it is selected.
- **Fix:** Emit one (start,end) range per segment when idRangeOffset is 0 and delta maps to nonzero glyphs. For idRangeOffset != 0, collect runs and cap total work, for example stop after 0x110000 code points or a fixed iteration budget. Run validation off the UI thread.

### R10-3 P2 (high): Maps TileLoader queues duplicate tiles every tick and refetches already-loaded tiles

`springboard/src/maptiles/loader.rs:164`

- **Failure:** window.rs:1282 calls loader.want(&wanted) on every tick. want() does queue.push_front(t) for every uncached tile that is not pending or failed, without checking whether it is already queued. With MAX_IN_FLIGHT=2 and a 500 ms request interval, the queue gains about 10 duplicates per tick while a view loads, and more while panning. dispatch() (line 176) skips only pending tiles, and the fresh-skip applies only when job_total > 0. After a tile has loaded, its stale duplicates are popped and fetched again, because fetch_tile never checks the cache. The app keeps sending already-cached tiles to the public tile server at 2 req/s for a long time after the view settles, against the tile usage policy it is meant to honour, and queue memory grows.
- **Fix:** Skip a tile in want() when it is already in the queue, or keep a HashSet of queued ids. In dispatch() and fetch_tile, skip tiles that are loaded and fresh. Drop queued tiles that are no longer wanted when the view moves.

### R10-4 P2 (medium): Disk Utility runs df, smartctl and lsblk synchronously on the app tick

`springboard/apps/diskutil-desktop/src/window.rs:197`

- **Failure:** tick() calls refresh() every 2 s (main.rs:181), which runs probe::read_df every 10 s (window.rs:197-199). probe_smart (window.rs:263) runs smartctl -H on selection, and Ask::Info runs lsblk (window.rs:361). All use Command::output() with no timeout on the app's main thread. A mounted removable or failing disk whose statfs stalls (the process goes into D state), or a USB bridge where smartctl takes tens of seconds, freezes the whole Disk Utility window, including the Unmount and Eject buttons that would help.
- **Fix:** Run the probes on a utility queue and deliver results through a channel, as actions.rs::spawn does. Add a kill-after-timeout, for example spawn plus wait_timeout, and treat a timeout as 'Not available'.

### R10-5 P3 (high): Maps tile cache is never pruned

`springboard/src/maptiles/cache.rs:41`

- **Failure:** TileCache::write adds files under tiles/<host>/<z>/<x>/<y>.png. MAX_TILE_AGE only triggers a refetch that overwrites the file. Nothing deletes tiles except the explicit offline-area delete (areas.rs:96). Browsing the map at all zoom levels for months, including satellite imagery, grows ~/.local/share/is.olibuijr.maps.desktop without bound. Several map types and overlays each get their own folder.
- **Fix:** Add a size cap with LRU eviction by mtime. Prune tiles older than N days (not belonging to a saved offline area) at startup or after writes.

### R10-6 P3 (high): CUPS transport reads an unbounded reply with no overall deadline

`springboard/ui/src/cups.rs:279`

- **Failure:** HttpTransport::send does stream.read_to_end(&mut raw) with no size cap. The 10 s timeout applies to each read, so a CUPS_SERVER pointing at a remote or hostile host can send one byte every 9 s forever, or an endless body that fills memory. This blocks the Settings worker, the Print Center serial queue, or the UI thread (see the first finding). The TcpStream::connect at line 156 has no connect timeout. Remote CUPS_SERVER traffic is also cleartext HTTP, with no authentication.
- **Fix:** Read with a cap (for example 8 MiB) through Read::take. Add an overall deadline. Use connect_timeout. Prefer ignoring non-local CUPS_SERVER values, or document them.

### R10-7 P3 (medium): scanimage -L is abandoned on timeout, not killed, and Print & Scan reloads often

`springboard/src/shell/print_scan_page.rs:72`

- **Failure:** bounded() spawns a thread that runs scanner::list(), which uses Command::output() with no timeout (ui/src/scanner.rs:196), and gives up after 5 s. The thread and the scanimage child stay alive until it exits. A hung network scanner discovery (airscan/SANE) leaves one extra scanimage process and thread for every View::load. That includes every page open, every default-printer change and every add (settings.rs:3092, 2108).
- **Fix:** Spawn the child with spawn() and kill it on timeout, or share one in-flight scan and skip starting another while one is running.

### R10-8 P3 (medium): Print Center lists every 1-5 s regardless of requests in flight

`springboard/apps/printers-desktop/src/window.rs:493`

- **Failure:** step() calls self.list() whenever t >= next_poll, without checking in_flight. Each List is 3 IPP exchanges plus a jobs query on a serial queue (worker.rs:49), each up to 10 s if cupsd is unresponsive. With an active job the poll is 1 s (model.rs:680). A stalled scheduler lets requests pile up in the queue and delay user actions such as Cancel Job, and replies arrive in bursts long after they were requested.
- **Fix:** Skip the poll while a List is in flight, or coalesce pending List requests to one.

### R10-9 P3 (medium): Add Printer mDNS browse trusts and loops over unauthenticated datagrams

`springboard/apps/printers-desktop/src/discover.rs:147`

- **Failure:** poll() reads with while let Ok(..) = socket.recv_from on a non-blocking UDP socket bound to 0.0.0.0, in the app's tick. It does not check that the source is link-local, and it has no per-poll limit. A host flooding the ephemeral port can keep tick in this loop and grow the ServiceBrowser instance list. found_from builds device_uri from the advertised SRV host and the TXT 'rp' value (discover.rs:60-72) without sanitising. The text can contain '?', '#', spaces or a remote host, and that URI is handed to Add-Modify-Printer after only a scheme-prefix check (valid_device_uri). The user does pick the row, but the details line shows the same data.
- **Fix:** Cap datagrams per poll, for example 64. Accept only link-local and private source addresses. Validate host and rp (reject control characters, spaces, '?' and '#'). Cap the number of instances.

### R10-10 P3 (medium): Wireless Diagnostics spawns iw and repaints the whole window every second, even when hidden

`springboard/apps/wireless-diagnostics-desktop/src/source.rs:81`

- **Failure:** spawn_poller reschedules itself forever, with no pause for a hidden or occluded window. Each reading is pushed through the main queue, so drain_readings always returns true when it takes a reading (state.rs:113-120). tick then reports changed and draw() damages the full surface (main.rs draw: d.all()). That is one fork/exec of iw plus a full repaint per second for the app's whole lifetime. run_iw has no timeout, so a hung iw also stops all later polls.
- **Fix:** Stop or slow the poller while the window is not visible. Return changed only when the reading differs. Add a timeout and kill to run_iw.

### R10-11 P3 (medium): Stickies board load treats read errors as an empty board and a later save overwrites the file

`springboard/apps/stickies-desktop/src/store.rs:21`

- **Failure:** load() does let Ok(text) = fs::read_to_string(..) else { return Board::default() }. A file that is not valid UTF-8, or a transient EIO or EACCES, is treated as 'no file' and is not moved to stickies.json.bad. The next save() then overwrites the user's notes. The .bad rename (line 26) also replaces any earlier .bad. In model.rs:1199 the run lengths 'n' are summed as usize with no overflow guard, so a hand-edited or damaged file can panic in debug builds or wrap in release builds.
- **Fix:** On any read or decode error other than NotFound, move the file aside with a unique name before starting empty. Use checked or saturating sums for run counts.

### R10-12 P3 (medium): Image Convert runs qemu-img on an untrusted source with auto-detected format

`springboard/apps/diskutil-desktop/src/images.rs:144`

- **Failure:** convert_command passes only -O <fmt> src dst, with no -f. qemu-img probes the source format, so a file that looks like raw but is a qcow2 or vmdk with a backing-file path, or a raw image whose header looks like another format, makes the conversion read arbitrary host files (such as the user's own files outside the granted file) into the output image. This crosses the open-panel grant that the app otherwise keeps by passing paths from /proc/self/fd.
- **Fix:** Pass -f <format> chosen from the file extension or the user's choice (raw when unknown), and consider --image-opts with backing-file=off or using -U carefully.

### R10-13 P3 (medium): Console shows executable and hash for whatever process now owns an old entry's PID

`springboard/apps/console-desktop/src/identity.rs:55`

- **Failure:** Journal::detail calls detail_for_pid(e.pid) for any selected entry. The entry's _PID is often long dead and recycled. The pane then reports 'Executable', 'SHA-256' and requirement results for an unrelated process in a pane framed as code identity. The Coredumps detail has the same issue with coredumpctl info <pid>, where the pid may match a different dump.
- **Fix:** Compare the process start time (/proc/<pid>/stat starttime) with the entry date, and show nothing if the process started after the entry. For core dumps, match on the dump's own time or id.

### R10-14 P3 (medium): Capture Archive can deadlock on a full stderr pipe and cannot be cancelled

`springboard/apps/console-desktop/src/capture.rs:50`

- **Failure:** run() pipes the child's stderr but reads it only in wait_with_output after stdout reaches EOF. If journalctl writes more than the pipe buffer (64 KB) to stderr while still writing stdout, both block forever. The Capture has no cancel path, and the save sheet stays pending until it ends.
- **Fix:** Drain stderr on a thread or redirect it to a bounded buffer. Add a cancel flag that kills the child.

### R10-15 P3 (medium): Maps HTTP client follows https to http redirects and shares one cookie jar across all hosts

`springboard/apps/maps-desktop/src/net.rs:1`

- **Failure:** maptiles::net::Live builds Client::new(Tls, Default::default()) without require_https(), so https_only is false (deildu/http.rs:497). A tile, geocoder or routing server on https can redirect to plain http, so a network attacker can substitute tiles or search results, and the shared cookie jar sends cookies across the services. Custom http:// servers are allowed, but the default OSM and imagery sources are not.
- **Fix:** Use require_https() when the configured source is https, so a redirect to http is refused. Use a jar per host, or no jar, for tile traffic.
