# Changelog

## 0.7.0

- New: a job waiting or held on another computer says so in the Status column ("Waiting on the other computer", "Held on the other computer").
- Tested: a printer whose device reports it offline reads as "Printer is offline" in its problem line.

## 0.6.0

- New: the info pane has a Share Printer or Stop Sharing button beside Save, for a queue that can be shared. A pool and a queue that sends its jobs to another computer are not offered.
- New: the Status column reads sentences for more job reasons from the scheduler: waiting for a password, wrong document password, not allowed to print, queued in the printer, service off line and others.
- The printer problem sentences are tested for paper jams, empty trays, toner, doors and offline reports.

## 0.4.0

- New: Add Printer lists the printers on the network that announce themselves; click one to add it under its own name with its model and location. The clipboard address stays as "Use Address from Clipboard" in the pane and the Printer menu.
- New: Print Center Help in the Help menu shows the topics in the window.
- New: a printers widget for Notification Centre that leads with a printer error, an offline or paused printer, or the jobs waiting.
- New: the Dock icon bounces when a printer gains an error, besides the notification.
- New: a document tile with the file type sits left of each job's name.
- New: Shortcuts actions List Printers, Pause Printer, Resume Printer, Set Default Printer, Cancel Print Job and Print File, with printers as choices.
- The queue is listed every second while a job waits or prints (every five seconds otherwise), so a change shows sooner.

## 0.5.0

- New: the job Status column reads "Canceling page 2 of 5" while a cancel of a printing job is on its way, until the scheduler drops the job.
- New: a queue that sends its jobs to a print queue on another computer shows "Remote" in its row and in the info pane, and is not offered for sharing.
- New: the info pane lists the printer's default paper, two-sided printing and colour printing, as the device reports them.
- New: Shortcuts actions Hold Job and Release Job, with a Job entity for the jobs of the selected printer's queue.

## 0.4.0

- New: Add Printer lists the printers on the network that announce themselves; click one to add it under its own name with its model and location. The clipboard address stays as "Use Address from Clipboard" in the pane and the Printer menu.
- New: Print Center Help in the Help menu shows the topics in the window.
- New: a printers widget for Notification Centre that leads with a printer error, an offline or paused printer, or the jobs waiting.
- New: the Dock icon bounces when a printer gains an error, besides the notification.
- New: a document tile with the file type sits left of each job's name.
- New: Shortcuts actions List Printers, Pause Printer, Resume Printer, Set Default Printer, Cancel Print Job and Print File, with printers as choices.
- The queue is listed every second while a job waits or prints (every five seconds otherwise), so a change shows sooner.

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
