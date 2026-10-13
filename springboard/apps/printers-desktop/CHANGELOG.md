# Changelog

## 0.3.0

- New: the queue shows Pages and Created columns, a progress bar under a printing job, and reasons in the Status column ("Printing 2 of 5", "Canceled on server", "Held for authentication").
- New: choose several jobs (drag over the rows, or Select All Jobs with Cmd+A); Hold, Release and Cancel apply to all of them. The Jobs menu uses the plural words ("Pause Selected Jobs").
- New: a secondary click on a job or a printer opens its menu (Hold, Release, Cancel; Pause, Set Default, Printer Info, Share, Remove).
- New: the Printer and Jobs menus in the menu bar with shortcuts (Cmd+N Add Printer, Cmd+P Print Document, Cmd+I Printer Info, Cmd+R Refresh, Cmd+. Cancel).
- New: printer rows say what is wrong in plain words (paper jam, out of paper, door open, offline) with a warning mark, and an unreachable printer is dimmed and marked Offline. A printer that gains an error is announced once as a notification.
- New: printer pools are listed beside the printers and removed as pools. Info pane: edit the description and location, see supply levels, and start or stop sharing.
- New: documents dropped on the window or the Dock icon, or chosen with Print Document, are sent to the selected printer. The Dock badge shows the jobs waiting while the app runs.
- New: control methods `printers.list`, `printers.jobs`, `printers.pause`, `printers.resume` and `printers.cancel_job`.
- The scheduler client (`bifrost_ui::cups`) reads state reasons, markers, shared flags, pools and job progress, and sets details and sharing.

## 0.2.0

- New: Add Printer adds the driverless printer whose address is on the clipboard (ipp://, ipps://, socket:// or usb://), under a queue name no existing queue uses. Delete Printer removes the selected queue after a second press. The queue naming is shared with Settings > Print & Scan (`bifrost_ui::cups::add_device`).

## 0.1.0

- New: Print Center. The printers the scheduler knows, with the default flagged and the queue of the selected printer. Pause and resume a printer, set the default, and hold, release or cancel a job. The scheduler is asked from a worker thread, so the window never waits on it. The model and the toolbar rules are tested.
