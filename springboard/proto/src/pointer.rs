//! Wire format of pointer events: `ToApp::Pointer` (tag `0x1f1`) and the interest an app announces with
//! `ToShell::PointerInterest` (tag `0x1f1`; the two enums have separate tag spaces).
//!
//! `ToApp::Pointer` is `u32 tag`, `u8 kind`, then by kind:
//! - 0 wheel: `f32 x`, `f32 y`, `f32 dx`, `f32 dy`, `u8 precise`, `u8 phase`, `u8 momentum phase`
//! - 1 hover: `f32 x`, `f32 y`
//! - 2 hover exit: nothing
//! - 3 magnify: `f32 x`, `f32 y`, `f32 magnification`, `u8 phase`
//! - 4 rotate: `f32 x`, `f32 y`, `f32 radians`, `u8 phase`
//!
//! Locations are surface pixels, like `ToApp::Touch`. A phase byte is `bifrost_ui::pointer_event::phase_to_wire`.
//! `ToShell::PointerInterest` is `u32 tag`, `u32 mask` (`bifrost_ui::pointer_event::{WHEEL, HOVER, GESTURES}`); bits
//! this version does not know are dropped on decode, so an app that asks for more than the shell knows still works.

use super::{f32_, u32_, DecodeError, Reader, ToApp, ToShell};
use bifrost_ui::pointer_event::{phase_from_wire, phase_to_wire, PointerEvent, ALL};

pub const TAG_POINTER: u32 = 0x1f1;
pub const TAG_INTEREST: u32 = 0x1f1;

pub(crate) fn encode_app(m: &ToApp, b: &mut Vec<u8>) {
    let ToApp::Pointer(ev) = m else { return };
    u32_(b, TAG_POINTER);
    encode_event(ev, b);
}

/// One event after its tag: the kind byte and the payload. `WindowInput::Pointer` carries the same bytes after its own
/// kind byte, so an extra window's wheel and hover use this layout too.
pub(crate) fn encode_event(ev: &PointerEvent, b: &mut Vec<u8>) {
    match *ev {
        PointerEvent::Wheel { x, y, dx, dy, precise, phase, momentum } => {
            b.push(0);
            for v in [x, y, dx, dy] {
                f32_(b, v);
            }
            b.push(u8::from(precise));
            b.push(phase_to_wire(phase));
            b.push(phase_to_wire(momentum));
        }
        PointerEvent::Hover { x, y } => {
            b.push(1);
            f32_(b, x);
            f32_(b, y);
        }
        PointerEvent::HoverExit => b.push(2),
        PointerEvent::Magnify { x, y, magnification, phase } => {
            b.push(3);
            for v in [x, y, magnification] {
                f32_(b, v);
            }
            b.push(phase_to_wire(phase));
        }
        PointerEvent::Rotate { x, y, radians, phase } => {
            b.push(4);
            for v in [x, y, radians] {
                f32_(b, v);
            }
            b.push(phase_to_wire(phase));
        }
    }
}

fn read_phase(r: &mut Reader) -> Result<bifrost_ui::wheel::Phase, DecodeError> {
    phase_from_wire(r.u8()?).ok_or(DecodeError::BadValue)
}

/// A number that must be finite: a NaN or infinite location or delta is refused rather than handed to an app.
fn finite(r: &mut Reader) -> Result<f32, DecodeError> {
    let v = r.f32()?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(DecodeError::BadValue)
    }
}

pub(crate) fn decode_app(t: u32, r: &mut Reader) -> Result<ToApp, DecodeError> {
    if t != TAG_POINTER {
        return Err(DecodeError::UnknownTag(t));
    }
    Ok(ToApp::Pointer(decode_event(r)?))
}

/// The kind byte and payload `encode_event` writes; a NaN or infinite number or an unknown kind or phase is refused.
pub(crate) fn decode_event(r: &mut Reader) -> Result<PointerEvent, DecodeError> {
    Ok(match r.u8()? {
        0 => {
            let (x, y, dx, dy) = (finite(r)?, finite(r)?, finite(r)?, finite(r)?);
            let precise = match r.u8()? {
                0 => false,
                1 => true,
                _ => return Err(DecodeError::BadValue),
            };
            let phase = read_phase(r)?;
            let momentum = read_phase(r)?;
            PointerEvent::Wheel { x, y, dx, dy, precise, phase, momentum }
        }
        1 => {
            let (x, y) = (finite(r)?, finite(r)?);
            PointerEvent::Hover { x, y }
        }
        2 => PointerEvent::HoverExit,
        3 => {
            let (x, y, magnification) = (finite(r)?, finite(r)?, finite(r)?);
            PointerEvent::Magnify { x, y, magnification, phase: read_phase(r)? }
        }
        4 => {
            let (x, y, radians) = (finite(r)?, finite(r)?, finite(r)?);
            PointerEvent::Rotate { x, y, radians, phase: read_phase(r)? }
        }
        _ => return Err(DecodeError::BadValue),
    })
}

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    let ToShell::PointerInterest { mask } = m else { return };
    u32_(b, TAG_INTEREST);
    u32_(b, *mask);
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    match t {
        TAG_INTEREST => Ok(ToShell::PointerInterest { mask: r.u32()? & ALL }),
        t => Err(DecodeError::UnknownTag(t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::pointer_event::{GESTURES, HOVER, WHEEL};
    use bifrost_ui::wheel::Phase;

    fn round(m: ToApp) {
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }

    #[test]
    fn every_pointer_event_round_trips() {
        round(ToApp::Pointer(PointerEvent::Wheel { x: 10.5, y: -2.0, dx: 0.0, dy: -30.0, precise: false, phase: Phase::None, momentum: Phase::None }));
        round(ToApp::Pointer(PointerEvent::Wheel { x: 1.0, y: 2.0, dx: 3.5, dy: 4.25, precise: true, phase: Phase::Began, momentum: Phase::MayBegin }));
        round(ToApp::Pointer(PointerEvent::Hover { x: 300.0, y: 200.5 }));
        round(ToApp::Pointer(PointerEvent::HoverExit));
        round(ToApp::Pointer(PointerEvent::Magnify { x: 5.0, y: 6.0, magnification: -0.25, phase: Phase::Changed }));
        round(ToApp::Pointer(PointerEvent::Rotate { x: 5.0, y: 6.0, radians: 1.5, phase: Phase::Ended }));
    }

    #[test]
    fn interest_round_trips_and_unknown_bits_are_dropped() {
        let m = ToShell::PointerInterest { mask: WHEEL | HOVER | GESTURES };
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        let wide = ToShell::PointerInterest { mask: 0xffff_fff1 };
        assert_eq!(ToShell::decode(&wide.encode()), Ok(ToShell::PointerInterest { mask: WHEEL }));
    }

    #[test]
    fn a_bad_kind_phase_flag_or_number_is_refused() {
        let good = ToApp::Pointer(PointerEvent::Wheel { x: 1.0, y: 2.0, dx: 3.0, dy: 4.0, precise: true, phase: Phase::Changed, momentum: Phase::None }).encode();
        // tag 0..4, kind 4, x 5..9, y 9..13, dx 13..17, dy 17..21, precise 21, phase 22, momentum 23
        let mut bad = good.clone();
        bad[4] = 9;
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue));
        let mut bad = good.clone();
        bad[21] = 2;
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue));
        let mut bad = good.clone();
        bad[22] = 77;
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue));
        let mut bad = good.clone();
        bad[5..9].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue));
        assert!(ToApp::decode(&good[..good.len() - 1]).is_err(), "a truncated packet is an error");
    }
}
