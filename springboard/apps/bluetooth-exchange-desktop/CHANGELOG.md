# Changelog

## 0.1.0

- First version. A device list from the shell's Bluetooth service (paired devices first, Scan for nearby ones, Pair for
  an unpaired one). Files in Downloads are queued to the selected device and sent one at a time over OBEX object push
  through BlueZ's obexd on the session bus, with progress, cancel and a clear-finished action. Files in
  Downloads/Bluetooth are listed as received.
