# audio-06: bifrost-audio-hal (client library)

Contract: `docs/audio-architecture.md` ("Client IO", "Real-time threads"). Task: `tasks/audio/06-hal-client.md`.

## What is implemented (`springboard/audio/hal/`)

- `conn.rs`: SEQPACKET framing with SCM_RIGHTS (`send_packet`, `recv_packet`, up to 4 fds, truncation is an error),
  and `Conn`: a reader thread (`audio-notify`) that sends replies to the waiting request and handles the unsolicited
  ones itself (`Changed` runs the listeners, `Overload` is counted per IO procedure id). One request in flight at a
  time (a mutex on the reply channel); 5 s reply timeout; a closed socket gives `NoServer`.
- `lib.rs`: `System::{connect, connect_path, from_socket, default_device, devices, get, set, listen, overloads}`,
  `Listener` (drop = unlisten; the server is told only when the last listener of an address goes), `Device::{
  add_io_proc, start, stop, remove_io_proc, io_proc_xruns, get, set}`. `add_io_proc` resolves `buffer_frames == 0`
  from the device's `BUFFER_FRAMES`, sends `AddIoProc`, maps the memfd of the reply and keeps the eventfd open.
  `start` creates the IO thread, waits until it has joined the workgroup, then sends `StartIoProc`; `stop` sends
  `StopIoProc` first, then ends the thread (stop flag plus a futex wake; the IOProc is kept for a restart).
- `iothread.rs`: the IO thread loop on the activation record's `wake` futex (250 ms backstop only to see the stop
  flag): status AWAKE, read `frames` from the input ring (zeros for a short read), run the IOProc, write the output
  ring, `shm::finish(act, &[], now)` (status FINISHED). All scratch is allocated before the loop; channel scratch
  is a stack array, so the limit is `MAX_CHANNELS = 32` per direction. Timestamps: `now` = zero stamp extrapolated
  to the host clock; input time = `now` minus one buffer; output time = `now` plus one buffer.
  `RtScope` plus `RtGuardAllocator` (debug builds): an app or test binary that installs the allocator has every
  allocation inside the IOProc counted (`RtScope::violations()`) and reported on stderr with a fixed `write(2)`
  message; it never panics (a global allocator must not unwind). With `BIFROST_RT_STRICT=1` (read once, at the
  first scope) the process aborts instead. Locks and syscalls cannot be detected from a library; the rule is documented.
- `workgroup.rs`: `join` (asks the server for SCHED_FIFO 80 via `Promote`, with a pidfd; then `sched_setattr`
  `sched_util_min` 512), `Membership` (drop clears uclamp), `set_affinity` hint, `set_util_min`. A refused
  promotion is not fatal for `start`: the thread runs at normal priority with the frequency hint and logs it.
- `examples/passthrough.rs`: capture to playback, `passthrough [seconds] [frames]`.

The per-client buffer size (k cycles) is the server's job (it accumulates and wakes the client every k cycles);
the client always handles `frames` per wake. `add_io_proc` rejects `frames > layout.ring_frames`.

## ui models (`ui/src/audio_hardware.rs`, `hardware_clock.rs`, `process_tap.rs`)

Not changed. They are in-memory models with kit tests, `bifrost-ui` does not depend on `bifrost-audio-hal`, and
making it do so (or the reverse) is a decision for the lead; the HAL crate has no dependency on the ui crate, so
there is no cycle and no conversion code yet. The `todo!("audio-06")` markers were only in `hal/src/lib.rs`.

## Assumptions about other tasks (check at merge)

1. The server wakes a client by incrementing `Activation::wake` once per cycle that the client must run, and
   futex-wakes it. It reads `status == STATUS_FINISHED` at its mix deadline (the client has no pointer to the
   driver's activation, so `finish` is called with no targets). If the server wants an extra wake of the driver,
   that needs a field in `ShmLayout` (additive).
2. The server initialises both `RingHeader`s (`frames`, `channels`) and the activation's `rate`.
3. `Request::PromoteThread` arrives with one pidfd in the ancillary data.
4. The reply `IoProcAdded` carries the memfd first, the eventfd second (optional).

## Tests

Pure: framing, fds, workgroup struct sizes, rt guard. With a scripted server on a socket pair (need audio-02's
codec and audio-03's shm, so they run only after the merge): `listener_fires_on_default_device_change`,
`io_proc_receives_input_and_output_for_the_same_cycle` (hand values: input 1 2 3 4 / 5 6 7 8 doubled to
2 4 6 8 / 10 12 14 16), error and version handling.

## Midget steps for the lead

1. `cd springboard && cargo test --offline -p bifrost-audio-hal` (host).
2. Build and push: `cargo build --offline --profile fast -p bifrost-audio-hal --example passthrough`, copy to Midget
   (`scp target/fast/examples/passthrough midget:/tmp/`), with `bifrost-audiod` running as the `audio` user.
3. `ssh midget /tmp/passthrough 600 64`; exit code 0 and `done: xruns 0, overloads 0` while the desktop is idle.
4. Check priority: `ssh midget 'chrt -p $(pgrep -f passthrough | head -1)'` is not enough (it shows the main
   thread); use `ps -eLo tid,cls,rtprio,comm | grep audio-io` and expect `FF 80`.
5. Frequency hint: `ssh midget 'cat /proc/$(pgrep passthrough)/task/*/sched | grep -i uclamp'` (kernel with
   `CONFIG_UCLAMP_TASK`) shows util_min 512 on the `audio-io-N` thread.
