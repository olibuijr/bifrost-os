//! Agent control of one terminal: screen text, cursor, key names. Used by `shell/terminal/control.rs`.
use super::*;

/// Bytes for one key spec: a named key ("Enter", "Up"), "ctrl+c", "alt+x", "F1"-"F12", or a single character.
pub fn spec_bytes(spec: &str, app_cursor: bool) -> Option<Vec<u8>> {
    let lower = spec.to_ascii_lowercase();
    if let Some(c) = lower.strip_prefix("ctrl+") {
        let b = match c.as_bytes() {
            [b @ b'a'..=b'z'] => *b - b'a' + 1,
            b"[" => 27,
            b"\\" => 28,
            b"]" => 29,
            b"space" | b"@" => 0,
            _ => return None,
        };
        return Some(vec![b]);
    }
    if lower.starts_with("alt+") {
        let mut v = vec![0x1b];
        v.extend(spec_bytes(&spec[4..], app_cursor)?);
        return Some(v);
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=12).contains(n)) {
        const F: [&str; 12] = ["\x1bOP", "\x1bOQ", "\x1bOR", "\x1bOS", "\x1b[15~", "\x1b[17~", "\x1b[18~", "\x1b[19~", "\x1b[20~", "\x1b[21~", "\x1b[23~", "\x1b[24~"];
        return Some(F[n - 1].as_bytes().to_vec());
    }
    let canon = match lower.as_str() {
        "enter" | "return" => "Enter",
        "backspace" => "Backspace",
        "tab" => "Tab",
        "escape" | "esc" => "Escape",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "delete" => "Delete",
        "space" => return Some(b" ".to_vec()),
        _ => "",
    };
    if !canon.is_empty() {
        return Some(key_bytes(canon, app_cursor));
    }
    (spec.chars().count() == 1).then(|| spec.as_bytes().to_vec())
}

/// Bytes for a space-separated list of key specs; the first unknown one is the error.
pub fn keys_bytes(keys: &str, app_cursor: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for k in keys.split_whitespace() {
        out.extend(spec_bytes(k, app_cursor).ok_or_else(|| format!("unknown key {k:?} (Enter, Tab, Escape, Up, ctrl+c, alt+x, F1..F12, or one character)"))?);
    }
    Ok(out)
}

impl Terminal {
    /// Grid size (columns, rows).
    pub fn size(&self) -> (usize, usize) {
        (self.term.cols(), self.term.rows())
    }

    /// Cursor (column, row) on the live screen.
    pub fn cursor_pos(&self) -> (usize, usize) {
        self.term.cursor()
    }

    pub fn app_cursor_mode(&self) -> bool {
        self.term.app_cursor()
    }

    /// The screen as text with `scrollback` history lines above it (clamped); trailing blanks trimmed.
    pub fn screen_text(&self, scrollback: usize) -> String {
        let total = self.term.line_count();
        let rows = self.term.rows().min(total);
        let start = total - rows - scrollback.min(total - rows);
        termcore::select::dump(total - start, |i| self.term.line(start + i))
    }

    /// Resizes the grid to `cols` x `rows` cells (until the window next changes size).
    pub fn resize_cells(&mut self, cols: usize, rows: usize) {
        let (x, y, ..) = self.area;
        let (cw, ch, pad) = (self.metrics.cw, self.metrics.ch, self.metrics.pad);
        self.resize((x, y, cols.max(1) as i32 * cw + 2 * pad, rows.max(1) as i32 * ch + 2 * pad));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_specs() {
        assert_eq!(keys_bytes("ctrl+c", false).unwrap(), [3]);
        assert_eq!(keys_bytes("ctrl+d Enter", false).unwrap(), b"\x04\r");
        assert_eq!(keys_bytes("alt+x", false).unwrap(), b"\x1bx");
        assert_eq!(keys_bytes("Up Up", true).unwrap(), b"\x1bOA\x1bOA");
        assert_eq!(keys_bytes("F1 q", false).unwrap(), b"\x1bOPq");
        assert!(keys_bytes("Hyper", false).is_err());
        assert!(keys_bytes("ctrl+1", false).is_err());
    }
}
