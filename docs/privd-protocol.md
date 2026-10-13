# privd protocol and login design (contract, proto 1)

Goal: boot lands on a real login window; after login the shell, apps, Term, file access and D-Bus run as the
installed user; root work goes through the small root daemon `bifrost-privd`. Wire types are frozen in
`springboard/src/privd/proto.rs`; tracks must not edit it, `lib.rs` or `Cargo.toml` (ask the lead). Only the lead builds.

## Trust model

The compositor and shell are already the user's uid at boot (as with macOS loginwindow). The lock screen is
enforced by `ext-session-lock-v1` (a dead client keeps the screen locked) plus privd. Pre-login the shell has no apps
running and no privileged capability. Apps and Term start only after login. Every privileged privd method is refused
until privd's session is Unlocked.

## Processes

1. `springboard.service` (root) runs bootweave and `/usr/lib/bifrost/session`, now a thin root supervisor.
2. `bifrost-privd.service` (root, `Before=springboard.service`, `Restart=always`) runs `bifrost-privd` (module
   `springboard::privd` + `src/bin/privd.rs`; not linked against the UI). It is not ordered after the UI.
3. `bifrost-wm` and dbus-daemon run as the user. The shell is launched as the user:
   `setpriv --reuid=$uid --regid=$gid --init-groups --inh-caps=-all --bounding-set=-all` (or `runuser -u`).
   Env: HOME, USER, LOGNAME, SHELL, `XDG_RUNTIME_DIR=/run/bifrost/user`, DBUS_SESSION_BUS_ADDRESS, WAYLAND_DISPLAY,
   `SPRINGBOARD_WAYLAND=1`, `SPRINGBOARD_PRIVD=/run/bifrost/privd.sock`,
   `SPRINGBOARD_DATA=$HOME/.local/share/springboard`. Groups seat, video, render, input, audio via useradd/usermod.

## Socket and framing

- `/run/bifrost` is root:root 0755. `/run/bifrost/privd.sock`: SOCK_STREAM, root, 0660, group `bifrost-session`
  (system group created by sysusers.d; the account is a member). `SO_PEERCRED` checked on every accept: uid must be the
  account uid or 0, and `/proc/<pid>/exe` must be `/usr/bin/springboard`, `bifrost-mcp` or a root tool. bwrap sandboxes
  never see the socket (not bound in app_policy).
- `/run/bifrost/privd-root.sock` (0600 root) serves root-only tools (ssh lab, MCP).
- Frame: 4-byte big-endian length + UTF-8 JSON, max 64 KiB.
- Request `{"id":N,"method":"..","params":{..}}`.
- Reply `{"id":N,"ok":true,"result":{..}}` or `{"id":N,"ok":false,"error":{"code":"..","message":"..", ...extra}}`.
  Extra fields (`fails`, `retry_secs`) sit flat in `error`.
- Events on a subscription connection: `{"event":"login.state"|"update.progress"|"install.progress", ...}`.

## Methods

UNAUTH = allowed on the lock screen. AUTH = needs Unlocked and the session token in params `t`.

