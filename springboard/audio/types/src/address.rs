//! Object ids, classes and property addresses: (selector, scope, element). Owner: task 01.

/// A four-character code, stored big-endian as the characters read (`FourCC(*b"nsrt")`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FourCC(pub [u8; 4]);

impl FourCC {
    pub const fn new(code: &[u8; 4]) -> FourCC {
        FourCC(*code)
    }

    pub const fn as_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub const fn from_u32(v: u32) -> FourCC {
        FourCC(v.to_be_bytes())
    }
}

/// An object in the audio object tree. 0 is never used; 1 is the system object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub u32);

/// The system object, the root of the tree.
pub const SYSTEM_OBJECT: ObjectId = ObjectId(1);

/// An object's class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    System,
    Plugin,
    Device,
    Stream,
    Volume,
    Mute,
    DataSource,
    Selector,
    Box,
    Clock,
    Tap,
    AggregateDevice,
    /// An application that uses audio: one per connected process, with its session (category, activity).
    Process,
}

/// Which side of a device a property addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Input,
    Output,
    PlayThrough,
}

/// The element (channel) a property addresses; 0 is the main element.
pub type Element = u32;

/// The main element.
pub const MAIN_ELEMENT: Element = 0;

/// A property's address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PropertyAddress {
    pub selector: FourCC,
    pub scope: Scope,
    pub element: Element,
}

impl PropertyAddress {
    /// The global, main-element address of `selector`.
    pub const fn global(selector: FourCC) -> PropertyAddress {
        PropertyAddress { selector, scope: Scope::Global, element: MAIN_ELEMENT }
    }

    pub const fn scoped(selector: FourCC, scope: Scope) -> PropertyAddress {
        PropertyAddress { selector, scope, element: MAIN_ELEMENT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_round_trip() {
        let c = FourCC::new(b"nsrt");
        assert_eq!(c.as_u32(), 0x6e737274);
        assert_eq!(FourCC::from_u32(0x6e737274), c);
    }

    #[test]
    fn address_constructors() {
        let a = PropertyAddress::global(FourCC::new(b"dev#"));
        assert_eq!((a.scope, a.element), (Scope::Global, MAIN_ELEMENT));
        let b = PropertyAddress::scoped(FourCC::new(b"stm#"), Scope::Output);
        assert_eq!(b.scope, Scope::Output);
        assert_eq!(SYSTEM_OBJECT, ObjectId(1));
    }
}
