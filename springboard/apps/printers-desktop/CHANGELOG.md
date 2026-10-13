# Changelog

## 0.2.0

- New: Add Printer adds the driverless printer whose address is on the clipboard (ipp://, ipps://, socket:// or usb://), under a queue name no existing queue uses. Delete Printer removes the selected queue after a second press. The queue naming is shared with Settings > Print & Scan (`bifrost_ui::cups::add_device`).

## 0.1.0

- New: Print Center. The printers the scheduler knows, with the default flagged and the queue of the selected printer. Pause and resume a printer, set the default, and hold, release or cancel a job. The scheduler is asked from a worker thread, so the window never waits on it. The model and the toolbar rules are tested.
