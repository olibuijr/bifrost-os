# 011 — Phase 9: strengthen update trust without breaking recovery

Status: investigation may start now (see 002 item 4). Implementation depends on 003, 005, and 007. Parent: `001`.

## Problem (verified)

- `springboard/src/shell/update.rs:27` embeds `tools/update-hmac.key` with `include_str!`. Line 24 says plain-`http://` manifests are signed with it (`tab publish` writes `latest.json.sig`). Because the key ships in the client, HMAC here is not an authenticity proof against a client that holds the key.
- PC pacman updates use a separate recovery guard. App installation verifies catalogue-provided hashes.
- Do not treat package recipe `sha256sums=SKIP` as proof that the installed repository lacks signature checks. Inspect the actual repository configuration.

## Steps

1. Map the trust chain for: PC system packages and PC app packages. For each: signatures, TLS reliance, hash provenance, trust roots, and known bypasses. Write this to `springboard/docs/platform-maturity.md`.
2. Decide open decision 1 in `001` (legacy plain-HTTP OTA) before changing that path.
3. Replace client-shared HMAC authentication for release authenticity with public-key verification.
4. Adopt a maintained TUF implementation or a justified signed-metadata design. Include expiry, trusted version persistence, key rotation, and compromise recovery. Reference: https://theupdateframework.io/
5. Keep private signing keys out of source and clients. Do not print existing key material.
6. Define a bootstrapped migration for existing installs. Keep distinct policies for rejecting a malicious old offer and for intentionally restoring a locally trusted failed-update generation.
7. Preserve the static recovery engine, matching root and kernel rollback, ext4 restrictions, and probation (`docs/system-recovery.md`, `packaging/arch/recovery/`).
8. Stage fault injection in disposable environments. Physical Midget corruption tests require explicit authorization.

## Acceptance

- Tampered payload or metadata, stale metadata, replayed releases, unknown keys, and interrupted trust-state writes are rejected (test each).
- Documented key rotation works with non-production signing fixtures.
- A failed startup still restores the prior trusted generation.
- Outstanding physical recovery acceptance is recorded separately.
