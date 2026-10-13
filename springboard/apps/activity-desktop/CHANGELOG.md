# Changelog

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
