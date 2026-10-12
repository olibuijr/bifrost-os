//! The Unlock screen in the loader (task 10): finds the encrypted root (the partition whose first sector holds a
//! LUKS2 header), reads the passphrase from UEFI keyboard input, draws the Unlock screen in the boot frame, and
//! returns the volume key once a passphrase opens a key slot. Nothing is printed: the passphrase is shown as dots.

use crate::luks::{self, Volume};
use alloc::string::String;
use alloc::vec::Vec;
use bootweave_frame::{render, Appearance, Frame, PixelOrder, Screen};
use core::time::Duration;
use uefi::boot::{self, SearchType};
use uefi::proto::console::gop::{BltOp, BltPixel, BltRegion, GraphicsOutput};
use uefi::proto::console::text::Key;
use uefi::proto::media::block::BlockIO;
use uefi::system;
use uefi::Handle;

/// Runs the unlock when the machine has an encrypted root. `None` when there is none, or when the header cannot be
/// opened by this loader (the boot then continues without a key and the initramfs reports the failure).
/// `user` is the account the session logs in (from `bifrost.user=` on the command line), shown on the screen.
pub fn run(user: &str) -> Option<Volume> {
    let handle = find_encrypted_partition()?;
    let binary = partition_read(handle, 0, luks::BINARY_LEN)?;
    let json_len = luks::json_len(&binary).ok()?;
    let json = partition_read(handle, luks::BINARY_LEN as u64, json_len)?;
    let (width, height) = gop_size()?;
    let mut read = |offset: u64, len: usize| partition_read(handle, offset, len);

    system::with_stdin(|stdin| {
        let _ = stdin.reset(false);
    });
    let mut password = String::new();
    let mut failed = false;
    loop {
        let frame = Frame {
            width,
            height,
            appearance: Appearance::Dark,
            progress: None,
            screen: Screen::Unlock { name: String::from(user), dots: password.chars().count() as u32, failed },
        };
        draw(&frame);
        match next_key() {
            Key::Printable(c) => match char::from(c) {
                '\r' | '\n' => {
                    let outcome = luks::unlock(&json, password.as_bytes(), &mut read);
                    wipe(&mut password);
                    match outcome {
                        Ok(volume) => return Some(volume),
                        Err(luks::Error::WrongPassword) => failed = true,
                        Err(_) => return None,
                    }
                }
                '\u{8}' => {
                    password.pop();
                }
                c if !c.is_control() => password.push(c),
                _ => {}
            },
            Key::Special(_) => {}
        }
    }
}

/// The first partition whose first sector is a LUKS2 header. Whole disks are skipped (their first sector is the
/// partition table).
fn find_encrypted_partition() -> Option<Handle> {
    let handles = boot::locate_handle_buffer(SearchType::from_proto::<BlockIO>()).ok()?;
    handles.iter().copied().find(|&h| {
        let partition = {
            let Ok(io) = boot::open_protocol_exclusive::<BlockIO>(h) else { return false };
            let media = io.media();
            media.is_media_present() && media.is_logical_partition()
        };
        partition && partition_read(h, 0, 6).is_some_and(|magic| magic[..] == luks::MAGIC[..])
    })
}

/// `len` bytes at a byte offset of a partition, reading whole blocks.
fn partition_read(handle: Handle, offset: u64, len: usize) -> Option<Vec<u8>> {
    let mut io = boot::open_protocol_exclusive::<BlockIO>(handle).ok()?;
    let (media_id, block) = {
        let media = io.media();
        (media.media_id(), media.block_size() as u64)
    };
    if block == 0 || !block.is_power_of_two() {
        return None;
    }
    let start = offset / block * block;
    let end = (offset + len as u64).div_ceil(block) * block;
    let mut buf = alloc::vec![0u8; (end - start) as usize];
    io.read_blocks(media_id, start / block, &mut buf).ok()?;
    let from = (offset - start) as usize;
    Some(buf[from..from + len].to_vec())
}

fn gop_size() -> Option<(u32, u32)> {
    let handle = boot::get_handle_for_protocol::<GraphicsOutput>().ok()?;
    let gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle).ok()?;
    let (w, h) = gop.current_mode_info().resolution();
    Some((w as u32, h as u32))
}

/// Draws one frame at the panel's current mode (the native mode the loader already set).
fn draw(frame: &Frame) {
    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else { return };
    let Ok(mut gop) = boot::open_protocol_exclusive::<GraphicsOutput>(handle) else { return };
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut pixels = alloc::vec![0u32; w * h];
    render(frame, &mut pixels, w, PixelOrder::Bgrx);
    // A Bgrx pixel is 0x00RRGGBB as a little-endian u32.
    let blt: Vec<BltPixel> = pixels
        .iter()
        .map(|p| BltPixel::new(((p >> 16) & 0xff) as u8, ((p >> 8) & 0xff) as u8, (p & 0xff) as u8))
        .collect();
    let _ = gop.blt(BltOp::BufferToVideo {
        buffer: &blt,
        src: BltRegion::Full,
        dest: (0, 0),
        dims: (w, h),
    });
}

/// The next key press, polled (the loader has no event loop of its own).
fn next_key() -> Key {
    loop {
        if let Some(key) = system::with_stdin(|stdin| stdin.read_key().ok().flatten()) {
            return key;
        }
        boot::stall(Duration::from_millis(15));
    }
}

/// Overwrites the passphrase's bytes before it is dropped.
fn wipe(s: &mut String) {
    let n = s.len();
    s.clear();
    for _ in 0..n {
        s.push('\0');
    }
    s.clear();
}
