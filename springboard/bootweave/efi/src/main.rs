//! BifrOSt boot splash: paints the logo the moment firmware hands over control, then starts the
//! Linux EFI stub from the same volume. The stub loads the initramfs itself (initrd= option), so the
//! logo stays on screen for the whole kernel/initramfs read instead of a black screen.
#![no_main]
#![no_std]
extern crate alloc;
mod boot_policy;
mod screen;
use alloc::vec::Vec;
use uefi::boot::{self, LoadImageSource};
use uefi::prelude::*;
use uefi::proto::console::gop::{BltOp, BltPixel, GraphicsOutput};
use uefi::proto::device_path::build::{media::FilePath, DevicePathBuilder};
use uefi::proto::device_path::DevicePath;
use uefi::proto::loaded_image::LoadedImage;
use uefi::{cstr16, CStr16};

const LOGO: &[u8] = include_bytes!("../logo.bgrx");
const LOGO_PX: usize = 160;
// EFI/BIFROST/boot.cfg is the installed-system contract; EFI/BOOT/cmdline.txt
// is the removable-media (USB) copy.
const CMDLINE_FILES: &[&CStr16] = &[
    cstr16!("\\EFI\\BIFROST\\boot.cfg"),
    cstr16!("\\EFI\\BOOT\\cmdline.txt"),
];
const DEFAULT_CMDLINE: &str = "initrd=\\arch\\boot\\x86_64\\initramfs-linux.img quiet splash";

/// Kernel command line from `\\EFI\\BOOT\\cmdline.txt` next to this loader (written at ISO build time).
fn read_text(dev: Handle, path: &CStr16) -> Option<alloc::string::String> {
    use uefi::proto::media::file::{File, FileAttribute, FileMode};
    use uefi::proto::media::fs::SimpleFileSystem;
    let mut fs = boot::open_protocol_exclusive::<SimpleFileSystem>(dev).ok()?;
    let mut root = fs.open_volume().ok()?;
    let mut file = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .ok()?
        .into_regular_file()?;
    let mut buf = [0u8; 2048];
    let n = file.read(&mut buf).ok()?;
    Some(
        alloc::string::String::from_utf8_lossy(&buf[..n])
            .trim()
            .into(),
    )
}
fn consume_attempt(dev: Handle) -> bool {
    use uefi::proto::media::file::{File, FileAttribute, FileMode};
    use uefi::proto::media::fs::SimpleFileSystem;
    let write = || -> Option<()> {
        let mut fs = boot::open_protocol_exclusive::<SimpleFileSystem>(dev).ok()?;
        let mut root = fs.open_volume().ok()?;
        let mut file = root
            .open(
                cstr16!("\\EFI\\BIFROST\\pending"),
                FileMode::ReadWrite,
                FileAttribute::empty(),
            )
            .ok()?
            .into_regular_file()?;
        file.write(b"1").ok()?;
        file.flush().ok()?;
        Some(())
    };
    write().is_some()
}
fn cmdline_from(dev: Handle, path: &CStr16) -> Option<alloc::string::String> {
    use uefi::proto::media::file::{File, FileAttribute, FileMode, RegularFile};
    use uefi::proto::media::fs::SimpleFileSystem;
    let mut sfs = boot::open_protocol_exclusive::<SimpleFileSystem>(dev).ok()?;
    let mut root = sfs.open_volume().ok()?;
    let h = root.open(path, FileMode::Read, FileAttribute::empty()).ok()?;
    let mut f: RegularFile = h.into_regular_file()?;
    let mut buf = [0u8; 2048];
    let n = f.read(&mut buf).ok()?;
    let text: alloc::string::String = alloc::string::String::from_utf8_lossy(&buf[..n]).trim().into();
    (!text.is_empty()).then_some(text)
}

fn cmdline(dev: Handle) -> alloc::string::String {
    CMDLINE_FILES
        .iter()
        .find_map(|path| cmdline_from(dev, path))
        .unwrap_or_else(|| DEFAULT_CMDLINE.into())
}

/// Microseconds since CPU reset from the TSC, the clock systemd's Boot Loader Interface expects.
fn tsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}
static PER_US: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// TSC ticks per microsecond, measured once over a 1 ms firmware stall.
fn tsc_per_us() -> u64 {
    use core::sync::atomic::Ordering::Relaxed;
    let known = PER_US.load(Relaxed);
    if known != 0 {
        return known;
    }
    let a = tsc();
    boot::stall(core::time::Duration::from_millis(1));
    let per = ((tsc() - a) / 1000).max(1);
    PER_US.store(per, Relaxed);
    per
}
/// Boot Loader Interface: systemd-analyze reads these to report firmware and loader time.
fn loader_var(name: &CStr16, text: &str) {
    use uefi::runtime::{VariableAttributes, VariableVendor};
    const LOADER: VariableVendor = VariableVendor(uefi::guid!("4a67b082-0a4c-41cf-b6c7-440b29bb8c4f"));
    let mut data: Vec<u8> = text.encode_utf16().chain(core::iter::once(0)).flat_map(u16::to_le_bytes).collect();
    data.shrink_to_fit();
    let _ = uefi::runtime::set_variable(name, &LOADER, VariableAttributes::BOOTSERVICE_ACCESS | VariableAttributes::RUNTIME_ACCESS, &data);
}