| Method | Auth | Params | Result |
|---|---|---|---|
| `system.ping` | no | - | `{version, proto:1}` |
| `login.account` | no | - | `{username, full_name, uid, has_password, locked_out_secs, password_hint}` (the hint comes from `password_hint=` in the account file, empty when unset, control characters dropped) |
| `login.authenticate` | no | `{password}` | `{ok:true, token}` (128-bit random; marks Unlocked), or error `wrong_password` {fails, retry_secs} / `locked_out` {retry_secs} / `no_password` |
| `power.shutdown` `power.restart` `power.sleep` | no | - | `{}` |
| `session.lock` / `session.logout` | token | `t` | clears Unlocked, invalidates token |
| `account.set_password` | token, except a first password | `{old,new,t}` | runs `chpasswd` as root. A usable hash needs the token and re-verifies `old` (a wrong `old` counts toward the lockout). An EMPTY hash may be given its first password without a token, only by the console shell (peer exe `/usr/bin/springboard`, installed uid). A locked hash (`!`, `*`, `passwd -l`) is never changed here. `new` is at least 8 bytes, no `:`, newline or NUL |
| `update.check` `update.apply {kind:"os"\|"shell"}` `update.status` | yes | | apply writes the same recovery state, then the shell exits 3 |
| `store.install {id}` `store.uninstall {id}` | yes | | `store.install` takes only the app id: privd fetches the catalogue and the package itself and checks the catalogue hash. No user-staged path or client hash is ever opened by root |
| `update.rollback` | yes | - | restores the previous shell package generation locally (`pacman_update::rollback_local`) |
| `time.set_timezone {name}` `time.set_ntp {on}` | yes | | |
| `locale.set {lang,region}` | yes | | `localectl set-locale`: `lang` for LANG, `region` for the formats (time, numbers, money, paper, measures) |
| `net.*` | yes | | only what NetworkManager D-Bus + polkit cannot do |
| `system.boot_attempt {event:"begin"\|"stable"}` | `begin` no, `stable` token | | privd owns persistent `boot_attempts`; only a logged-in session clears the latch. The shell retries `stable` until it has a token |
| `backup.restore_apply` | yes | | applies the staged backup restore; replies `{files}` |
| `disk.loop_attach {path,read_only}` `disk.loop_detach {device}` | yes | | loop devices for mounting disk images (`privd/loopdev.rs`) |
| `user.add {name,full_name,admin}` `user.delete` `user.set_admin` `user.set_password` `group.add` `group.delete` `group.member` | yes | | Settings > Users & Groups (`privd/accounts.rs`). Names are plain lowercase identifiers, the installed account and system accounts (uid below 1000) are never touched, and the last administrator cannot be demoted or removed. Administrators are members of the sudo group |
| `system.set_names {computer, local}` | yes | | computer name (any text, 64 characters at most) and local host name label |
| `service.set {id,on}` | yes | | the sharing switches only: `remote_login` (`sshd.service`), `file_sharing` (`smb.service`), `screen_sharing` (`wayvnc.service`); nothing else can be enabled |
| `disk.erase_disk {device,fs,label}` `disk.erase_volume {device,fs,label}` `disk.partition_disk {device,partitions:[{size_mib,fs,label}]}` `disk.verify_volume {device}` `disk.repair_volume {device}` | yes | | Erase, partition and first-aid requests from `bifrost_core::disk_request`. Each is planned against a fresh snapshot of `/sys/block` and `/proc/mounts` (refused when mounted, or a partition where a disk is needed, or a disk where a partition is); the file system type of a checked volume comes from `blkid`. Result `{device, steps:[tool,...]}`; refusals are `bad_params` with the reason as the message. Runs as root with fixed argv |

Rules: validate all params; ids and paths canonicalised against allow-listed roots; fixed argv, no shell strings; every
call logged to the journal; passwords never logged.

Error codes: `unauthorized`, `locked`, `bad_params`, `wrong_password`, `locked_out`, `no_password`, `internal`.

## Authentication (inside privd)

- PAM service `/etc/pam.d/bifrost-login`: `auth required pam_faillock.so preauth`;
  `auth [success=1 default=bad] pam_unix.so`; `auth [default=die] pam_faillock.so authfail`;
  `auth sufficient pam_faillock.so authsucc`; `account required pam_unix.so`. faillock deny=5 unlock_time=300, plus an
  in-daemon exponential delay (old lockout_secs curve: 5 fails gives 60 s).
- libpam FFI (`#[link(name="pam")]`, conversation callback copies then zeroes the password). Fallback if libpam is
  missing at build time: shadow + libcrypt `crypt_r`, constant-time compare. Hash `!`/`*`/empty = no login, recovery path.
- The account is always the installed username from `/var/lib/springboard/account`; the client never names a user.
- The old PIN leaves the lock path. Settings passcode page becomes "Change login password..." (`account.set_password`).

## Lock flow

Boot: shell starts in `Screen::Lock` in login mode (avatar, full name, password field, Shut Down/Restart/Sleep). Enter
calls `login.authenticate`; on ok the shell keeps the token in memory, plays `Anim::Unlock`, starts autostart apps. Wrong
password shakes the window and shows the retry text. Any later lock (Ctrl+Cmd+Q, idle, lid) shows the same view and
requires the password; apps keep running. Logout/restart/shutdown go through privd. If privd is dead the shell shows
"Login service unavailable. Restart"; chord and ssh still work.

