# Theme 20: process signal, working directory and root directory events

Branch `parity/theme-20`, built on wave 1 (`parity/integration`). Models only: pure Rust, std only, no hardware
access, no unsafe, no new crates. Nothing here is compiled or run yet (no-build rule for this round), so the tests
are written but unexecuted.

## What was built

| Module | Crate | Purpose |
|---|---|---|
| `springboard/core/src/notify_center.rs` | bifrost-core | Name-keyed notifications with integer tokens: `register`, `register_fd` (a datagram pair, one byte per post), `check` (polled, clears the mark), `suspend`/`resume`/`cancel`/`is_valid`, `post`, per-name `set_state`/`state`, and the `SYSTEM_EVENTS` name list. |
| `springboard/sdk/src/process_events.rs` | bifrost-app (sdk) | `ProcessEvent` records (signal, chdir, chroot, trace, remote-thread create, suspend/resume, mount, unmount, remount), `EventKind`, and `EventSubscriptions` (subscribe, unsubscribe, process mutes, `accepts`). A mount has no source, so a mute never drops it. |
| `springboard/core/src/disk_events.rs` | bifrost-core | `Disk` snapshots, `diff(prev, next)` giving `Appeared` / `Disappeared` / `DescriptionChanged` in a fixed order, and `Approvals`, an ordered gate chain for mount, unmount and eject where the first refusal wins (`Dissent` carries the reason). |

`pub mod` lines added: `springboard/core/src/lib.rs` (`notify_center`, `disk_events`) and
`springboard/sdk/src/lib.rs` (`process_events`).

Gaps left open on purpose: no POSIX signal delivery for notifications (`notify_register_signal`), no kernel
event source for process or mount events, no shell broadcast of the system names, no disk polling source, and no
client or cache clearing. Every row is therefore `partial`, not `parity`.

## Official API names mirrored

- Darwin Notify: `notify_check`, `notify_register_file_descriptor`, `notify_set_state`, `notify_get_state`,
  `notify_suspend`, `notify_resume`, `notify_cancel`, and the system event name constants (time zone, clock, volume
  mount and unmount, low disk space, network, host name, disk cache, log database).
- Endpoint Security: `es_event_signal_t`, `es_event_chdir_t`, `es_event_chroot_t`, `es_event_trace_t`,
  `es_event_remote_thread_create_t`, `es_event_proc_suspend_resume_t`, `es_event_mount_t`, `es_event_unmount_t`,
  `es_event_remount_t`, `es_subscribe`/`es_unsubscribe`/`es_unsubscribe_all`, `es_mute_process`/`es_unmute_process`.
- Disk Arbitration: `DARegisterDiskAppearedCallback`, `DARegisterDiskDisappearedCallback`,
  `DARegisterDiskDescriptionChangedCallback`, `DARegisterDiskMountApprovalCallback`,
  `DARegisterDiskUnmountApprovalCallback`, `DARegisterDiskEjectApprovalCallback`, `DADissenter`.

## REA evidence

Rendered official pages, recorded with `tools/parity/rea-page.sh` (Chromium on 127.0.0.1:9333):

- Darwin Notify: `ev_f3dfdae4c1014d2bbb17b934831e87d851678e3db62c93c7ade5b3fe5a76aad4`
  (https://developer.apple.com/documentation/darwinnotify)
- Endpoint Security: `ev_d3807408734284150490e496c8de86349fb782435ffc14688d892deecf55ed10`
  (https://developer.apple.com/documentation/endpointsecurity)
- Disk Arbitration: `ev_70caf273832a8eefe32be0f7502f6dbc998713ac0ae783754e686078598a0470`
  (https://developer.apple.com/documentation/diskarbitration)

The matrix already carries rea_evidence ids for these frameworks (`ev_525d...` for Darwin Notify, `ev_7c40...` for
Endpoint Security, `ev_44fd...` for Disk Arbitration). Those were not re-recorded here.

## Matrix rows closed

Feature strings are copied from `docs/parity-matrix/data/<slug>.json`.

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| endpoint-security | Process signal, working directory and root directory events | partial | `springboard/sdk/src/process_events.rs` (`ProcessEvent::Signal`, `Chdir`, `Chroot`) |
| endpoint-security | Interprocess events (trace, remote thread creation, suspend and resume) | partial | `springboard/sdk/src/process_events.rs` (`Trace`, `RemoteThreadCreate`, `SuspendResume`) |
| endpoint-security | Subscribing to events | partial | `springboard/sdk/src/process_events.rs` (`EventSubscriptions::subscribe`, `unsubscribe`, `accepts`) |
| endpoint-security | Mount, unmount and remount events | partial | `springboard/sdk/src/process_events.rs` (`ProcessEvent::Mount`, `Unmount`, `Remount`) |
| endpoint-security | Muting and clearing the cache | partial | `springboard/sdk/src/process_events.rs` (`mute_process`, `unmute_process`, `unmute_all`; no cache clear) |
| disk-arbitration | Disk appeared, disappeared and description-changed callbacks (DARegisterDiskAppearedCallback, DARegisterDiskDisappearedCallback, DARegisterDiskDescriptionChangedCallback) | partial | `springboard/core/src/disk_events.rs` (`diff`, `DiskEvent`) |
| disk-arbitration | Mount and unmount approval (DARegisterDiskMountApprovalCallback, DARegisterDiskUnmountApprovalCallback, DARegisterDiskEjectApprovalCallback) | partial | `springboard/core/src/disk_events.rs` (`Approvals`, `Operation`, `Dissent`) |
| darwin-notify | Polling a registration token (notify_check) | partial | `springboard/core/src/notify_center.rs` (`NotifyCenter::check`) |
| darwin-notify | File descriptor delivery (notify_register_file_descriptor) | partial | `springboard/core/src/notify_center.rs` (`NotifyCenter::register_fd`) |
| darwin-notify | Per-name state values (notify_set_state, notify_get_state) | partial | `springboard/core/src/notify_center.rs` (`set_state`, `state`) |
| darwin-notify | Token lifecycle (notify_cancel, notify_suspend, notify_resume, notify_is_valid_token) | partial | `springboard/core/src/notify_center.rs` (`cancel`, `suspend`, `resume`, `is_valid`); extends the existing `springboard/sdk/src/notification.rs` row |
| darwin-notify | System event names (kNotifyTimeZoneChange, kNotifyClockSet, kNotifyVFSMount, kNotifyVFSUnmount, kNotifyVFSLowDiskSpace, kNotifySCNetworkChange, kNotifySCHostNameChange, kNotifyDSCacheInvalidation*, kNotifyASLDBUpdate) | partial | `springboard/core/src/notify_center.rs` (`SYSTEM_EVENTS`, names only; the shell does not post them yet) |

Still gap after this theme: Endpoint Security client creation, authorization, socket, pseudoterminal, code-signing
and memory-mapping events; Disk Arbitration session, claims, commands and lookup; Darwin Notify signal delivery
(`notify_register_signal`).
