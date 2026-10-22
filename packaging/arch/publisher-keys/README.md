# Publisher keys

Public keys (ML-DSA-65, one raw key per `*.pub` file) of the people who sign PC App Store packages. `package-files.sh`
installs every `*.pub` here to `/usr/share/bifrost/publisher-keys`, and the shell reads that directory together with
`/etc/bifrost/publisher-keys` (the keys an administrator adds). Once any key is present, a store download must carry a
detached `<package>.sig` from one of them, or it is discarded before anything is unpacked.

Make a pair with `springboard --publisher-keygen DIR NAME`: commit `NAME.pub` here, keep `NAME.key` secret, and publish
with `BIFROST_PUBLISHER_KEY=DIR/NAME.key tools/pc-app.py publish APP_DIR`, which signs the package. Do not commit a key
before the publish script uploads the `.sig` next to every package, or store installs start failing as unsigned.
