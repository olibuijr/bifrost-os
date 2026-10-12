# Live boot: no black gap before the installer

Owner report: "loading screen becomes black for too long before installer, not smooth".

## Causes found (by reading the path)

1. The command line was already text-free (`tools/prepare-mkarchiso.py`: quiet, loglevel=3, no cursor, no fbc/psr),
   so it is not a cause. Debug ISOs override it on purpose.
2. `bifrost-installer.service` ran with `TTYReset=yes` and `TTYVHangup=yes` on tty1. Both reset/hang up the console,
   and fbcon answers by clearing the framebuffer: the loader's frame turned black until the installer drew.
3. `src/bin/installer.rs` searched disks for an unattended preset (`unattended::find_stash`, mounts every disk, seconds)
   BEFORE opening the display, so no frame was drawn during that time.

## Fixes

- Service: `TTYReset=no`, `TTYVHangup=no`. The EFI frame stays in simpledrm until KMS opens.
- Installer: open the display, present the boot frame (`bootweave_frame::Frame::boot`), then search for the preset,
  load fonts/images and build the app; the existing one-time cross-fade into Welcome is unchanged. The headless
  fallback (no display) still searches the preset.

## Midget verification (run by the lead, not by this branch)

1. Build and deploy the lab ISO, then reinstall/boot it on Midget with `bifrost pc midget` wipe flow.
2. `python3 tools/boot-frames.py` while booting: expect no all-black frame between the loader frame and the Welcome
   fade (gap under one frame interval), and `cat /proc/cmdline` showing `quiet loglevel=3`.
3. `cat /run/bifrost/boot-marks`: `installer-display` then `installer-first-ui` close together; `tools/boot-bench.py`
   shows no extra modeset lines.
