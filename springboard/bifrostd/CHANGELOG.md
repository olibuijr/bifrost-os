# bifrostd changelog

## Unreleased
- Also supervises the system services the OS package ships under `/usr/lib/bifrost/services/<id>/` (read-only); a store-installed `<root>/apps/<id>/current` wins only when its manifest version is newer. Sockets stay `<root>/run/<id>.sock`.
- New manifest field `user` (`root` by default, `session`, or a system user name): the service is started with the user's uid, gid and supplementary groups (`setgroups`, `setgid`, `setuid`, no new privileges) and `HOME`/`USER`/`XDG_RUNTIME_DIR`; its socket and data directory belong to that user, every socket is mode 0600. A `session` service waits until a session user is known and is never started as root.

## 0.1.0 — 2026-10-05
- First version of the service supervisor: restarts services with backoff and swaps in new versions only after they answer a health check.
