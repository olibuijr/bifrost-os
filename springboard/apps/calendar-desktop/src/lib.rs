//! The Calendar model and its Golden Gate window, as a library: the app binary (`main.rs`) hosts it through the SDK,
//! and `tests/compliance.rs` drives it headless at the shipped size in both appearances.

pub mod calendar;
pub mod icon;
pub mod scripting;

#[cfg(test)]
pub fn test_ui() -> bifrost_app::Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = bifrost_app::Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}