## Data split

- Root: `/var/lib/springboard` account, apps/, store-cache.json, boot_attempts, last_start/last_exit, crash.log,
  shell-version, run/*.sock, seed assets.
- User: `~/.local/share/springboard` (= `SPRINGBOARD_DATA`; fallback `platform::data_dir(false, ..)`): settings.ini,
  passcode.ini, wallpaper, Terminal, keychain, backup staging, notification DBs, `appdata/<id>`.
- Heartbeat/takeover state dir: `/run/bifrost/user/state` (user tmpfs); privd owns persistent `boot_attempts`.
  App packages stay read-only for the user and are installed by privd.

## Recovery and lab

- Recovery chord unchanged (user in `input`), shell exits 71, systemd restarts the session.
- `/etc/bifrost/no-login` (root creates over ssh) is the one explicit lab opt-in: privd reports `has_password:false` and
  accepts an empty password. `tools/midget.py recovery` prints whether it exists; `midget.py login-bypass on|off`.
- There is no automatic bypass (security review 2026-10-11): a failed PAM start at privd boot only logs loudly and the
  login window stays locked; root recovers over ssh with `login-bypass on`. PAM errors at run time (resource exhaustion,
  `AUTHINFO_UNAVAIL`, faillock trouble) only fail that attempt and never open the lock.
- Stage 1 (0.5.30): privd admits root only (the shell is root); the account is not in `bifrost-session`. The lockout
  counters are persistent (`/var/lib/springboard/privd-fails`, faillock `dir=/var/lib/faillock`), so a reboot from the
  login window does not reset them. With a login backend every unlock route (Return, the swipe, a notification tap)
  only brings the login window up; privd's token is the only unlock.
- Fail closed: without the user-data marker the shell starts on a fresh `~/.local/share/springboard`; root never
  chgrp/chmods or creates directories inside paths the user can write (they are made as the user via `setpriv`).
- The shell runs with `setpriv --no-new-privs` (skipped when `/usr/bin/bwrap` is setuid), `PR_SET_DUMPABLE=0` and
  `kernel.yama.ptrace_scope=1` (`sysctl-bifrost.conf`), because it holds the session token. Residual risk: the account is in
  group `input`, so a same-uid process (Term child, app) can read raw key events and so the password typed at the login
  window. A udev ACL on the evdev nodes or reading the chord through the compositor would close it; not done yet.
- privd loads libpam with `dlopen`, so no other binary links it; it serves at most 16 connections at once with a 30 s
  idle timeout. The password check runs with the auth lock released; overlapping checks are refused.
- Guard rollback restores the previous springboard-shell package including units (verify the rollback tarball in
  `packaging/arch/recovery/system.rs`).
- Root ssh and the lab key untouched (`PermitRootLogin prohibit-password`).
- Control/MCP TCP port: `unlock` removed or privd-gated. `bifrost pc midget unlock` types the password stored in
  `~/.config/bifrost/midget-login` (lab only), replacing `key enter`.

## Migration (`bifrost-migrate-user`, oneshot, Before=springboard.service)

Idempotent; marker `/var/lib/springboard/.user-session-v1`. If missing: stop when the account is invalid or root; copy
(`cp -a --reflink=auto`, never move) user-state files to `~/.local/share/springboard`; `chown -R -P --no-dereference` to
uid:gid, plus the user's per-app data dirs; list originals in `/var/lib/springboard/.pre-user-backup-<ts>.list`; write
the marker. Rollback = delete marker and new dir (old files stay). Fail closed: on error no marker; the session script
then launches the shell as the user with `SPRINGBOARD_DATA` unset and the legacy dir readable via group
`bifrost-session` (mode 770). A passwordless/locked account (`passwd -S` L/NP) gets `has_password:false` and the window
asks to "Set a password".

## Names

Env: `SPRINGBOARD_PRIVD`, `SPRINGBOARD_DATA`. PAM service `bifrost-login`. Group `bifrost-session`. Markers:
`/var/lib/springboard/.user-session-v1`, `/etc/bifrost/no-login`.
