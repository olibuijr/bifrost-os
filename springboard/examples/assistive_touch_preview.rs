//! Host preview: cargo run --example assistive_touch_preview
//! Writes target/assistive_touch_{button,menu}.ppm.
use springboard::font::Font;
use springboard::gfx::{rgba, Surface};
use springboard::shell::assistive_touch::AssistiveTouch;
use springboard::shell::ui::{Ui, Weight, WHITE};
use springboard::shell::Event;
use std::fs;

const W: usize = 1200;
const H: usize = 1920;

fn save(overlay: &mut AssistiveTouch, ui: &Ui, path: &str) {
    let mut pixels = vec![rgba(40, 60, 86, 255); W * H];
    let mut surface = Surface {
        w: W,
        h: H,
        stride: W,
        px: &mut pixels,
        y0: 0,
    };
    let title = ui.chrome("AssistiveTouch", 28.0, Weight::Semibold, WHITE);
    surface.blit(
        &title,
        (W - title.w) as i32 / 2,
        ui.pt(85.0) as i32,
        255,
        255,
    );
    overlay.draw(&mut surface, ui, W as i32, H as i32, 100.0);
    let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
    for pixel in pixels {
        ppm.extend([pixel as u8, (pixel >> 8) as u8, (pixel >> 16) as u8]);
    }
    fs::write(path, ppm).unwrap();
    eprintln!("{path}");
}

fn main() {
    let bytes = Box::leak(fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let ui = Ui::new(Font::parse(bytes).unwrap(), W as f32 / 768.0);
    let mut overlay = AssistiveTouch::new(&ui, W as i32, H as i32);
    overlay.set_enabled(true);
    save(&mut overlay, &ui, "target/assistive_touch_button.ppm");
    let (x, y, d, _) = overlay.button_rect(&ui, W as i32, H as i32);
    overlay.event(
        Event::Down(x + d / 2.0, y + d / 2.0),
        10.0,
        &ui,
        W as i32,
        H as i32,
    );
    overlay.event(
        Event::Up(x + d / 2.0, y + d / 2.0),
        30.0,
        &ui,
        W as i32,
        H as i32,
    );
    save(&mut overlay, &ui, "target/assistive_touch_menu.ppm");
}
