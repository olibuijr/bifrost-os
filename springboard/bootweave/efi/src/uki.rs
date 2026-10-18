//! The unified-image start path (docs/boot-architecture.md, "Prepared, not probed"). A boot line with a `uki=<path>`
//! token starts that PE image (kernel, initramfs, microcode and command line in one file, written by the updater)
//! directly; the token itself is not passed to the kernel. Lines without the token keep the kernel-plus-initrd path.
//! Pure, host-tested.

/// The image path named by the `uki=` token, if the line has one (an empty value is no token).
pub fn target(line: &str) -> Option<&str> {
    line.split_whitespace()
        .find_map(|t| t.strip_prefix("uki="))
        .filter(|p| !p.is_empty())
}

/// The kernel command line: the boot line without the `uki=` token, the order of the rest kept.
pub fn options(line: &str) -> alloc::string::String {
    line.split_whitespace()
        .filter(|t| !t.starts_with("uki="))
        .collect::<alloc::vec::Vec<_>>()
        .join(" ")
}

/// The file the loader starts for `line`: the `uki=` image, else the kernel next to the `initrd=` file (the
/// older install layout): `initramfs-<kernel>.img` starts `vmlinuz-<kernel>`; other names use stock `vmlinuz-linux`.
/// `None` when the path cannot be a firmware path (a NUL, or a character outside UCS-2), so
/// the caller takes the rollback path instead of panicking in the firmware.
pub fn kernel_file(line: &str) -> Option<alloc::string::String> {
    let file = match target(line) {
        Some(file) => alloc::string::String::from(file),
        None => {
            let initrd = line.split_whitespace().find_map(|t| t.strip_prefix("initrd="));
            let dir = initrd.and_then(|p| p.rfind('\\').map(|i| &p[..i])).unwrap_or("\\arch\\boot\\x86_64");
            let kernel = initrd.and_then(|p| p.rsplit('\\').next())
                .and_then(|name| name.strip_prefix("initramfs-")?.strip_suffix(".img"))
                .filter(|name| !name.is_empty()).unwrap_or("linux");
            alloc::format!("{dir}\\vmlinuz-{kernel}")
        }
    };
    file.chars().all(|c| c != '\0' && (c as u32) <= 0xFFFF).then_some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_file_names_the_image_or_the_kernel_next_to_the_initrd() {
        assert_eq!(kernel_file("uki=\\EFI\\BIFROST\\bifrost.efi quiet").as_deref(), Some("\\EFI\\BIFROST\\bifrost.efi"));
        assert_eq!(kernel_file("initrd=\\boot\\initramfs.img quiet").as_deref(), Some("\\boot\\vmlinuz-linux"));
        assert_eq!(kernel_file("initrd=\\initramfs-linux-rt.img quiet").as_deref(), Some("\\vmlinuz-linux-rt"));
        assert_eq!(kernel_file("initrd=\\initramfs-linux-lts-t2.img quiet").as_deref(), Some("\\vmlinuz-linux-lts-t2"));
        assert_eq!(kernel_file("initrd=\\initramfs-linux.img quiet").as_deref(), Some("\\vmlinuz-linux"));
        assert_eq!(kernel_file("quiet").as_deref(), Some("\\arch\\boot\\x86_64\\vmlinuz-linux"));
    }

    #[test]
    fn kernel_file_rejects_paths_the_firmware_cannot_hold() {
        assert_eq!(kernel_file("uki=\\EFI\\\u{1F600}.efi quiet"), None, "outside UCS-2");
        assert_eq!(kernel_file("initrd=\\a\0b\\initramfs.img"), None, "embedded NUL");
    }

    #[test]
    fn finds_the_image_path_wherever_the_token_sits() {
        assert_eq!(target("uki=\\EFI\\BIFROST\\bifrost.efi root=UUID=abc quiet"), Some("\\EFI\\BIFROST\\bifrost.efi"));
        assert_eq!(target("root=UUID=abc quiet uki=\\EFI\\BIFROST\\bifrost-previous.efi"), Some("\\EFI\\BIFROST\\bifrost-previous.efi"));
    }

    #[test]
    fn kernel_lines_and_empty_values_are_not_images() {
        assert_eq!(target("initrd=\\initramfs-linux.img root=UUID=abc quiet"), None);
        assert_eq!(target("uki= root=UUID=abc"), None);
        assert_eq!(target(""), None);
    }

    #[test]
    fn options_drop_only_the_image_token_and_keep_order() {
        assert_eq!(
            options("uki=\\EFI\\BIFROST\\bifrost.efi root=UUID=abc rootflags=subvol=@system rw quiet"),
            "root=UUID=abc rootflags=subvol=@system rw quiet"
        );
    }

    #[test]
    fn options_normalise_spacing_and_leave_kernel_lines_alone() {
        assert_eq!(options("  root=UUID=abc   quiet\n"), "root=UUID=abc quiet");
        assert_eq!(options("initrd=\\x root=1"), "initrd=\\x root=1");
    }

    #[test]
    fn rollback_line_selects_the_previous_image() {
        let line = "uki=\\EFI\\BIFROST\\bifrost-previous.efi root=UUID=abc rootflags=subvol=@rollback bifrost.recovery=1";
        assert_eq!(target(line), Some("\\EFI\\BIFROST\\bifrost-previous.efi"));
        assert!(!options(line).contains("uki="));
        assert!(options(line).ends_with("bifrost.recovery=1"));
    }
}
