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
