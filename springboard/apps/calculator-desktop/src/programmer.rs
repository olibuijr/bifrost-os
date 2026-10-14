//! Programmer mode arithmetic: a 64-bit word shown in base 2, 8, 10 or 16, with bitwise operators, shifts,
//! rotates, byte and word flips, two's complement and an ASCII view. Independent of the touch UI.
#![allow(dead_code)] // the Programmer segment in the toolbar is the next step; tests drive it

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Bin,
    Oct,
    Dec,
    Hex,
}

impl Base {
    pub const ALL: [Base; 4] = [Base::Hex, Base::Dec, Base::Oct, Base::Bin];

    pub fn radix(self) -> u32 {
        match self {
            Self::Bin => 2,
            Self::Oct => 8,
            Self::Dec => 10,
            Self::Hex => 16,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bin => "BIN",
            Self::Oct => "OCT",
            Self::Dec => "DEC",
            Self::Hex => "HEX",
        }
    }
}

/// Word width the value is truncated to after every operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
    Dword,
    Qword,
}

impl Width {
    pub fn bits(self) -> u32 {
        match self {
            Self::Byte => 8,
            Self::Word => 16,
            Self::Dword => 32,
            Self::Qword => 64,
        }
    }

    pub fn mask(self) -> u64 {
        if self == Self::Qword {
            u64::MAX
        } else {
            (1u64 << self.bits()) - 1
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Nor,
    /// Shift left by the right operand.
    Shl,
    /// Logical shift right by the right operand.
    Shr,
    RotL,
    RotR,
}

#[derive(Clone, Debug)]
pub struct Programmer {
    pub base: Base,
    pub width: Width,
    value: u64,
    pending: Option<(u64, BinOp)>,
    /// Digits typed since the last operator; `None` means the display shows a result.
    entry: Option<String>,
}

impl Default for Programmer {
    fn default() -> Self {
        Self::new()
    }
}

impl Programmer {
    pub fn new() -> Self {
        Programmer { base: Base::Dec, width: Width::Qword, value: 0, pending: None, entry: None }
    }

    pub fn value(&self) -> u64 {
        self.value
    }

    pub fn set_value(&mut self, value: u64) {
        self.value = value & self.width.mask();
        self.entry = None;
    }

    /// The value as a signed number of the current width (two's complement).
    pub fn signed(&self) -> i64 {
        let bits = self.width.bits();
        if bits == 64 {
            self.value as i64
        } else if self.value >> (bits - 1) & 1 == 1 {
            (self.value | !self.width.mask()) as i64
        } else {
            self.value as i64
        }
    }

    pub fn set_base(&mut self, base: Base) {
        self.base = base;
        self.entry = None;
    }

    pub fn set_width(&mut self, width: Width) {
        self.width = width;
        self.value &= width.mask();
        self.entry = None;
    }

    /// Whether `digit` (0-15) is a legal keypad digit in the current base.
    pub fn digit_enabled(&self, digit: u32) -> bool {
        digit < self.base.radix()
    }

    /// Types a digit; illegal digits for the base and overflow past the word width are ignored.
    pub fn digit(&mut self, digit: u32) -> bool {
        if !self.digit_enabled(digit) {
            return false;
        }
        let mut text = self.entry.clone().unwrap_or_default();
        text.push(char::from_digit(digit, 16).unwrap_or('0'));
        match u64::from_str_radix(&text, self.base.radix()) {
            Ok(v) if v <= self.width.mask() => {
                self.value = v;
                self.entry = Some(text);
                true
            }
            _ => false,
        }
    }

    pub fn backspace(&mut self) {
        if let Some(text) = self.entry.as_mut() {
            text.pop();
            self.value = u64::from_str_radix(text, self.base.radix()).unwrap_or(0);
            if text.is_empty() {
                self.entry = None;
            }
        } else {
            self.value = 0;
        }
    }

    pub fn clear(&mut self) {
        self.value = 0;
        self.pending = None;
        self.entry = None;
    }

    /// Sets the pending operator; an earlier pending one is resolved first, as a chained calculation.
    pub fn operator(&mut self, op: BinOp) {
        self.equals();
        self.pending = Some((self.value, op));
        self.entry = None;
    }

    /// Resolves the pending operation. Divide and modulo by zero leave the left operand unchanged.
    pub fn equals(&mut self) {
        if let Some((left, op)) = self.pending.take() {
            self.value = self.apply(left, op, self.value);
        }
        self.entry = None;
    }

    fn apply(&self, left: u64, op: BinOp, right: u64) -> u64 {
        let mask = self.width.mask();
        let bits = self.width.bits();
        let out = match op {
            BinOp::Add => left.wrapping_add(right),
            BinOp::Sub => left.wrapping_sub(right),
            BinOp::Mul => left.wrapping_mul(right),
            BinOp::Div => left.checked_div(right).unwrap_or(left),
            BinOp::Mod => left.checked_rem(right).unwrap_or(left),
            BinOp::And => left & right,
            BinOp::Or => left | right,
            BinOp::Xor => left ^ right,
            BinOp::Nor => !(left | right),
            BinOp::Shl => if right >= bits as u64 { 0 } else { left << right },
            BinOp::Shr => if right >= bits as u64 { 0 } else { left >> right },
            BinOp::RotL => rotate(left, right, bits, true),
            BinOp::RotR => rotate(left, right, bits, false),
        };
        out & mask
    }

    pub fn not(&mut self) {
        self.value = !self.value & self.width.mask();
        self.entry = None;
    }

    /// Two's complement negation.
    pub fn negate(&mut self) {
        self.value = self.value.wrapping_neg() & self.width.mask();
        self.entry = None;
    }

    /// Reverses the order of the bytes in the word.
    pub fn flip_bytes(&mut self) {
        let bytes = (self.width.bits() / 8) as usize;
        let mut out = 0u64;
        for i in 0..bytes {
            out |= ((self.value >> (8 * i)) & 0xff) << (8 * (bytes - 1 - i));
        }
        self.value = out & self.width.mask();
        self.entry = None;
    }

    /// Swaps the 16-bit words of the value.
    pub fn flip_words(&mut self) {
        let words = (self.width.bits() / 16).max(1) as usize;
        let mut out = 0u64;
        for i in 0..words {
            out |= ((self.value >> (16 * i)) & 0xffff) << (16 * (words - 1 - i));
        }
        self.value = out & self.width.mask();
        self.entry = None;
    }

    /// Toggles one bit (0 is the least significant), as a tap on the binary view does.
    pub fn toggle_bit(&mut self, bit: u32) {
        if bit < self.width.bits() {
            self.value ^= 1u64 << bit;
            self.entry = None;
        }
    }

    /// The value in `base`, upper case, grouped like the display (nibbles in binary, bytes in hex).
    pub fn text(&self, base: Base) -> String {
        let raw = match base {
            Base::Bin => format!("{:b}", self.value),
            Base::Oct => format!("{:o}", self.value),
            Base::Dec => self.signed().to_string(),
            Base::Hex => format!("{:X}", self.value),
        };
        let group = match base {
            Base::Bin => 4,
            Base::Hex => 2,
            Base::Oct => 3,
            Base::Dec => 3,
        };
        if base == Base::Dec {
            return group_from_right(raw.trim_start_matches('-'), group, ',', raw.starts_with('-'));
        }
        group_from_right(&raw, group, ' ', false)
    }

    /// The binary view: one entry per bit from the most significant of the width down to bit 0.
    pub fn bits(&self) -> Vec<bool> {
        (0..self.width.bits()).rev().map(|b| self.value >> b & 1 == 1).collect()
    }

    /// The bytes of the value as printable ASCII, `.` for anything else, most significant byte first.
    pub fn ascii(&self) -> String {
        let bytes = self.width.bits() / 8;
        (0..bytes)
            .rev()
            .map(|i| {
                let b = (self.value >> (8 * i)) as u8;
                if (0x20..0x7f).contains(&b) { b as char } else { '.' }
            })
            .collect()
    }
}

fn rotate(value: u64, by: u64, bits: u32, left: bool) -> u64 {
    let by = (by % bits as u64) as u32;
    if by == 0 {
        return value;
    }
    let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
    let v = value & mask;
    let (a, b) = if left { (by, bits - by) } else { (bits - by, by) };
    ((v << a) | (v >> b)) & mask
}

fn group_from_right(digits: &str, size: usize, sep: char, negative: bool) -> String {
    let mut out = String::new();
    let len = digits.chars().count();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i) % size == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    if negative { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(p: &mut Programmer, digits: &[u32]) {
        for d in digits {
            p.digit(*d);
        }
    }

    #[test]
    fn typing_respects_the_base() {
        let mut p = Programmer::new();
        p.set_base(Base::Bin);
        assert!(!p.digit(2));
        typed(&mut p, &[1, 0, 1]);
        assert_eq!(p.value(), 5);
        p.set_base(Base::Hex);
        p.set_value(0);
        typed(&mut p, &[15, 15]);
        assert_eq!(p.value(), 255);
    }

    #[test]
    fn width_stops_overflow_and_masks_results() {
        let mut p = Programmer::new();
        p.set_width(Width::Byte);
        typed(&mut p, &[2, 5, 5]);
        assert_eq!(p.value(), 255);
        assert!(!p.digit(0));
        p.operator(BinOp::Add);
        typed(&mut p, &[1]);
        p.equals();
        assert_eq!(p.value(), 0);
    }

    #[test]
    fn bitwise_operators() {
        let mut p = Programmer::new();
        p.set_value(0b1100);
        p.operator(BinOp::And);
        typed(&mut p, &[1, 0]);
        p.equals();
        assert_eq!(p.value(), 0b1000);
        p.set_value(0b1100);
        p.operator(BinOp::Xor);
        typed(&mut p, &[1, 0]);
        p.equals();
        assert_eq!(p.value(), 0b0110);
        p.set_width(Width::Byte);
        p.set_value(0b1100);
        p.operator(BinOp::Nor);
        typed(&mut p, &[3]);
        p.equals();
        assert_eq!(p.value(), 0b1111_0000);
    }

    #[test]
    fn shifts_and_rotates_obey_the_width() {
        let mut p = Programmer::new();
        p.set_width(Width::Byte);
        p.set_value(0b1000_0001);
        p.operator(BinOp::RotL);
        typed(&mut p, &[1]);
        p.equals();
        assert_eq!(p.value(), 0b0000_0011);
        p.set_value(0b1000_0001);
        p.operator(BinOp::Shl);
        typed(&mut p, &[1]);
        p.equals();
        assert_eq!(p.value(), 0b0000_0010);
        p.set_value(0b1000_0001);
        p.operator(BinOp::Shr);
        typed(&mut p, &[9]);
        p.equals();
        assert_eq!(p.value(), 0);
    }

    #[test]
    fn negate_and_signed_view_are_twos_complement() {
        let mut p = Programmer::new();
        p.set_width(Width::Byte);
        p.set_value(1);
        p.negate();
        assert_eq!(p.value(), 0xff);
        assert_eq!(p.signed(), -1);
        assert_eq!(p.text(Base::Dec), "-1");
        p.not();
        assert_eq!(p.value(), 0);
    }

    #[test]
    fn flips_reverse_bytes_and_words() {
        let mut p = Programmer::new();
        p.set_width(Width::Dword);
        p.set_value(0x1122_3344);
        p.flip_bytes();
        assert_eq!(p.value(), 0x4433_2211);
        p.set_value(0x1122_3344);
        p.flip_words();
        assert_eq!(p.value(), 0x3344_1122);
    }

    #[test]
    fn division_by_zero_keeps_the_left_operand() {
        let mut p = Programmer::new();
        typed(&mut p, &[7]);
        p.operator(BinOp::Div);
        typed(&mut p, &[0]);
        p.equals();
        assert_eq!(p.value(), 7);
    }

    #[test]
    fn views_group_and_ascii_prints() {
        let mut p = Programmer::new();
        p.set_width(Width::Word);
        p.set_value(0x4142);
        assert_eq!(p.text(Base::Hex), "41 42");
        assert_eq!(p.text(Base::Bin), "100 0001 0100 0010");
        assert_eq!(p.ascii(), "AB");
        assert_eq!(p.bits().len(), 16);
        p.toggle_bit(0);
        assert_eq!(p.value(), 0x4143);
        p.set_width(Width::Qword);
        p.set_value(1234567);
        assert_eq!(p.text(Base::Dec), "1,234,567");
    }

    #[test]
    fn backspace_walks_back_through_the_entry() {
        let mut p = Programmer::new();
        typed(&mut p, &[1, 2, 3]);
        p.backspace();
        assert_eq!(p.value(), 12);
        p.backspace();
        p.backspace();
        assert_eq!(p.value(), 0);
    }
}
