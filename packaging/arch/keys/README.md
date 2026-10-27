# Repository signing key (public half)

This directory holds the public key that signs the BifrOSt package repository (`springboard-shell` and the `bifrost`
database). Only the public key is ever committed. The secret key lives in a GnuPG home under `out/lab/repo-signing` (ignored
by git) on the publishing machine.

File: `bifrost-repo.gpg`, the ASCII-armored export of the public key. Its fingerprint is the key `tools/pc-publish.py`
signs with, so there is nothing to export into the environment. The package build, the ISO build and the installer all refuse
to continue without this file.

## Create the key (once, on the publishing machine)

    export GNUPGHOME=$PWD/out/lab/repo-signing
    install -d -m700 "$GNUPGHOME"
    gpg --batch --quick-generate-key "BifrOSt Repository <repo@bifrost.olibuijr.com>" ed25519 sign never
    fpr=$(gpg --list-keys --with-colons | awk -F: '$1=="fpr"{print $10; exit}')
    gpg --export --armor "$fpr" > packaging/arch/keys/bifrost-repo.gpg

The key committed here was first generated in `out/lab/gnupg`; moving that directory to `out/lab/repo-signing` keeps it
(fingerprint `0E6ED141285756654C6C347A02421144AA8469AE`). A new key needs a bootstrap release, see below.

## Publishing

    bifrost pc publish

signs every package (`makepkg --sign`) and the database (`repo-add --sign`), checks that every signature exists, and
publishes the `.sig` files next to the files. If the secret key is not in `BIFROST_REPO_GNUPGHOME` (default
`out/lab/repo-signing`) the publish stops before it builds anything. It never falls back to unsigned. The one explicit
exception is `bifrost pc publish --bootstrap-unsigned`, used once to bring the key to machines that do not trust it yet.

## How installed machines come to trust it

pacman rejects a signed database whose key is not in the machine's keyring, even under `SigLevel = Optional`. So the first
release that carries the key has to be accepted without a signature, and every release after it is signed:

1. Bootstrap release: `bifrost pc publish --bootstrap-unsigned`, then `bifrost pc midget ota`. The old `[bifrost]` setting
   (Optional) accepts it. `springboard-shell` installs the key at `/usr/share/bifrost/keys/bifrost-repo.gpg`; its scriptlet
   (`bifrost_repo_trust` in `springboard.install`) imports and locally signs it, and only then rewrites the `[bifrost]`
   section of `/etc/pacman.conf` to `SigLevel = Required DatabaseRequired`.
2. Every release after that is a plain `bifrost pc publish` (signed). The machine verifies it against the key from step 1.

A machine that already has the key imported and `Required DatabaseRequired` (Midget, switched by hand on 2026-10-20) skips
step 1 and takes signed releases directly.

- The installer reads the key from the live medium at the same path (`tools/build-iso.sh` copies this directory there and
  fails without it) and imports it into the target before the first `pacman -Sy`. New installs get `Required
  DatabaseRequired` from the start; an installer without the key stops with an error.
- If the key import fails on an upgrade, the scriptlet leaves the `[bifrost]` setting as it is and prints why.
