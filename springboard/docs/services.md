# bifrostd and the audio service (design, OLI-693)

Status: supervisor, wire protocol, `audiod` (0.3.0), `netd` and `updated` (0.3.5) implemented; Podcasts and Voice Memos are packages (`apps/podcasts`, `apps/voicememos`, 0.3.2); Music is still in the shell.
Extends "Package kinds" in `apps.md` (`kind = "service"`).

## Why
Music, Podcasts and Voice Memos each open their own `AudioPlayer` inside the shell
(`shell/music`, `shell/podcasts`, `shell/voicememos`, ~7000 lines), so audio dies with the shell,
two apps cannot share the speaker, and an app cannot leave the shell until it has no device
code. One service owns the speaker and the microphone; apps become pure UI clients.

## bifrostd (crate `bifrostd`, std only)
- Started by `tools/boot.sh` before the shell when `$DIR/bifrostd` exists; outlives shell restarts.
- Scans `<root>/apps/<id>/current/manifest.toml` for `kind = "service"` every 500 ms, so install,
  update and removal need no restart.
- Spawns `exec` with `BIFROST_SERVICE_SOCK=<root>/run/<id>.sock`, `BIFROST_DATA`, `BIFROST_SHELL_DIR`.
- Exit => restart with backoff 1, 2, 4 ... 30 s (a run of 60 s resets it). Never gives up.
- Update: when `current` points to a new version dir, start it on `<id>.sock.new`, wait up to 10 s
  for a ping answer, then stop the old one (SIGTERM, SIGKILL after 3 s) and rename the socket into
  place. If it never answers the old version keeps running and that version is not retried until
  `current` moves again. Clients reconnect to `<id>.sock` and re-subscribe, so they just see a short gap.

## Wire protocol (`bifrost_proto::svc`)
`SOCK_STREAM` unix socket; frame = `u32 len` + (`u32 tag` + fields); max 256 KiB; unknown tags skipped.
Every service answers `Ping` (tag 1) with `Pong { id, version }` (0x101).
Why not SEQPACKET like apps: std has no SEQPACKET socket, and services carry no fds.

### Audio (`svc::audio`, id `is.olibuijr.audio`)
Commands: `Subscribe`, `PlayQueue{tracks,start,shuffle,seed}`, `Play`, `Pause`, `Toggle`, `Next`, `Prev`,
`JumpTo`, `SeekMs`, `SetShuffle`, `SetRepeat`, `SetVolume`, `SetMuted`, `Stop`, `RecordStart{path}`, `RecordStop`.
Events: `State(Snapshot)` (same fields as `media::audioplayer::Snapshot` plus `muted`), `Record{active,elapsed_ms,level,error}`,
`RecordDone{path,duration_ms}`. A subscriber gets `State` on every change and once per second of
playback; during a recording it also gets `Record` ticks (meter).
- One queue, one speaker: the last `PlayQueue` wins (audio focus is "latest player").
  Podcasts and Music use the same queue API; live radio is a `Track` with a `url`.
- Muting stays the shell's job (Control Center volume): the shell sends `SetMuted`/`SetVolume`.
  The service starts muted and never makes a sound without a command.
- The microphone is only opened by `RecordStart`, which the Voice Memos UI sends on an explicit tap.
  The service writes the WAV to `path` (an app's data dir) and sends `RecordDone`.

## Migration (phase 2, one issue each, after this merges)
1. (done, 0.3.1) SDK: `bifrost_app::audio::Client` (`springboard::svc::audio`, shared with the shell) (connect, reconnect, `subscribe`, cached `Snapshot`, wake fd for the event loop).
2. (done, 0.3.1; the in-shell Music engine is the fallback until step 3) Shell: Control Center / lock screen Now Playing and hardware keys talk to the same socket instead of `Music::player()`.
3. (Music done, 0.3.2: `apps/music`, a client of audiod; the in-shell engine is gone; Podcasts and Voice Memos done, OLI-703: playback and `RecordStart`/`RecordStop` through the client; Podcasts re-adopts an episode the service is still playing when it starts; a recording is stopped and saved when its connection drops.) Music -> `apps/music` (`system-app`), then Podcasts -> `apps/podcasts`, Voice Memos -> `apps/voicememos`,
   each moving its `shell/<app>` module into the app crate, replacing `AudioPlayer`/`Recorder` with the client,
   and migrating data from the legacy shell dirs on first launch (as the ten earlier apps did).
4. Delete the shell's `AudioPlayer` ownership once all three are out (`media::audioplayer` stays as the service's engine).

## Network service (`svc::net`, id `is.olibuijr.net`, binary `netd`)
Wraps `net::wifi::Wifi` (supplicant, DHCP, routes, own thread) behind the socket; the root is `BIFROST_SHELL_DIR`.
Commands: `Subscribe`, `Scan`, `Join{ssid,psk}`, `Select`, `Forget`, `SetAutoJoin`, `Move`, `Enable`.
Events: `Status` (state 0 off .. 4 connected, ssid, ip, router, rssi, scan list, saved networks, per-network notes, error)
on subscribe and on every change (polled every 250 ms). Hand-typed networks (`JoinOther`: hidden, enterprise) stay in the
shell until the Settings page migrates; the wifi health recovery (`bare::wifi_health`) too.
Only one owner may run the supplicant: the shell must stop calling `Wifi::start` when it switches to `netd`.

## Update agent (`svc::update`, id `is.olibuijr.update`, binary `updated`)
Wraps `shell::ota::Ota` (schedule, OS manifest check, store updates, silent system package installs, OS download to
`springboard.new` in `BIFROST_SHELL_DIR`). The agent has no idea what the shell runs, so the shell sends
`Configure{version, git, auto_download, url}` on connect and `Conditions{wifi, low_power, charging}` when they change;
nothing is checked before the first `Configure`. `Check` and `Download` serve Settings > Software Update.
Events: `State{phase, offer, fraction, apps, system, error}` (on every change, once a second while downloading) and
`SystemInstalled{ids}` so the shell hot-reloads what those packages provide. Banners stay the shell's job: it compares
`offer`/`apps` against its prefs (`ota::should_notify`). Installing an OS binary is still the shell's restart.

## Migration (phase 2 for network and update)
1. SDK clients `bifrost_app::net::Client` / `update::Client` (same shape as the audio client).
2. Shell: Wi-Fi status bar, Control Center and Settings > Wi-Fi read `Status`; `Ota` is replaced by the update client.
3. Delete the shell-side `Wifi::start` / `Ota` threads once both pages use the sockets.

## Not in this change
Services ship as `kind = "service"` packages through the App Store (`tools/aap.py pack system/<name>
--bin <binary>`, then `publish-app.sh`); they are hidden from the home screen.
