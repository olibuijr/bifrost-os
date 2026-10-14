//! Shared BifrOSt rendering foundation; independent of shell, installer and device services.
pub mod cff;
pub mod cg;
pub mod font;
pub mod gfx;
pub mod canvas;
pub mod display_list;
pub mod gvar;
pub mod emoji;
pub mod symbols;
pub mod sdf_glyphs;
pub mod imgdec;
pub mod image_tiles;
pub mod ui;
pub mod cache;
pub mod purgeable;
pub mod clipboard;
pub mod appearance;
pub mod glass;
pub mod vibrancy;
pub mod motion;
pub mod assets;
pub mod fontprefs;
pub mod fontfallback;
pub mod fontinfo;
pub mod fontobj;
pub mod kern;
pub mod shape_ot;
pub mod filter;
pub mod integral;
pub mod mesh;
pub mod texture;
pub mod colorfx;
pub mod colorcube;
pub mod gradient;
pub mod colorspace;
pub mod rendering_intent;
pub mod icc;
pub mod pixel_layout;
pub mod bezier;
pub mod colorlab;
pub mod colormap;
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
pub mod effects;
pub mod blur_kernels;
pub mod blur_filters;
pub mod effect_chain;
pub mod composite_filters;
pub mod distortion_filters;
pub mod stylize_filters;
pub mod halftone_tile_filters;
pub mod filter_catalogue;
pub mod filter_input;
pub mod gamma_fade;
pub mod video_transfer;
pub mod clip;
pub mod affine;
pub mod transform_stack;
pub mod block_motion;
pub mod kernel;
pub mod gaussian_filter;
pub mod lanczos;
pub mod generators;
pub mod spot_light;
pub mod comic_filter;
pub mod morphology;
pub mod histogram;

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

pub mod hdr_frame;
pub mod imgtile;
pub mod hdr_transfer;
pub mod hdr_metadata;
pub mod bayer;
pub mod lazy_image;
pub mod depth_blur;
pub mod render_intent;
pub mod color_curve;
pub mod photo_effects;
pub mod feature_detect;
pub mod masked_blur;
pub mod displacement_filters;
pub mod edge_tone_filters;
