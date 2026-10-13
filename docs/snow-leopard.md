# Snow Leopard round: system resources first

Mac OS X 10.6 (2009) shipped "0 new features": it made the system smaller, quieter and faster. BifrOSt adopts the
same rules. A Snow Leopard round ships only resource, speed and correctness work, and every claim is a measured
before and after on Midget.

## Principles (Apple's, applied here)

| Snow Leopard | BifrOSt |
|---|---|
| Idle means idle: launchd on demand, no polling | No polling loops or periodic timers at idle; block on fds; no frames when nothing changed |
| Grand Central Dispatch: one thread pool per machine | `bifrost_app::dispatch` queues on one per-process pool sized to the cores; main-queue work wakes the app loop |
| Purgeable memory, NSCache | `bifrost_app::Cache` evicts on memory pressure (PSI) |
| Sudden termination | Clean apps are killed at once on quit, logout, shutdown and OTA restart; apps with unsaved work get a real deadline |
| 7 GB freed: one architecture, compressed, on demand | Release-profile and dependency pass; dead tablet code removed where touched |
| Measured speedups in the release notes | Budgets in `docs/budgets.toml`, enforced by acceptance; numbers in `docs/perf-history.jsonl` |

## Baseline (2026-10-10, Midget, 0.5.7, idle desktop, 30 s)

- RAM used 779 MiB of 15.5 GiB. Shell 130 MiB PSS, 16 threads; labwc 55 MiB PSS, 8 threads. `/usr/bin/springboard` 13 MiB.
- CPU (percent of one core): springboard 0.60, labwc 0.07, dbus-broker 0.07.
- Wakeups per second (context switches): bifrost-bootweave 51.3, dbus-broker 25.8, springboard 8.6, labwc 2.1,
  i915 flip worker 3.6 (frames presented at idle).
- Kernel: `CONFIG_HZ=1000`, `NO_HZ_FULL=y`, `threadirqs`; per-core ktimers about 28/s, ksoftirqd about 50/s.

## Findings

1. `bifrost-bootweave` polls its child every 20 ms for the whole session (`springboard/bootweave/supervisor.rs`): 50 wakeups/s.
2. D-Bus about 26 wakeups/s at idle. Measured cause (busctl monitor): the shell's `gather()` ran `timedatectl show`
   twice per call (`net/ntp.rs` `status_text`), a new system-bus connection each time (~3/s), plus
   systemd-timedated GetAll. Fixed in 0.5.8 (cached 15 s). The "Round 1 findings" guess below is superseded.
3. The shell presents about 3.6 frames a second on an unchanged desktop.
4. Apps start raw threads (87 `thread::spawn` sites) and poll results with `next_wake`.
5. Quit waits 20 ms, then kills (`src/shell/apphost.rs` `kill`): too short for a save, too long for a clean app.

### Round 1 findings (non-shell agent)

- bootweave: fixed. The main thread now blocks in `child.wait()`; a watcher thread polls for `wayland-0` and
  `shell-ready` every 20 ms only until the first frame, then exits. Idle: 0 wakeups/s (was 50).
- D-Bus 26 wakeups/s: Midget was unreachable from the agent (no route / key), so the sender could not be measured with
  `busctl --system monitor`. Code audit: no non-shell binary talks to the system bus. The only system-bus clients in
  this repo are the shell's `src/shell/desktop_network.rs` (spawns `dbus-monitor --system` with five match rules:
  NetworkManager `PropertiesChanged` under Devices, ActiveConnection and AccessPoint, plus PowerProfiles) and
  `src/shell/atspi/bridge.rs`. Likely cause: NetworkManager emits `AccessPoint` / Device `PropertiesChanged`
  (signal strength, `LastScan`, bandwidth) continuously while Wi-Fi is up, and `dbus-monitor` is itself a broker
  eavesdropper (each message is copied to it). Fix for the shell agent: drop the `AccessPoint` rule (strength jitter),
  match `ActiveConnection` and Device `State`/`ActiveConnection` only (arg0 filter on the NM Device interface), and
  prefer an in-process zbus subscription over a `dbus-monitor` child. Verify with `busctl --system monitor` on Midget.
- netd and audiod tickers (250 ms / 200 ms) now park on a `Condvar` until a subscriber connects: 0 wakeups with no
  clients (was 4 + 5/s each). updated ticks every 15 s unless Checking or Downloading (was 1/s). bifrostd supervisor
  ticks every 2 s (was 500 ms). bifrost-portal blocks on inotify of the runtime dir (was 2/s).

## Results

