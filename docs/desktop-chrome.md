# Desktop chrome

The menu bar, Dock and panels are drawn by the shell (shell 0.7.25: `src/shell/desktop_bar.rs`, `desktop_dock.rs`,
`dock_stack.rs`, `desktop_metrics.rs`). A second implementation runs as separate layer-shell clients of `bifrost-wm`
(`springboard/chrome`, binaries `menubar`, `dock`, `panels`; the shell side is `src/shell/chrome_host/`). The shell
keeps the models and serves them over `$XDG_RUNTIME_DIR/bifrost-chrome.sock`. The clients start only with
`BIFROST_CHROME=1` (set through `packaging/arch/wayland/session-policy`; safe mode never starts them). Design and status:
`docs/graphics-architecture.md` and `docs/graphics/g2-chrome-*.md`.

## Menu bar

Network and power icons, with the CPU and RAM readings beside them by default. The readings come from cheap counter
reads (`bifrost_core::host`) taken on the shell's main loop, with no sampler thread. The interval is 5 s idle and 1 s
while a CPU or RAM menu is open (Activity Monitor "Very often"), with 10 % tolerance so the wake-up can coalesce with
another. Nothing is read while the screen is off or locked. CPU uses successive /proc/stat counters (guest counters
are not counted twice); RAM is MemTotal minus MemAvailable. The dropdowns show processor count, load averages,
uptime, available/installed/cache memory and swap.

The File menu can maximize or restore the active SDK or native window. XDG configure state updates the SDK window
model, including changes the compositor makes. The green SDK control toggles maximize and restore and shows its state.

## Dock

- Right-click a dock icon to open or show, quit, or pin and unpin it. Drag an icon onto another to reorder pins.
- Right-click the dock, or use BifrOSt > Dock Settings, for Bottom, Left or Right placement, icon size (32 to 80),
  magnification and its size (48 to 128), and autohide. A hidden dock reveals at the chosen screen edge and hides
  after the pointer leaves.
- Choices persist in `desktop-dock.ini` (`position`, `size`, `autohide`, `dock_magnification`,
  `dock_magnification_size`, `pinned`), written atomically, with app identities as keys. Running unpinned apps stay visible.
- The Downloads stack sits left of the Trash: the newest files of the account's Downloads folder as a fanned stack; a
  click opens the folder in Files. The folder is listed on a timer, never per frame.
- Applications lists shell apps next to Linux apps, so removing a pin does not remove the application.

BifrOSt Browser is the HTTP and HTTPS default. A new setup installs it from the PC App Store; normal system packages
carry the launcher and association, and browser updates stay independent. Files uses the SDK context-menu API.

## CLI

`bifrost pc midget` (`tools/midget.py`): `context X Y`, `hover X Y`, `open-app ID`, `open-url URL`, `open-folder ID PATH`,
`show-in-files PATH`, `state`. Store apps open by their installed slot, and a window closes when its app exits. The state
output includes dock choices and visibility, the current menu and the CPU and memory percentages.
