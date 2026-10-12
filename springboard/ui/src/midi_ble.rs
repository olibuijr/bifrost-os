//! BLE MIDI packets: the wire format that carries MIDI messages over the MIDI I/O characteristic of a Bluetooth Low
//! Energy MIDI peripheral.
//!
//! A packet is one header byte (bit 7 set, then the high six bits of a 13-bit millisecond timestamp), followed by
//! messages. Each message is preceded by a timestamp byte (bit 7 set, then the low seven bits). A data byte with no
//! status byte after a timestamp repeats the last channel status (running status). A system exclusive message may
//! carry timestamp bytes between its data bytes, and it ends at 0xF7.
//!
//! Pure model: no radio and no GATT write happen here. `encode` returns the bytes a transport writes to the
//! characteristic, and `decode` reads the bytes a transport received. A system exclusive message that does not
//! finish inside one packet is reported as `Truncated`; reassembling it across packets is left to the caller.

/// The MIDI service and MIDI I/O characteristic UUID of a BLE MIDI peripheral.
pub const MIDI_SERVICE: &str = "03B80E5A-EDE8-4B33-A751-6CE34EC4C700";
/// Timestamps are 13 bits, in milliseconds, and wrap at this value.
pub const TIMESTAMP_MODULUS: u16 = 0x2000;
/// The ATT write header takes three bytes of every MTU, which leaves the rest for the packet.
const ATT_HEADER: usize = 3;

/// The MIDI service UUID as a `Uuid`.
pub fn midi_service() -> crate::gatt::Uuid {
    crate::gatt::Uuid::parse(MIDI_SERVICE).expect("the MIDI service UUID constant is valid")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BleMidiError {
    /// The timestamp does not fit in 13 bits.
    TimestampTooLarge,
    /// The packet would be longer than the MTU allows. `max` is the largest packet for that MTU.
    PacketTooLong { max: usize },
    /// A message is empty, is not a complete message, or carries a data byte with the high bit set.
    InvalidMessage,
    /// The first byte of the packet has no header bit.
    NotAPacket,
    /// The packet ends in the middle of a message.
    Truncated,
    /// The packet breaks the framing: a message has no timestamp, or data arrives with no running status.
    Malformed,
}

/// One MIDI message received in a packet, with the timestamp it carried (0 to 8191 ms).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BleMessage {
    pub timestamp: u16,
    pub bytes: Vec<u8>,
}

/// The number of bytes in a complete channel, system common or real-time message, status byte included.
/// Returns `None` for system exclusive (which has no fixed length) and for undefined status bytes.
pub fn message_len(status: u8) -> Option<usize> {
    match status {
        0x80..=0xBF | 0xE0..=0xEF | 0xF2 => Some(3),
        0xC0..=0xDF | 0xF1 | 0xF3 => Some(2),
        0xF6 | 0xF8..=0xFF => Some(1),
        _ => None,
    }
}

fn check_message(m: &[u8]) -> Result<(), BleMidiError> {
    let status = *m.first().ok_or(BleMidiError::InvalidMessage)?;
    if status & 0x80 == 0 {
        return Err(BleMidiError::InvalidMessage);
    }
    if status == 0xF0 {
        if m.len() < 2 || m.last() != Some(&0xF7) {
            return Err(BleMidiError::InvalidMessage);
        }
        if m[1..m.len() - 1].iter().any(|b| b & 0x80 != 0) {
            return Err(BleMidiError::InvalidMessage);
        }
        return Ok(());
    }
    if message_len(status) != Some(m.len()) {
        return Err(BleMidiError::InvalidMessage);
    }
    if m[1..].iter().any(|b| b & 0x80 != 0) {
        return Err(BleMidiError::InvalidMessage);
    }
    Ok(())
}

/// Builds one packet at `timestamp` (milliseconds, 0 to 8191) holding `messages`, each sent with a timestamp byte.
/// The packet must fit `mtu` less the three ATT header bytes.
pub fn encode(timestamp: u16, messages: &[&[u8]], mtu: u16) -> Result<Vec<u8>, BleMidiError> {
    if timestamp >= TIMESTAMP_MODULUS {
        return Err(BleMidiError::TimestampTooLarge);
    }
    if messages.is_empty() {
        return Err(BleMidiError::InvalidMessage);
    }
    let mut out = vec![0x80 | ((timestamp >> 7) & 0x3F) as u8];
    let low = 0x80 | (timestamp & 0x7F) as u8;
    for m in messages {
        check_message(m)?;
        out.push(low);
        out.extend_from_slice(m);
    }
    let max = (mtu as usize).saturating_sub(ATT_HEADER);
    if out.len() > max {
        return Err(BleMidiError::PacketTooLong { max });
    }
    Ok(out)
}

