//! The math font BifrOSt installs: Noto Sans Math (SIL OFL, the `noto-fonts-extra` package in
//! `packaging/arch/runtime-packages.txt`). It has a MATH table, so its constants drive the layout. When the file
//! is missing, layout uses estimated metrics and the painter falls back to the UI font.

use std::sync::OnceLock;

use super::metrics::{FaceMetrics, MathConstants};
use crate::canvas::register_face;
use crate::fontinfo::Tables;

/// Where Arch installs Noto Sans Math, first match wins.
pub const MATH_FONT_PATHS: [&str; 2] = [
    "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
    "/usr/share/fonts/TTF/NotoSansMath-Regular.ttf",
];

/// The installed math font: its registered face and the constants of its MATH table.
pub struct MathFont {
    pub face: u64,
    pub constants: Option<MathConstants>,
}

/// The installed math font, loaded once. `None` when no file is found or it does not parse.
pub fn math_font() -> Option<&'static MathFont> {
    static FONT: OnceLock<Option<MathFont>> = OnceLock::new();
    FONT.get_or_init(|| {
        let bytes = MATH_FONT_PATHS.iter().find_map(|p| std::fs::read(p).ok())?;
        Some(load(bytes))
    })
    .as_ref()
}

/// Registers `bytes` as a face and reads its MATH constants (units per em from `head`).
fn load(bytes: Vec<u8>) -> MathFont {
    let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let constants = Tables::new(data).and_then(|t| {
        let head = t.table(b"head")?;
        let upem = u16::from_be_bytes([*head.get(18)?, *head.get(19)?]);
        MathConstants::from_math_table(t.table(b"MATH")?, upem)
    });
    MathFont { face: register_face(data), constants }
}

/// Metrics for layout: the math font with its constants when installed, else estimates.
pub fn metrics() -> FaceMetrics {
    match math_font() {
        Some(m) => FaceMetrics { face: m.face, constants: m.constants },
        None => FaceMetrics { face: 0, constants: None },
    }
}
