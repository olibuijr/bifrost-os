//! Test fixtures: a `Ui` with the shell's font and symbols.
use springboard::shell::ui::Ui;

pub fn ui() -> Ui {
    let assets = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{assets}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = Ui::new(bifrost_app::kit::Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(assets);
    ui
}