| Key (Midget, idle 30 s) | 0.5.7 | 0.5.8 |
|---|---|---|
| Userspace wakeups/s (all threads) | 179.3 | 82.8 |
| springboard wakeups/s (all threads) | 79.3 | 71.5 |
| bifrost-bootweave wakeups/s | 49.6 | 0 |
| dbus-broker wakeups/s | 25.5 | 2.2 |
| Total idle CPU % (one core) | 1.66-1.89 | 0.86 |
| springboard CPU % | 0.53 | 0.40 |
| Shell PSS / threads | 130 MiB / 16 | 130 MiB / 16 |

Note: the first baseline's 8.6 wakeups/s for springboard counted the main thread only; the tools now sum all threads.

## Footprint (round 2, measured on Midget 2560x1440, 0.5.8)

`smaps_rollup`: Rss 151 MiB, Pss 130 MiB (Pss_Anon 100, Pss_File 29). By mapping (PSS, MiB):

| Mapping | MiB | What |
|---|---|---|
| `[heap]` | 79 | Rust allocations: full-screen images below, glyph/icon caches |
| `[anon]` | 17 | one 14 MiB region (Surface or GL staging) plus thread stacks |
| libgallium + libLLVM | 17 | Mesa shader compiler loaded by the GL presenter (file-backed, shared) |
| `/usr/bin/springboard` | 8 | the binary (already `strip = true`) |
| wallpaper.raw + SFUI.ttf | 5 | mmapped stills and the font (once) |

Full-screen (14 MiB at 2560x1440) heap images the shell held for a desktop panel: `fit_home` (still scaled to cover),
`home_dim` (70 % copy for the lock screen), `rot.wallpaper` (a 1:1 `cover()` copy of an already panel-sized still),
and the Today page frost (`Frost::new` of the whole panel). That is about 56 MiB for pictures that are almost never
on screen on a desktop. The GPU presenter uploads `fit_home` to a texture once (`set_wallpaper`).

