# Changelog

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
