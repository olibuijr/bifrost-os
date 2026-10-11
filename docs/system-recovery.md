# System filesystem and update recovery

New PC installations default to Btrfs, with zstd:3 compression and noatime.
The EFI system partition remains FAT32. The installer creates:

| Subvolume | Mount | Purpose |
|---|---|---|
| @system | / | OS files and package database |
| @home | /home | User documents and account data |
| @state | /var/lib/springboard | Apps, preferences and application data |
| @recovery | /var/lib/bifrost-recovery | Independent recovery records |
| @cache | /var/cache | Download caches |
| @logs | /var/log | Logs |

EFI cmdline selects rootflags=subvol=@system. genfstab records the mounted
subvolumes by UUID. Live ISO and target package lists include btrfs-progs.
The installer loads the Btrfs kernel module before partitioning, explicitly
mounts Btrfs, and includes its module in the installed initramfs. Live assets
are staged without directory nesting. The bootstrap package uses the recovery subvolume because
arch-chroot replaces /run and /tmp. Existing ext4 systems require an offline reinstall
or migration; changing the package alone does not change their filesystem.

On the installer-defined Btrfs layout, managed updates snapshot @system into
@rollback and save the matching kernel/initramfs on the ESP. Home, app state,
recovery records, caches and logs remain separate. The package database belongs
to the system snapshot. The static Rust recovery engine lives in @recovery and
runs independently of the shell.

Before the package transaction the EFI pending marker selects the previous
kernel/root if power is interrupted. A completed transaction permits one boot
attempt of the new generation. A consumed attempt or EFI load failure selects
the previous kernel and @rollback. Recovery then clones the previous root back
to @system, restores the matching boot files and reboots normally. Failed roots
are retained as @failed-* for diagnosis.

Probation begins only after reboot. Acceptance requires 30 uninterrupted
seconds with the installed shell running, a Wayland socket, responsive state
requests and an advancing heartbeat. After 90 seconds without acceptance the
guard requests a reset into the previous generation. Runtime/reboot watchdogs
are configured for 60/120 seconds where hardware supports them. These are
startup/event-loop checks, not proof that every input device works.

On ext4 the guard still protects shell-package updates only and refuses base
upgrades. Direct administrator pacman commands are outside this policy. EFI
loader/firmware corruption and disk failure are outside snapshot recovery.

Read status with `bifrost pc midget recovery` or `bifrost-update-guard status`.
Explicit administrator recovery is `sudo bifrost-update-guard rollback`.
The unit exits when probation finishes, so it adds no permanent polling thread.
The saved package remains available after acceptance.

Verification: static guard unit tests and EFI boot-choice tests passed. Previous
ext4 Midget fault testing restored shell 0.4.24 after a failed executable;
0.4.25 then passed responsive-session probation. Fresh Btrfs installation completed on Midget, followed by a guarded published
0.4.27 update and reboot into @system. Whole-kernel/root corruption fallback
has not yet been fault-tested on Midget. The user requested clean stock state;
previous app/user data remains only in the private host backup.
