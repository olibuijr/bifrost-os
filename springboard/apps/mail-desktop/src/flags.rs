//! Message flags as IMAP names them (`\Seen`, `\Flagged`, ...), kept as a small set.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Seen,
    Flagged,
    Answered,
    Draft,
    Deleted,
}

impl Flag {
    pub const ALL: [Flag; 5] = [Flag::Seen, Flag::Flagged, Flag::Answered, Flag::Draft, Flag::Deleted];

    /// The IMAP system flag name.
    pub fn imap_name(self) -> &'static str {
        match self {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
            Flag::Answered => "\\Answered",
            Flag::Draft => "\\Draft",
            Flag::Deleted => "\\Deleted",
        }
    }

    /// Reads an IMAP flag name, ignoring case. Names outside the set (`\Recent`, keywords) give `None`.
    pub fn from_imap(name: &str) -> Option<Flag> {
        Flag::ALL.into_iter().find(|f| f.imap_name().eq_ignore_ascii_case(name))
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// The colour of a flag (Message > Flag). The flag colour is kept in the keywords `$MailFlagBit0`, `$MailFlagBit1` and
/// `$MailFlagBit2` next to `\Flagged`: the three bits, least significant first, give the colour's position in
/// [`FlagColor::ALL`]. No bits is red, so a message flagged by another client reads as red.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

/// The keyword names of the colour bits, least significant first.
pub const COLOR_KEYWORDS: [&str; 3] = ["$MailFlagBit0", "$MailFlagBit1", "$MailFlagBit2"];

impl FlagColor {
    pub const ALL: [FlagColor; 7] =
        [FlagColor::Red, FlagColor::Orange, FlagColor::Yellow, FlagColor::Green, FlagColor::Blue, FlagColor::Purple, FlagColor::Gray];

    pub fn label(self) -> &'static str {
        match self {
            FlagColor::Red => "Red",
            FlagColor::Orange => "Orange",
            FlagColor::Yellow => "Yellow",
            FlagColor::Green => "Green",
            FlagColor::Blue => "Blue",
            FlagColor::Purple => "Purple",
            FlagColor::Gray => "Gray",
        }
    }

    /// The colour's three keyword bits as a number (0 to 6).
    pub fn bits(self) -> u8 {
        self as u8
    }

    fn from_bits(bits: u8) -> FlagColor {
        FlagColor::ALL.get(bits as usize).copied().unwrap_or(FlagColor::Red)
    }
}

/// A set of flags for one message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    bits: u8,
    /// The three colour keyword bits (see [`FlagColor`]).
    color_bits: u8,
}

impl Flags {
    pub fn empty() -> Flags {
        Flags::default()
    }

    /// Builds the set from IMAP flag names; unknown names are skipped.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Flags {
        let mut flags = Flags::empty();
        for name in names {
            if let Some(flag) = Flag::from_imap(name) {
                flags = flags.with(flag, true);
            } else if let Some(bit) = COLOR_KEYWORDS.iter().position(|k| k.eq_ignore_ascii_case(name)) {
                flags.color_bits |= 1 << bit;
            }
        }
        flags
    }

    /// The colour of a flagged message, or None when it is not flagged.
    pub fn color(self) -> Option<FlagColor> {
        self.contains(Flag::Flagged).then(|| FlagColor::from_bits(self.color_bits))
    }

    /// The set flagged in `color`.
    pub fn with_color(self, color: FlagColor) -> Flags {
        Flags { color_bits: color.bits(), ..self.with(Flag::Flagged, true) }
    }

    /// The set with the colour bits cleared (used when the flag is removed).
    pub fn without_color(self) -> Flags {
        Flags { color_bits: 0, ..self }
    }

    pub fn contains(self, flag: Flag) -> bool {
        self.bits & flag.bit() != 0
    }

    /// The set with `flag` switched on or off.
    pub fn with(self, flag: Flag, on: bool) -> Flags {
        if on {
            Flags { bits: self.bits | flag.bit(), ..self }
        } else {
            Flags { bits: self.bits & !flag.bit(), ..self }
        }
    }

    /// The set with `flag` toggled.
    pub fn toggled(self, flag: Flag) -> Flags {
        self.with(flag, !self.contains(flag))
    }

    /// The parenthesised list used by `STORE`, for example `(\Seen \Flagged)`.
    pub fn to_imap_list(self) -> String {
        let mut names: Vec<&str> = Flag::ALL.into_iter().filter(|f| self.contains(*f)).map(Flag::imap_name).collect();
        names.extend(COLOR_KEYWORDS.iter().enumerate().filter(|(i, _)| self.color_bits & (1 << *i) != 0).map(|(_, k)| *k));
        format!("({})", names.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_unknown_names_are_skipped() {
        let flags = Flags::from_names(["\\seen", "\\Flagged", "\\Recent", "$Label1"]);
        assert!(flags.contains(Flag::Seen));
        assert!(flags.contains(Flag::Flagged));
        assert!(!flags.contains(Flag::Deleted));
        assert_eq!(flags.to_imap_list(), "(\\Seen \\Flagged)");
        assert_eq!(Flags::empty().to_imap_list(), "()");
    }

    #[test]
    fn a_flag_colour_is_read_from_the_keyword_bits() {
        let green = Flags::from_names(["\\Flagged", "$MailFlagBit0", "$mailflagbit1"]);
        assert_eq!(green.color(), Some(FlagColor::Green));
        assert_eq!(Flags::from_names(["\\Flagged"]).color(), Some(FlagColor::Red));
        assert_eq!(Flags::from_names(["$MailFlagBit0"]).color(), None, "bits without \\Flagged are no flag");
    }

    #[test]
    fn every_colour_survives_a_round_trip_through_its_keywords() {
        for color in FlagColor::ALL {
            let list = Flags::empty().with_color(color).to_imap_list();
            let names: Vec<&str> = list.trim_matches(|c| c == '(' || c == ')').split(' ').collect();
            assert_eq!(Flags::from_names(names).color(), Some(color), "{list}");
        }
        assert_eq!(Flags::empty().with_color(FlagColor::Blue).to_imap_list(), "(\\Flagged $MailFlagBit2)");
        assert_eq!(Flags::empty().with_color(FlagColor::Orange).without_color().to_imap_list(), "(\\Flagged)");
    }

    #[test]
    fn toggle_and_with() {
        let read = Flags::empty().toggled(Flag::Seen);
        assert!(read.contains(Flag::Seen));
        assert!(!read.toggled(Flag::Seen).contains(Flag::Seen));
        assert_eq!(read.with(Flag::Seen, true), read);
        assert_eq!(read.with(Flag::Seen, false), Flags::empty());
    }
}
