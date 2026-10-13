//! Wire format of touch events: `ToApp::Touches`, tag `0x1f0`. One packet carries every touch of one frame
//! ([`TouchEvent`]), so an app never sees half of a two-finger gesture.
//!
//! Layout: `u32 timestamp_ms`, `u32 count`, then per touch `u32 id`, `u8 phase`, `u8 kind`, `u8 flags`, `u32 tap_count`,
//! four `f32` (x, y, previous x, previous y), `u32 timestamp_ms`, then `f32 force` and `f32 max_force` when
//! `flags & HAS_FORCE`, and `f32 major_radius` when `flags & HAS_RADIUS`. Times are the low 32 bits of the shell's
//! monotonic millisecond clock, like `ToApp::Touch::t_ms`. Locations are surface pixels, like `ToApp::Touch`.
//!
//! This message rides next to `ToApp::Touch`, which stays the primary-touch stream for apps that do not use touch
//! events. The shell sends a frame before the matching `ToApp::Touch`, so an SDK that sees frames can drop the plain
//! stream for apps that take touch events (see the `bifrost_app` runtime).

use super::{f32_, u32_, DecodeError, Reader, ToApp};
use bifrost_ui::touch::{Touch, TouchEvent, TouchPhase, TouchType, MAX_TOUCHES};

pub const TAG_TOUCHES: u32 = 0x1f0;

const HAS_FORCE: u8 = 1;
const HAS_RADIUS: u8 = 2;

/// The wire carries milliseconds as the low 32 bits of the shell clock.
fn wire_ms(t: f64) -> u32 {
    if t.is_finite() && t >= 0.0 {
        t as u64 as u32
    } else {
        0
    }
}

pub(crate) fn encode_app(m: &ToApp, b: &mut Vec<u8>) {
    let ToApp::Touches(ev) = m else { return };
    u32_(b, TAG_TOUCHES);
    u32_(b, wire_ms(ev.timestamp));
    let n = ev.touches.len().min(MAX_TOUCHES);
    u32_(b, n as u32);
    for t in &ev.touches[..n] {
        u32_(b, t.id);
        b.push(t.phase as u8);
        b.push(t.kind as u8);
        let force = match (t.force, t.max_force) {
            (Some(f), Some(mf)) => Some((f, mf)),
            _ => None,
        };
        let mut flags = 0;
        if force.is_some() {
            flags |= HAS_FORCE;
        }
        if t.major_radius.is_some() {
            flags |= HAS_RADIUS;
        }
        b.push(flags);
        u32_(b, t.tap_count);
        f32_(b, t.x);
        f32_(b, t.y);
        f32_(b, t.prev_x);
        f32_(b, t.prev_y);
        u32_(b, wire_ms(t.timestamp));
        if let Some((f, mf)) = force {
            f32_(b, f);
            f32_(b, mf);
        }
        if let Some(r) = t.major_radius {
            f32_(b, r);
        }
    }
}

