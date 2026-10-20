# Repository signing key (public half)

This directory holds the public key that signs the BifrOSt package repository (`springboard-shell` and the `bifrost`
database). Only the public key is ever committed. The secret key lives in a GnuPG home under `out/lab/gnupg` (ignored by git)
on the publishing machine.

Expected file: `bifrost-repo.gpg` (a binary or ASCII-armored export of the public key). While it is absent the package ships
no key, the installer logs a warning and the repository is published unsigned.

## Generate the key (once, on the publishing machine)

    export GNUPGHOME=$PWD/out/lab/gnupg
    install -d -m700 "$GNUPGHOME"
    gpg --batch --quick-generate-key "BifrOSt Repository <repo@bifrost.olibuijr.com>" ed25519 sign never
    gpg --list-keys --with-colons | awk -F: '$1=="fpr"{print $10; exit}'      # the fingerprint
    gpg --export --armor <fingerprint> > packaging/arch/keys/bifrost-repo.gpg

Then publish with the key:

    export BIFROST_REPO_SIGN_KEY=<fingerprint>
    export BIFROST_REPO_GNUPGHOME=$PWD/out/lab/gnupg     # the default
    bifrost pc publish

`tools/pc-publish.py` signs every package (`makepkg --sign`) and the database (`repo-add --sign`) and publishes the `.sig`
files next to them. A configured key that cannot sign stops the publish; with no key the publish continues, unsigned, with a
warning.

## Where the key goes

- `springboard-shell` installs it at `/usr/share/bifrost/keys/bifrost-repo.gpg` (`packaging/arch/package-files.sh`); its
  scriptlet imports it into pacman's keyring and signs it locally (`pacman-key --add`, `--lsign-key`).
- The installer reads it from the live medium at the same path (`tools/build-iso.sh` copies this directory there) and imports
  it into the target before the first `pacman -Sy`.
- The installed `[bifrost]` repository uses `SigLevel = Optional TrustedOnly` (phase 1). When every machine holds the key,
  switch the installer to `Required DatabaseRequired` (phase 2, `docs/installer-and-boot.md`, "Package signing").
