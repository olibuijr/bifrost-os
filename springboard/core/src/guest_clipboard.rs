//! Clipboard sharing between the host and a guest: who owns the copy, what formats it offers, and how data moves.
//!
//! Pure model. Sharing works like a selection: one side grabs the clipboard and offers formats; the other side may
//! request one format; the owner delivers the bytes. A new grab from either side takes ownership and bumps the
//! generation, so a late delivery for an older grab is refused. The official Virtualization documentation lists
//! "Clipboard sharing" as a device topic and names the SPICE agent as the transport. This module models only the
//! exchange rules; the transport is outside it.

/// BifrOSt payload limit for one transfer. This is a project choice, not a documented framework limit.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Host,
    Guest,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Host => Side::Guest,
            Side::Guest => Side::Host,
        }
    }
}

/// Formats the clipboard can carry. Each has one MIME type on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Text,
    Html,
    Png,
    UriList,
}

impl Format {
    pub fn mime(self) -> &'static str {
        match self {
            Format::Text => "text/plain;charset=utf-8",
            Format::Html => "text/html",
            Format::Png => "image/png",
            Format::UriList => "text/uri-list",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The grab offered no formats.
    EmptyOffer,
    /// Nothing is on the shared clipboard.
    NoGrab,
    /// The side that owns the copy cannot request its own data.
    AlreadyOwner,
    /// The requested format is not in the owner's offer.
    FormatNotOffered,
    /// Only the owner delivers data.
    NotOwner,
    /// The delivery is for an older grab.
    StaleGeneration { current: u64 },
    /// No request is waiting for a delivery.
    NoRequest,
    /// The delivery is larger than [`MAX_PAYLOAD`]; the size is reported.
    TooLarge(usize),
}

/// What the other side must be told after a grab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub generation: u64,
    pub formats: Vec<Format>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clipboard {
    owner: Option<Side>,
    formats: Vec<Format>,
    generation: u64,
    pending: Option<(Side, Format)>,
}

impl Clipboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn owner(&self) -> Option<Side> {
        self.owner
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Takes ownership with `formats`. Duplicates are dropped and the first occurrence keeps its place.
    /// The previous offer and any pending request are discarded.
    pub fn grab(&mut self, side: Side, formats: &[Format]) -> Result<Notice, Error> {
        let mut unique: Vec<Format> = Vec::new();
        for f in formats {
            if !unique.contains(f) {
                unique.push(*f);
            }
        }
        if unique.is_empty() {
            return Err(Error::EmptyOffer);
        }
        self.generation += 1;
        self.owner = Some(side);
        self.formats = unique.clone();
        self.pending = None;
        Ok(Notice { generation: self.generation, formats: unique })
    }

    /// The non-owner asks for one format of the current grab.
    pub fn request(&mut self, side: Side, format: Format) -> Result<(), Error> {
        let owner = self.owner.ok_or(Error::NoGrab)?;
        if side == owner {
            return Err(Error::AlreadyOwner);
        }
        if !self.formats.contains(&format) {
            return Err(Error::FormatNotOffered);
        }
        self.pending = Some((side, format));
        Ok(())
    }

    /// The owner sends the bytes for the pending request. Returns the requester and the bytes.
    pub fn deliver(&mut self, side: Side, generation: u64, data: Vec<u8>) -> Result<(Side, Vec<u8>), Error> {
        if self.owner != Some(side) {
            return Err(Error::NotOwner);
        }
        if generation != self.generation {
            return Err(Error::StaleGeneration { current: self.generation });
        }
        if data.len() > MAX_PAYLOAD {
            return Err(Error::TooLarge(data.len()));
        }
        let (to, _format) = self.pending.take().ok_or(Error::NoRequest)?;
        Ok((to, data))
    }

    /// The owner gives the clipboard back. Other sides can no longer request data.
    pub fn release(&mut self, side: Side) -> Result<(), Error> {
        if self.owner != Some(side) {
            return Err(Error::NotOwner);
        }
        self.owner = None;
        self.formats.clear();
        self.pending = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_grab_then_host_request_is_delivered_to_host() {
        let mut c = Clipboard::new();
        let n = c.grab(Side::Guest, &[Format::Text, Format::Html]).unwrap();
        assert_eq!(n.generation, 1);
        assert_eq!(n.formats, vec![Format::Text, Format::Html]);
        c.request(Side::Host, Format::Text).unwrap();
        let (to, bytes) = c.deliver(Side::Guest, 1, b"hi".to_vec()).unwrap();
        assert_eq!(to, Side::Host);
        assert_eq!(bytes, b"hi".to_vec());
    }

    #[test]
    fn requesting_a_format_that_was_not_offered_is_refused() {
        let mut c = Clipboard::new();
        c.grab(Side::Host, &[Format::Text]).unwrap();
        assert_eq!(c.request(Side::Guest, Format::Png), Err(Error::FormatNotOffered));
        assert_eq!(c.request(Side::Host, Format::Text), Err(Error::AlreadyOwner));
    }

    #[test]
    fn new_grab_from_other_side_makes_old_delivery_refused() {
        let mut c = Clipboard::new();
        c.grab(Side::Guest, &[Format::Text]).unwrap();
        c.grab(Side::Host, &[Format::Text]).unwrap();
        assert_eq!(c.generation(), 2);
        assert_eq!(c.owner(), Some(Side::Host));
        assert_eq!(c.deliver(Side::Guest, 1, vec![1]), Err(Error::NotOwner));
    }

    #[test]
    fn stale_generation_and_missing_request_are_refused() {
        let mut c = Clipboard::new();
        c.grab(Side::Guest, &[Format::Png]).unwrap();
        assert_eq!(c.deliver(Side::Guest, 1, vec![1]), Err(Error::NoRequest));
        c.request(Side::Host, Format::Png).unwrap();
        assert_eq!(c.deliver(Side::Guest, 0, vec![1]), Err(Error::StaleGeneration { current: 1 }));
    }

    #[test]
    fn oversized_payload_is_refused_and_keeps_the_request() {
        let mut c = Clipboard::new();
        c.grab(Side::Host, &[Format::Text]).unwrap();
        c.request(Side::Guest, Format::Text).unwrap();
        let big = vec![0u8; MAX_PAYLOAD + 1];
        assert_eq!(c.deliver(Side::Host, 1, big), Err(Error::TooLarge(MAX_PAYLOAD + 1)));
        assert!(c.deliver(Side::Host, 1, vec![7]).is_ok());
    }

    #[test]
    fn grab_dedupes_and_empty_offer_is_refused() {
        let mut c = Clipboard::new();
        assert_eq!(c.grab(Side::Host, &[]), Err(Error::EmptyOffer));
        let n = c.grab(Side::Host, &[Format::Text, Format::Png, Format::Text]).unwrap();
        assert_eq!(n.formats, vec![Format::Text, Format::Png]);
    }

    #[test]
    fn release_by_owner_clears_the_offer() {
        let mut c = Clipboard::new();
        c.grab(Side::Guest, &[Format::UriList]).unwrap();
        assert_eq!(c.release(Side::Host), Err(Error::NotOwner));
        c.release(Side::Guest).unwrap();
        assert_eq!(c.request(Side::Host, Format::UriList), Err(Error::NoGrab));
        assert_eq!(Format::UriList.mime(), "text/uri-list");
        assert_eq!(Side::Host.other(), Side::Guest);
    }
}
