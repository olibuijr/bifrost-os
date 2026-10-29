//! Kernel command line policy for the loader: the text-free baseline and the verbose transform for a held Ctrl+V
//! (docs/boot-architecture.md, "Text-free on every path"). Pure string logic with no UEFI calls, so the host can
//! test it (`rustc --edition 2021 --test springboard/bootweave/efi/src/cmdline.rs`; the file uses only alloc).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// The baseline when no boot.cfg or cmdline.txt is readable: the same text-free flags the installer writes
/// (`springboard/src/installer/plan.rs`, `cmdline_with`). `splash` is gone: nothing draws a boot splash through the
/// kernel any more, the loader's frame is the only picture. `threadirqs` runs interrupt handlers as kernel threads,
/// so the audio server can raise the sound card's IRQ thread to SCHED_FIFO (docs/audio/21-lowest-latency-path.md).
/// It is not a text-free key, so a verbose boot keeps it.
pub const DEFAULT_CMDLINE: &str = "initrd=\\arch\\boot\\x86_64\\initramfs-linux.img quiet loglevel=3 \
    systemd.show_status=false rd.systemd.show_status=false udev.log_level=3 rd.udev.log_level=3 \
    systemd.log_target=journal vt.global_cursor_default=0 threadirqs";

/// Tokens the verbose transform removes before it adds its own (every token the baseline sets).
const TEXT_FREE_KEYS: &[&str] = &[
    "quiet",
    "splash",
    "loglevel=",
    "systemd.show_status=",
    "rd.systemd.show_status=",
    "systemd.log_target=",
    "systemd.log_level=",
    "udev.log_level=",
    "rd.udev.log_level=",
    "vt.global_cursor_default=",
];

/// Added in place of the removed tokens for a verbose boot: kernel messages at debug level and systemd's status
/// lines on the console, for support. Only for the one boot that was started with Ctrl+V held.
const VERBOSE_TOKENS: &str = "loglevel=7 systemd.show_status=true rd.systemd.show_status=true systemd.log_level=info udev.log_level=info";

fn is_text_free_token(token: &str) -> bool {
    TEXT_FREE_KEYS.iter().any(|key| token == *key || (key.ends_with('=') && token.starts_with(key)))
}

/// Whether the loader should look for an encrypted root at all: only a line that names a mapped root
/// (`root=/dev/mapper/..`) or carries the explicit `bifrost.unlock` token does. Every other boot skips the scan of
/// every block device, which would otherwise run before anything is painted.
pub fn wants_unlock(line: &str) -> bool {
    line.split_whitespace().any(|t| t == "bifrost.unlock" || t.starts_with("root=/dev/mapper/"))
}

/// The panel size a `video=` token pins (`video=DP-1:2560x1440@165` or `video=2560x1440`), `None` when the line has
/// none (`video=eDP-1:d`, `video=efifb:off`, no token). The kernel's display driver then sets that mode itself.
pub fn video_size(line: &str) -> Option<(u32, u32)> {
    line.split_whitespace().filter_map(|t| t.strip_prefix("video=")).find_map(|spec| {
        // [<connector>:]<xres>x<yres>[M][R][-<bpp>][@<refresh>][i][m][e|D|d]
        let spec = spec.rsplit_once(':').map_or(spec, |(_, mode)| mode);
        let (w, rest) = spec.split_once('x')?;
        let h: String = rest.chars().take_while(char::is_ascii_digit).collect();
        match (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?) {
            (0, _) | (_, 0) => None,
            size => Some(size),
        }
    })
}

/// Whether the loader sets the largest GOP mode. Each mode change makes the monitor retrain its link (a visible
/// blink), so when `video=` pins a mode the kernel will change the mode once anyway, and the loader leaves the
/// firmware's mode alone: one change from firmware to desktop instead of two. Without `video=` the loader's change to
/// the largest mode is the boot's only one (the kernel and the window server keep it).
pub fn should_set_native_mode(current_area: usize, largest_area: usize, kernel_sets_mode: bool) -> bool {
    !kernel_sets_mode && largest_area > current_area
}

