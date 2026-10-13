//! Per-argument log privacy and formatting (`OSLogPrivacy`, `OSLogIntegerFormatting`, `OSLogFloatFormatting`).
//! Render with [`LogArg::render`] and format the result into a message passed to `log::Logger`;
//! private values use the `log::private` markers so the subsystem's `private` mode still applies.

use crate::log::private;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mask {
    None,
    Hash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privacy {
    Auto,
    Public,
    Private(Mask),
    Sensitive(Mask),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntFormat {
    Decimal,
    Hex { include_prefix: bool, uppercase: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left(usize),
    Right(usize),
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Str(String),
    Int(i64),
    Float(f64, Option<usize>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogArg {
    value: Value,
    privacy: Privacy,
    int_format: IntFormat,
    align: Align,
}

impl LogArg {
    fn new(value: Value) -> Self {
        LogArg { value, privacy: Privacy::Auto, int_format: IntFormat::Decimal, align: Align::None }
    }
    pub fn string(v: impl Into<String>) -> Self {
        Self::new(Value::Str(v.into()))
    }
    pub fn int(v: i64) -> Self {
        Self::new(Value::Int(v))
    }
    /// `precision` is the number of fraction digits (`OSLogFloatFormatting.fixed(precision:)`); `None` is `.fixed`.
    pub fn float(v: f64, precision: Option<usize>) -> Self {
        Self::new(Value::Float(v, precision))
    }
    pub fn privacy(mut self, p: Privacy) -> Self {
        self.privacy = p;
        self
    }
    pub fn format(mut self, f: IntFormat) -> Self {
        self.int_format = f;
        self
    }
    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }

    fn text(&self) -> String {
        let s = match &self.value {
            Value::Str(s) => s.clone(),
            Value::Int(i) => match self.int_format {
                IntFormat::Decimal => i.to_string(),
                IntFormat::Hex { include_prefix, uppercase } => {
                    let h = if uppercase { format!("{:X}", *i as u64) } else { format!("{:x}", *i as u64) };
                    if include_prefix { format!("0x{h}") } else { h }
                }
            },
            Value::Float(f, Some(p)) => format!("{f:.p$}"),
            Value::Float(f, None) => format!("{f:.6}"),
        };
        match self.align {
            Align::None => s,
            Align::Left(c) => format!("{s:<c$}"),
            Align::Right(c) => format!("{s:>c$}"),
        }
    }

    fn effective(&self) -> Privacy {
        match (self.privacy, &self.value) {
            (Privacy::Auto, Value::Str(_)) => Privacy::Private(Mask::None),
            (Privacy::Auto, _) => Privacy::Public,
            (p, _) => p,
        }
    }

    pub fn render(&self) -> String {
        let t = self.text();
        match self.effective() {
            Privacy::Public | Privacy::Auto => t,
            Privacy::Private(Mask::None) | Privacy::Sensitive(Mask::None) => private(t),
            Privacy::Private(Mask::Hash) | Privacy::Sensitive(Mask::Hash) => hash_mask(&t),
        }
    }
}

/// `<mask.hash: 'base64'>`: stable per value, so equal values correlate without revealing them.
pub fn hash_mask(v: &str) -> String {
    let d = crate::md5::digest(v.as_bytes());
    format!("<mask.hash: '{}'>", crate::plist::base64_encode(&d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::redact;

    #[test]
    fn private_arg_is_redacted_unless_debug() {
        let m = format!("opened {}", LogArg::string("/home/a").privacy(Privacy::Private(Mask::None)).render());
        assert_eq!(redact(&m, false), "opened <private>");
        assert_eq!(redact(&m, true), "opened /home/a");
    }

    #[test]
    fn hash_mask_is_stable_and_hides_value() {
        let a = LogArg::string("bob").privacy(Privacy::Private(Mask::Hash)).render();
        let b = LogArg::string("bob").privacy(Privacy::Private(Mask::Hash)).render();
        let c = LogArg::string("eve").privacy(Privacy::Private(Mask::Hash)).render();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("<mask.hash: '") && a.ends_with("'>") && !a.contains("bob"));
        assert_eq!(redact(&a, false), a);
    }

    #[test]
    fn public_passes() {
        let r = LogArg::string("x").privacy(Privacy::Public).render();
        assert_eq!(redact(&r, false), "x");
    }

    #[test]
    fn auto_strings_private_integers_public() {
        assert_eq!(redact(&LogArg::string("s").render(), false), "<private>");
        assert_eq!(redact(&LogArg::int(7).render(), false), "7");
        assert_eq!(redact(&LogArg::float(1.5, Some(2)).render(), false), "1.50");
    }

    #[test]
    fn sensitive_is_redacted() {
        assert_eq!(redact(&LogArg::int(7).privacy(Privacy::Sensitive(Mask::None)).render(), false), "<private>");
    }

    #[test]
    fn hex_and_decimal() {
        let hex = |p, u| LogArg::int(255).format(IntFormat::Hex { include_prefix: p, uppercase: u }).render();
        assert_eq!(hex(true, false), "0xff");
        assert_eq!(hex(false, true), "FF");
        assert_eq!(LogArg::int(-3).format(IntFormat::Decimal).render(), "-3");
    }

    #[test]
    fn alignment_columns() {
        assert_eq!(LogArg::int(42).align(Align::Right(5)).render(), "   42");
        assert_eq!(LogArg::int(42).align(Align::Left(5)).render(), "42   ");
    }
}
