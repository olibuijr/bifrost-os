# Bluetooth File Exchange

Bluetooth File Exchange (`springboard/apps/bluetooth-exchange-desktop`, crate `bifrost-bluetooth-exchange`, id
`is.olibuijr.bluetoothexchange.desktop`, version 0.1.0) sends files to a Bluetooth device over OBEX object push and
lists the files received. It is a full-size window: a device list on the left, and on the right the files to send,
the transfers with their progress, and the received files, with a footer for the last message.

## What it does

- **Devices.** The list comes from the shell's `bluetooth` service (`bifrost_app::bluetooth_service`): paired devices
  first, then the nearby ones. Refresh reads the list again every five seconds. Scan asks the shell to look for nearby
  devices. Pair asks the shell to pair the selected device when it is not paired yet.
- **Files to send.** The files in `~/Downloads` (hidden names left out, newest first). Tapping a file queues it for the
  selected device. Nothing is sent until a device is selected.
- **Transfers.** The queue sends one file at a time, oldest first. Each row shows the name, the state (`Waiting`,
  `n of m` while sending, `Sent to <device>`, or the reason for a failure) and a progress bar. Tapping a waiting
  transfer cancels it; tapping a sending one asks obexd to cancel it. A cancelled row stays until Clear Finished removes the transfers that are over.
- **Received.** The files in `~/Downloads/Bluetooth`, newest first, as many as fit above the footer.

## How the bytes move

Transfers go through BlueZ's `obexd` on the session bus (`org.bluez.obex`):

1. `Client1.CreateSession(address, {Target: "opp"})` opens an object push session to the device.
2. `ObjectPush1.SendFile(path)` starts the send. obexd runs the OBEX protocol and the RFCOMM or L2CAP link itself.
3. `Properties.Get` on the transfer reads `Transferred` and `Status` every 250 ms (`Transfer1`).
4. `Client1.RemoveSession` closes the session when the transfer ends, whatever the outcome.

A transfer that sends nothing for two minutes is given up on. If obexd is not running, the transfer fails with
"The file transfer service (obexd) is not running".

The shell's own radio code (`springboard/src/bare/bluetooth.rs`) does not use D-Bus. It talks to the kernel's
management socket. File transfer is the one part of this app that uses D-Bus, through the `zbus` crate's blocking API.

The OBEX packet codec in `springboard/core/src/obex_packet.rs` is not used here, because obexd builds the packets.

## Code layout

| File | Role |
|---|---|
| `src/model.rs` | The rules: the queue and its phases, the folder listings, sizes, the BlueZ status words, the folder paths. Pure. |
| `src/transport.rs` | The obexd client over the session bus (create and remove session, send, progress, cancel). |
| `src/worker.rs` | The thread that runs the bus and disk work, so a slow transfer never stalls the window. |
| `src/window.rs` | The window state, the geometry shared by drawing and taps, the toolbar, the shell requests. |
| `src/draw.rs` | The painting and the accessibility nodes for the toolbar buttons. |
| `src/main.rs` | The `App` implementation that connects the window to the SDK. |

## Limits of this version

- Files come only from `~/Downloads`. A native open panel (the shell's file chooser) would let any file be picked; that
  is the next step.
- Received files are listed, not accepted. obexd needs an agent registered for incoming pushes before it accepts a file
  from another device, and this app does not register one yet. Incoming files arrive only if an obexd receive service
  is configured to save into `~/Downloads/Bluetooth`.
- One transfer at a time. Pairing shows the result, and the device list follows the shell's events.
- The list shows at most eight files to send and four transfers. The label says how many files there are.
- No icon yet. The manifest has no `icon` line; add one with the other apps' icons.

## Tests

The model has unit tests for the queue, the phases, the progress rules, the listings and the sizes. The transport and
the worker have small tests for the error wording, the missing-folder case and a cancel with nothing running. The window
has tests for the geometry, the queue and the footer. Nothing in the tests touches the radio or the disk outside the
test process.