Changes: `rotate::refresh_wallpaper` no longer copies a still that already has the frame's size (-14 MiB);
on a desktop `set_wallpaper` no longer builds `home_dim` or the Today frost, `Shell::lazy_wallpapers` builds them
when the lock screen or Today is in view and frees `home_dim` after (expected -28 MiB idle, rebuilt in one frame
on lock). Not changed: `fit_home` (the presenter's source), `lock_dim`/`fit_lock` (only when a lock wallpaper is set).

Release profile: `strip = true`, `panic = "unwind"`, LTO and one codegen unit are set. Under `panic = "abort"` the
`catch_unwind` calls in `sdk/src/dispatch.rs`, `render/src/gfx/pool.rs` and `src/deildu/jobs.rs` caught nothing and any
panic killed the shell; unwind makes a panic end only the thread or job that hit it (the crash-log hook still writes
first). Accepted cost: a few percent of binary size. `opt-level = "s"` was not applied: the software raster paths are hot.

## Round 2 plan (next)

1. `recovery-chord` thread (`src/bare/chord_recovery.rs`): 64.8 of the shell's 71.5 wakeups/s. It sleeps 200 ms
   and re-lists and ioctls every input device each time. Make it event-driven: poll the evdev fds (and inotify on
   /dev/input for hot-plug), only time the 3 s hold once Ctrl+Alt+Backspace is down.
2. Remaining shell threads (5.5, 3.1, 1.8, 1.8, 1.1/s): name the unnamed `springboard` threads, then fix each.
3. Dispatch: cap the pool's total width at the core count across QoS lanes (today each lane may use all cores).
4. Purgeable `bifrost_app::Cache` evicted on memory pressure (PSI); glyph, icon and image caches move to it.
5. Move apps from `thread::spawn` + polling to dispatch (Podcasts, Books first; Haiku agents).
6. Footprint: shell PSS 130 MiB (find the big allocations), release profile, duplicate crates (mostly the browser
   engine: html5ever, cssparser, phf, digest).
7. Idle frames: i915 flip workers ~4.5/s; read `frame_reason()` with `bifrost pc midget trace on`.
8. Re-record the host bench baseline (`tools/bench.py`; the pre-round host record was lost, not committed).

## Round 2 results (0.5.9, clean reboot)

| Key | 0.5.8 | 0.5.9 |
|---|---|---|
| springboard wakeups/s | 71.5 | 6.4 |
| Userspace wakeups/s | 83 | 35 |
| Idle CPU % | 0.76 | 0.43 |
| Shell PSS | 130 MiB | 112 MiB |
| dbus-broker wakeups/s | 2.2 | 13.5 (over budget, round 3) |

## Round 3 plan

1. dbus-broker 13.5 wakeups/s with 1-3 msgs/s on the bus: find the waker (shell `dbus-monitor` child, broker audit).
2. Shell PSS 112 MiB: heap profile; the 14 MiB anon region; Mesa 17 MiB.
3. Testing kit: pin clock and random seed so every app snapshot is deterministic.
4. Gate: icon size and format check.

## Round 3 results (0.5.10, 0.5.11)

| Item | Result |
|---|---|
| 1. dbus-broker waker | Found: the shell spawned `timedatectl` twice every 15 s (bus connect plus timedated start, ~25 broker wakeups per burst). Now read from `/run/systemd/timesync/synchronized` and the unit links. The bench's own ssh login also woke the broker; the sampler now settles 10 s first. |
| 2. Shell PSS | `bifrost_core::host::pressure_relief()` (`malloc_zone_pressure_relief` parity) when the shell goes idle after drawing: heap trimmed. |
| 3. Testing kit pin | `testing::pin`, `pinned`, `unix_secs`, `system_time`, `seed`; Books, Console, Games, Weather gated again. |
| 4. Gate icon check | APP-10: icon must be a 180x180 PNG. |

Midget, clean reboot then unlocked, 0.5.11:

| Metric | 0.5.9 | 0.5.11 |
|---|---|---|
| userspace wakeups/s | 35 | 8.6 |
| dbus-broker wakeups/s | 13.5 | 0 |
| shell wakeups/s | 6.4 | 4.4 |
| idle CPU total | 0.43 % | 0.10 % |
| shell PSS | 112 MiB | 87 MiB |

`tools/bench.py --midget --reboot` now does the clean-boot measurement in one command (~80 s; no unlock, so it reads
slightly higher than the unlocked numbers above).

## Round 4 plan

1. Shell's remaining 4.4 wakeups/s: per-thread diff, then remove timers that are not user-visible.
2. Shell PSS 87 MiB: heap profile of what stays live (frame buffers, glyph caches, wallpaper copies).
3. Compositor (labwc) 55 MiB PSS: Mesa/LLVM share; check whether llvmpipe is loaded at all.
4. App launch: cold launch time and footprint per app, budget them.

## App launch

`tools/bench.py --midget` ends with an app-launch phase; `--midget --apps` runs only it. One ssh session runs a script
on Midget for the first five installed apps (`/var/lib/springboard/apps`, sorted). Per app: `open-app <id>` through
the control socket, then poll (20 ms, 6 s cap) until a process runs from `apps/<id>/` and `state` shows one more
`window=External` line; that time is `midget_launch_ms_<app>`. After 3 s it reads PSS and threads and ends the app
(SIGTERM). The phase stops at 50 s and fails when it measured nothing. Budgets: `docs/budgets.toml`.

## Round 4 results (0.5.12, clean reboot, `bench.py --midget --reboot`)

| Metric | 0.5.11 | 0.5.12 |
|---|---|---|
| shell wakeups/s | 4.8 | 2.0 |
| userspace wakeups/s | 17.9 | 13.5 |
| idle CPU total | 0.40 % | 0.27 % |
| shell PSS | 94 MiB | 95 MiB |
| compositor PSS | 55 MiB | 55 MiB |

| App (cold launch) | Launch | PSS | Threads |
|---|---|---|---|
| Assistant | 77 ms | 7.1 MiB | 2 |
| Browser | 71 ms | 11.5 MiB | 2 |
| Files | 55 ms | 9.1 MiB | 3 |
| Notes | 57 ms | 7.1 MiB | 2 |
| Reminders | 56 ms | 8.9 MiB | 2 |

- Wakeups: Wi-Fi settled tick 5 s (was 700 ms whenever not Connected), nm-worker blocks instead of a 60 s poll and
  backs off re-reads, input hot-plug through inotify.
- Shell memory: the idle trim of the chrome text cache, bar backdrop and dock icons did not move PSS. The heap (45 MiB)
  is not explained by the caches; a real heap profile is needed. The 14 MiB full-screen CPU raster stays (presenter work).
- Compositor: the env (two arenas, early trim, gles2) and the config (no SVG icons, no switcher OSD) are live, but
  the heap stays 18.4 MiB and librsvg stays mapped (labwc links it). Finding: labwc's heap is not arena slack.

## Round 5 results (compositor and wakeups)

Measured on Midget (read-only, 2026-10-11, uptime 6 min).

labwc heap (18.4 MiB anonymous, `[heap]`, 4638 pages, only 275 zero): not arena slack. Scanning the heap
strings shows it is dominated by the icon theme index (`/stock/io`, `/emblems`, `/actions`, `Scalable`: 80+
paths each, from the Adwaita theme tree via libsfdo) and xkb keymap/udev property data (`LevelThree`,
`modifier_map`, `MODALIAS`, `ID_PATH_TAG`). Cursor theme is not the cause (xcursor images are mmapped, 63 files).
Other labwc RSS is shared libraries: libLLVM 15 MiB and libgallium 13 MiB (Mesa, not ours).
Change: `rc.xml` `<theme><icon>hicolor</icon>` so labwc indexes the 112 KiB hicolor tree instead of the 12 MiB
Adwaita tree. Titlebar has no icon button, so no visible change. NOT yet measured after the change (no restart
allowed in this round): expect the heap to drop; re-measure after the next Midget update.

Userspace context switches per second, 20 s window, per thread (kernel threads excluded):

| Thread | /s |
|---|---|
| bifrost-portal `blocking-1` | 2.15 |
| springboard main | 2.1 |
| springboard `wifi` | 0.6 |
| labwc main / sh0 | 0.55 / 0.2 |
| wpa_supplicant, NetworkManager, polkitd | 0.4, 0.25, 0.25 |
| springboard `heartbeat-watch` | 0.25 |

(`psimon` and `bifrost-mcp` rows were the measuring ssh session itself.) The biggest waker we own is
`bifrost-portal`: its `blocking-1` thread (the `blocking` crate pool behind zbus) wakes every ~500 ms, which is the
pool idle timeout, so a task reaches it about twice a second although the bus is quiet and
`$XDG_RUNTIME_DIR` does not change. Cause not yet found (no strace/perf on Midget). Next step: build a portal with
`RUST_LOG=zbus=trace`, or replace zbus with a minimal hand-written D-Bus server for the three Settings methods.
Shell main thread 2.1/s: the poll timeout is already minute-aligned, so the wakes are Wayland events from labwc;
a per-fd count is the next measurement.

## Round 5 plan

1. Shell heap profile on Midget (`heaptrack` or an `LD_PRELOAD` counting allocator) and fix the top live allocations.
2. labwc 18 MiB heap: profile the same way (scene buffers, cursor theme sizes, xkb).
3. Shell's remaining 2 wakeups/s and the other userspace 11.5/s by process.
4. Free the fitted wallpaper copies after GPU upload (needs `Shell` to own the views; see round 4 notes).

## Budgets

See `docs/budgets.toml`. Budgets only get stricter, like the SDK coverage floor.

## Round 5 item 1: shell heap (measured 2026-10-11)

Midget shell: Pss 125 MiB, `[heap]` 43 MiB (59 MiB mapped), two 15 MiB `memfd:bifrost-frame` buffers, no gdb on the device.
Finding: glibc's dynamic mmap threshold rises to the largest freed block (up to 32 MiB), after which 14 MiB
frame/wallpaper copies come from brk and fragment `[heap]`; the earlier cache trims freed small blocks only.
Change: `bifrost_core::host::tune_allocator()` (mallopt: fixed 256 KiB mmap threshold, 1 MiB trim threshold, 2 arenas)
at shell start, so every full-screen buffer is a private mapping returned on free. Not done (not built or measured here):
making `Shell` own the wallpaper views instead of `'static` transmutes (`rotate.rs:137`) so `fit_home` can be freed
after GPU upload. Next: measure `[heap]` after this lands; if above 25 MiB, do the ownership change.

## Round 5 item 2: full-screen wallpaper images (code only, not built or measured)

`Shell` now owns the fitted stills; the `'static` transmute in `rotate::fit_wallpaper` is gone (it returns `Option<Image>`,
read through `rotate::still`). Desktop lifecycle: `fit_home` is freed once the GPU presenter has the texture
(`note_wallpaper_uploaded`, the shell stops passing the view) and is refitted only while the lock screen or Today shows;
`home_dim` already followed that rule; `fit_lock` and `lock_dim` are built when the lock screen shows and freed on unlock.
Expected saving on a 2560x1440 panel: 14 MiB per image no longer live at idle, i.e. `fit_home` (when the still is not
already panel-sized) plus `lock_dim` (dim defaults on) plus `fit_lock` (when the lock still differs), so about 14-42 MiB
anonymous, from the 37 MiB measured. Not changed: `dock_bg` is already a small cropped image (CPU path only);
a 1/4-size backdrop for the CPU bar/menu blur is not needed because the GPU path skips them.
Next: the two `memfd:bifrost-frame` buffers (15.5 MiB each) are presenter double buffering (`bare/wayland.rs` slots,
`copy_damage`); at idle the GPU path draws chrome only, so check whether the second slot can be allocated lazily on
first damage while the first is still busy. Presenter untouched here.

## Round 5 results (0.5.13, 0.5.14)

`bench.py --midget --reboot` now unlocks before measuring (the lab session boots to the lock screen, which builds the
lock wallpaper copies); numbers before 0.5.14 were taken at the lock screen.

| Metric (unlocked desktop) | 0.5.12 | 0.5.14 |
|---|---|---|
| shell PSS | 125.7 MiB (anon 64.7) | 68 MiB after clean boot; 104.7 (anon 43.4) after a lock/unlock cycle |
| shell `[heap]` | 43-45 MiB | 16 MiB (fixed mmap threshold) |
| compositor PSS | 55 MiB | 55 MiB |
| shell wakeups/s | 2.0 (locked) | 1.7 locked, 2.2 unlocked |

- `tune_allocator` (mallopt) moved the full-screen buffers out of `[heap]`; the wallpaper ownership refactor freed
  them after GPU upload. After lock/unlock the shell stays higher (104.7): the lock and Today copies or refits are not
  all freed again. Next item.
- labwc: the `hicolor` icon theme and malloc settings did not move its 18.4 MiB heap. Its remaining heap is xkb, udev
  and wlroots state; Mesa (LLVM + gallium, 15.7 MiB) is the other half and comes with the distribution build.
- Two `memfd:bifrost-frame` buffers (15.5 MiB each) are the presenter's double buffer; a lazily allocated second
  slot is the next memory item.
- Open: Lock Screen on the desktop turns the display off and the tablet-era sleep policy then suspends the machine
  (`src/bare/sleep.rs`); macOS keeps the system awake and lets Energy settings decide. Midget needs Wake-on-LAN after a lock.

## Round 6 plan

1. Free the lock/Today wallpaper copies after unlock (104.7 → ~70 MiB after a lock cycle).
2. Presenter: allocate the second frame buffer only while frames are in flight (−15 MiB).
3. Lock Screen does not suspend; system sleep follows an Energy setting (macOS parity).
4. bifrost-portal `blocking-1` thread, 2.15 wakeups/s (largest owned waker).

## Round 6 results (0.5.15, 2026-10-11)

- Lock buffers: unlock frees the dimmed/fitted lock stills and trims the heap. Clean-boot unlocked shell PSS 68 MiB
  (unchanged). An explicit lock cycle was not re-measured: the control socket has no lock verb (round 7).
- Presenter: background starts with 1 drawable, grows to 2 only while the compositor holds it, shrinks after 5 s idle.
- Sleep: Lock Screen/display-off never suspend; `system_sleep_minutes` (default never); Apple menu > Sleep suspends.
- Portal: inotify filtered by file name. Userspace idle wakeups 13.9 → 10.3/s; dbus-broker 1.37 → 0.60/s;
  springboard 2.5/s; RAM used 725 → 696 MiB.

## Round 7 plan

1. Control-socket `lock` verb, and a bench phase measuring shell PSS after a lock cycle.
2. Shell PSS grows 68 → 102 MiB after the bench's app launches; find what it keeps.
3. Settings > Energy page for `system_sleep_minutes`.
4. springboard wakeups 2.5/s back toward 2.0.

## Round 7 results (0.5.16 to 0.5.18, 2026-10-11)

- `lock` control verb (`bifrost pc midget lock`); bench phase `midget_pss_shell_after_lock_mib` (budget 80).
- Closing an external window drops its cached pixel buffers; icon cache capped at 48; heap trimmed.
- Settings > Energy (from Battery) sets `system_sleep_minutes`, applied live.
- Menu-bar CPU/RAM refresh 5 s; GPU pool trim is a one-shot deadline, armed only when the spare drawable is
  free (0.5.16 re-armed 1 ms while the compositor held it: 130 wakeups/s, fixed in 0.5.17).
- 0.5.18: chrome layer-surface configure matched by object, not id (unlock after display-off restarted the session).
- Midget on 0.5.18: shell PSS 68 MiB, 68 MiB after a lock cycle; shell wakeups 2.1/s; userspace 9.7/s;
  shell idle CPU 0.03 %; RAM used 689 MiB. Books launch 85 ms over its 70 ms budget (once; re-measure).

## Round 8 plan

1. **Opening an app while the lock screen shows restarts the session** (labwc "error in client communication",
   shell "Broken pipe", peak 575 MB). Repro: `bifrost pc midget lock`, wait 8 s, `open-app is.olibuijr.notes.desktop`.
2. Unlock after display-off needs 1 to 4 Enter presses (first key only wakes; then alternates). One key should do.
3. Books launch time back under 70 ms.
4. Quick Look panel drawn by the shell; spell checker adopted in Notes; Observable adopted in one app.
