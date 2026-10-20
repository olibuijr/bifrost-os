# bifrostd changelog

## Unreleased
- Also supervises the system services the OS package ships under `/usr/lib/bifrost/services/<id>/` (read-only); a store-installed `<root>/apps/<id>/current` wins only when its manifest version is newer. Sockets stay `<root>/run/<id>.sock`; the audio socket is mode 0666 so sandboxed apps (own accounts) can connect.

## 0.1.0 — 2026-10-05
- First version of the service supervisor: restarts services with backoff and swaps in new versions only after they answer a health check.