/// Returns `line` with every text-free token replaced by the verbose set. Every other token (root, subvolume,
/// the initrd path, the hardware flags) is kept in order.
pub fn verbose(line: &str) -> String {
    let kept: Vec<&str> = line.split_whitespace().filter(|token| !is_text_free_token(token)).collect();
    let mut out = kept.join(" ");
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(VERBOSE_TOKENS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_runs_only_for_a_mapped_root_or_the_token() {
        assert!(wants_unlock("quiet root=/dev/mapper/bifrost-root rw"));
        assert!(wants_unlock("quiet bifrost.unlock"));
        assert!(!wants_unlock("quiet root=UUID=abcd rw"));
        assert!(!wants_unlock("quiet bifrost.unlocked=1 root=/dev/sda2"));
        assert!(!wants_unlock(""));
    }

    #[test]
    fn video_size_reads_the_pinned_mode() {
        assert_eq!(video_size("quiet video=DP-1:2560x1440@165 rw"), Some((2560, 1440)));
        assert_eq!(video_size("video=1920x1080"), Some((1920, 1080)));
        assert_eq!(video_size("video=eDP-1:1280x800M@60e"), Some((1280, 800)));
        assert_eq!(video_size("video=HDMI-A-1:1920x1080-32@60 video=DP-1:d"), Some((1920, 1080)));
    }

    #[test]
    fn video_size_ignores_tokens_that_pin_no_size() {
        assert_eq!(video_size(""), None);
        assert_eq!(video_size("quiet root=UUID=1 rw"), None);
        assert_eq!(video_size("video=eDP-1:d"), None);
        assert_eq!(video_size("video=efifb:off"), None);
        assert_eq!(video_size("video=DP-1:0x0@60"), None);
        assert_eq!(video_size("video=DP-1:2560x@60"), None);
        assert_eq!(video_size("novideo=DP-1:2560x1440"), None);
    }

    #[test]
    fn the_loader_keeps_the_firmware_mode_when_the_kernel_will_set_one() {
        let (current, largest) = (1024 * 768, 2560 * 1440);
        // no video=: the loader's change is the boot's one mode change
        assert!(should_set_native_mode(current, largest, false));
        // video= pins a mode: the kernel changes it, so the loader does not add a second change
        assert!(!should_set_native_mode(current, largest, true));
        // already at the largest mode (or larger): never changed
        assert!(!should_set_native_mode(largest, largest, false));
        assert!(!should_set_native_mode(largest, current, false));
    }

    #[test]
    fn default_is_text_free() {
        let tokens: Vec<&str> = DEFAULT_CMDLINE.split_whitespace().collect();
        assert!(tokens.contains(&"quiet"));
        assert!(tokens.contains(&"loglevel=3"));
        assert!(tokens.contains(&"systemd.show_status=false"));
        assert!(tokens.contains(&"rd.systemd.show_status=false"));
        assert!(tokens.contains(&"udev.log_level=3"));
        assert!(tokens.contains(&"systemd.log_target=journal"));
        assert!(tokens.contains(&"vt.global_cursor_default=0"));
        assert!(!tokens.contains(&"splash"));
    }

    #[test]
    fn default_runs_threaded_irqs_and_a_verbose_boot_keeps_them() {
        let count = |line: &str| line.split_whitespace().filter(|t| *t == "threadirqs").count();
        assert_eq!(count(DEFAULT_CMDLINE), 1);
        assert!(!is_text_free_token("threadirqs"));
        let out = verbose(DEFAULT_CMDLINE);
        assert_eq!(count(&out), 1);
        assert_eq!(out, format!("initrd=\\arch\\boot\\x86_64\\initramfs-linux.img threadirqs {VERBOSE_TOKENS}"));
    }

    #[test]
    fn verbose_drops_quiet_and_adds_debug_level() {
        let out = verbose("initrd=\\x.img root=UUID=abc quiet loglevel=3 rw");
        assert_eq!(
            out,
            "initrd=\\x.img root=UUID=abc rw loglevel=7 systemd.show_status=true rd.systemd.show_status=true systemd.log_level=info udev.log_level=info"
        );
    }

    #[test]
    fn verbose_removes_splash_and_text_free_keys_but_keeps_hardware_flags() {
        let line = "initrd=\\a root=UUID=1 quiet splash loglevel=3 systemd.show_status=false rd.udev.log_level=3 threadirqs i915.enable_psr=0";
        let out = verbose(line);
        assert!(!out.split_whitespace().any(|t| t == "splash" || t == "quiet"));
        assert!(!out.contains("show_status=false"));
        assert!(!out.contains("rd.udev.log_level=3"));
        assert!(out.contains("threadirqs"));
        assert!(out.contains("i915.enable_psr=0"));
        assert!(out.contains("root=UUID=1"));
    }

    #[test]
    fn verbose_is_idempotent() {
        let once = verbose(DEFAULT_CMDLINE);
        assert_eq!(verbose(&once), once);
    }

    #[test]
    fn verbose_keeps_unrelated_loglevel_like_tokens_out() {
        // "loglevelx" is not "loglevel=": only the exact key and its `=` prefix count.
        assert!(!is_text_free_token("loglevelx=1"));
        assert!(is_text_free_token("loglevel=3"));
        assert!(is_text_free_token("quiet"));
        assert!(!is_text_free_token("quietly"));
    }

    #[test]
    fn verbose_of_empty_line_is_only_the_verbose_set() {
        assert_eq!(verbose(""), VERBOSE_TOKENS);
    }

    #[test]
    fn verbose_collapses_extra_spaces() {
        assert_eq!(verbose("  a   quiet  b "), format!("a b {VERBOSE_TOKENS}"));
    }
}
