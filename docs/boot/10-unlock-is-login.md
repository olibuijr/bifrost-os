# Boot task 10: unlock is login

Contract: `docs/boot-architecture.md`, "Unlock is login" (and the phase table, row 5). Task spec:
`tasks/boot/10-unlock-is-login.md`. Branch `boot/10-unlock-is-login`.

Status: code written and reviewed by reading only. Nothing here has been compiled or run (the boot team does not build;
the lead builds once all waves are merged, then checks on Midget). Read "Open items" before testing.

## What is implemented

| Piece | File | What it does |
|---|---|---|
| Loader wiring | `springboard/bootweave/efi/src/main.rs` | After the boot options are read, runs `unlock::run(user)` with the account from `bifrost.user=` on the command line, and publishes the volume key on success. |
| Unlock screen | `springboard/bootweave/efi/src/unlock.rs` | Finds the partition whose first sector is a LUKS2 header, reads the JSON area, draws `Screen::Unlock` through `bootweave-frame` (name, dots, failed flag), reads keys through UEFI `SimpleTextInput`, and returns the volume key once a passphrase opens a keyslot. Enter tries, Backspace deletes, a wrong passphrase keeps the screen with the failed flag. The passphrase is wiped from memory after each try. |
| LUKS2 unlock | `springboard/bootweave/efi/src/luks/mod.rs`, `luks/json.rs` | Binary header (magic, version 2, size), JSON area parser (bounded nesting, LUKS2 string numbers), keyslot KEK by PBKDF2-HMAC-SHA-256, AES-XTS keyslot decryption (sector numbers from the area start), LUKS1 anti-forensic merge, digest check by PBKDF2. Segment offset and the `aes-xts-plain64` / 512-byte check. |
| Crypto | `springboard/bootweave/efi/src/crypto/` | `sha256.rs` (FIPS 180-4), `aes.rs` (AES-128/256, S-box computed at compile time), `xts.rs` (IEEE 1619, whole 512-byte sectors), `mod.rs` (HMAC-SHA-256, PBKDF2-HMAC-SHA-256). No unsafe, no dependencies. |
| Handover | `springboard/bootweave/efi/src/handover.rs` | Writes the one-time EFI variable `BifrostVolumeKey` (GUID `b9a7c3e1-3f2d-4e8a-9c61-0b5e7d2a4f10`), attributes BOOTSERVICE_ACCESS and RUNTIME_ACCESS and no NON_VOLATILE, so it is volatile and gone at power-off. Payload: segment byte offset (u64 LE), then the 32 or 64 byte volume key. |
| initramfs hook | `packaging/arch/initcpio/hooks/bifrost-unlock`, `install/bifrost-unlock` | Reads the variable through efivarfs, takes the offset and key, deletes the variable at once (`chattr -i`, `rm`), opens the LUKS2 partition as a plain dm-crypt mapping `bifrost-root` (same cipher, key and offset), and writes the account name to `/run/bifrost/autologin` in the new root from a late hook. |
| Shell autologin | `springboard/src/shell/login.rs` (`AUTOLOGIN`, `autologin_name`, `take_autologin`), `springboard/src/shell/mod.rs` (`Shell::new`) | At shell start, if the autologin file exists and holds a valid account name, the file is consumed and the desktop starts on `Screen::Home`; otherwise the Lock screen as before. |
| Tests | in each file above | SHA-256 (FIPS "", "abc", two-block), HMAC (RFC 4231 cases 1 and 2), PBKDF2 (the two standard `password`/`salt` vectors and a multi-block check), AES (FIPS 197 C.1 and C.3, S-box values), XTS (IEEE 1619 vector 1, round trip, GF multiply), JSON (nesting, NUL padding, trailing text, escapes, bounded depth), LUKS header sizes and magic, base64, anti-forensic merge and diffusion, segment checks, missing-disk error path, handover payload layout, autologin name validation. |

## How it maps to the contract

