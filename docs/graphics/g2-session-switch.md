# G2: session switch (`session-switch`)

Status: partial. The packaging, the session script and its pure policy are done and tested on the host. The Settings
switch, the shell-side chrome start and the Midget run are not done (see "Left").

## What is implemented

- **Packaging** (`packaging/arch/PKGBUILD`): `build()` builds `bifrost-wm` (`-p bifrost-wm --bin bifrost-wm`) and the
  three chrome binaries (`-p bifrost-chrome --bins`: `bifrost-menubar`, `bifrost-dock`, `bifrost-panels`) with
  `--locked`. `package()` installs them into `/usr/bin`. No Cargo.toml or Cargo.lock change.
  The shaders are not installed as files: they are compiled into the binary (`springboard/gpu`).
- **Session policy** (`packaging/arch/wayland/session-policy`, new, installed to `/usr/lib/bifrost/session-policy`
  by `packaging/arch/package-files.sh`). Two pure bash functions with no side effects:
  - `session_compositor_flags <compositor> <renderer> <chrome>` prints the environment words for the new stack:
    `BIFROST_DIRECT=1` whenever `bifrost-wm` is selected, `BIFROST_WM_RENDERER=vulkan` when the renderer is `vulkan`,
    `BIFROST_CHROME=1` when the chrome switch is `1`. For labwc it prints nothing.
  - `wm_exit_action <now> <exits_file>` records one unplanned `bifrost-wm` exit, drops exits older than 60 s, and
    prints `fallback` when three exits remain, otherwise `restart`.
- **Session script** (`packaging/arch/wayland/session`):
  - Reads `/etc/bifrost/compositor` (as before), `/etc/bifrost/renderer` and `/etc/bifrost/chrome` (new, plain words).
  - Falls back to labwc when `/run/bifrost/compositor-fallback` exists (set for the rest of the boot).
  - Passes the flags to `bifrost-wm` and to the shell.
  - With `chrome = 1`, starts `bifrost-menubar`, `bifrost-dock` and `bifrost-panels` when they are installed.
  - When `bifrost-wm` exits on its own, `wm_exit_action` counts it in `/run/bifrost/wm-exits`. At three exits within
    60 s the script creates the fallback marker and exits with status 1, so systemd (`Restart=on-failure`) restarts
    the session, which now starts labwc. The counter and the marker live under `/run`, so they survive session
    restarts and clear at boot.
- **Tests** (`tools/test_session_policy.py`, 13 tests, all passing with `python3 -I tools/test_session_policy.py`):
  flag assembly for each switch combination, and the 60 s window at the boundary (exactly 60 s old is dropped), the
  three-exit threshold, spread-out exits, and garbage lines.
- `bash -n` passes for `session` and `session-policy`. `python3 tools/check-names.py` reports no hits.

## How it maps to the contract

- `docs/graphics-architecture.md`: the switch is still behind `/etc/bifrost/compositor = bifrost-wm`, with
  `BIFROST_WM_RENDERER=vulkan`, `BIFROST_DIRECT=1` and `BIFROST_CHROME=1` as the environment names. The old path
  (labwc, GLES) is unchanged when the switch is off.
- `docs/window-server.md` phase 5 ("Switch"): the session script starts `bifrost-wm` with labwc as the fallback.

## Deviations and decisions

- The chrome clients are started by the session script, not by the shell. The shell's `chrome_host` module is
  owned by `chrome-core`, and it does not spawn clients yet. Moving the start into the shell is a follow-up.
- `/etc/bifrost/chrome` is a new file, my choice for the chrome switch. The contract does not name a file for it.
- `BIFROST_DIRECT` is set whenever `bifrost-wm` is selected, so the shell and the apps use the direct path whenever the
  window server is the new one.

## Left (todo_left)

1. Settings > Developer "New Window Server" switch (`springboard/src/shell/pages.rs`, `settings.rs`). It must write
   `/etc/bifrost/compositor`, `/etc/bifrost/renderer = vulkan` and `/etc/bifrost/chrome` through a privileged helper
   (bifrost-privd or a root action), since the shell cannot write `/etc` itself. The switch takes effect after logout.
   Not done in this round.
2. The shell does not hide its own menu bar, Dock and panels when `BIFROST_CHROME=1`. It draws them as well, which
   double-draws once the chrome clients run.
3. Midget acceptance: the Vulkan server has not been run end to end, so the desktop, a direct app, glass, shadows and
   `bench --midget` are unverified.
4. The `session` script has not been run under systemd. The crash-loop path (three exits, fallback, session restart)
   is covered by the pure-policy tests only.
5. `springboard/assets` is a git-ignored symlink in this worktree. Packaging copies it from the checkout.

## Verify on Midget

Midget is `ssh midget` (192.168.20.140). The Intel HD 530 runs Mesa ANV, Vulkan 1.4.

1. Confirm the baseline: `bifrost pc midget state` shows the desktop running labwc. `cat /etc/bifrost/compositor`
   prints nothing or `labwc`.
2. Deploy the build: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
   `python3 tools/midget.py recovery` must say `phase=accepted` before the next step.
3. Select the new stack (as root on Midget):
   `printf 'bifrost-wm\n' > /etc/bifrost/compositor; printf 'vulkan\n' > /etc/bifrost/renderer; printf '1\n' > /etc/bifrost/chrome`,
   then log out, or `systemctl restart springboard`.
4. Check the session: `ls /run/bifrost/`, `cat /run/bifrost/compositor-fallback` (must be absent),
   `pgrep -a bifrost-wm`, `pgrep -a bifrost-menubar`. Take a screenshot with `bifrost pc midget screen /tmp/g2.png`.
5. Crash-loop test: `pkill -9 bifrost-wm` three times within 60 s (restart the session between kills if needed). After
   the third, `/run/bifrost/compositor-fallback` exists, and `pgrep labwc` must find labwc.
6. Clean up: remove `/etc/bifrost/compositor` and `/etc/bifrost/renderer` (or set them to labwc and gles) and reboot,
   or `rm -f /run/bifrost/compositor-fallback /run/bifrost/wm-exits`, to return to labwc. The GLES and labwc paths must
   still start with the switch off.
