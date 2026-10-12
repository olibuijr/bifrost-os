//! MIDI over Bluetooth Low Energy: packet framing for the MIDI I/O characteristic.
//!
//! Pure: no radio is touched and no connection is opened. A BLE link carries MIDI in packets of at most one MTU.
//! The first byte of a packet is a header that holds the top six bits of a 13-bit millisecond timestamp. Each MIDI
//! message in the packet is preceded by a timestamp byte that holds the low seven bits. Running status may follow a
//! timestamp, and a SysEx message may continue into later packets, each of which starts with its own timestamp byte.
//! The framing follows the public MIDI-over-BLE specification of the MIDI Association.

/// One MIDI message with its timestamp in milliseconds (0 to 8191, then the clock wraps).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedMessage {
    pub timestamp_ms: u16,
    /// A complete message: a status byte and its data, or a whole SysEx message from F0 to F7.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BleMidiError {
    /// The packet is empty, or its first byte does not have the header bit set.
    BadHeader,
    /// The packet body does not start with a timestamp byte.
    MissingTimestamp,
    /// A message ends before all its data bytes arrived.
    Truncated,
    /// A status byte interrupts a SysEx message, which is not allowed.
    Malformed,
    /// A message to send is empty, has a bad status or data byte, or is a SysEx that does not run from F0 to F7.
    InvalidMessage,
    /// The timestamp does not fit in 13 bits.
    TimestampOutOfRange,
    /// The MTU is too small for a header, a timestamp and the message.
    MtuTooSmall,
}

/// The largest timestamp that BLE MIDI can carry: 13 bits of milliseconds.
pub const MAX_TIMESTAMP_MS: u16 = 0x1FFF;

/// Smallest usable MTU: a header, a timestamp, a status byte, one data byte and the timestamp before F7.
const MIN_MTU: usize = 5;

/// Number of data bytes that follow a status byte.
fn data_length(status: u8) -> usize {
    match status {
        0x80..=0xEF => {
            if matches!(status & 0xF0, 0xC0 | 0xD0) {
                1
            } else {
                2
            }
        }
        0xF1 | 0xF3 => 1,
        0xF2 => 2,
        _ => 0,
    }
}

/// Splits messages into BLE MIDI packets that carry at most `mtu` bytes each.
///
/// Messages that share a 128 ms block share a packet while they fit. A SysEx message is split over as many packets as
/// it needs, each continuation packet starting with the message's timestamp byte.
pub fn encode(messages: &[TimedMessage], mtu: usize) -> Result<Vec<Vec<u8>>, BleMidiError> {
    if mtu < MIN_MTU {
        return Err(BleMidiError::MtuTooSmall);
    }
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut high = 0u16;
    for m in messages {
        if m.timestamp_ms > MAX_TIMESTAMP_MS {
            return Err(BleMidiError::TimestampOutOfRange);
        }
        let first = *m.bytes.first().ok_or(BleMidiError::InvalidMessage)?;
        if first & 0x80 == 0 || first == 0xF7 {
            return Err(BleMidiError::InvalidMessage);
        }
        if first == 0xF0 {
            if !current.is_empty() {
                packets.push(std::mem::take(&mut current));
            }
            packets.extend(encode_sysex(m, mtu)?);
            continue;
        }
        if m.bytes[1..].iter().any(|&b| b & 0x80 != 0) {
            return Err(BleMidiError::InvalidMessage);
        }
        if 2 + m.bytes.len() > mtu {
            return Err(BleMidiError::MtuTooSmall);
        }
        let m_high = m.timestamp_ms >> 7;
        if current.is_empty() || m_high != high || current.len() + 1 + m.bytes.len() > mtu {
            if !current.is_empty() {
                packets.push(std::mem::take(&mut current));
            }
            high = m_high;
            current.push(0x80 | ((high & 0x3F) as u8));
        }
        current.push(0x80 | ((m.timestamp_ms & 0x7F) as u8));
        current.extend_from_slice(&m.bytes);
    }
    if !current.is_empty() {
        packets.push(current);
    }
    Ok(packets)
}