- "The loader shows the unlock screen in the boot frame": `Screen::Unlock` from `bootweave-frame`, drawn with `render`
  into a GOP blt at the panel's native mode. The frame's `render` is still `todo!("boot-02")` on the shared branch, so
  this path panics until task 02 lands.
- "Unlocks the LUKS key slot with the passphrase in EFI memory": the passphrase lives in a `String` in loader memory
  and is wiped after the try.
- "Hands the key to the initramfs through a one-time EFI variable": `handover::publish`, deleted by the hook before the
  root is opened.
- "Marks the session for automatic login of that user": the hook writes the account name, the shell consumes it.
- "No second login prompt, no text console prompt": the Unlock screen is the only prompt; the shell skips the Lock
  screen when autologin is present.

## Open items

1. The LUKS2 keyslot KDF is PBKDF2 only. cryptsetup's default for LUKS2 is argon2id, which this loader does not
   implement, so an encrypted install must be created with `--pbkdf pbkdf2`. The installer's encryption option is not
   in this branch's scope; it must be changed by the installer owner (see the command below).
2. The anti-forensic diffusion is written from the LUKS1 description (SHA-256 of big-endian block index and each
   digest-sized block). It is verified only by its own structural test, not by a reference vector. If the Midget
   unlock always reports a wrong passphrase with the right one, check this function first.
3. The loader's `open_protocol_exclusive` on the partition's BlockIO assumes no other driver holds the handle open.
   If the open fails on Midget, switch to a non-exclusive open.
4. The hook writes `/new_root/run/bifrost/autologin` in the late hook. This is only seen by the desktop if the
   initramfs moves `/run` into the new root (mkinitcpio's `init` does this on current releases; check). If `/run` is
   mounted fresh by the init system, the file is hidden, the shell falls back to the Lock screen, and nothing is
   unsafe, but the autologin does not happen. Verify on Midget with `ls /run/bifrost` in the session.
5. The Unlock screen uses the Dark appearance; the user's Light/Dark choice is not read yet (the timing record would
   carry it).
6. The hook never sets the variable again, so a second boot needs a fresh unlock; that is intended.
7. A non-LUKS or missing-variable boot skips everything; a LUKS header this loader cannot open (argon2, other cipher)
   returns no volume and the boot continues to the Lock screen path, which is a regression in the boot flow to watch
   for.

## Commands for the installer owner

The encrypted root must be created with the parameters the loader accepts:

    cryptsetup luksFormat --type luks2 --pbkdf pbkdf2 --pbkdf-force-iterations 1000000 \
        --hash sha256 --cipher aes-xts-plain64 --key-size 512 /dev/<partition>

## Midget verification (for the lead)

Prerequisites: the boot frame (task 02) renders, the installer creates the LUKS2 root above, the initramfs includes the
`bifrost-unlock` hook before `filesystems`, and the kernel command line has `root=/dev/mapper/bifrost-root` and
`bifrost.user=<account>`.

1. Install the encrypted Midget image (only the installer erases Midget). Confirm `cryptsetup luksDump` shows
   `pbkdf2` in the keyslot and digest, and `aes-xts-plain64` with 512-bit keys.
2. Power on. The Unlock screen appears in the boot frame with the account name and an empty dot field. No text is
   printed.
3. Type a wrong passphrase and press Enter. The screen stays with the failed state. Retype the correct passphrase and
   press Enter. The boot continues.
4. The desktop appears without a login prompt (`Screen::Home`, no Lock screen).
5. After boot, check that the variable is gone: `ls /sys/firmware/efi/efivars | grep BifrostVolumeKey` prints nothing.
6. `cryptsetup status bifrost-root` shows the mapping is active. `journalctl -b | grep -i bifrost` shows the hook ran
   and did not print the key.
7. `ssh midget` then check `/run/bifrost` is empty after the shell started (the file was consumed).
8. Power off and on: the unlock screen appears again (the variable is volatile).

Record the Midget screenshots of steps 2 and 4 with `bifrost pc midget screen PATH`.
