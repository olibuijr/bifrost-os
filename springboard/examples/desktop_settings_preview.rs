//! Preview the desktop Displays pane at a typical window size.
use springboard::{font::Font, gfx::Surface, shell::{settings::Settings, prefs::Prefs, ui::Ui, Info}};
use std::{fs, rc::Rc};
fn main() {
    let font = Box::leak(fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let ui = Ui::new(Font::parse(font).unwrap(), 1.0);
    let info = Info { display_mode: Some((2048, 1152, 60)), ..Info::default() };
    let mut settings = Settings::new(&ui, &info, &Prefs::memory(), 1000, 700, Rc::new(vec![]));
    settings.open_page(&ui, "Display & Brightness", 0.0);
    let mut px = vec![0; 1000 * 700];
    settings.draw(&mut Surface { w: 1000, h: 700, stride: 1000, y0: 0, px: &mut px }, &ui, 1000.0);
    let path = std::env::args().nth(1).expect("output PNG path");
    let mut encoder = png::Encoder::new(fs::File::create(&path).unwrap(), 1000, 700);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = px.iter().flat_map(|p| [*p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect();
    encoder.write_header().unwrap().write_image_data(&rgb).unwrap();
    println!("{path}");
}
