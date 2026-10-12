# Wave 4: IOUSBHost transfer planning (pure model)

## What was built

`springboard/core/src/usb_transfer.rs` (new, one `pub mod` line in `springboard/core/src/lib.rs`). It is a pure
model in std Rust with no device access. It extends the wave 2 USB model (`springboard/core/src/usb_descriptors.rs`,
which reads descriptors and builds control requests) with the next step a host driver takes: choosing an interface
alternate setting and an endpoint, checking that a transfer matches the endpoint's type and direction, and splitting
the buffer into packets or isochronous frames.

- `Pipe` (from an `EndpointDescriptor`): address, transfer type, direction, payload size per packet, and high-speed
  transactions per microframe (bits 11 and 12 of `wMaxPacketSize`).
- `Pipe::check`, `Pipe::packet_count`, `Pipe::packet_ranges`: refuse a wrong type or direction, count packets (an
  empty transfer is one zero-length packet), and give each packet's byte range.
- `Pipe::bytes_per_frame`, `Pipe::frame_list`, `plan_isochronous`: per-frame capacity at full or high speed. Bytes are
  spread evenly over the frames, with the remainder in the first frames. A frame list that cannot hold the bytes is
  refused with `FrameOverflow`, not truncated.
- `IsochFrame` (request and complete counts), `summarize`, `duration_micros`, `frame_number_after` (64-bit wrap).
- `select_alternate`, `alternate_settings`, `copy_pipe`: interface and endpoint lookup on a configuration.

Not built: the driver objects themselves (no claim, no I/O queue, no completion handler), stream I/O, the
bandwidth adjustment calls, the idle timeout, and the isochronous transaction list with its transfer options. These
need a device node that the app sandbox does not bind (`springboard/src/bare/app_policy.rs`), so they stay gaps.

## Official API names mirrored

Reference pages read through `python3 -I tools/parity/docc.py` and the rendered pages:

- `IOUSBHostInterface`: `selectAlternateSetting(_:)`, `copyPipe(withAddress:)`, `configurationDescriptor`,
  `interfaceDescriptor` (https://developer.apple.com/documentation/iousbhost/iousbhostinterface)
- `IOUSBHostPipe`: `enqueueIORequest(with:completionTimeout:completionHandler:)` for bulk and interrupt;
  `enqueueIORequest(with:frameList:frameListCount:firstFrameNumber:completionHandler:)` and
  `sendIORequest(with:frameList:frameListCount:firstFrameNumber:)` for isochronous; and the transaction-list forms
  with `IOUSBHostIsochronousTransferOptions` (https://developer.apple.com/documentation/iousbhost/iousbhostpipe)
- `IOUSBHostIsochronousFrame`: `requestCount`, `completeCount`, `status`, `timeStamp`
  (https://developer.apple.com/documentation/iousbhost/iousbhostisochronousframe)

The `IOUSBHostIsochronousTransfer` page path rendered as "page not found", so no fact is taken from it. The 1 ms frame
and 125 us microframe lengths come from the USB specification, which the framework page cites (USB 3.2 and the 2017
USB-IF revision), not from the Apple pages.

## REA evidence

- `ev_0ae6fad1d7535712fcd271f1f8ae1860a37d8b3605c98d86bda3ef8e16045b9d`: framework landing page
  (https://developer.apple.com/documentation/iousbhost)
- `ev_906c62ebaf39ba5dbf22bd38fb1c848c58686d9c39de1f9b20d98ad800fd38fc`: `IOUSBHostPipe` page
  (https://developer.apple.com/documentation/iousbhost/iousbhostpipe)
- `ev_fdb38f84325b31e16edf82713aed17c7f256748cc357813865e2929996c76fc8`: `IOUSBHostIsochronousTransfer` page path,
  which rendered as "page not found". Recorded only as a negative result.

## Matrix rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| IOUSBHost | Interface, pipe and stream I/O (control, bulk, interrupt, isochronous) | partial | springboard/core/src/usb_transfer.rs |
| IOUSBHost | Isochronous transfer options | partial | springboard/core/src/usb_transfer.rs |

Left unchanged: "Device driver access" (partial, wave 2), "Scanners and printers over USB" (gap; a bulk pipe model
exists now, but there is no scanner or print transport), and the OS-only rows.

## Tests

`cargo` was not run, because this task forbids builds. The unit tests in `usb_transfer.rs` cover: packet geometry from
the `wMaxPacketSize` bits; packet counts and ranges; type and direction refusal; endpoint and alternate lookup;
full-speed frame spreading (750 bytes over 8 frames gives six 94s and two 93s); high-speed capacity (8192 bytes per
frame) and overflow; summaries; frame timing; and frame-number wrap.