/// Frames one SysEx message (F0 to F7) into packets. Every packet begins with the header and a timestamp byte, and
/// the F7 is always preceded by a timestamp byte in the same packet.
fn encode_sysex(m: &TimedMessage, mtu: usize) -> Result<Vec<Vec<u8>>, BleMidiError> {
    let (last, payload) = m.bytes.split_last().ok_or(BleMidiError::InvalidMessage)?;
    let (first, data) = payload.split_first().ok_or(BleMidiError::InvalidMessage)?;
    if *first != 0xF0 || *last != 0xF7 || data.iter().any(|&b| b & 0x80 != 0) {
        return Err(BleMidiError::InvalidMessage);
    }
    let header = 0x80 | (((m.timestamp_ms >> 7) & 0x3F) as u8);
    let stamp = 0x80 | ((m.timestamp_ms & 0x7F) as u8);
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut packet = vec![header, stamp, 0xF0];
    let mut rest = data;
    loop {
        let space = mtu - packet.len();
        if rest.len() + 2 <= space {
            // A timestamp byte must come right before F7. Right after a timestamp byte there is no need for another.
            if packet.len() > 2 {
                packet.push(stamp);
            }
            packet.push(0xF7);
            packets.push(packet);
            return Ok(packets);
        }
        let take = space.min(rest.len());
        packet.extend_from_slice(&rest[..take]);
        rest = &rest[take..];
        packets.push(packet);
        packet = vec![header, stamp];
    }
}

/// Reads BLE MIDI packets in arrival order. It keeps the running status and an open SysEx message between packets.
/// After an error the decoder is reset, so the next packet starts from a clean state.
#[derive(Clone, Debug, Default)]
pub struct Decoder {
    running: Option<u8>,
    /// The timestamp of the F0 and the bytes of the SysEx message so far.
    sysex: Option<(u16, Vec<u8>)>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a SysEx message waits for its F7 in a later packet.
    pub fn in_sysex(&self) -> bool {
        self.sysex.is_some()
    }

