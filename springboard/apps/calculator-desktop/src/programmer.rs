//! Programmer mode: an integer calculator on an unsigned word of 8, 16, 32 or 64 bits, shown in binary, octal,
//! decimal (signed) or hexadecimal, with the bitwise operations, shifts, rotates, byte and word flips and
//! two's complement. Pure logic, no UI.

/// The base the value is typed and shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Bin,
    Oct,
    Dec,
    Hex,
}

impl Base {
    pub const ALL: [Base; 4] = [Base::Bin, Base::Oct, Base::Dec, Base::Hex];

    pub fn radix(self) -> u32 {
        match self {
            Base::Bin => 2,
            Base::Oct => 8,
            Base::Dec => 10,
            Base::Hex => 16,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Base::Bin => "BIN",
            Base::Oct => "OCT",
            Base::Dec => "DEC",
            Base::Hex => "HEX",
        }
    }
}

/// A two-operand operation: the pending operator waits for its right operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    And,
    Or,
    Xor,
    Nor,
    /// `x << y`: shift the left operand by the right one.
    Shl,
    /// `x >> y` (logical).
    Shr,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "×",
            BinOp::Div => "÷",
            BinOp::And => "AND",
            BinOp::Or => "OR",
            BinOp::Xor => "XOR",
            BinOp::Nor => "NOR",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }
}

/// What a programmer key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PKey {
    /// A digit 0 to 15; digits at or above the base are ignored.
    Digit(u8),
    /// The same digit twice (`00`, `FF`).
    Pair(u8),
    Op(BinOp),
    Equals,
    Not,
    /// Two's complement (change sign).
    Negate,
    /// Shift left or right by one bit.
    Shl1,
    Shr1,
    /// Rotate by one bit inside the word.
    Rol,
    Ror,
    /// Reverse the byte order, or the order of the 16-bit words.
    FlipBytes,
    FlipWords,
    SetBase(Base),
    SetBits(u8),
    Clear,
    Backspace,
}

/// The word sizes in bits.
pub const WORD_SIZES: [u8; 4] = [8, 16, 32, 64];

