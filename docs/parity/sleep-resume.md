# Sleep and resume (slug: sleep-resume)

Scope: lid close and idle sleep through logind, the shell locking before sleep, GPU presenter and
outputs after resume, audio and network services after resume, clock jumps. Reviewed in
`springboard/src/bare/sleep.rs`, `bare/heartbeat.rs`, `bare/boost.rs`, `bare/wayland.rs`,
`bare/kms.rs`, `gpu/wl.rs`, `bin/shell.rs`. The reference extract for this slice was not in
`~/Projects/macos-ref/extract/` at the time of the round, so the evidence is the existing code and
its tests; no REA evidence ids.

## What is done in this round

- Wi-Fi power save after a wake (`Boost::restore_wifi_power_save`, `Boost::wifi_power_save`).
  Before: `Sleeper::screen_off` forced nl80211 power save on, and nothing turned it off again until
  the power profile changed, so Performance and Balanced kept Wi-Fi in power save after the first
  screen-off. Now the shell rewrites the active profile's mode on every `ScreenOn` and after a
  resume. Desktop mode included (the profile hand-off skips Wi-Fi).
  Test: `bare::boost::tests::wifi_power_save_follows_the_active_profile`.
- Resume re-anchors the wall clock at once (`shell.set_wall(sys::wall_time(), sys::now_ms())`).
  Before: CLOCK_MONOTONIC stops while asleep, so the 1 s info tick after a resume could see a
  stale wall anchor for up to a second; the anchor is now refreshed on the first loop turn.
- Resume forces a full repaint (`Shell::force_full_redraw`) because the kernel may reset the
  scanout and the surfaces while asleep.

## Row-by-row (what was already there, and what is still open)

- Lid close through logind: done before this round. `LidInhibitor` only blocks logind's lid action
  when `lid_close=ignore`; otherwise logind acts. Midget check: close lid, confirm suspend.
- Idle sleep: done before. `Policy` suspends after screen-off for `system_sleep_minutes` (default
  never), with busy inhibition from audio and downloads and a 5 s grace after each resume.
  Explicit Sleep (menu, login window) suspends at once. Midget check: timeout and explicit sleep.
- Shell locks before sleep: done before. `sleep_screen` puts the shell on the lock screen before
  `SleepSystem` is emitted (desktop bar Sleep, login window Sleep).
- Sleep inhibitors: done before. `systemd-inhibit` sleep block inhibitors refuse the suspend and
  are logged by name.
- GPU presenter and outputs after resume: partly. The presenter in `gpu/wl.rs` and the lost-flip
  presumption in `bare/kms.rs` (`handle_event`) already tolerate a lost page flip. A full
  re-probe of DRM and re-creation of the dma-buf presenter after resume is NOT done; it needs Midget
  evidence first (whether the Intel panel comes back black after resume).
- Audio and network services after resume: audiod and netd are separate services managed by
  systemd and resume on their own. The shell side (Wi-Fi power save) is done above. Midget check:
  audio playback and Wi-Fi throughput after a resume.
- Clock jumps and timers: alarms and timers are anchored to the wall clock (`clock::Wall`) and
  re-anchored each second and now on resume. Timers that were started on the monotonic clock
  before a suspend still expire late by the sleep time; not changed here.
- Heartbeat: `heartbeat::watch` rebases the stall window after a resume (CLOCK_BOOTTIME minus
  CLOCK_MONOTONIC grows), so a suspend is never counted as a stall. Already done.

## Midget checks still needed

1. Lid close and reopen, idle sleep after the set timeout, explicit Sleep from the menu bar.
2. After each resume: clock is right at once, Wi-Fi is not in power save on Performance, the
   screen repaints without a black frame.
3. Audio keeps playing after a resume (audiod), and a wake from the power key lands on the lock
   screen.
