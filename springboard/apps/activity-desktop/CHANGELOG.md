# Changelog

## 0.10.7

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.10.6

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.10.5

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.10.4 — 2026-10-23

- Extra windows receive wheel and hover events.

## 0.10.3

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.10.2

- The app declares `entitlements = ["system-monitor"]`. Its sandbox now shares the host PID and network namespaces and reads `/proc` read-only, so the CPU tab lists every process of the machine and the Network tab sees its interfaces and sockets (before, it saw only itself and `bwrap`). Quit and Force Quit still signal only what the app account may signal.

## 0.10.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.10.0 — 2026-10-19

- The Network tab lists processes instead of interfaces: Received/s, Sent/s, Bytes Received, Bytes Sent and (optional) Connections per process, busiest first. The figures come from the kernel's socket counters (`ss -tinpH`, `sample_net.rs`), read once per update while the tab is shown. They cover TCP sockets that belong to your own processes; a socket that closes takes its bytes with it, so the totals count only the sockets open now. Show Deltas counts the byte totals from the baseline. The interface totals moved to the footer.
- Memory tab: Real Mem (proportional set size, so shared pages are split between the processes that map them), Purgeable (pages freed with `MADV_FREE`, which the kernel may take back) and Kind (the program's machine, read from its ELF header) join the optional columns. Kind is also an optional CPU column. Each is read only while its column is shown, from one `smaps_rollup` read per process.
- Window > GPU History opens a floating window with the busy share of the GPU over the last minute and the process using it most. The GPU is read while the window is open. Window > Keep CPU and GPU Windows on Top floats all three monitor windows.

## 0.9.0 — 2026-10-17

- Cmd+W in a CPU Usage or CPU History window closes it, as its traffic light does (`window_input`). The main window's keys and menu items are unchanged.

## 0.8.0

- Window > CPU Usage opens a small floating window with the total and one bar per core, and closes again from the same menu item.
- Window > CPU History opens a floating window with one history strip per core (the same strips as View > CPU History).
- Window > Keep CPU Windows on Top floats both CPU windows above the normal windows; the choice is remembered. It needs the shell that carries `ToShell::WindowLevelOf` (min_shell 0.8.3).
- The drawing lives in draw.rs (`draw_cpu_usage`, `draw_cpu_history_window`); the window sizes follow the core count.

## 0.7.3 — 2026-10-14

- Job progress now wakes through the SDK main queue instead of a short polling timer; dropping a job requests cancellation.

## 0.7.2 — 2026-10-14

- Use the shared grouped-card radius token for graph cards.
- Move process copying to Process > Copy so it no longer duplicates the shell's Edit menu.

## 0.7.1

- Standard Copy now uses the shell Edit command. Added the SDK and design compliance test harness.
## 0.7.0

- The parity matrix rows for the Memory breakdown, the 12 hr Average, the GPU shares and Edit > Copy now cite the functions that do the work (draw_pressure, EnergyAverages, parse_pmon, copy_text and row_text). The Memory row's note lists the App, Kernel, Shared, Cached Files, Compressed and Swap Used line the footer shows. Behaviour is unchanged.

## 0.6.0

- Removed an unused model module. Show Deltas for Process and Sample Process already run in model.rs and sample_proc.rs; behaviour is unchanged.
- The parity matrix rows for Show Deltas, Sample Process and the Columns submenu now cite the code that does the work.

## 0.4.0

- View > Columns shows or hides the User, Threads and PID columns, and remembers the choice.
- The Memory graph's detail line adds shared, buffer and compressed memory beside cached files and swap.

## 0.5.0

- Quit Process and Force Quit Process ask first, in a sheet with Cancel, Force Quit and Quit. Return chooses the default button and Escape closes the sheet.
- The signal sheet takes the arrow keys and Return.
- Inspect Process fills the window with a panel: Memory, Statistics, and Open Files and Ports (the open files, then the TCP and UDP sockets with their endpoints and state). Reveal in Files shows the program in Files. Done and Escape close it.
- Process > Sample Duration chooses 3, 10 or 30 seconds.
- View > Run System Diagnostics adds a short system-wide sample: every thread of every process, five looks 200 ms apart.
- With Show Deltas on, the CPU time, bytes and GPU time columns sort by what each process did since its baseline. The optional Since Baseline column on the CPU tab shows how long the baseline has been running.
- View > Columns adds Ports (open sockets), Private Memory and Idle Wake Ups (voluntary context switches per second, on the Energy tab).
- The sidebar and View > Filter Processes add Applications in the Last 8 Hours, by process start time.
- The Memory tab's footer shows the last minute of memory pressure as bars, green, yellow or red.
- The 12 hr Average column keeps its means across launches.
- The GPU tab reads per-process shares from the vendor GPU tool when the driver publishes no DRM counters.

## 0.4.0

- View > Columns chooses the columns of each tab (CPU Time, Parent PID, Virtual Size, 12 hr Average, Preventing Sleep, Bytes Read and Written, GPU Time, Packets In and Out) and remembers the choice.
- Process > Show Deltas for Process counts a process's CPU time, bytes read and written and GPU time from the moment you turn it on.
- A GPU tab shows each process's share of the graphics engines and the machine's total, read from the kernel's per-client usage counters. The sidebar and View > Filter Processes add GPU Processes and Selected Processes (the selected process and everything it started).
- View > CPU History replaces the CPU graph with one strip per core.
- The Memory tab splits used memory into App, Kernel, Shared, Cached Files, Compressed and Swap.
- The Energy tab shows which processes keep the computer awake (a blocking sleep or idle lock held with the login manager) and a 12 hour average per process name.
- The sidebar and View > Filter Processes add Windowed Processes, the ones that own a window on the desktop.
- The Network tab adds packet counts.
- The process inspector shows the parent process, page faults and context switches.
- Process > Sample Process watches every thread of the selected process for ten seconds and reports where each one waited, in a report you can copy or save.
- View > Run System Diagnostics gathers kernel counters, the process list, the journal tail, the kernel log and disk usage into one archive and asks where to save it.
- Process > Send Signal... opens a sheet with the eight signals, what each does, and Cancel and Send.
- View > Dock Icon shows the CPU, memory, disk or network figure as the Dock badge with a bar under it, and remembers the choice.
- The app asks for the documents permission, for the save panel, and the notifications permission, for the Dock badge.

## 0.3.0

- The sidebar and View > Filter Processes add System, Active and Inactive processes.
- Process > Send Signal to Process sends HUP, INT, KILL, TERM, USR1, USR2, STOP or CONT to a process of your own user.
- View > Update Frequency reads the machine every 1, 2 or 5 seconds, and remembers the choice.
- Edit > Copy (cmd+C) puts the selected row on the pasteboard as tab-separated text.

## 0.2.0

- Inspect (toolbar and Process menu) opens the process inspector in place of the graph: the command, executable, working folder, open files and sockets, and threads with their state, refreshed each second. It closes when the process exits.
- Force Quit Process sends SIGKILL to a process of your own user, beside Quit Process.

## 0.1.0

- First release: a process table with sorting, search and the My, Other and All filters.
- CPU, Memory, Energy, Disk and Network tabs with a one-minute graph each, read from `/proc`.
- Quit Process sends SIGTERM to a process of your own user.
