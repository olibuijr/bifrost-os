# Changelog

## 0.5.0

- New GPU tab with a per-process % GPU column, the GPU Processes filter and Window > GPU History (cmd+4), read from the graphics driver's per-process usage files.
- The sidebar and View > Filter Processes add Windowed, GPU and Selected Processes.
- Process > Send Signal to Process... opens a sheet with the eight signals.
- The inspector is a sheet with Memory, Statistics and Open Files and Ports segments (parent process, page faults, context switches, sockets with their addresses) and Copy, Sample and Reveal in Files buttons (cmd+I).
- Process > Sample Process (and the Sample toolbar button) samples where the threads of a process wait for ten seconds and shows the report, with Save... and Copy.
- View > Run System Diagnostics... collects counters, processes, disks, sockets, the kernel log and the journal tail into a zip and offers the save panel.
- View > Show Deltas for Process counts CPU time, idle wake ups and bytes read and written from the moment it is switched on.
- View > Columns adds Parent PID, State, Real Mem, Private Mem, Compressed Mem, CPU Time, Idle Wake Ups, Bytes Read, Bytes Written, % GPU and Preventing Sleep.
- Window > CPU Usage (cmd+2) and CPU History (cmd+3) show one bar or history strip per core.
- View > Dock Icon shows the CPU, memory, disk or network reading on the Dock tile.
- The graph cards list their figures beside the plot; Memory also plots pressure and lists shared, buffers, compressed and swap.
- The Energy tab adds a twelve hour mean and Preventing Sleep (sleep and idle inhibitor locks); the Network tab adds packet counts.

## 0.4.0

- View > Columns shows or hides the User, Threads and PID columns, and remembers the choice.
- The Memory graph's detail line adds shared, buffer and compressed memory beside cached files and swap.

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
