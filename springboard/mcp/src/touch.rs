//! Touch input through one persistent uinput touchscreen, created on first use. The compositor offers no
//! virtual pointer, so click, drag and scroll land here: the kernel reports real finger touches, which the
//! shell and every Wayland client treat like the panel's own. The device is shaped like the real panel
//! (its ABS ranges), found in /proc/bus/input/devices as the first direct-input device that is not ours.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

pub struct Touch {
    dev: File,
    /// Panel (digitizer) range, in units: pw x ph.
    pw: i32,
    ph: i32,
    /// Screen size in compositor pixels: sw x sh.
    sw: i32,
    sh: i32,
}

const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
const UI_SET_ABSBIT: libc::c_ulong = 0x40045567;
const UI_SET_PROPBIT: libc::c_ulong = 0x4004556e;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
/// EVIOCGABS(abs): reads struct input_absinfo (6 x i32) for one ABS code.
const EVIOCGABS: libc::c_ulong = 0x80184540;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const BTN_TOUCH: u16 = 0x14a;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const INPUT_PROP_DIRECT: u32 = 1;

impl Touch {
    /// Creates the touchscreen for a screen of `screen` pixels; the panel is the real one when found.
    pub fn open(screen: (u32, u32)) -> Result<Touch, String> {
        let (sw, sh) = (screen.0 as i32, screen.1 as i32);
        if sw <= 0 || sh <= 0 {
            return Err("no screen size yet; touch needs an output".into());
        }
        let (pw, ph) = panel_size().unwrap_or((sw, sh));
        let dev = OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open("/dev/uinput").map_err(|e| format!("/dev/uinput: {e}"))?;
        let fd = dev.as_raw_fd();
        unsafe {
            libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_int);
            libc::ioctl(fd, UI_SET_EVBIT, EV_ABS as libc::c_int);
            libc::ioctl(fd, UI_SET_KEYBIT, BTN_TOUCH as libc::c_int);
            libc::ioctl(fd, UI_SET_PROPBIT, INPUT_PROP_DIRECT as libc::c_int); // a touchscreen, not a touchpad
        }
        let axes = [(ABS_X, pw - 1), (ABS_Y, ph - 1), (ABS_MT_SLOT, 9), (ABS_MT_POSITION_X, pw - 1), (ABS_MT_POSITION_Y, ph - 1), (ABS_MT_TRACKING_ID, 65535)];
        // struct uinput_user_dev: name[80], id {bustype, vendor, product, version}, ff_effects_max, absmax[64], absmin[64], absfuzz[64], absflat[64]
        let mut setup = vec![0u8; 80 + 8 + 4 + 4 * 64 * 4];
        let name = b"bifrost-mcp Touchscreen";
        setup[..name.len()].copy_from_slice(name);
        setup[80..82].copy_from_slice(&3u16.to_ne_bytes()); // BUS_USB
        setup[82..84].copy_from_slice(&0x1d6bu16.to_ne_bytes());
        setup[84..86].copy_from_slice(&0x0105u16.to_ne_bytes());
        setup[86..88].copy_from_slice(&1u16.to_ne_bytes());
        for (code, max) in axes {
            unsafe {
                libc::ioctl(fd, UI_SET_ABSBIT, code as libc::c_int);
            }
            let at = 92 + 4 * code as usize;
            setup[at..at + 4].copy_from_slice(&max.to_ne_bytes());
        }
        (&dev).write_all(&setup).map_err(|e| e.to_string())?;
        if unsafe { libc::ioctl(fd, UI_DEV_CREATE) } < 0 {
            return Err("UI_DEV_CREATE failed".into());
        }
        // the compositor needs a moment to add a new device
        std::thread::sleep(Duration::from_millis(1500));
        Ok(Touch { dev, pw, ph, sw, sh })
    }

    /// Screen pixel (x, y) -> digitizer units. Matches tools/lib/touch.py and was verified on the Galaxy Tab A8:
    /// a portrait panel under a landscape screen has the screen's bottom edge on the panel's left edge.
    pub fn map(&self, x: f64, y: f64) -> (i32, i32) {
        to_panel((self.pw, self.ph), (self.sw, self.sh), x, y)
    }

    fn ev(&mut self, kind: u16, code: u16, value: i32) {
        let mut e = [0u8; 24];
        e[16..18].copy_from_slice(&kind.to_ne_bytes());
        e[18..20].copy_from_slice(&code.to_ne_bytes());
        e[20..24].copy_from_slice(&value.to_ne_bytes());
        // a failed write to a uinput device only means the event was dropped; there is nothing to recover
        let _ = self.dev.write_all(&e);
    }

    fn report(&mut self) {
        self.ev(EV_SYN, SYN_REPORT, 0);
    }

    fn at(&mut self, x: f64, y: f64) {
        let (px, py) = self.map(x, y);
        for (code, v) in [(ABS_MT_POSITION_X, px), (ABS_MT_POSITION_Y, py), (ABS_X, px), (ABS_Y, py)] {
            self.ev(EV_ABS, code, v);
        }
    }

    /// One finger down at (x, y) in screen pixels.
    pub fn down(&mut self, x: f64, y: f64) {
        self.ev(EV_ABS, ABS_MT_SLOT, 0);
        self.ev(EV_ABS, ABS_MT_TRACKING_ID, 1);
        self.at(x, y);
        self.ev(EV_KEY, BTN_TOUCH, 1);
        self.report();
    }

    /// Moves the finger (down) to (x, y).
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.at(x, y);
        self.report();
    }

    /// Lifts the finger.
    pub fn up(&mut self) {
        self.ev(EV_ABS, ABS_MT_SLOT, 0);
        self.ev(EV_ABS, ABS_MT_TRACKING_ID, -1);
        self.ev(EV_KEY, BTN_TOUCH, 0);
        self.report();
    }

    /// A finger down at (x, y) for `hold_ms`, then up.
    pub fn tap(&mut self, x: f64, y: f64, hold_ms: u64) {
        self.down(x, y);
        std::thread::sleep(Duration::from_millis(hold_ms));
        self.up();
    }

    /// A drag from (x1, y1) to (x2, y2) over `ms`, one move about every 16 ms.
    pub fn swipe(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, ms: f64) {
        let steps = ((ms / 16.0) as u32).max(2);
        self.down(x1, y1);
        for k in 1..=steps {
            std::thread::sleep(Duration::from_secs_f64(ms / 1000.0 / steps as f64));
            let t = k as f64 / steps as f64;
            self.move_to(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
        }
        self.up();
    }
}