pub(crate) fn decode_app(t: u32, r: &mut Reader) -> Result<ToApp, DecodeError> {
    match t {
        TAG_TOUCHES => {
            let timestamp = r.u32()? as f64;
            let n = r.u32()? as usize;
            if n > MAX_TOUCHES {
                return Err(DecodeError::BadValue);
            }
            let mut touches = Vec::with_capacity(n);
            for _ in 0..n {
                let id = r.u32()?;
                let phase = TouchPhase::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?;
                let kind = TouchType::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?;
                let flags = r.u8()?;
                if flags & !(HAS_FORCE | HAS_RADIUS) != 0 {
                    return Err(DecodeError::BadValue);
                }
                let tap_count = r.u32()?;
                let (x, y, prev_x, prev_y) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
                let ts = r.u32()? as f64;
                let (force, max_force) = if flags & HAS_FORCE != 0 { (Some(r.f32()?), Some(r.f32()?)) } else { (None, None) };
                let major_radius = if flags & HAS_RADIUS != 0 { Some(r.f32()?) } else { None };
                touches.push(Touch { id, phase, kind, x, y, prev_x, prev_y, timestamp: ts, tap_count, force, max_force, major_radius });
            }
            Ok(ToApp::Touches(TouchEvent { timestamp, touches }))
        }
        t => Err(DecodeError::UnknownTag(t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_PACKET;
    use bifrost_ui::touch::{Sample, TouchTracker};

    fn touch(id: u32, phase: TouchPhase) -> Touch {
        Touch {
            id,
            phase,
            kind: TouchType::Direct,
            x: 10.5,
            y: -2.0,
            prev_x: 9.0,
            prev_y: -1.5,
            timestamp: 1234.0,
            tap_count: 2,
            force: None,
            max_force: None,
            major_radius: None,
        }
    }

    #[test]
    fn a_frame_round_trips() {
        let full = Touch { force: Some(0.5), max_force: Some(4.0), major_radius: Some(7.25), kind: TouchType::Pencil, ..touch(7, TouchPhase::Moved) };
        let ev = TouchEvent { timestamp: 99000.0, touches: vec![touch(1, TouchPhase::Began), full, touch(3, TouchPhase::Cancelled)] };
        let m = ToApp::Touches(ev);
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }

    #[test]
    fn an_empty_frame_round_trips() {
        let m = ToApp::Touches(TouchEvent { timestamp: 0.0, touches: vec![] });
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }

    #[test]
    fn force_without_its_maximum_is_not_sent() {
        let half = Touch { force: Some(1.0), max_force: None, ..touch(1, TouchPhase::Began) };
        let m = ToApp::Touches(TouchEvent { timestamp: 5.0, touches: vec![half] });
        let Ok(ToApp::Touches(back)) = ToApp::decode(&m.encode()) else { panic!("decode") };
        assert_eq!((back.touches[0].force, back.touches[0].max_force), (None, None));
    }

    #[test]
    fn a_tracked_gesture_survives_the_wire() {
        let mut tr = TouchTracker::new();
        tr.down(Sample::at(1, 10.0, 10.0), 100.0);
        let ev = tr.down(Sample::at(2, 200.0, 40.0).with_radius(6.0), 110.0).unwrap();
        let m = ToApp::Touches(ev);
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }

    #[test]
    fn a_full_frame_fits_a_packet() {
        let touches: Vec<Touch> = (0..MAX_TOUCHES as u32).map(|i| Touch { force: Some(1.0), max_force: Some(2.0), major_radius: Some(3.0), ..touch(i, TouchPhase::Moved) }).collect();
        let m = ToApp::Touches(TouchEvent { timestamp: 1.0, touches });
        assert!(m.encode().len() < MAX_PACKET / 8);
    }

    #[test]
    fn extra_touches_are_cut_on_encode_and_refused_on_decode() {
        let touches: Vec<Touch> = (0..MAX_TOUCHES as u32 + 3).map(|i| touch(i, TouchPhase::Stationary)).collect();
        let Ok(ToApp::Touches(back)) = ToApp::decode(&ToApp::Touches(TouchEvent { timestamp: 1.0, touches }).encode()) else { panic!("decode") };
        assert_eq!(back.touches.len(), MAX_TOUCHES);
        let mut raw = Vec::new();
        u32_(&mut raw, TAG_TOUCHES);
        u32_(&mut raw, 0);
        u32_(&mut raw, MAX_TOUCHES as u32 + 1);
        assert_eq!(ToApp::decode(&raw), Err(DecodeError::BadValue));
    }

    #[test]
    fn bad_values_and_truncation_are_errors() {
        let m = ToApp::Touches(TouchEvent { timestamp: 1.0, touches: vec![touch(1, TouchPhase::Began)] });
        let good = m.encode();
        // offsets: tag 0..4, timestamp 4..8, count 8..12, id 12..16, phase 16, kind 17, flags 18
        let mut bad = good.clone();
        bad[16] = 9;
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue), "phase");
        let mut bad = good.clone();
        bad[17] = 9;
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue), "kind");
        let mut bad = good.clone();
        bad[18] = 0x80;
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue), "flags");
        assert_eq!(ToApp::decode(&good[..good.len() - 1]), Err(DecodeError::Truncated));
    }

    #[test]
    fn the_time_is_the_low_32_bits() {
        let m = ToApp::Touches(TouchEvent { timestamp: (u32::MAX as f64) + 11.0, touches: vec![] });
        assert_eq!(ToApp::decode(&m.encode()), Ok(ToApp::Touches(TouchEvent { timestamp: 10.0, touches: vec![] })));
        assert_eq!(wire_ms(-5.0), 0);
        assert_eq!(wire_ms(f64::NAN), 0);
    }
}
