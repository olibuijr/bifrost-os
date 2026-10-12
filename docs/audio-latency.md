# Audio latency: benches, targets and the physical floor

## Running the benches

`bifrost-audio-bench <midi-latency|audio-roundtrip|xrun-sweep|midi-throughput> --driver null|loopback|alsa:<card> --quantum N`
prints one JSON object (`bench`, `host`, `driver`, `rate`, `quantum`, `samples`, `p50_ms`, `p99_ms`, `max_ms`, `min_ms`,
`jitter_ms`, `reported_ms`, `xruns`, `dropped`, `extra`). `tools/audio-bench.py` drives it:

| Command | What it does |
|---|---|
| `bifrost pc audio-bench host` | every bench with `--driver loopback` at quantum 16, 32, 64, 128 (binary from `springboard/target/fast` or `release`) |
| `bifrost pc audio-bench midget [--aloop]` | the same over `ssh midget` with `/usr/bin/bifrost-audio-bench` and `--driver alsa:<card>`; the card comes from `aplay -l` (a real card wins over Loopback); `--aloop` runs `modprobe snd-aloop` first |
| `bifrost pc audio-bench compare` | p50/p99/max per bench and quantum against the previous run in `docs/perf-history.jsonl` |
| `--append` | writes one line per result to `docs/perf-history.jsonl` (`date`, `git`, `bench: "audio-..."`, then the bench fields) |

`tools/bench.py --audio [--midget]` adds `audio_<bench>_q<N>_p99_ms` keys. `python3 -m unittest tools/test_audio_bench.py`
covers parsing, budgets and the table. Exit code 3 means a budget was exceeded.

## Budgets

| Bench | Budget |
|---|---|
| midi-latency | p99 <= the reported output path (`reported_ms`: output latency + safety offset + one quantum) + 0.5 ms |
| audio-roundtrip | measured p50 within 1 frame of `reported_ms` (what the HAL latency properties predict) |
| xrun-sweep (Midget) | smallest stable quantum (`extra.smallest_clean_quantum`) <= 64 |

## Kernel

Measured 2026-10-12 on Midget with `bifrost pc midget kab` (`kbench` on each kernel, one boot into linux-rt; `tools/midget/kbench.sh`, results in `docs/perf-history.jsonl` as `kernel-latency`).

Method: `stress-ng --cpu 8 --io 2 --vm 2 --vm-bytes 256M --hdd 1 --hdd-bytes 64M --timer 4 --switch 2` as the load; `cyclictest -m -S -p 95 -i 200` for scheduling latency, idle and under the load; `aplay -D hw:0,0` (S16_LE, 48 kHz, stereo) at FIFO 80 with 2 periods of 32, 64 and 128 frames under the load, counting `underrun!!!` lines per minute. `cyclictest` needs numactl (libnuma).

| Kernel | cyclictest max idle | cyclictest max under stress | aplay underruns per minute at 32 / 64 / 128 frames |
|---|---|---|---|
| stock 7.2.9-arch1-1 (PREEMPT_DYNAMIC full, `threadirqs`) | 206 us | 337 us | 259 / 648 / 0 |
| linux-rt 7.2.9-rt3-arch1-1-rt (PREEMPT_RT) | 29 us | 38 us | 0 / 0 / 0 |

Owner decision 2026-10-12: linux-rt becomes the default kernel. Packaging done: the installer installs `linux-rt` (`BASE_PACKAGES`), writes the `linux-rt` mkinitcpio preset and the `initramfs-linux-rt.img` boot line, the loader and update guard follow the image in `/boot`, and `tools/lab-mirror.sh` mirrors it. If the package source has no linux-rt (`pacman -Sp linux-rt` fails), or the machine has an NVIDIA GPU, the installer logs it and installs the stock `linux` (`docs/installer-and-boot.md` section 3); the live ISO stays on `linux`. Even the stock kernel needs a period of 128 frames (2.7 ms) to play clean under load; the RT kernel is clean at 32 frames (0.67 ms).

## The physical floor

"As low as the hardware allows" in numbers, at 48 kHz (1 frame = 0.0208 ms):

- One quantum each way: quantum 64 = 1.33 ms, quantum 32 = 0.67 ms, quantum 16 = 0.33 ms. A round trip pays two.
- HDA codec converter group delay: roughly 0.3 to 1 ms per direction (ADC and DAC), fixed by the codec.
- USB audio: 1 ms frames (125 us microframes on high speed), plus the device's own buffering.
- Safety offsets the driver reports (`reported_ms` includes them) cover scheduling jitter on the DMA pointer.

So an HDA round trip at quantum 64 has a floor near 2 x 1.33 + 2 x (0.3 to 1) = 3.3 to 4.7 ms; MIDI note to audio is
one quantum plus the output latency. Loopback and Null drivers have no converter delay, so host runs measure only the
software path.
