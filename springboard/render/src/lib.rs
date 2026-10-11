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
pub mod filter;
pub mod mesh;
pub mod texture;
pub mod colorfx;
pub mod colorcube;
pub mod gradient;
pub mod colorspace;
pub mod pixel_layout;
pub mod bezier;
pub mod gradient;

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
