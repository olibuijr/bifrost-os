# 007 — Phase 5: separate graphical session privilege from system authority

Status: planned. Depends on: 005 (boundaries from 006 must be preserved). Parent: `001`.

## Problem (verified)

- `springboard/src/bare/app_policy.rs` checks identity and refuses uid 0 for app launch (line 45). It shells out to `/usr/bin/chown` (line 147), `/usr/bin/bwrap` (173), `/usr/bin/systemd-run` (305), and `/usr/bin/systemctl` (327).
- `bare/app_manager.rs` attaches processes to cgroups and enforces limits. Its resource sampler runs on a worker.
- Other root-only operations live in the shell, `updated`, `netd`, `audiod`, and `bifrostd`. These have not been inventoried.

## Steps

1. Write the inventory first, in `springboard/docs/platform-maturity.md`: every root-required shell operation, open privileged descriptor, system command, service socket, and packaging assumption. Include KMS and input access, account and state ownership, cgroups, update recovery, networking, power, and Remote sharing. Separate PC session needs from the minimal ARM PID1 and Samsung bring-up needs.
2. Implement narrow authenticated brokers, one at a time. Reuse existing daemons and the recovery guard where their boundaries fit (see `docs/system-recovery.md`).
3. Each broker endpoint has: a fixed operation schema, caller authorization from the channel (not caller fields), bounded worker execution, structured results, and a defined lifetime.
4. No broker provides an arbitrary command, unit, mount, or executable API.
5. Define descriptor delegation and compositor privileged-protocol restrictions for native apps.
6. Migrate the PC graphical shell to the local session identity only after its essential operations work through brokers. Provide packaging and service-ownership changes and a tested upgrade transition.
7. Recovery stays independent of the shell and fails closed when brokers are unavailable.
8. Keep Remote's intentional service lifetime beyond closing its UI.

## Acceptance

- The PC graphical shell runs non-root. App, update, power, and network flows work.
- Spoofed callers are rejected (negative tests for identity, unauthorized operations, and descriptor leakage).
- Broker failure produces recoverable UI errors.
- Physical Midget acceptance is recorded separately and only with the required authorization.
