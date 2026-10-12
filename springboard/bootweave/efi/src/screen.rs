//! Full-panel screens the loader draws with the one boot frame (`bootweave-frame`): the rollback notice, shown while the
//! previous generation starts after a failed update. Drawn through GOP at the current mode, the same pixels as the
//! loader's frame 0 (docs/boot-architecture.md, "One frame"). Owner: boot-11.

use alloc::vec;
use alloc::vec::Vec;
use bootweave_frame::{render, Appearance, Frame, PixelOrder, Screen};
use uefi::boot;
use uefi::proto::console::gop::{BltOp, BltPixel, BltRegion, GraphicsOutput};

/// The loader's appearance until the boot timing record can say (the user's choice is stored there).
const APPEARANCE: Appearance = Appearance::Dark;

/// Draws `screen` over the whole panel. Returns false when there is no graphics output (the caller starts the boot
/// anyway: a missing screen is not a reason to stop).
pub fn show(screen: Screen) -> bool {
    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
        return false;
    };
    let Ok(mut gop) = boot::open_protocol_exclusive::<GraphicsOutput>(handle) else {
        return false;
    };
    let (w, h) = gop.current_mode_info().resolution();
    let frame = Frame { width: w as u32, height: h as u32, appearance: APPEARANCE, progress: None, screen };
    let mut pixels = vec![0u32; w * h];
    render(&frame, &mut pixels, w, PixelOrder::Bgrx);
    let blt: Vec<BltPixel> = pixels.iter().map(|&px| bgrx_to_blt(px)).collect();
    gop.blt(BltOp::BufferToVideo { buffer: &blt, src: BltRegion::Full, dest: (0, 0), dims: (w, h) })
        .is_ok()
}

/// A frame pixel (`render` with `PixelOrder::Bgrx`: the little-endian u32 is 0x00RRGGBB, so the bytes are B, G, R)
/// as a GOP pixel, whose constructor takes (red, green, blue).
fn bgrx_to_blt(px: u32) -> BltPixel {
    let [b, g, r, _] = px.to_le_bytes();
    BltPixel::new(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The efi crate is a UEFI target (no_std with a panic handler), so these run only where the crate is built for
    // a host test harness; they are kept with the code they check.
    #[test]
    fn a_frame_pixel_splits_into_blt_channels() {
        // 0x00RRGGBB: red 0x11, green 0x22, blue 0x33
        let px = bgrx_to_blt(0x0011_2233);
        assert_eq!((px.red, px.green, px.blue), (0x11, 0x22, 0x33));
    }

    #[test]
    fn black_and_white_map_to_zero_and_full() {
        let black = bgrx_to_blt(0);
        assert_eq!((black.red, black.green, black.blue), (0, 0, 0));
        let white = bgrx_to_blt(0x00FF_FFFF);
        assert_eq!((white.red, white.green, white.blue), (255, 255, 255));
    }
}
