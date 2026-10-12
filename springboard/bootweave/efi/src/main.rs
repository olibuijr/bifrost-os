//! BifrOSt loader (UEFI). Draws boot frame 0 (`bootweave-frame`) at the panel's native mode, reads the boot timing
//! record, drives the progress bar, offers the boot options on a held Shift, then starts the Linux EFI stub with the
//! chosen command line. The bar's state is handed to the kernel side in the `BifrostBootProgress` variable.
//! Contract: `docs/boot-architecture.md`, "One frame" and "The progress bar".
#![no_main]
#![no_std]
extern crate alloc;
mod boot_policy;

use alloc::string::String;
use alloc::vec::Vec;
use bootweave_frame::progress::Progress;
use bootweave_frame::render::{bar_rect, render_bar};
use bootweave_frame::timing::{BootTiming, Handoff, Stage};
use bootweave_frame::{render, Appearance, Frame, PixelOrder, Screen};
use uefi::boot::{self, LoadImageSource, ScopedProtocol};
use uefi::prelude::*;
use uefi::proto::console::gop::{BltOp, BltPixel, BltRegion, GraphicsOutput};
use uefi::proto::console::text::{Input, InputEx, Key, ScanCode};
use uefi::proto::device_path::build::{media::FilePath, DevicePathBuilder};
use uefi::proto::device_path::DevicePath;
use uefi::proto::loaded_image::LoadedImage;
use uefi::runtime::{VariableAttributes, VariableVendor};
use uefi::{cstr16, CStr16};

// EFI/BIFROST/boot.cfg is the installed-system contract; EFI/BOOT/cmdline.txt
// is the removable-media (USB) copy.
const CMDLINE_FILES: &[&CStr16] = &[
    cstr16!("\\EFI\\BIFROST\\boot.cfg"),
    cstr16!("\\EFI\\BOOT\\cmdline.txt"),
];
const DEFAULT_CMDLINE: &str = "initrd=\\arch\\boot\\x86_64\\initramfs-linux.img quiet splash";
const TIMING_PATH: &CStr16 = cstr16!("\\EFI\\BIFROST\\boot-timing");
const PREVIOUS_PATH: &CStr16 = cstr16!("\\EFI\\BIFROST\\rollback.txt");
const RECOVERY_PATH: &CStr16 = cstr16!("\\EFI\\BIFROST\\recovery.txt");
const PENDING_PATH: &CStr16 = cstr16!("\\EFI\\BIFROST\\pending");
/// The BifrOSt vendor GUID of the hand-off variable (`BifrostBootProgress`), read by Bootweave (task 06).
const HANDOFF_VENDOR: VariableVendor = VariableVendor(uefi::guid!("d1f0b07c-5b2e-4a8d-9c3f-6e4a2b7d9f10"));
/// Keyboard Shift bits of `EFI_KEY_STATE.KeyShiftState` (UEFI spec, Simple Text Input Ex): RIGHT_SHIFT, LEFT_SHIFT.
const SHIFT_BITS: u32 = 0x0000_0003;

