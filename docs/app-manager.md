# Desktop app resource manager

Since shell 0.4.24, installed SDK and native packages share a common kernel resource
boundary. `bare/app_manager.rs` creates cgroup v2 groups under
`/sys/fs/cgroup/bifrost-apps/<app-id>`. The launcher attaches itself before exec,
so nested bubblewrap, Servo renderers and workers inherit the same group.
Missing required controllers or failed attachment rejects launch.

RAM-scaled limits: per-app memory.high is 25% of RAM when focused and 12.5%
when unfocused, with a 256 MiB floor. This threshold requests kernel reclaim;
it is not a kill threshold. Per-app memory.max is 75% of RAM (512 MiB floor).
The app subtree as a whole has high=75%, max=90%, reserving capacity for the
shell and other system services. The kernel can kill an entire app group under
its hard limit; memory.oom.group prevents orphaned content processes. The app
has a 4096-process limit. CPU weight is 100 focused, 25 unfocused; this changes
relative scheduling priority under contention, not a fixed CPU quota.

SDK Suspend/Resume adjusts resource priority alongside existing save-state
callbacks. Native focus comes from Wayland foreign-window events. Unfocused
desktop apps are not frozen unless the Settings switch "Suspend Background Apps"
is on (see Background suspension below). SDK close still sends Quit before terminating the
whole owned cgroup. Remote's explicitly brokered sharing service lives outside
its UI cgroup, so closing the UI does not stop sharing.

A worker samples per-group memory, CPU, process count and memory events every
five seconds into `/run/bifrost/app-resources.json`, written by atomic rename.
Use `bifrost pc midget apps` (or `python3 tools/midget.py apps`) to read the snapshot.
Inspect `memory_events` for high/max/oom/oom_kill before reporting results. Resource budgets for the whole device
(idle CPU, wakeups, PSS per process) are separate and live in `docs/budgets.toml` ([snow-leopard.md](snow-leopard.md)).
No resource polling or sleep is added to the render loop.

Desktop native app ids with open windows are saved to `open-native-windows.json`
in the shell data dir when that set changes, and reopened at shell start. Window
geometry and documents are not restored. This resource manager does not add
app-specific memory-warning callbacks. Apps
retain their existing state-save behavior. Unpackaged Arch launchers remain
outside installed-package policy. Budgets are system defaults, not tuned
per-app measurements or claimed memory savings.

## Background suspension (`springboard/src/shell/background.rs`)

With Settings > "Suspend Background Apps" on (`suspend_background`), an external app that nothing shows is stopped with
`SIGSTOP` after the grace period in `bifrost_core::occlusion`. It is resumed with `SIGCONT` when it is focused or seen
again. Turning the switch off resumes every app the shell stopped. The shell keeps one model per external app slot and
feeds it whether the app is focused and whether any of its windows is on screen. Covering is not tracked yet, so an app
with any shown window counts as on screen.

## Window lifecycle on the desktop (2026-10-08)

An app that exits takes its window with it (0.4.49). Before, the last frame stayed on screen until
the window was closed by hand. Opening the app again starts a new process; the old window is gone.
Store apps open by their installed slot (`apphost::slot_of`), so `open-app` and Show in Files work for
them as well as for built-in apps (0.4.48).
