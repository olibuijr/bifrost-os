//! Keyboard pressed-state visuals: key shadow and the contextual return key.
//! bifrost shows no key popup; a pressed letter takes the special-key shade,
//! a pressed special key takes the letter shade, and the return key turns
//! blue with the field's action word (Search, Go, Done...).

use springboard::gfx::{pack, rgba, Surface};

/// What the focused field wants its return key to say.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ReturnKind {
    #[default]
    Return,
    Search,
    Go,
    Done,
    Next,
    Send,
}

impl ReturnKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Return => "return",
            Self::Search => "search",
            Self::Go => "go",
            Self::Done => "done",
            Self::Next => "next",
            Self::Send => "send",
        }
    }

    /// Only plain return keeps the grey special-key shade.
    pub fn blue(self) -> bool {
        self != Self::Return
    }
}

/// Fill of the return key, pressed or not; None means use the normal rule.
pub fn return_fill(kind: ReturnKind, down: bool) -> Option<u32> {
    if !kind.blue() {
        return None;
    }
    Some(if down { pack(10, 100, 210, 255) } else { pack(10, 132, 255, 255) })
}

/// The one-pixel-ish shadow under a key, drawn before the key itself.
pub fn key_shadow(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, r: f32, scale: f32) {
    let d = (scale * 1.0).max(1.0);
    s.fill_rrect(x, y + d, w, h, r, rgba(0, 0, 0, 110));
}