/// Reads a whole small file from the boot volume, up to `cap` bytes.
fn read_file(dev: Handle, path: &CStr16, cap: usize) -> Option<Vec<u8>> {
    use uefi::proto::media::file::{File, FileAttribute, FileMode};
    use uefi::proto::media::fs::SimpleFileSystem;
    let mut fs = boot::open_protocol_exclusive::<SimpleFileSystem>(dev).ok()?;
    let mut root = fs.open_volume().ok()?;
    let mut file = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .ok()?
        .into_regular_file()?;
    let mut buf = alloc::vec![0u8; cap];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

fn read_text(dev: Handle, path: &CStr16) -> Option<String> {
    let bytes = read_file(dev, path, 2048)?;
    Some(String::from_utf8_lossy(&bytes).trim().into())
}

/// A non-empty text file, for the boot lines that are optional.
fn read_line(dev: Handle, path: &CStr16) -> Option<String> {
    read_text(dev, path).filter(|t| !t.is_empty())
}

fn consume_attempt(dev: Handle) -> bool {
    use uefi::proto::media::file::{File, FileAttribute, FileMode};
    use uefi::proto::media::fs::SimpleFileSystem;
    let write = || -> Option<()> {
        let mut fs = boot::open_protocol_exclusive::<SimpleFileSystem>(dev).ok()?;
        let mut root = fs.open_volume().ok()?;
        let mut file = root
            .open(PENDING_PATH, FileMode::ReadWrite, FileAttribute::empty())
            .ok()?
            .into_regular_file()?;
        file.write(b"1").ok()?;
        file.flush().ok()?;
        Some(())
    };
    write().is_some()
}

fn cmdline(dev: Handle) -> String {
    CMDLINE_FILES
        .iter()
        .find_map(|path| read_line(dev, path))
        .unwrap_or_else(|| DEFAULT_CMDLINE.into())
}

/// Microseconds since CPU reset from the TSC, the clock systemd's Boot Loader Interface expects.
fn tsc() -> u64 {
    // SAFETY: `_rdtsc` only reads the processor's time-stamp counter; it has no memory effects and is available on
    // every x86_64 CPU that runs UEFI.
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

/// A rendered pixel as the firmware's blit wants it: `0x00RRGGBB` (the `PixelOrder::Bgrx` value) to a blit pixel.
fn to_blt(v: u32) -> BltPixel {
    BltPixel::new(((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8)
}

/// The panel's graphics output and an in-memory copy of the frame, drawn with `bootweave-frame`.
struct Display {
    gop: ScopedProtocol<GraphicsOutput>,
    width: u32,
    height: u32,
    appearance: Appearance,
    pixels: Vec<u32>,
}

impl Display {
    fn open(appearance: Appearance) -> Option<Display> {
        let h = boot::get_handle_for_protocol::<GraphicsOutput>().ok()?;
        let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(h).ok()?;
        native_mode(&mut gop);
        let (w, hgt) = gop.current_mode_info().resolution();
        Some(Display { gop, width: w as u32, height: hgt as u32, appearance, pixels: alloc::vec![0u32; w * hgt] })
    }

    fn frame(&self, screen: Screen, progress: Option<f32>) -> Frame {
        Frame { width: self.width, height: self.height, appearance: self.appearance, progress, screen }
    }

    /// Renders `frame` into the copy and blits the whole panel.
    fn draw(&mut self, frame: &Frame) {
        render(frame, &mut self.pixels, self.width as usize, PixelOrder::Bgrx);
        let px: Vec<BltPixel> = self.pixels.iter().map(|&v| to_blt(v)).collect();
        let _ = self.gop.blt(BltOp::BufferToVideo {
            buffer: &px,
            src: BltRegion::Full,
            dest: (0, 0),
            dims: (self.width as usize, self.height as usize),
        });
    }

    /// Moves only the progress bar: re-renders its rectangle and blits that rectangle.
    fn tick(&mut self, progress: f32) {
        render_bar(self.width, self.height, progress, self.appearance, &mut self.pixels, self.width as usize, PixelOrder::Bgrx);
        let (x, y, bw, bh) = bar_rect(self.width, self.height);
        let stride = self.width as usize;
        let mut px = Vec::with_capacity((bw * bh) as usize);
        for row in 0..bh as usize {
            let start = (y as usize + row) * stride + x as usize;
            px.extend(self.pixels[start..start + bw as usize].iter().map(|&v| to_blt(v)));
        }
        let _ = self.gop.blt(BltOp::BufferToVideo {
            buffer: &px,
            src: BltRegion::Full,
            dest: (x as usize, y as usize),
            dims: (bw as usize, bh as usize),
        });
    }
}

/// The boot options (held Shift): the user picks one of `entries` with the arrow keys and Enter. `None` when the
/// keyboard cannot be read, so the normal boot goes on.
fn choose_option(display: &mut Display, entries: &[(&str, String)]) -> Option<String> {
    let h = boot::get_handle_for_protocol::<Input>().ok()?;
    let mut input = boot::open_protocol_exclusive::<Input>(h).ok()?;
    let labels: Vec<String> = entries.iter().map(|(label, _)| String::from(*label)).collect();
    let mut selected: usize = 0;
    loop {
        let frame = display.frame(Screen::Options { entries: labels.clone(), selected: selected as u32 }, None);
        display.draw(&frame);
        let event = input.wait_for_key_event().ok()?;
        boot::wait_for_event(&[event]).ok()?;
        match input.read_key() {
            Ok(Some(Key::Special(ScanCode::UP))) => selected = selected.saturating_sub(1),
            Ok(Some(Key::Special(ScanCode::DOWN))) => selected = (selected + 1).min(entries.len().saturating_sub(1)),
            Ok(Some(Key::Printable(c))) if u16::from(c) == 0x000d => return entries.get(selected).map(|(_, line)| line.clone()),
            _ => {}
        }
    }
}

/// Whether a Shift key was pressed in the keys already waiting at loader start. Drains them; never waits, so the
/// loader's time is not spent here.
fn shift_at_start() -> bool {
    let Ok(h) = boot::get_handle_for_protocol::<InputEx>() else {
        return false;
    };
    let Ok(mut input) = boot::open_protocol_exclusive::<InputEx>(h) else {
        return false;
    };
    let mut shift = false;
    while let Ok(Some(key)) = input.read_key() {
        if key.key_state.key_shift_state.is_some_and(|s| s.bits() & SHIFT_BITS != 0) {
            shift = true;
        }
    }
    shift
}

/// The loader's state for one boot: the progress bar, the boot frame, and the handles it boots with.
struct Session {
    image: Handle,
    dev: Handle,
    init: u64,
    per_us: u64,
    progress: Progress,
    display: Option<Display>,
}

impl Session {
    /// Milliseconds since the loader started.
    fn elapsed_ms(&self) -> u32 {
        (tsc().wrapping_sub(self.init) / self.per_us / 1000) as u32
    }

    /// Frame 0: the boot frame with the bar at its current position (0 at the first moment).
    fn first_frame(&mut self) {
        let pos = self.progress.at(self.elapsed_ms());
        let visible = self.progress.visible();
        if let Some(display) = self.display.as_mut() {
            let frame = display.frame(Screen::Boot, visible.then_some(pos));
            display.draw(&frame);
        }
    }

    fn options(&mut self, entries: &[(&str, String)]) -> Option<String> {
        choose_option(self.display.as_mut()?, entries)
    }

    /// Starts the kernel stub with `line`. The loader's stage ends here: the bar snaps to the loader checkpoint, the
    /// frame is dropped (the kernel side takes the panel over), and `BifrostBootProgress` carries the bar to Bootweave.
    fn boot_line(&mut self, line: &str) -> Status {
        let Ok(dp) = boot::open_protocol_exclusive::<DevicePath>(self.dev) else {
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
            self.image,
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
        // SAFETY: `opts` is a NUL-terminated UTF-16 buffer that lives until `start_image` returns, and its byte length
        // is `opts.len() * 2`, the length the LoadedImage protocol expects for load options.
        unsafe { kli.set_load_options(opts.as_ptr() as *const u8, (opts.len() * 2) as u32) };
        drop(kli);

        self.progress.stage_done(Stage::Loader);
        let pos = self.progress.at(self.elapsed_ms());
        if self.progress.visible() {
            if let Some(display) = self.display.as_mut() {
                display.tick(pos);
            }
        }
        self.display = None;

        let handoff = Handoff {
            timing: self.progress.timing,
            loader_ms: self.elapsed_ms(),
            shown_permille: (pos * 1000.0) as u32,
        };
        let _ = uefi::runtime::set_variable(
            cstr16!("BifrostBootProgress"),
            &HANDOFF_VENDOR,
            VariableAttributes::BOOTSERVICE_ACCESS | VariableAttributes::RUNTIME_ACCESS,
            &handoff.encode(),
        );
        loader_var(cstr16!("LoaderTimeExecUSec"), &alloc::format!("{}", tsc().wrapping_sub(self.init) / self.per_us));
        match boot::start_image(k) {
            Ok(()) => Status::SUCCESS,
            Err(e) => e.status(),
        }
    }
}

#[entry]
fn main() -> Status {
    let init = tsc();
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
    let marker = read_text(dev, PENDING_PATH);
    let previous = read_line(dev, PREVIOUS_PATH);
    let recovery = read_line(dev, RECOVERY_PATH);
    let timing = read_file(dev, TIMING_PATH, 64)
        .and_then(|bytes| BootTiming::decode(&bytes))
        .unwrap_or(BootTiming::DEFAULT);

    let mut session = Session {
        image,
        dev,
        init,
        per_us,
        progress: Progress::new(timing),
        display: Display::open(timing.appearance),
    };
    session.first_frame();

    if shift_at_start() {
        let mut entries = alloc::vec![("Current", current.clone())];
        if let Some(line) = previous.clone() {
            entries.push(("Previous", line));
        }
        if let Some(line) = recovery {
            entries.push(("Recovery", line));
        }
        if let Some(line) = session.options(&entries) {
            return session.boot_line(&line);
        }
    }

    let fallback = match boot_policy::choose(marker.as_deref()) {
        boot_policy::Choice::Current => false,
        boot_policy::Choice::Attempt => !consume_attempt(dev),
        boot_policy::Choice::Previous => true,
    };
    if fallback {
        return previous
            .as_deref()
            .map(|line| session.boot_line(line))
            .unwrap_or(Status::LOAD_ERROR);
    }
    let status = session.boot_line(&current);
    if marker.is_some() && status != Status::SUCCESS {
        return previous
            .as_deref()
            .map(|line| session.boot_line(line))
            .unwrap_or(status);
    }
    status
}