/// The one modeset of the boot: the panel's largest GOP mode. Linux (simpledrm, then the GPU driver with fastboot)
/// and the BifrOSt display code keep this mode, so the screen is never reprogrammed again.
fn native_mode(gop: &mut GraphicsOutput) {
    let area = |m: &uefi::proto::console::gop::Mode| { let (w, h) = m.info().resolution(); w * h };
    let current = { let (w, h) = gop.current_mode_info().resolution(); w * h };
    if let Some(best) = gop.modes().max_by_key(area) {
        if area(&best) > current {
            let _ = gop.set_mode(&best);
        }
    }
}

fn paint() {
    let Ok(h) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
        return;
    };
    let Ok(mut gop) = boot::open_protocol_exclusive::<GraphicsOutput>(h) else {
        return;
    };
    native_mode(&mut gop);
    let (w, hgt) = gop.current_mode_info().resolution();
    let _ = gop.blt(BltOp::VideoFill {
        color: BltPixel::new(0, 0, 0),
        dest: (0, 0),
        dims: (w, hgt),
    });
    // Layout as fractions of the screen height so the monitor's
    // scaling of the firmware mode cannot change it: mark 160 px and bar 240x4 px at 1440p,
    // mark top 110 px above the middle, bar 110 px below it.
    let l = (LOGO_PX * hgt / 1440).max(16);
    let px: Vec<BltPixel> = (0..l * l)
        .map(|i| {
            let (sx, sy) = ((i % l) * LOGO_PX / l, (i / l) * LOGO_PX / l);
            let c = &LOGO[(sy * LOGO_PX + sx) * 4..][..4];
            BltPixel::new(c[2], c[1], c[0])
        })
        .collect();
    let (x, y) = ((w - l) / 2, (hgt / 2).saturating_sub(110 * hgt / 1440));
    let _ = gop.blt(BltOp::BufferToVideo {
        buffer: &px,
        src: uefi::proto::console::gop::BltRegion::Full,
        dest: (x, y),
        dims: (l, l),
    });
    let (bw, bh) = (240 * hgt / 1440, (4 * hgt / 1440).max(2));
    let _ = gop.blt(BltOp::VideoFill {
        color: BltPixel::new(60, 60, 62),
        dest: ((w - bw) / 2, hgt / 2 + 110 * hgt / 1440),
        dims: (bw, bh),
    });
}

#[entry]
fn main() -> Status {
    let init = tsc();
    paint();
    let per_us = tsc_per_us();
    loader_var(cstr16!("LoaderTimeInitUSec"), &alloc::format!("{}", init / per_us));
    loader_var(cstr16!("LoaderInfo"), "BifrOSt loader");
    let image = boot::image_handle();
    let Ok(li) = boot::open_protocol_exclusive::<LoadedImage>(image) else {
        return Status::LOAD_ERROR;
    };
    let Some(dev) = li.device() else {
        return Status::LOAD_ERROR;
    };
    drop(li);
    let current = cmdline(dev);
    let marker = read_text(dev, cstr16!("\\EFI\\BIFROST\\pending"));
    let previous = read_text(dev, cstr16!("\\EFI\\BIFROST\\rollback.txt"));
    let fallback = match boot_policy::choose(marker.as_deref()) {
        boot_policy::Choice::Current => false,
        boot_policy::Choice::Attempt => !consume_attempt(dev),
        boot_policy::Choice::Previous => true,
    };
    if fallback {
        return rollback(image, dev, previous.as_deref()).unwrap_or(Status::LOAD_ERROR);
    }
    let status = boot_line(image, dev, &current);
    if marker.is_some() && status != Status::SUCCESS {
        return rollback(image, dev, previous.as_deref()).unwrap_or(status);
    }
    status
}
/// Starts the previous generation under the rollback screen. `None` when there is no previous generation to start.
fn rollback(image: Handle, dev: Handle, previous: Option<&str>) -> Option<Status> {
    let line = previous?;
    screen::show(bootweave_frame::Screen::Rollback);
    Some(boot_line(image, dev, line))
}
fn boot_line(image: Handle, dev: Handle, line: &str) -> Status {
    let Ok(dp) = boot::open_protocol_exclusive::<DevicePath>(dev) else {
        return Status::LOAD_ERROR;
    };
    let mut buf = Vec::new();
    let mut b = DevicePathBuilder::with_vec(&mut buf);
    for n in dp.node_iter() {
        b = b.push(&n).unwrap();
    }
    let dir = line
        .split_whitespace()
        .find_map(|t| t.strip_prefix("initrd="))
        .and_then(|p| p.rfind('\\').map(|i| &p[..i]))
        .unwrap_or("\\arch\\boot\\x86_64");
    let kernel_path =
        uefi::CString16::try_from(alloc::format!("{dir}\\vmlinuz-linux").as_str()).unwrap();
    let full = b
        .push(&FilePath {
            path_name: &kernel_path,
        })
        .unwrap()
        .finalize()
        .unwrap();
    let Ok(k) = boot::load_image(
        image,
        LoadImageSource::FromDevicePath {
            device_path: full,
            boot_policy: uefi::proto::BootPolicy::ExactMatch,
        },
    ) else {
        return Status::NOT_FOUND;
    };
    let mut opts: Vec<u16> = line.encode_utf16().collect();
    opts.push(0);
    let Ok(mut kli) = boot::open_protocol_exclusive::<LoadedImage>(k) else {
        return Status::LOAD_ERROR;
    };
    unsafe { kli.set_load_options(opts.as_ptr() as *const u8, (opts.len() * 2) as u32) };
    drop(kli);
    loader_var(cstr16!("LoaderTimeExecUSec"), &alloc::format!("{}", tsc() / tsc_per_us()));
    match boot::start_image(k) {
        Ok(()) => Status::SUCCESS,
        Err(e) => e.status(),
    }
}
