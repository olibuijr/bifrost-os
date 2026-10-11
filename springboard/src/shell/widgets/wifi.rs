//! The Wi-Fi widget: network name, address and signal fan.

use super::{put, surface, tile, Kind, PAD_PT};
use crate::gfx::Image;
use crate::shell::ui::{Ui, Weight, SECONDARY, WHITE};
use crate::shell::Info;
use crate::status;

pub fn key(info: &Info) -> String {
    format!("wifi:{}:{}:{}:{}:{}:{}", info.online, info.wired, info.wifi_adapter, info.wifi, info.ip, info.wifi_bars)
}

pub fn render(ui: &Ui, info: &Info) -> Image {
    let s = ui.s;
    let (w, h) = Kind::Wifi.size(s);
    let mut img = tile(s, w, h);
    let pad = ui.pt(PAD_PT);
    let fan = ui.pt(status::WIFI_PT * 1.3);
    status::draw_wifi_bars(&mut surface(&mut img), pad, pad + ui.pt(2.0), fan, if info.online { info.wifi_bars } else { 0 }, WHITE);
    let title = ui.text("Wi-Fi", 13.0, Weight::Semibold, SECONDARY);
    put(&mut img, &title, pad + 2.0 * fan + ui.pt(8.0), pad + fan - title.h as f32 / 2.0 + ui.pt(1.0));
    let name = if info.online {
        info.wifi.as_str()
    } else if !info.wifi_adapter {
        "No Wi-Fi adapter"
    } else if info.wifi_on {
        "Not Connected"
    } else {
        "Off"
    };
    let name = ui.text(&ui.fit(name, 20.0, w as f32 - 2.0 * pad), 20.0, Weight::Semibold, WHITE);
    put(&mut img, &name, pad, ui.pt(60.0));
    if info.online {
        let ip = ui.text(&ui.fit(&info.ip, 13.0, w as f32 - 2.0 * pad), 13.0, Weight::Regular, SECONDARY);
        put(&mut img, &ip, pad, ui.pt(60.0) + name.h as f32 + ui.pt(2.0));
    }
    let foot = if info.online {
        "Connected"
    } else if info.wired {
        "Ethernet connected"
    } else {
        "No network"
    };
    let foot = ui.text(foot, 12.0, Weight::Medium, SECONDARY);
    put(&mut img, &foot, pad, h as f32 - pad - foot.h as f32);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_tracks_the_fields_shown_and_offline_hides_the_address() {
        let ui = crate::shell::tests::ui();
        let on = Info { online: true, wifi: "Home".into(), ip: "10.0.0.2".into(), wifi_bars: 3, wifi_on: true, ..Info::default() };
        let mut off = on.clone();
        off.online = false;
        assert_ne!(key(&on), key(&off));
        let a = render(&ui, &on);
        let b = render(&ui, &off);
        assert_eq!((a.w, a.h), Kind::Wifi.size(ui.s));
        let (top, bottom) = (ui.pt(60.0 + 30.0) as usize, ui.pt(60.0 + 56.0) as usize);
        let ink = |img: &Image| img.px[top * img.w..bottom * img.w].iter().filter(|&&p| p & 0xff > 120).count();
        assert!(ink(&a) > ink(&b), "the address row is drawn only when online");
    }
}
