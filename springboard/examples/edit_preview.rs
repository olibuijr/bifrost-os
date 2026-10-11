//! Renders the text-editing UI to target/edit-preview.ppm on the host: a
//! field with a selected word, its handles and the Cut/Copy/Paste menu; a
//! field after a long press with the Select/Select All/Paste menu; and the
//! Terminal with a long-press selection and its Copy/Paste menu. Compare
//! with ~/Pictures/tab/bifrost-ref/clipboard/web-select.png.
use springboard::font::Font;
use springboard::gfx::Surface;
use springboard::shell::termtouch::TermTouch;
use springboard::shell::textfield::TextField;
use springboard::shell::ui::{Ui, CELL};
use springboard::shell::{clipboard, Event};
use springboard::term::Terminal;
use std::fs;

const W: usize = 1200;
const H: usize = 1500;

fn tap(f: &mut TextField, ui: &Ui, x: f32, frame: (f32, f32, f32, f32), t: f64) {
    let y = frame.1 + frame.3 / 2.0;
    f.touch(ui, Event::Down(x, y), frame, W as f32, t);
    f.touch(ui, Event::Up(x, y), frame, W as f32, t + 40.0);
}

fn main() {
    let leak = |p: &str| -> &'static [u8] { Box::leak(fs::read(p).unwrap().into_boxed_slice()) };
    let ui = Ui::new(Font::parse(leak("assets/SFUI.ttf")).unwrap(), W as f32 / 768.0);
    clipboard::set_text("pasted text");
    let mut buf = vec![0xff00_0000u32; W * H];
    let mut s = Surface { w: W, h: H, stride: W, px: &mut buf, y0: 0 };

    // a selected word with its handles and menu
    let frame = (200.0, 300.0, 800.0, ui.pt(44.0));
    s.fill_rrect(frame.0 - ui.pt(16.0), frame.1, frame.2 + ui.pt(32.0), frame.3, ui.pt(10.0), CELL);
    let mut field = TextField::new(&ui, "Name", 17.0, false, 60);
    field.set_text(&ui, "This domain is for use in documentation examples");
    let x = frame.0 + ui.measure("This domain is for use in docu", 17.0, springboard::shell::ui::Weight::Regular);
    tap(&mut field, &ui, x, frame, 0.0);
    tap(&mut field, &ui, x, frame, 100.0);
    field.draw(&mut s, &ui, frame, 0.0);

    // a long press: caret menu
    let frame2 = (200.0, 520.0, 800.0, ui.pt(44.0));
    s.fill_rrect(frame2.0 - ui.pt(16.0), frame2.1, frame2.2 + ui.pt(32.0), frame2.3, ui.pt(10.0), CELL);
    let mut caret = TextField::new(&ui, "Name", 17.0, false, 60);
    caret.set_text(&ui, "góðan daginn");
    let (cx, cy) = (frame2.0 + ui.pt(30.0), frame2.1 + frame2.3 / 2.0);
    caret.touch(&ui, Event::Down(cx, cy), frame2, W as f32, 1000.0);
    caret.tick(&ui, 1400.0);
    caret.touch(&ui, Event::Up(cx, cy), frame2, W as f32, 1450.0);
    caret.draw(&mut s, &ui, frame2, 0.0);

    // the terminal with a long-press-drag selection
    let area = (0, 700, W as i32, 800);
    let cmd = ["/bin/sh", "-c", "printf '$ ls -la /data/local\\ntotal 12\\ndrwxr-xr-x 3 root root 4096 Oct  4 00:00 springboard\\n-rw------- 1 root root   11 Oct  4 00:00 clipboard.txt\\n$ '; /bin/sleep 2"];
    let mut term = Terminal::spawn(leak("assets/SFUIMono.ttf"), area, ui.pt(15.0), &cmd).unwrap();
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < 500 {
        term.pump();
    }
    let centre = |t: &Terminal, c: usize, r: usize| {
        let (x, y, w, h) = t.cell_rect(c, r);
        (x + w / 2.0, y + h / 2.0)
    };
    let mut tt = TermTouch::default();
    let (x0, y0) = centre(&term, 0, 2);
    tt.down(&ui, &mut term, x0, y0, 0.0);
    tt.tick(&mut term, 400.0);
    let (x1, y1) = centre(&term, 12, 3);
    tt.moved(&ui, &mut term, x1, y1);
    tt.up(&ui, &mut term, x1, y1, W as f32);
    term.draw(&mut s);
    tt.draw(&mut s, &ui);

    let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
    for p in buf {
        ppm.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    fs::write("target/edit-preview.ppm", ppm).unwrap();
}
