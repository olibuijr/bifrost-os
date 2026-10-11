//! Key chords through one persistent uinput keyboard, created on first use. Kernel-level, so the
//! shell's evdev shortcuts (Cmd+Space, Cmd+Q) and every Wayland client see it like a real keyboard.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

pub struct Keyboard {
    dev: File,
}

const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;

impl Keyboard {
    pub fn open() -> Result<Keyboard, String> {
        let dev = OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open("/dev/uinput").map_err(|e| format!("/dev/uinput: {e}"))?;
        let fd = dev.as_raw_fd();
        unsafe {
            libc::ioctl(fd, UI_SET_EVBIT, 1 as libc::c_int);
            for code in 1..249 {
                libc::ioctl(fd, UI_SET_KEYBIT, code as libc::c_int);
            }
        }
        // struct uinput_user_dev: name[80], id {bustype, vendor, product, version}, ff_effects_max, abs arrays
        let mut setup = vec![0u8; 80 + 8 + 4 + 4 * 64 * 4];
        let name = b"BifrOSt MCP keyboard";
        setup[..name.len()].copy_from_slice(name);
        setup[80..82].copy_from_slice(&3u16.to_ne_bytes()); // BUS_USB
        setup[82..84].copy_from_slice(&0x1d6bu16.to_ne_bytes());
        setup[84..86].copy_from_slice(&0x0104u16.to_ne_bytes());
        (&dev).write_all(&setup).map_err(|e| e.to_string())?;
        if unsafe { libc::ioctl(fd, UI_DEV_CREATE) } < 0 {
            return Err("UI_DEV_CREATE failed".into());
        }
        // libinput and the shell need a moment to open a new device
        std::thread::sleep(Duration::from_millis(400));
        Ok(Keyboard { dev })
    }

    fn ev(&mut self, kind: u16, code: u16, value: i32) -> Result<(), String> {
        let mut e = [0u8; 24];
        e[16..18].copy_from_slice(&kind.to_ne_bytes());
        e[18..20].copy_from_slice(&code.to_ne_bytes());
        e[20..24].copy_from_slice(&value.to_ne_bytes());
        self.dev.write_all(&e).map_err(|e| e.to_string())
    }

    fn key(&mut self, code: u16, down: bool) -> Result<(), String> {
        self.ev(1, code, down as i32)?;
        self.ev(0, 0, 0)
    }

    /// Presses a chord such as `cmd+shift+s` (modifiers down in order, released in reverse).
    pub fn chord(&mut self, spec: &str, hold_ms: u64) -> Result<(), String> {
        let codes = parse(spec)?;
        for &c in &codes {
            self.key(c, true)?;
            std::thread::sleep(Duration::from_millis(8));
        }
        std::thread::sleep(Duration::from_millis(hold_ms));
        for &c in codes.iter().rev() {
            self.key(c, false)?;
            std::thread::sleep(Duration::from_millis(4));
        }
        Ok(())
    }
}

impl Keyboard {
    /// Types ASCII as a US keyboard would (code, Shift). Readers of raw key codes (the shell) and
    /// layout-aware clients both get the right characters on the default layout.
    pub fn type_ascii(&mut self, text: &str, delay_ms: u64) -> Result<(), String> {
        for c in text.chars() {
            let (code, shift) = ascii_key(c).ok_or_else(|| format!("{c:?} is not US-keyboard ASCII"))?;
            if shift {
                self.key(42, true)?;
            }
            self.key(code, true)?;
            self.key(code, false)?;
            if shift {
                self.key(42, false)?;
            }
            std::thread::sleep(Duration::from_millis(delay_ms.max(2)));
        }
        Ok(())
    }
}

/// The US key and Shift state that type `c`.
pub fn ascii_key(c: char) -> Option<(u16, bool)> {
    const PLAIN: &str = "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./";
    const SHIFTED: &str = "~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:\"ZXCVBNM<>?";
    const CODES: [u16; 47] = [41, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 43, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53];
    match c {
        ' ' => return Some((57, false)),
        '\n' => return Some((28, false)),
        '\t' => return Some((15, false)),
        _ => {}
    }
    if let Some(i) = PLAIN.chars().position(|p| p == c) {
        return Some((CODES[i], false));
    }
    SHIFTED.chars().position(|p| p == c).map(|i| (CODES[i], true))
}

/// `cmd+shift+s` -> evdev codes. Names follow the US layout's physical keys.
pub fn parse(spec: &str) -> Result<Vec<u16>, String> {
    if spec == "+" {
        return Err("\"+\" is text: use the type tool, or plus for the shifted key".into());
    }
    spec.split('+').map(|p| code(p.trim()).ok_or_else(|| format!("unknown key {p:?} (see the key tool description)"))).collect()
}

fn code(name: &str) -> Option<u16> {
    let n = name.to_lowercase();
    let row = |s: &str, first: u16| s.find(n.as_str()).filter(|_| n.len() == 1).map(|i| first + i as u16);
    Some(match n.as_str() {
        "cmd" | "super" | "meta" | "win" | "command" => 125,
        "ctrl" | "control" => 29,
        "shift" => 42,
        "alt" | "option" | "opt" => 56,
        "rightalt" | "altgr" => 100,
        "enter" | "return" => 28,
        "esc" | "escape" => 1,
        "tab" => 15,
        "space" => 57,
        "backspace" => 14,
        "delete" | "del" => 111,
        "insert" => 110,
        "home" => 102,
        "end" => 107,
        "pageup" => 104,
        "pagedown" => 109,
        "up" => 103,
        "down" => 108,
        "left" => 105,
        "right" => 106,
        "capslock" => 58,
        "minus" | "-" => 12,
        "equal" | "=" => 13,
        "plus" => 13,
        "[" => 26,
        "]" => 27,
        ";" => 39,
        "'" => 40,
        "`" | "grave" => 41,
        "\\" => 43,
        "comma" | "," => 51,
        "period" | "." => 52,
        "slash" | "/" => 53,
        "volumeup" => 115,
        "volumedown" => 114,
        "mute" => 113,
        "power" => 116,
        "brightnessup" => 225,
        "brightnessdown" => 224,
        "printscreen" | "print" => 99,
        f if f.len() >= 2 && f.starts_with('f') && f[1..].parse::<u16>().is_ok_and(|k| (1..=12).contains(&k)) => {
            let k: u16 = f[1..].parse().unwrap();
            if k <= 10 { 58 + k } else { 76 + k }
        }
        _ => row("1234567890", 2).or_else(|| row("qwertyuiop", 16)).or_else(|| row("asdfghjkl", 30)).or_else(|| row("zxcvbnm", 44))?,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn chords_map_to_evdev() {
        assert_eq!(super::parse("cmd+shift+s").unwrap(), vec![125, 42, 31]);
        assert_eq!(super::parse("enter").unwrap(), vec![28]);
        assert_eq!(super::parse("f11").unwrap(), vec![87]);
        assert_eq!(super::parse("0").unwrap(), vec![11]);
        assert!(super::parse("cmd+nope").is_err());
        assert_eq!(super::ascii_key('a'), Some((30, false)));
        assert_eq!(super::ascii_key('A'), Some((30, true)));
        assert_eq!(super::ascii_key('+'), Some((13, true)));
        assert_eq!(super::ascii_key('/'), Some((53, false)));
        assert_eq!(super::ascii_key('?'), Some((53, true)));
        assert_eq!(super::ascii_key('\\'), Some((43, false)));
        assert_eq!(super::ascii_key('þ'), None);
    }
}
