//! The Calendar widget: the weekday in red over a big day number.

use super::{put, tile, Kind, PAD_PT};
use crate::gfx::Image;
use crate::shell::ui::{Ui, Weight, SECONDARY, WHITE};
use crate::shell::Info;

const DAYS: [&str; 7] = ["SUNDAY", "MONDAY", "TUESDAY", "WEDNESDAY", "THURSDAY", "FRIDAY", "SATURDAY"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

pub fn key(info: &Info) -> String {
    format!("calendar:{}:{}:{}", info.wday, info.mday, info.mon)
}

pub fn render(ui: &Ui, info: &Info) -> Image {
    let s = ui.s;
    let (w, h) = Kind::Calendar.size(s);
    let mut img = tile(s, w, h);
    let pad = ui.pt(PAD_PT);
    let day = ui.text(DAYS[info.wday.rem_euclid(7) as usize], 13.0, Weight::Semibold, super::battery::RED);
    put(&mut img, &day, pad, pad - ui.pt(2.0));
    let num = ui.text(&info.mday.to_string(), 46.0, Weight::Regular, WHITE);
    put(&mut img, &num, pad - ui.pt(3.0), pad + ui.pt(14.0));
    let month = ui.text(MONTHS[info.mon.rem_euclid(12) as usize], 13.0, Weight::Regular, SECONDARY);
    put(&mut img, &month, pad, pad + ui.pt(14.0) + num.h as f32 - ui.pt(4.0));
    let note = ui.text("Open Calendar", 12.0, Weight::Medium, SECONDARY);
    put(&mut img, &note, pad, h as f32 - pad - note.h as f32);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_date_and_keys_on_it() {
        let ui = crate::shell::tests::ui();
        let a = Info { wday: 0, mday: 4, mon: 9, ..Info::default() };
        let mut b = a.clone();
        b.mday = 5;
        assert_ne!(key(&a), key(&b));
        let img = render(&ui, &a);
        assert_eq!((img.w, img.h), Kind::Calendar.size(ui.s));
        let pad = ui.pt(PAD_PT) as usize;
        let red = img.px[pad * img.w..(pad + 20) * img.w].iter().any(|&p| p & 0xff > 200 && (p >> 8) & 0xff < 120);
        assert!(red, "the weekday is red");
    }
}
