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

/// A set of flags for one message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    bits: u8,
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
            }
        }
        flags
    }

    pub fn contains(self, flag: Flag) -> bool {
        self.bits & flag.bit() != 0
    }

    /// The set with `flag` switched on or off.
    pub fn with(self, flag: Flag, on: bool) -> Flags {
        if on {
            Flags { bits: self.bits | flag.bit() }
        } else {
            Flags { bits: self.bits & !flag.bit() }
        }
    }

    /// The set with `flag` toggled.
    pub fn toggled(self, flag: Flag) -> Flags {
        self.with(flag, !self.contains(flag))
    }

    /// The parenthesised list used by `STORE`, for example `(\Seen \Flagged)`.
    pub fn to_imap_list(self) -> String {
        let names: Vec<&str> = Flag::ALL.into_iter().filter(|f| self.contains(*f)).map(Flag::imap_name).collect();
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
    fn toggle_and_with() {
        let read = Flags::empty().toggled(Flag::Seen);
        assert!(read.contains(Flag::Seen));
        assert!(!read.toggled(Flag::Seen).contains(Flag::Seen));
        assert_eq!(read.with(Flag::Seen, true), read);
        assert_eq!(read.with(Flag::Seen, false), Flags::empty());
    }
}
