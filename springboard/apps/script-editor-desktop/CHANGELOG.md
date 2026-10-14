# Changelog

## 0.1.0

- First version: a shell script editor with Run and Stop, a result pane for standard output and standard error, the status of the last run, and the usual File menu (New, Open, Open Recent, Save, Save As) through the shell's document panels.
- Scripts run in /bin/sh from the home directory on a worker thread, so the window keeps drawing while they work.