/// A finished calculation for the history: the expression, the result as signed decimal text and its value.
pub type Finished = (String, String, f64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Programmer {
    value: u64,
    /// The pending operator and its left operand.
    pending: Option<(BinOp, u64)>,
    /// The next digit starts a new number.
    fresh: bool,
    pub base: Base,
    bits: u8,
    error: bool,
    /// The line above the value: the left operand and the pending operator, or the finished calculation.
    pub expression: String,
}

impl Default for Programmer {
    fn default() -> Self {
        Self { value: 0, pending: None, fresh: true, base: Base::Dec, bits: 64, error: false, expression: String::new() }
    }
}

impl Programmer {
    /// The word size in bits.
    pub fn bits(&self) -> u8 {
        self.bits
    }

    fn mask(&self) -> u64 {
        if self.bits >= 64 {
            u64::MAX
        } else {
            (1u64 << self.bits) - 1
        }
    }

    /// The value as an unsigned word.
    pub fn value(&self) -> u64 {
        self.value
    }

    /// The value read as a two's complement number of the word size.
    pub fn signed(&self) -> i64 {
        self.signed_of(self.value)
    }

    fn signed_of(&self, value: u64) -> i64 {
        let shift = 64 - u32::from(self.bits);
        ((value << shift) as i64) >> shift
    }

    /// AC when there is nothing to clear (a zero value and no calculation under way), else C.
    pub fn clear_label(&self) -> &'static str {
        if self.error || (self.value == 0 && self.pending.is_none() && self.expression.is_empty()) {
            "AC"
        } else {
            "C"
        }
    }

    pub fn is_error(&self) -> bool {
        self.error
    }

    /// `value` written in `base` (decimal is signed).
    pub fn format(&self, value: u64, base: Base) -> String {
        match base {
            Base::Bin => format!("{value:b}"),
            Base::Oct => format!("{value:o}"),
            Base::Hex => format!("{value:X}"),
            Base::Dec => self.signed_of(value).to_string(),
        }
    }

    /// The value as the display shows it.
    pub fn text(&self) -> String {
        if self.error {
            "Error".into()
        } else {
            self.format(self.value, self.base)
        }
    }

    /// The Unicode scalar the value stands for, when it is a printable one (the ASCII / Unicode readout).
    pub fn character(&self) -> Option<char> {
        let c = u32::try_from(self.value).ok().and_then(char::from_u32)?;
        (c == ' ' || !(c.is_control() || c.is_whitespace())).then_some(c)
    }

    /// The readout under the value: the code point and the character it stands for.
    pub fn readout(&self) -> String {
        if self.error {
            return String::new();
        }
        let scalar = u32::try_from(self.value).ok().and_then(char::from_u32);
        match (scalar, self.character()) {
            (Some(_), Some(c)) => format!("U+{:04X}  {c}", self.value),
            (Some(_), None) => format!("U+{:04X}", self.value),
            _ => String::new(),
        }
    }

    /// Whether bit `index` (0 is the least significant) is set.
    pub fn bit(&self, index: u8) -> bool {
        index < 64 && (self.value >> index) & 1 == 1
    }

    /// Flips bit `index` of the value inside the word (the clickable binary row).
    pub fn toggle_bit(&mut self, index: u8) {
        if index >= self.bits || self.error {
            return;
        }
        self.value ^= 1u64 << index;
        self.fresh = true;
    }

    /// Sets the value to `value` (Paste, a recalled result); false when it is not a whole number that fits.
    pub fn set_value(&mut self, value: f64) -> bool {
        if !value.is_finite() || value.fract() != 0.0 || value.abs() >= 9.2e18 {
            return false;
        }
        self.error = false;
        self.value = (value as i64 as u64) & self.mask();
        self.fresh = true;
        true
    }

    /// The value as a plain number for Copy and the history: signed in decimal, unsigned otherwise.
    pub fn as_f64(&self) -> f64 {
        if self.base == Base::Dec {
            self.signed() as f64
        } else {
            self.value as f64
        }
    }

    fn apply(&self, op: BinOp, left: u64, right: u64) -> Option<u64> {
        let word = u64::from(self.bits);
        let v = match op {
            BinOp::Add => left.wrapping_add(right),
            BinOp::Sub => left.wrapping_sub(right),
            BinOp::Mul => left.wrapping_mul(right),
            BinOp::Div => left.checked_div(right)?,
            BinOp::And => left & right,
            BinOp::Or => left | right,
            BinOp::Xor => left ^ right,
            BinOp::Nor => !(left | right),
            BinOp::Shl => {
                if right >= word {
                    0
                } else {
                    left << right
                }
            }
            BinOp::Shr => {
                if right >= word {
                    0
                } else {
                    left >> right
                }
            }
        };
        Some(v & self.mask())
    }

    fn digit(&mut self, d: u8) {
        let radix = self.base.radix();
        if u32::from(d) >= radix {
            return;
        }
        if self.fresh {
            self.value = 0;
            self.fresh = false;
        }
        let next = self.value.checked_mul(u64::from(radix)).and_then(|v| v.checked_add(u64::from(d)));
        if let Some(v) = next.filter(|v| *v <= self.mask()) {
            self.value = v;
        }
    }

    fn fail(&mut self) {
        self.error = true;
        self.pending = None;
        self.fresh = true;
        self.value = 0;
        self.expression = "Cannot divide by zero".into();
    }

    /// Finishes the pending operation with the current value as the right operand; false on a division by zero.
    fn resolve(&mut self) -> bool {
        let Some((op, left)) = self.pending.take() else { return true };
        let right = self.value;
        match self.apply(op, left, right) {
            Some(v) => {
                self.expression = format!("{} {} {} =", self.format(left, self.base), op.symbol(), self.format(right, self.base));
                self.value = v;
                true
            }
            None => {
                self.fail();
                false
            }
        }
    }

    fn reset(&mut self) {
        *self = Self { base: self.base, bits: self.bits, ..Self::default() };
    }

    /// Applies `key`. A finished calculation is returned for the history.
    pub fn press(&mut self, key: PKey) -> Option<Finished> {
        if self.error {
            match key {
                PKey::Clear | PKey::Digit(_) | PKey::Pair(_) | PKey::Backspace => self.reset(),
                PKey::SetBase(_) | PKey::SetBits(_) => {}
                _ => return None,
            }
        }
        let mask = self.mask();
        let bits = u32::from(self.bits);
        match key {
            PKey::Digit(d) => self.digit(d),
            PKey::Pair(d) => {
                self.digit(d);
                self.digit(d);
            }
            PKey::Op(op) => {
                if self.pending.is_some() && !self.fresh && !self.resolve() {
                    return None;
                }
                self.pending = Some((op, self.value));
                self.fresh = true;
                self.expression = format!("{} {}", self.format(self.value, self.base), op.symbol());
            }
            PKey::Equals => {
                if self.pending.is_some() {
                    if !self.resolve() {
                        return None;
                    }
                    self.fresh = true;
                    return Some((self.expression.clone(), self.format(self.value, Base::Dec), self.signed() as f64));
                }
                self.fresh = true;
            }
            PKey::Not => {
                self.value = !self.value & mask;
                self.fresh = true;
            }
            PKey::Negate => {
                self.value = self.value.wrapping_neg() & mask;
                self.fresh = true;
            }
            PKey::Shl1 => {
                self.value = (self.value << 1) & mask;
                self.fresh = true;
            }
            PKey::Shr1 => {
                self.value >>= 1;
                self.fresh = true;
            }
            PKey::Rol => {
                self.value = ((self.value << 1) | (self.value >> (bits - 1))) & mask;
                self.fresh = true;
            }
            PKey::Ror => {
                self.value = ((self.value >> 1) | (self.value << (bits - 1))) & mask;
                self.fresh = true;
            }
            PKey::FlipBytes => {
                if bits >= 16 {
                    self.value = self.value.swap_bytes() >> (64 - bits);
                }
                self.fresh = true;
            }
            PKey::FlipWords => {
                if bits >= 32 {
                    let words = bits / 16;
                    let mut out = 0u64;
                    for k in 0..words {
                        out |= ((self.value >> (16 * k)) & 0xFFFF) << (16 * (words - 1 - k));
                    }
                    self.value = out;
                }
                self.fresh = true;
            }
            PKey::SetBase(base) => {
                self.base = base;
            }
            PKey::SetBits(size) => {
                if WORD_SIZES.contains(&size) {
                    self.bits = size;
                    let mask = self.mask();
                    self.value &= mask;
                    if let Some((op, left)) = self.pending {
                        self.pending = Some((op, left & mask));
                    }
                }
            }
            PKey::Clear => self.reset(),
            PKey::Backspace => {
                if !self.fresh {
                    self.value /= u64::from(self.base.radix());
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types `src`: hex digits, `+ - * /`, `&`, `|`, `^`, `<`, `>` and `=`.
    fn run(p: &mut Programmer, src: &str) {
        for c in src.chars() {
            let key = match c {
                '0'..='9' => PKey::Digit(c as u8 - b'0'),
                'A'..='F' => PKey::Digit(c as u8 - b'A' + 10),
                '+' => PKey::Op(BinOp::Add),
                '-' => PKey::Op(BinOp::Sub),
                '*' => PKey::Op(BinOp::Mul),
                '/' => PKey::Op(BinOp::Div),
                '&' => PKey::Op(BinOp::And),
                '|' => PKey::Op(BinOp::Or),
                '^' => PKey::Op(BinOp::Xor),
                '<' => PKey::Op(BinOp::Shl),
                '>' => PKey::Op(BinOp::Shr),
                '=' => PKey::Equals,
                _ => panic!("unknown key {c}"),
            };
            p.press(key);
        }
    }

    fn hex() -> Programmer {
        let mut p = Programmer::default();
        p.press(PKey::SetBase(Base::Hex));
        p
    }

    #[test]
    fn digits_follow_the_base_and_the_word_size() {
        let mut p = Programmer::default();
        run(&mut p, "255");
        assert_eq!(p.text(), "255");
        p.press(PKey::SetBase(Base::Hex));
        assert_eq!(p.text(), "FF");
        p.press(PKey::SetBase(Base::Bin));
        assert_eq!(p.text(), "11111111");
        p.press(PKey::Digit(2));
        assert_eq!(p.text(), "11111111", "2 is not a binary digit");
        p.press(PKey::SetBase(Base::Oct));
        assert_eq!(p.text(), "377");
        let mut p = Programmer::default();
        p.press(PKey::SetBits(8));
        run(&mut p, "300");
        assert_eq!(p.text(), "30", "a digit that would not fit the word is ignored");
        assert_eq!(p.bits(), 8);
    }

    #[test]
    fn hex_keys_and_pairs() {
        let mut p = hex();
        run(&mut p, "DEAD");
        assert_eq!(p.value(), 0xDEAD);
        p.press(PKey::Clear);
        p.press(PKey::Pair(15));
        assert_eq!(p.text(), "FF");
        p.press(PKey::Pair(0));
        assert_eq!(p.text(), "FF00");
        p.press(PKey::Backspace);
        assert_eq!(p.text(), "FF0");
    }

    #[test]
    fn arithmetic_is_integer_and_wraps_in_the_word() {
        let mut p = Programmer::default();
        run(&mut p, "7/2=");
        assert_eq!(p.value(), 3);
        p.press(PKey::Clear);
        run(&mut p, "6*7=");
        assert_eq!(p.text(), "42");
        p.press(PKey::Clear);
        p.press(PKey::SetBits(8));
        run(&mut p, "200+100=");
        assert_eq!(p.value(), 44, "300 wraps to 44 in a byte");
        p.press(PKey::Clear);
        run(&mut p, "5-6=");
        assert_eq!(p.text(), "-1");
        assert_eq!(p.value(), 0xFF);
        p.press(PKey::Clear);
        run(&mut p, "5/0=");
        assert!(p.is_error());
        assert_eq!(p.text(), "Error");
        p.press(PKey::Digit(1));
        assert_eq!(p.text(), "1");
    }

    #[test]
    fn chained_operators_resolve_left_to_right() {
        let mut p = Programmer::default();
        run(&mut p, "2+3*4=");
        assert_eq!(p.value(), 20);
        let mut q = Programmer::default();
        run(&mut q, "9-4");
        assert_eq!(q.press(PKey::Equals), Some(("9 - 4 =".to_string(), "5".to_string(), 5.0)));
    }

    #[test]
    fn bitwise_operators() {
        let mut p = hex();
        run(&mut p, "F0&3C=");
        assert_eq!(p.value(), 0x30);
        p.press(PKey::Clear);
        run(&mut p, "F0|0F=");
        assert_eq!(p.value(), 0xFF);
        p.press(PKey::Clear);
        run(&mut p, "FF^0F=");
        assert_eq!(p.value(), 0xF0);
        p.press(PKey::Clear);
        p.press(PKey::SetBits(8));
        run(&mut p, "F0");
        p.press(PKey::Op(BinOp::Nor));
        run(&mut p, "0F=");
        assert_eq!(p.value(), 0, "NOR of F0 and 0F in a byte");
        p.press(PKey::Clear);
        run(&mut p, "F0");
        p.press(PKey::Not);
        assert_eq!(p.value(), 0x0F);
    }

    #[test]
    fn shifts_and_rotates_stay_inside_the_word() {
        let mut p = hex();
        p.press(PKey::SetBits(8));
        run(&mut p, "81");
        p.press(PKey::Shl1);
        assert_eq!(p.value(), 0x02);
        p.press(PKey::Clear);
        run(&mut p, "81");
        p.press(PKey::Rol);
        assert_eq!(p.value(), 0x03);
        p.press(PKey::Ror);
        assert_eq!(p.value(), 0x81);
        p.press(PKey::Shr1);
        assert_eq!(p.value(), 0x40);
        p.press(PKey::Clear);
        run(&mut p, "1<3=");
        assert_eq!(p.value(), 8);
        p.press(PKey::Clear);
        run(&mut p, "80>7=");
        assert_eq!(p.value(), 1);
        p.press(PKey::Clear);
        run(&mut p, "1<8=");
        assert_eq!(p.value(), 0, "shifting by the word size or more empties the word");
    }

    #[test]
    fn flips_and_twos_complement() {
        let mut p = hex();
        run(&mut p, "1122334455667788");
        p.press(PKey::FlipBytes);
        assert_eq!(p.value(), 0x8877665544332211);
        p.press(PKey::FlipWords);
        assert_eq!(p.value(), 0x2211443366558877);
        let mut p = hex();
        p.press(PKey::SetBits(16));
        run(&mut p, "1234");
        p.press(PKey::FlipBytes);
        assert_eq!(p.value(), 0x3412);
        p.press(PKey::FlipWords);
        assert_eq!(p.value(), 0x3412, "a 16-bit word has no word order to flip");
        let mut p = Programmer::default();
        p.press(PKey::SetBits(8));
        run(&mut p, "1");
        p.press(PKey::Negate);
        assert_eq!(p.value(), 0xFF);
        assert_eq!(p.text(), "-1");
        p.press(PKey::Negate);
        assert_eq!(p.value(), 1);
    }

    #[test]
    fn changing_the_word_size_masks_the_value() {
        let mut p = hex();
        run(&mut p, "1FF");
        p.press(PKey::SetBits(8));
        assert_eq!(p.value(), 0xFF);
        p.press(PKey::SetBits(7));
        assert_eq!(p.bits(), 8, "7 is not a word size");
        assert_eq!(p.signed(), -1);
        p.press(PKey::SetBits(64));
        assert_eq!(p.bits(), 64);
    }

    #[test]
    fn the_binary_row_toggles_bits() {
        let mut p = Programmer::default();
        p.press(PKey::SetBits(16));
        p.toggle_bit(0);
        p.toggle_bit(3);
        assert_eq!(p.value(), 9);
        assert!(p.bit(3) && !p.bit(1));
        p.toggle_bit(16);
        assert_eq!(p.value(), 9, "bit 16 is outside the word");
        p.toggle_bit(0);
        assert_eq!(p.value(), 8);
        p.press(PKey::Digit(5));
        assert_eq!(p.value(), 5, "typing after a toggle starts a new number");
    }

    #[test]
    fn the_readout_names_the_character() {
        let mut p = Programmer::default();
        run(&mut p, "65");
        assert_eq!(p.character(), Some('A'));
        assert_eq!(p.readout(), "U+0041  A");
        p.press(PKey::Clear);
        run(&mut p, "7");
        assert_eq!(p.character(), None);
        assert_eq!(p.readout(), "U+0007");
        p.press(PKey::Clear);
        run(&mut p, "55296");
        assert_eq!(p.readout(), "", "a surrogate is not a character");
        let mut p = hex();
        run(&mut p, "20AC");
        assert_eq!(p.character(), Some('€'));
    }

    #[test]
    fn pasting_takes_whole_numbers_only() {
        let mut p = Programmer::default();
        assert!(p.set_value(255.0));
        assert_eq!(p.value(), 255);
        assert!(p.set_value(-1.0));
        assert_eq!(p.value(), u64::MAX);
        assert!(!p.set_value(1.5));
        assert!(!p.set_value(f64::NAN));
        assert!(!p.set_value(1e19));
        p.press(PKey::SetBits(8));
        assert!(p.set_value(300.0));
        assert_eq!(p.value(), 44);
        assert_eq!(p.as_f64(), 44.0);
    }
}