impl Drop for Touch {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.dev.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

/// The mapping behind `Touch::map`, on plain values.
fn to_panel(panel: (i32, i32), screen: (i32, i32), x: f64, y: f64) -> (i32, i32) {
    let ((pw, ph), (sw, sh)) = ((panel.0 as f64, panel.1 as f64), (screen.0 as f64, screen.1 as f64));
    let (px, py) = if panel.0 < panel.1 && screen.0 > screen.1 {
        // landscape screen on a portrait panel: x runs down the panel, y runs across it from the right
        ((pw - 1.0) - y * pw / sh, x * ph / sw)
    } else {
        (x * pw / sw, y * ph / sh)
    };
    ((px.round().clamp(0.0, pw - 1.0)) as i32, (py.round().clamp(0.0, ph - 1.0)) as i32)
}

/// The real touchscreen's panel size (max ABS_MT_POSITION + 1), from the first direct-input device in
/// /proc/bus/input/devices that is not this server's own device.
fn panel_size() -> Option<(i32, i32)> {
    let devices = std::fs::read_to_string("/proc/bus/input/devices").ok()?;
    for block in devices.split("\n\n") {
        let (mut name, mut prop, mut handler) = ("", 0u64, None);
        for line in block.lines() {
            if let Some(n) = line.strip_prefix("N: Name=") {
                name = n.trim_matches('"');
            } else if let Some(p) = line.strip_prefix("B: PROP=") {
                prop = u64::from_str_radix(p.trim(), 16).unwrap_or(0);
            } else if let Some(h) = line.strip_prefix("H: Handlers=") {
                handler = h.split_whitespace().find(|t| t.starts_with("event")).map(String::from);
            }
        }
        if prop & (1 << INPUT_PROP_DIRECT) == 0 || name.starts_with("bifrost-mcp") {
            continue;
        }
        let Some(event) = handler else { continue };
        let file = OpenOptions::new().read(true).open(format!("/dev/input/{event}")).ok()?;
        let max_x = abs_max(&file, ABS_MT_POSITION_X)?;
        let max_y = abs_max(&file, ABS_MT_POSITION_Y)?;
        return Some((max_x + 1, max_y + 1));
    }
    None
}

/// The maximum of one ABS axis of an input device (EVIOCGABS).
fn abs_max(file: &File, code: u16) -> Option<i32> {
    let mut info = [0i32; 6]; // value, minimum, maximum, fuzz, flat, resolution
    let r = unsafe { libc::ioctl(file.as_raw_fd(), EVIOCGABS + code as libc::c_ulong, info.as_mut_ptr() as *mut libc::c_void) };
    (r >= 0).then_some(info[2])
}

#[cfg(test)]
mod tests {
    use super::to_panel;

    #[test]
    fn landscape_screen_maps_onto_portrait_panel() {
        // the panel is 1200x1920, the screen 1920x1200 landscape
        assert_eq!(to_panel((1200, 1920), (1920, 1200), 75.0, 870.0), (329, 75));
        assert_eq!(to_panel((1200, 1920), (1920, 1200), 0.0, 0.0), (1199, 0));
        assert_eq!(to_panel((1200, 1920), (1920, 1200), 1919.0, 1199.0), (0, 1919));
    }

    #[test]
    fn same_orientation_scales_straight() {
        assert_eq!(to_panel((1200, 1920), (1200, 1920), 600.0, 960.0), (600, 960));
        assert_eq!(to_panel((2000, 1000), (1000, 500), 500.0, 250.0), (1000, 500));
        assert_eq!(to_panel((2000, 1000), (1000, 500), 5000.0, -4.0), (1999, 0));
    }
}