/// Reads one packet into its messages, in order. Running status is applied, and a real-time byte inside a packet is
/// its own one-byte message that leaves the running status in place.
pub fn decode(bytes: &[u8]) -> Result<Vec<BleMessage>, BleMidiError> {
    let header = *bytes.first().ok_or(BleMidiError::Truncated)?;
    if header & 0x80 == 0 {
        return Err(BleMidiError::NotAPacket);
    }
    let high = (header & 0x3F) as u16;
    let mut out = Vec::new();
    let mut running: Option<u8> = None;
    let mut i = 1;
    while i < bytes.len() {
        let t = bytes[i];
        if t & 0x80 == 0 {
            return Err(BleMidiError::Malformed);
        }
        let timestamp = (high << 7) | (t & 0x7F) as u16;
        i += 1;
        if i >= bytes.len() {
            return Err(BleMidiError::Truncated);
        }
        let first = bytes[i];
        let status = if first & 0x80 != 0 {
            i += 1;
            first
        } else {
            running.ok_or(BleMidiError::Malformed)?
        };
        let msg: Vec<u8> = match status {
            0xF0 => {
                running = None;
                let mut m = vec![0xF0];
                loop {
                    let b = *bytes.get(i).ok_or(BleMidiError::Truncated)?;
                    i += 1;
                    if b == 0xF7 {
                        m.push(b);
                        break;
                    }
                    // Inside system exclusive a high-bit byte is a timestamp, not data.
                    if b & 0x80 == 0 {
                        m.push(b);
                    }
                }
                m
            }
            0xF8..=0xFF => vec![status],
            _ => {
                let len = message_len(status).ok_or(BleMidiError::Malformed)?;
                running = if status < 0xF0 { Some(status) } else { None };
                let mut m = vec![status];
                while m.len() < len {
                    let b = *bytes.get(i).ok_or(BleMidiError::Truncated)?;
                    if b & 0x80 != 0 {
                        return Err(BleMidiError::Malformed);
                    }
                    m.push(b);
                    i += 1;
                }
                m
            }
        };
        out.push(BleMessage { timestamp, bytes: msg });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_note_on_with_a_timestamp_byte() {
        let packet = encode(0, &[&[0x90, 60, 100]], 23).unwrap();
        assert_eq!(packet, vec![0x80, 0x80, 0x90, 60, 100]);
    }

    #[test]
    fn splits_the_thirteen_bit_timestamp_across_header_and_timestamp_byte() {
        // 8191 = 0b1_1111_1111_1111: high six bits 0x3F, low seven bits 0x7F.
        let packet = encode(8191, &[&[0x90, 60, 100]], 23).unwrap();
        assert_eq!(packet, vec![0xBF, 0xFF, 0x90, 60, 100]);
        assert_eq!(
            encode(8192, &[&[0x90, 60, 100]], 23),
            Err(BleMidiError::TimestampTooLarge)
        );
    }

    #[test]
    fn packet_longer_than_the_mtu_is_refused_and_a_larger_mtu_accepts_it() {
        // A 30-byte system exclusive message plus a 2-byte header and timestamp is 32 bytes.
        let mut sysex = vec![0xF0];
        sysex.extend([0x01u8; 28]);
        sysex.push(0xF7);
        assert_eq!(encode(0, &[&sysex], 23), Err(BleMidiError::PacketTooLong { max: 20 }));
        assert!(encode(0, &[&sysex], 185).is_ok());
    }

    #[test]
    fn decodes_two_messages_that_share_a_packet() {
        let bytes = [0x81, 0x80, 0xB0, 7, 100, 0x80, 0x90, 60, 0];
        let msgs = decode(&bytes).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], BleMessage { timestamp: 128, bytes: vec![0xB0, 7, 100] });
        assert_eq!(msgs[1], BleMessage { timestamp: 128, bytes: vec![0x90, 60, 0] });
        // Two messages encode back to the same bytes.
        let again = encode(128, &[&[0xB0, 7, 100], &[0x90, 60, 0]], 23).unwrap();
        assert_eq!(again, bytes.to_vec());
    }

    #[test]
    fn running_status_repeats_the_last_channel_status() {
        // The second message has no status byte, so it is note on (0x90) again.
        let bytes = [0x81, 0x80, 0x90, 60, 100, 0x80, 61, 90];
        let msgs = decode(&bytes).unwrap();
        assert_eq!(msgs[0].bytes, vec![0x90, 60, 100]);
        assert_eq!(msgs[1].bytes, vec![0x90, 61, 90]);
        // Without a prior status there is nothing to repeat.
        assert_eq!(decode(&[0x80, 0x80, 61, 90]), Err(BleMidiError::Malformed));
    }

    #[test]
    fn sysex_skips_timestamp_bytes_inside_it() {
        // Timestamp byte 0x81 sits between data bytes 2 and 3 and is not part of the message.
        let bytes = [0x80, 0x80, 0xF0, 1, 2, 0x81, 3, 0xF7];
        let msgs = decode(&bytes).unwrap();
        assert_eq!(msgs, vec![BleMessage { timestamp: 0, bytes: vec![0xF0, 1, 2, 3, 0xF7] }]);
        assert_eq!(encode(0, &[&[0xF0, 1, 2, 0xF7]], 23).unwrap(), vec![0x80, 0x80, 0xF0, 1, 2, 0xF7]);
        assert_eq!(encode(0, &[&[0xF0, 0x90, 0xF7]], 23), Err(BleMidiError::InvalidMessage));
    }

    #[test]
    fn rejects_a_packet_without_a_header_or_cut_short() {
        assert_eq!(decode(&[0x01, 0x80, 0x90, 60, 100]), Err(BleMidiError::NotAPacket));
        assert_eq!(decode(&[0x80, 0x80, 0x90, 60]), Err(BleMidiError::Truncated));
    }
}
