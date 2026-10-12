# Foundation audit (task 10), 2026-10-12

Scope read: bifrostd, netd, updated, `src/bin/privd.rs` (a thin launcher; request validation lives in `src/privd/`, owned elsewhere).

| ID | category | severity | file:lines | problem | fix | test |
|---|---|---|---|---|---|---|
| FND-1 | security | medium | bifrostd/src/lib.rs scan | A service manifest `exec` may be a symlink resolving outside the package dir (`..` was filtered, symlinks were not) | `exec_inside` requires the canonical path to stay in the package dir. FIXED | `exec_symlink_outside_the_package_is_rejected` |
| FND-2 | security | medium | bifrostd/src/lib.rs bind | Service sockets are bound with the default umask; any local user may connect | chmod 0660 and set the group; needs a group decision. OPEN | connect as another uid |
| FND-3 | security | medium | updated/src/main.rs Configure | Any socket client can set the update `url`; integrity then rests on the manifest hash from that same server | Restrict Configure to the shell uid via SO_PEERCRED. OPEN | peer-uid test |
| FND-4 | robustness | low | netd/src/main.rs, updated/src/main.rs serve | One thread per connection, uncapped; Core mutex held across blocking writes (500 ms timeout each) | cap connections, write outside the lock. OPEN | open many sockets |
| FND-5 | duplication | low | core/render font and image loaders | Not audited in depth | OPEN | n/a |
| FND-6 | stale | low | tools/*.py | Unreferenced scripts not enumerated | OPEN | n/a |
