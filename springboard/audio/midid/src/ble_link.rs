#![allow(dead_code)]
//! BLE MIDI ingress: turns the packets a GATT transport receives into timestamped UMP on the host clock.
//!
//! BLE MIDI timestamps are 13-bit milliseconds that wrap. The first packet anchors the device clock to the host
//! clock; later timestamps are unwrapped against the previous one and mapped relative to the anchor. An event is
//! never stamped later than the moment its packet arrived. The GATT link itself (scanning, connecting, the MIDI I/O
//! characteristic) belongs to the Bluetooth service and calls `ingest` with each notification.

use bifrost_midi::ble::{self, BleMidiError, TIMESTAMP_MODULUS};
use bifrost_midi::ump::{Midi1ToUmp, Ump};

pub struct BleIngress {
    conv: Midi1ToUmp,
    /// Unwrapped device milliseconds of the last message, and the anchor pair.
    last_dev: Option<u16>,
    unwrapped_ms: u64,
    anchor: Option<(u64, u64)>, // (unwrapped device ms, host ns)
}

impl BleIngress {
    pub fn new(group: u8) -> BleIngress {
        BleIngress { conv: Midi1ToUmp::new(group), last_dev: None, unwrapped_ms: 0, anchor: None }
    }

    /// Decodes one notification received at `now_ns` and returns its packets with host timestamps, in order.
    pub fn ingest(&mut self, bytes: &[u8], now_ns: u64) -> Result<Vec<(u64, Ump)>, BleMidiError> {
        let messages = ble::decode(bytes)?;
        let mut out = Vec::new();
        for m in messages {
            let t = m.timestamp % TIMESTAMP_MODULUS;
            match self.last_dev {
                None => self.unwrapped_ms = t as u64,
                Some(prev) => {
                    let delta = (t + TIMESTAMP_MODULUS - prev) % TIMESTAMP_MODULUS;
                    self.unwrapped_ms += delta as u64;
                }
            }
            self.last_dev = Some(t);
            let (a_dev, a_host) = *self.anchor.get_or_insert((self.unwrapped_ms, now_ns));
            let host = (a_host + (self.unwrapped_ms - a_dev) * 1_000_000).min(now_ns);
            let mut pk = Vec::new();
            for &b in &m.bytes {
                self.conv.push(b, &mut pk);
            }
            out.extend(pk.into_iter().map(|p| (host, p)));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Header 0x80 (timestamp high bits 0), timestamp byte 0x80|low, then the message.
    fn packet(ts: u16, msg: &[u8]) -> Vec<u8> {
        ble::encode(ts, &[msg], 64).unwrap()
    }

    #[test]
    fn first_packet_anchors_and_timestamps_follow() {
        let mut b = BleIngress::new(0);
        let a = b.ingest(&packet(100, &[0x90, 60, 100]), 5_000_000_000).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].0, 5_000_000_000);
        assert_eq!(a[0].1.words[0], 0x2090_3C64);
        // 10 ms later on the device clock; the packet arrives 12 ms after the first (link jitter).
        let c = b.ingest(&packet(110, &[0x80, 60, 0]), 5_012_000_000).unwrap();
        assert_eq!(c[0].0, 5_010_000_000);
    }

    #[test]
    fn the_13_bit_timestamp_wraps() {
        let mut b = BleIngress::new(0);
        b.ingest(&packet(8190, &[0x90, 60, 100]), 1_000_000_000).unwrap();
        // 8190 -> 3 is 5 ms across the wrap.
        let c = b.ingest(&packet(3, &[0x80, 60, 0]), 1_050_000_000).unwrap();
        assert_eq!(c[0].0, 1_005_000_000);
    }

    #[test]
    fn an_event_is_never_stamped_in_the_future() {
        let mut b = BleIngress::new(0);
        b.ingest(&packet(0, &[0x90, 60, 100]), 1_000_000_000).unwrap();
        // The device says 500 ms later but the packet arrived only 100 ms later: clamp to arrival.
        let c = b.ingest(&packet(500, &[0x80, 60, 0]), 1_100_000_000).unwrap();
        assert_eq!(c[0].0, 1_100_000_000);
    }

    #[test]
    fn garbage_is_an_error() {
        let mut b = BleIngress::new(0);
        assert!(b.ingest(&[0x00, 0x01], 0).is_err());
    }
}