    /// Decodes one packet into the complete messages it ends.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<TimedMessage>, BleMidiError> {
        let result = self.decode_packet(packet);
        if result.is_err() {
            *self = Decoder::new();
        }
        result
    }

    fn decode_packet(&mut self, packet: &[u8]) -> Result<Vec<TimedMessage>, BleMidiError> {
        let header = *packet.first().ok_or(BleMidiError::BadHeader)?;
        if header & 0x80 == 0 {
            return Err(BleMidiError::BadHeader);
        }
        let high = ((header & 0x3F) as u16) << 7;
        let mut out = Vec::new();
        let mut ts = high;
        let mut have_ts = false;
        let mut i = 1;
        while i < packet.len() {
            if packet[i] & 0x80 != 0 {
                ts = high | ((packet[i] & 0x7F) as u16);
                have_ts = true;
                i += 1;
                if i == packet.len() {
                    break;
                }
            } else if !have_ts {
                return Err(BleMidiError::MissingTimestamp);
            }
            i = self.item(packet, i, ts, &mut out)?;
        }
        Ok(out)
    }

    /// Reads one message, or the next part of a SysEx, that starts at `start`. Returns the index after it.
    fn item(&mut self, packet: &[u8], start: usize, ts: u16, out: &mut Vec<TimedMessage>) -> Result<usize, BleMidiError> {
        let mut i = start;
        if let Some((started, mut bytes)) = self.sysex.take() {
            if packet[i] == 0xF7 {
                bytes.push(0xF7);
                out.push(TimedMessage { timestamp_ms: started, bytes });
                return Ok(i + 1);
            }
            if packet[i] & 0x80 != 0 {
                return Err(BleMidiError::Malformed);
            }
            while i < packet.len() && packet[i] & 0x80 == 0 {
                bytes.push(packet[i]);
                i += 1;
            }
            self.sysex = Some((started, bytes));
            return Ok(i);
        }
        let byte = packet[i];
        if byte == 0xF0 {
            self.sysex = Some((ts, vec![0xF0]));
            return Ok(i + 1);
        }
        let status = if byte & 0x80 != 0 {
            i += 1;
            byte
        } else {
            self.running.ok_or(BleMidiError::Malformed)?
        };
        match status {
            0xF7 => return Ok(i),
            0xF8..=0xFF => {
                out.push(TimedMessage { timestamp_ms: ts, bytes: vec![status] });
                return Ok(i);
            }
            0xF1..=0xF6 => self.running = None,
            _ => self.running = Some(status),
        }
        let mut bytes = vec![status];
        for _ in 0..data_length(status) {
            match packet.get(i) {
                Some(&d) if d & 0x80 == 0 => {
                    bytes.push(d);
                    i += 1;
                }
                _ => return Err(BleMidiError::Truncated),
            }
        }
        out.push(TimedMessage { timestamp_ms: ts, bytes });
        Ok(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(ts: u16, bytes: &[u8]) -> TimedMessage {
        TimedMessage { timestamp_ms: ts, bytes: bytes.to_vec() }
    }

    #[test]
    fn a_message_at_time_zero_is_header_timestamp_and_bytes() {
        let packets = encode(&[msg(0, &[0x90, 0x3C, 0x64])], 20).unwrap();
        assert_eq!(packets, vec![vec![0x80, 0x80, 0x90, 0x3C, 0x64]]);
    }

    #[test]
    fn timestamps_split_into_six_high_bits_and_seven_low_bits() {
        // 200 = 1 * 128 + 72. The header carries 1, and the timestamp byte is 0x80 | 72 = 0xC8.
        let packets = encode(&[msg(200, &[0xB0, 0x07, 0x7F])], 20).unwrap();
        assert_eq!(packets, vec![vec![0x81, 0xC8, 0xB0, 0x07, 0x7F]]);
        // 8191 is the largest value: header bits 63 (0xBF) and timestamp bits 127 (0xFF).
        let packets = encode(&[msg(8191, &[0x90, 0x40, 0x7F])], 20).unwrap();
        assert_eq!(&packets[0][..2], &[0xBF, 0xFF][..]);
        let back = Decoder::new().decode(&packets[0]).unwrap();
        assert_eq!(back, vec![msg(8191, &[0x90, 0x40, 0x7F])]);
    }

    #[test]
    fn two_messages_in_one_block_share_the_header_and_decode_back() {
        let input = vec![msg(10, &[0x90, 0x3C, 0x64]), msg(20, &[0x80, 0x3C, 0x00])];
        let packets = encode(&input, 20).unwrap();
        // One header (0x80), then 0x8A for 10 ms and 0x94 for 20 ms.
        assert_eq!(packets, vec![vec![0x80, 0x8A, 0x90, 0x3C, 0x64, 0x94, 0x80, 0x3C, 0x00]]);
        assert_eq!(Decoder::new().decode(&packets[0]).unwrap(), input);
    }

    #[test]
    fn a_message_that_would_pass_the_mtu_starts_a_new_packet() {
        // Each packet holds the header, one timestamp byte and three message bytes: 5 bytes, the MTU.
        let input = vec![msg(0, &[0x90, 0x3C, 0x64]), msg(0, &[0x90, 0x3E, 0x64])];
        let packets = encode(&input, 5).unwrap();
        assert_eq!(
            packets,
            vec![vec![0x80, 0x80, 0x90, 0x3C, 0x64], vec![0x80, 0x80, 0x90, 0x3E, 0x64]]
        );
    }

    #[test]
    fn running_status_after_a_timestamp_repeats_the_last_status() {
        // The second timestamp (0x80) is followed by data only, so the status 0x90 carries over.
        let packet = [0x80, 0x80, 0x90, 0x3C, 0x64, 0x80, 0x3E, 0x40];
        let out = Decoder::new().decode(&packet).unwrap();
        assert_eq!(out, vec![msg(0, &[0x90, 0x3C, 0x64]), msg(0, &[0x90, 0x3E, 0x40])]);
    }

    #[test]
    fn a_sysex_message_spans_two_packets_and_decodes_whole() {
        let input = vec![msg(5, &[0xF0, 0x01, 0x02, 0xF7])];
        let packets = encode(&input, 5).unwrap();
        assert_eq!(packets, vec![vec![0x80, 0x85, 0xF0, 0x01, 0x02], vec![0x80, 0x85, 0xF7]]);
        let mut decoder = Decoder::new();
        assert!(decoder.decode(&packets[0]).unwrap().is_empty());
        assert!(decoder.in_sysex());
        assert_eq!(decoder.decode(&packets[1]).unwrap(), input);
        assert!(!decoder.in_sysex());
    }

    #[test]
    fn malformed_packets_are_refused_and_a_realtime_byte_is_one_message() {
        assert_eq!(Decoder::new().decode(&[]), Err(BleMidiError::BadHeader));
        assert_eq!(Decoder::new().decode(&[0x00u8]), Err(BleMidiError::BadHeader));
        assert_eq!(Decoder::new().decode(&[0x80u8, 0x01]), Err(BleMidiError::MissingTimestamp));
        // A note-on whose second data byte never arrives.
        assert_eq!(Decoder::new().decode(&[0x80u8, 0x80, 0x90, 0x3C]), Err(BleMidiError::Truncated));
        assert_eq!(Decoder::new().decode(&[0x80u8, 0x80, 0xF8]).unwrap(), vec![msg(0, &[0xF8])]);
    }

    #[test]
    fn encoder_refuses_bad_input() {
        let note = msg(0, &[0x90, 0x3C, 0x64]);
        assert_eq!(encode(&[note.clone()], 4), Err(BleMidiError::MtuTooSmall));
        assert_eq!(encode(&[msg(8192, &[0x90, 0x3C, 0x64])], 20), Err(BleMidiError::TimestampOutOfRange));
        assert_eq!(encode(&[msg(0, &[0x90, 0xC0])], 20), Err(BleMidiError::InvalidMessage));
        assert_eq!(encode(&[msg(0, &[])], 20), Err(BleMidiError::InvalidMessage));
    }
}
