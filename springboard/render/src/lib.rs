//! Shared BifrOSt rendering foundation; independent of shell, installer and device services.
pub mod cff;
pub mod font;
pub mod gfx;
pub mod gvar;
pub mod emoji;
pub mod symbols;
pub mod imgdec;
pub mod ui;
pub mod cache;
pub mod purgeable;
pub mod clipboard;
pub mod appearance;
pub mod glass;
pub mod motion;
pub mod assets;
pub mod fontprefs;
pub mod fontregistry;
pub mod kern;
pub mod filter;
pub mod integral;
pub mod mesh;
pub mod texture;
pub mod colorfx;
pub mod colorcube;
pub mod gradient;
pub mod colorspace;
pub mod pixel_layout;
pub mod bezier;
pub mod colorlab;
pub mod colormap;
pub mod clip;
pub mod pattern;
pub mod function;
pub mod raw_params;
pub mod upscale;
pub mod temporal_upscale;
pub mod frame_interp;
pub mod replicator;
pub mod emitter;
pub mod plane_stats;
pub mod plane_arith;
pub mod distance;
pub mod guided;
pub mod blend;
pub mod blur_kernels;
pub mod effect_chain;
pub mod gamma_fade;
pub mod video_transfer;
pub mod block_motion;
pub mod kernel;

#[cfg(test)]
mod tests {
    static EMOJI: std::sync::Mutex<()> = std::sync::Mutex::new(());
    pub(crate) fn emoji_lock() -> std::sync::MutexGuard<'static, ()> {
        EMOJI.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn ui() -> crate::ui::Ui {
        let data = include_bytes!("../../assets/SFUI.ttf");
        let mut ui = crate::ui::Ui::new(crate::font::Font::parse(data).unwrap(), 1.5625);
        ui.symbols = crate::symbols::Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets"));
        ui
    }
}
pub mod pixel_format;
pub mod buffer_attach;
pub mod buffer_pool;
