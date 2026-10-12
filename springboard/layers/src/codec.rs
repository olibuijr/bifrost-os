//! The wire encoding of a `Transaction` for `bifrost_layer_context_v1.commit` (a Wayland `array`).
//!
//! Format: little endian; a header `b"BLT1"`, `u64 seq`, `u8 implicit` (0 none, 1 then `f64 duration` and a timing
//! function), `u32 count`, then each change as `u8 tag` and its fields. Floats are IEEE `f32`/`f64`; enums are `u8`.
//! A decoder never panics on hostile input: lengths are checked, unknown tags are an error, the whole blob is at most
//! `MAX_LEN` bytes.
//!
//! Layout of the enum tags (the wire numbers, stable from version 1):
//! - `Change`: 0 Create, 1 Destroy, 2 SetRoot, 3 Insert, 4 RemoveFromSuperlayer, 5 Set, 6 SetHidden,
//!   7 SetMasksToBounds, 8 SetBorder, 9 SetShadow, 10 SetMaterial, 11 SetContents, 12 SetContentsGravity,
//!   13 AddAnimation, 14 RemoveAnimation, 15 RemoveAllAnimations.
//! - `Value`: 0 Scalar, 1 Point, 2 Rect, 3 Transform, 4 Color, 5 Shadow.
//! - `Animation`: 0 Basic, 1 Keyframe, 2 Spring, 3 Group (nested groups at most `MAX_DEPTH` deep).
//! - `TimingFunction`: 0 Linear, 1 EaseIn, 2 EaseOut, 3 EaseInEaseOut, 4 Default, 5 CubicBezier.
//! - `Property`: 0 Bounds ... 14 ZPosition, in declaration order. `FillMode`: 0 Removed, 1 Forwards, 2 Backwards,
//!   3 Both. `ContentsGravity`: 0 Resize, 1 ResizeAspect, 2 ResizeAspectFill, 3 Center, 4 TopLeft.
//! - `Material`: 0 None, 1 Glass. `LayerContents`: 0 None, 1 Buffer. `Option`: 0 None, 1 Some.
//!
//! Owner: wave G1 agent `layers-codec`.

use crate::animation::{Animation, AnimationId, FillMode, Keyframes, MediaTiming, Spring, Value};
use crate::geometry::{Affine, Color, Point, Rect};
use crate::layer::{
    Border, ContentsGravity, ContentsId, LayerContents, LayerId, Material, Property, Shadow,
};
use crate::timing::TimingFunction;
use crate::transaction::{Change, Transaction};

/// The largest accepted transaction blob (the Wayland array limit leaves room; bigger trees commit in several).
pub const MAX_LEN: usize = 1 << 20;
pub const MAGIC: &[u8; 4] = b"BLT1";

/// Nested animation groups deeper than this are refused (the decoder recurses; hostile input must not overflow the
/// stack).
pub const MAX_DEPTH: u32 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooLong,
    BadMagic,
    Truncated,
    UnknownTag(u8),
    BadValue,
}

type R<T> = Result<T, DecodeError>;

/// Encodes a transaction. The encoder writes what it is given; callers keep blobs within `MAX_LEN` (the decoder
/// refuses bigger ones).
pub fn encode(tx: &Transaction) -> Vec<u8> {
    let mut w = Writer {
        buf: Vec::with_capacity(64 + tx.changes.len() * 24),
    };
    w.bytes(MAGIC);
    w.u64(tx.seq);
    w.opt(&tx.implicit, |w, (duration, function)| {
        w.f64(*duration);
        w.timing_fn(function);
    });
    w.list(tx.changes.as_slice(), |w, c| w.change(c));
    w.buf
}

/// Decodes a transaction; `decode(&encode(tx)) == Ok(tx)` for every transaction whose ids are not 0 and whose
/// implicit duration is finite and not negative.
pub fn decode(bytes: &[u8]) -> Result<Transaction, DecodeError> {
    if bytes.len() > MAX_LEN {
        return Err(DecodeError::TooLong);
    }
    let mut r = Reader { bytes, pos: 0 };
    if r.take(MAGIC.len())? != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let seq = r.u64()?;
    let implicit = r.opt(|r| {
        let duration = r.duration()?;
        let function = r.timing_fn()?;
        Ok((duration, function))
    })?;
    let changes = r.list(|r| r.change())?;
    if r.remaining() != 0 {
        return Err(DecodeError::BadValue);
    }
    Ok(Transaction {
        seq,
        changes,
        implicit,
    })
}

/// Property wire codes, in declaration order.
fn property_code(p: Property) -> u8 {
    match p {
        Property::Bounds => 0,
        Property::Position => 1,
        Property::AnchorPoint => 2,
        Property::Transform => 3,
        Property::Opacity => 4,
        Property::Hidden => 5,
        Property::CornerRadius => 6,
        Property::MasksToBounds => 7,
        Property::BackgroundColor => 8,
        Property::Border => 9,
        Property::Shadow => 10,
        Property::Material => 11,
        Property::Contents => 12,
        Property::ContentsGravity => 13,
        Property::ZPosition => 14,
    }
}

fn property_from_code(code: u8) -> Option<Property> {
    Some(match code {
        0 => Property::Bounds,
        1 => Property::Position,
        2 => Property::AnchorPoint,
        3 => Property::Transform,
        4 => Property::Opacity,
        5 => Property::Hidden,
        6 => Property::CornerRadius,
        7 => Property::MasksToBounds,
        8 => Property::BackgroundColor,
        9 => Property::Border,
        10 => Property::Shadow,
        11 => Property::Material,
        12 => Property::Contents,
        13 => Property::ContentsGravity,
        14 => Property::ZPosition,
        _ => return None,
    })
}

fn gravity_code(g: ContentsGravity) -> u8 {
    match g {
        ContentsGravity::Resize => 0,
        ContentsGravity::ResizeAspect => 1,
        ContentsGravity::ResizeAspectFill => 2,
        ContentsGravity::Center => 3,
        ContentsGravity::TopLeft => 4,
    }
}

fn fill_mode_code(m: FillMode) -> u8 {
    match m {
        FillMode::Removed => 0,
        FillMode::Forwards => 1,
        FillMode::Backwards => 2,
        FillMode::Both => 3,
    }
}

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }

    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }

    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }

    fn opt<T>(&mut self, v: &Option<T>, f: impl FnOnce(&mut Self, &T)) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                f(self, x);
            }
        }
    }

    fn list<T>(&mut self, items: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.u32(items.len() as u32);
        for item in items {
            f(self, item);
        }
    }

    fn layer(&mut self, id: LayerId) {
        self.u32(id.0);
    }

    fn point(&mut self, p: Point) {
        self.f32(p.x);
        self.f32(p.y);
    }

    fn rect(&mut self, r: Rect) {
        self.f32(r.x);
        self.f32(r.y);
        self.f32(r.w);
        self.f32(r.h);
    }

    fn color(&mut self, c: Color) {
        self.f32(c.r);
        self.f32(c.g);
        self.f32(c.b);
        self.f32(c.a);
    }

    fn affine(&mut self, t: &Affine) {
        self.f32(t.a);
        self.f32(t.b);
        self.f32(t.c);
        self.f32(t.d);
        self.f32(t.tx);
        self.f32(t.ty);
    }

    fn shadow(&mut self, s: &Shadow) {
        self.color(s.color);
        self.f32(s.opacity);
        self.f32(s.radius);
        self.point(s.offset);
    }

    fn border(&mut self, b: &Border) {
        self.f32(b.width);
        self.color(b.color);
    }

    fn material(&mut self, m: &Material) {
        match *m {
            Material::None => self.u8(0),
            Material::Glass {
                blur,
                tint,
                saturation,
            } => {
                self.u8(1);
                self.f32(blur);
                self.color(tint);
                self.f32(saturation);
            }
        }
    }

    fn contents(&mut self, c: &LayerContents) {
        match *c {
            LayerContents::None => self.u8(0),
            LayerContents::Buffer(ContentsId(id)) => {
                self.u8(1);
                self.u32(id);
            }
        }
    }

    fn value(&mut self, v: &Value) {
        match *v {
            Value::Scalar(s) => {
                self.u8(0);
                self.f32(s);
            }
            Value::Point(p) => {
                self.u8(1);
                self.point(p);
            }
            Value::Rect(r) => {
                self.u8(2);
                self.rect(r);
            }
            Value::Transform(t) => {
                self.u8(3);
                self.affine(&t);
            }
            Value::Color(c) => {
                self.u8(4);
                self.color(c);
            }
            Value::Shadow(s) => {
                self.u8(5);
                self.shadow(&s);
            }
        }
    }

    fn timing_fn(&mut self, f: &TimingFunction) {
        match *f {
            TimingFunction::Linear => self.u8(0),
            TimingFunction::EaseIn => self.u8(1),
            TimingFunction::EaseOut => self.u8(2),
            TimingFunction::EaseInEaseOut => self.u8(3),
            TimingFunction::Default => self.u8(4),
            TimingFunction::CubicBezier(a, b, c, d) => {
                self.u8(5);
                self.f32(a);
                self.f32(b);
                self.f32(c);
                self.f32(d);
            }
        }
    }

    fn media(&mut self, m: &MediaTiming) {
        self.opt(&m.begin_time, |w, t| w.f64(*t));
        self.f64(m.duration);
        self.f32(m.speed);
        self.f64(m.time_offset);
        self.f32(m.repeat_count);
        self.bool(m.autoreverses);
        self.u8(fill_mode_code(m.fill_mode));
    }

    fn spring(&mut self, s: &Spring) {
        self.f32(s.mass);
        self.f32(s.stiffness);
        self.f32(s.damping);
        self.f32(s.initial_velocity);
    }

    fn keyframes(&mut self, k: &Keyframes) {
        self.list(k.values.as_slice(), |w, v| w.value(v));
        self.list(k.key_times.as_slice(), |w, t| w.f32(*t));
        self.list(k.timing_functions.as_slice(), |w, f| w.timing_fn(f));
    }

    fn animation(&mut self, a: &Animation) {
        match a {
            Animation::Basic {
                id,
                key,
                from,
                to,
                timing,
                function,
                additive,
            } => {
                self.u8(0);
                self.u32(id.0);
                self.u8(property_code(*key));
                self.opt(from, |w, v| w.value(v));
                self.opt(to, |w, v| w.value(v));
                self.media(timing);
                self.timing_fn(function);
                self.bool(*additive);
            }
            Animation::Keyframe {
                id,
                key,
                keyframes,
                timing,
            } => {
                self.u8(1);
                self.u32(id.0);
                self.u8(property_code(*key));
                self.keyframes(keyframes);
                self.media(timing);
            }
            Animation::Spring {
                id,
                key,
                from,
                to,
                spring,
                timing,
            } => {
                self.u8(2);
                self.u32(id.0);
                self.u8(property_code(*key));
                self.opt(from, |w, v| w.value(v));
                self.opt(to, |w, v| w.value(v));
                self.spring(spring);
                self.media(timing);
            }
            Animation::Group {
                id,
                animations,
                timing,
            } => {
                self.u8(3);
                self.u32(id.0);
                self.list(animations.as_slice(), |w, a| w.animation(a));
                self.media(timing);
            }
        }
    }

    fn change(&mut self, c: &Change) {
        match c {
            Change::Create(l) => {
                self.u8(0);
                self.layer(*l);
            }
            Change::Destroy(l) => {
                self.u8(1);
                self.layer(*l);
            }
            Change::SetRoot(l) => {
                self.u8(2);
                self.layer(*l);
            }
            Change::Insert {
                parent,
                child,
                index,
            } => {
                self.u8(3);
                self.layer(*parent);
                self.layer(*child);
                self.u32(*index);
            }
            Change::RemoveFromSuperlayer(l) => {
                self.u8(4);
                self.layer(*l);
            }
            Change::Set { layer, key, value } => {
                self.u8(5);
                self.layer(*layer);
                self.u8(property_code(*key));
                self.value(value);
            }
            Change::SetHidden(l, on) => {
                self.u8(6);
                self.layer(*l);
                self.bool(*on);
            }
            Change::SetMasksToBounds(l, on) => {
                self.u8(7);
                self.layer(*l);
                self.bool(*on);
            }
            Change::SetBorder(l, b) => {
                self.u8(8);
                self.layer(*l);
                self.border(b);
            }
            Change::SetShadow(l, s) => {
                self.u8(9);
                self.layer(*l);
                self.shadow(s);
            }
            Change::SetMaterial(l, m) => {
                self.u8(10);
                self.layer(*l);
                self.material(m);
            }
            Change::SetContents(l, c) => {
                self.u8(11);
                self.layer(*l);
                self.contents(c);
            }
            Change::SetContentsGravity(l, g) => {
                self.u8(12);
                self.layer(*l);
                self.u8(gravity_code(*g));
            }
            Change::AddAnimation { layer, animation } => {
                self.u8(13);
                self.layer(*layer);
                self.animation(animation);
            }
            Change::RemoveAnimation { layer, id } => {
                self.u8(14);
                self.layer(*layer);
                self.u32(id.0);
            }
            Change::RemoveAllAnimations(l) => {
                self.u8(15);
                self.layer(*l);
            }
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self
            .bytes
            .get(self.pos..end)
            .ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> R<[u8; N]> {
        let s = self.take(N)?;
        <[u8; N]>::try_from(s).map_err(|_| DecodeError::Truncated)
    }

    fn u8(&mut self) -> R<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn bool(&mut self) -> R<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadValue),
        }
    }

    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> R<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> R<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// A time or other f64 that must be finite.
    fn finite(&mut self) -> R<f64> {
        let v = f64::from_bits(self.u64()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(DecodeError::BadValue)
        }
    }

    /// A duration: finite and not negative.
    fn duration(&mut self) -> R<f64> {
        let v = self.finite()?;
        if v >= 0.0 {
            Ok(v)
        } else {
            Err(DecodeError::BadValue)
        }
    }

    fn opt<T>(&mut self, f: impl FnOnce(&mut Self) -> R<T>) -> R<Option<T>> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(f(self)?)),
            _ => Err(DecodeError::BadValue),
        }
    }

    /// A count-prefixed list. The allocation is bounded by the bytes left, so a hostile count cannot exhaust memory.
    fn list<T>(&mut self, mut f: impl FnMut(&mut Self) -> R<T>) -> R<Vec<T>> {
        let n = self.u32()? as usize;
        let mut out = Vec::with_capacity(n.min(self.remaining()));
        for _ in 0..n {
            out.push(f(self)?);
        }
        Ok(out)
    }

    /// A layer id; 0 is never used, so it is refused.
    fn layer(&mut self) -> R<LayerId> {
        match self.u32()? {
            0 => Err(DecodeError::BadValue),
            v => Ok(LayerId(v)),
        }
    }

    fn property(&mut self) -> R<Property> {
        property_from_code(self.u8()?).ok_or(DecodeError::BadValue)
    }

    fn gravity(&mut self) -> R<ContentsGravity> {
        Ok(match self.u8()? {
            0 => ContentsGravity::Resize,
            1 => ContentsGravity::ResizeAspect,
            2 => ContentsGravity::ResizeAspectFill,
            3 => ContentsGravity::Center,
            4 => ContentsGravity::TopLeft,
            _ => return Err(DecodeError::BadValue),
        })
    }

    fn point(&mut self) -> R<Point> {
        Ok(Point {
            x: self.f32()?,
            y: self.f32()?,
        })
    }

    fn rect(&mut self) -> R<Rect> {
        Ok(Rect {
            x: self.f32()?,
            y: self.f32()?,
            w: self.f32()?,
            h: self.f32()?,
        })
    }

    fn color(&mut self) -> R<Color> {
        Ok(Color {
            r: self.f32()?,
            g: self.f32()?,
            b: self.f32()?,
            a: self.f32()?,
        })
    }

    fn affine(&mut self) -> R<Affine> {
        Ok(Affine {
            a: self.f32()?,
            b: self.f32()?,
            c: self.f32()?,
            d: self.f32()?,
            tx: self.f32()?,
            ty: self.f32()?,
        })
    }

    fn shadow(&mut self) -> R<Shadow> {
        Ok(Shadow {
            color: self.color()?,
            opacity: self.f32()?,
            radius: self.f32()?,
            offset: self.point()?,
        })
    }

    fn border(&mut self) -> R<Border> {
        Ok(Border {
            width: self.f32()?,
            color: self.color()?,
        })
    }

    fn material(&mut self) -> R<Material> {
        Ok(match self.u8()? {
            0 => Material::None,
            1 => Material::Glass {
                blur: self.f32()?,
                tint: self.color()?,
                saturation: self.f32()?,
            },
            _ => return Err(DecodeError::BadValue),
        })
    }

    fn contents(&mut self) -> R<LayerContents> {
        Ok(match self.u8()? {
            0 => LayerContents::None,
            1 => LayerContents::Buffer(ContentsId(self.u32()?)),
            _ => return Err(DecodeError::BadValue),
        })
    }

    fn value(&mut self) -> R<Value> {
        Ok(match self.u8()? {
            0 => Value::Scalar(self.f32()?),
            1 => Value::Point(self.point()?),
            2 => Value::Rect(self.rect()?),
            3 => Value::Transform(self.affine()?),
            4 => Value::Color(self.color()?),
            5 => Value::Shadow(self.shadow()?),
            t => return Err(DecodeError::UnknownTag(t)),
        })
    }

    fn timing_fn(&mut self) -> R<TimingFunction> {
        Ok(match self.u8()? {
            0 => TimingFunction::Linear,
            1 => TimingFunction::EaseIn,
            2 => TimingFunction::EaseOut,
            3 => TimingFunction::EaseInEaseOut,
            4 => TimingFunction::Default,
            5 => TimingFunction::CubicBezier(self.f32()?, self.f32()?, self.f32()?, self.f32()?),
            t => return Err(DecodeError::UnknownTag(t)),
        })
    }

    fn media(&mut self) -> R<MediaTiming> {
        let begin_time = self.opt(|r| r.finite())?;
        let duration = self.duration()?;
        let speed = self.f32()?;
        let time_offset = self.finite()?;
        let repeat_count = self.f32()?;
        let autoreverses = self.bool()?;
        let fill_mode = match self.u8()? {
            0 => FillMode::Removed,
            1 => FillMode::Forwards,
            2 => FillMode::Backwards,
            3 => FillMode::Both,
            _ => return Err(DecodeError::BadValue),
        };
        Ok(MediaTiming {
            begin_time,
            duration,
            speed,
            time_offset,
            repeat_count,
            autoreverses,
            fill_mode,
        })
    }

    fn spring(&mut self) -> R<Spring> {
        Ok(Spring {
            mass: self.f32()?,
            stiffness: self.f32()?,
            damping: self.f32()?,
            initial_velocity: self.f32()?,
        })
    }

    fn keyframes(&mut self) -> R<Keyframes> {
        let values = self.list(|r| r.value())?;
        let key_times = self.list(|r| r.f32())?;
        let timing_functions = self.list(|r| r.timing_fn())?;
        Ok(Keyframes {
            values,
            key_times,
            timing_functions,
        })
    }

    fn animation(&mut self, depth: u32) -> R<Animation> {
        if depth > MAX_DEPTH {
            return Err(DecodeError::BadValue);
        }
        let tag = self.u8()?;
        let id = AnimationId(self.u32()?);
        Ok(match tag {
            0 => {
                let key = self.property()?;
                let from = self.opt(|r| r.value())?;
                let to = self.opt(|r| r.value())?;
                let timing = self.media()?;
                let function = self.timing_fn()?;
                let additive = self.bool()?;
                Animation::Basic {
                    id,
                    key,
                    from,
                    to,
                    timing,
                    function,
                    additive,
                }
            }
            1 => {
                let key = self.property()?;
                let keyframes = self.keyframes()?;
                let timing = self.media()?;
                Animation::Keyframe {
                    id,
                    key,
                    keyframes,
                    timing,
                }
            }
            2 => {
                let key = self.property()?;
                let from = self.opt(|r| r.value())?;
                let to = self.opt(|r| r.value())?;
                let spring = self.spring()?;
                let timing = self.media()?;
                Animation::Spring {
                    id,
                    key,
                    from,
                    to,
                    spring,
                    timing,
                }
            }
            3 => {
                let animations = self.list(|r| r.animation(depth + 1))?;
                let timing = self.media()?;
                Animation::Group {
                    id,
                    animations,
                    timing,
                }
            }
            t => return Err(DecodeError::UnknownTag(t)),
        })
    }

    fn change(&mut self) -> R<Change> {
        let tag = self.u8()?;
        Ok(match tag {
            0 => Change::Create(self.layer()?),
            1 => Change::Destroy(self.layer()?),
            2 => Change::SetRoot(self.layer()?),
            3 => Change::Insert {
                parent: self.layer()?,
                child: self.layer()?,
                index: self.u32()?,
            },
            4 => Change::RemoveFromSuperlayer(self.layer()?),
            5 => {
                let layer = self.layer()?;
                let key = self.property()?;
                let value = self.value()?;
                Change::Set { layer, key, value }
            }
            6 => Change::SetHidden(self.layer()?, self.bool()?),
            7 => Change::SetMasksToBounds(self.layer()?, self.bool()?),
            8 => Change::SetBorder(self.layer()?, self.border()?),
            9 => Change::SetShadow(self.layer()?, self.shadow()?),
            10 => Change::SetMaterial(self.layer()?, self.material()?),
            11 => Change::SetContents(self.layer()?, self.contents()?),
            12 => Change::SetContentsGravity(self.layer()?, self.gravity()?),
            13 => {
                let layer = self.layer()?;
                let animation = self.animation(0)?;
                Change::AddAnimation { layer, animation }
            }
            14 => {
                let layer = self.layer()?;
                let id = AnimationId(self.u32()?);
                Change::RemoveAnimation { layer, id }
            }
            15 => Change::RemoveAllAnimations(self.layer()?),
            t => return Err(DecodeError::UnknownTag(t)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(tx: &Transaction) {
        let bytes = encode(tx);
        assert!(bytes.len() <= MAX_LEN);
        assert_eq!(decode(&bytes), Ok(tx.clone()));
    }

    fn color(v: f32) -> Color {
        Color {
            r: v,
            g: v * 0.5,
            b: 0.25,
            a: 1.0,
        }
    }

    fn timing() -> MediaTiming {
        MediaTiming {
            begin_time: Some(1.5),
            duration: 0.4,
            speed: 2.0,
            time_offset: 0.125,
            repeat_count: f32::INFINITY,
            autoreverses: true,
            fill_mode: FillMode::Both,
        }
    }

    fn shadow() -> Shadow {
        Shadow {
            color: Color::BLACK,
            opacity: 0.4,
            radius: 12.0,
            offset: Point { x: 0.0, y: 6.0 },
        }
    }

    /// One of every change variant, with nonzero ids and distinct values.
    fn every_change() -> Vec<Change> {
        let l = LayerId(3);
        vec![
            Change::Create(l),
            Change::Destroy(LayerId(4)),
            Change::SetRoot(l),
            Change::Insert {
                parent: l,
                child: LayerId(5),
                index: 2,
            },
            Change::RemoveFromSuperlayer(LayerId(5)),
            Change::Set {
                layer: l,
                key: Property::Opacity,
                value: Value::Scalar(0.5),
            },
            Change::SetHidden(l, true),
            Change::SetMasksToBounds(l, false),
            Change::SetBorder(
                l,
                Border {
                    width: 1.0,
                    color: color(0.2),
                },
            ),
            Change::SetShadow(l, shadow()),
            Change::SetMaterial(
                l,
                Material::Glass {
                    blur: 24.0,
                    tint: color(0.9),
                    saturation: 1.8,
                },
            ),
            Change::SetContents(l, LayerContents::Buffer(ContentsId(7))),
            Change::SetContentsGravity(l, ContentsGravity::ResizeAspectFill),
            Change::AddAnimation {
                layer: l,
                animation: Animation::Basic {
                    id: AnimationId(9),
                    key: Property::Position,
                    from: None,
                    to: Some(Value::Point(Point { x: 10.0, y: 20.0 })),
                    timing: timing(),
                    function: TimingFunction::EaseOut,
                    additive: false,
                },
            },
            Change::RemoveAnimation {
                layer: l,
                id: AnimationId(9),
            },
            Change::RemoveAllAnimations(l),
        ]
    }

    fn tx_of(changes: Vec<Change>) -> Transaction {
        Transaction {
            seq: 42,
            changes,
            implicit: Some((0.3, TimingFunction::CubicBezier(0.1, 0.2, 0.3, 0.4))),
        }
    }

    #[test]
    fn empty_transaction_round_trips_and_has_the_hand_worked_bytes() {
        let tx = Transaction::new(7);
        let bytes = encode(&tx);
        // magic (4) + seq u64 (8) + implicit none (1) + count u32 (4) = 17 bytes.
        let expected: Vec<u8> = [
            b'B', b'L', b'T', b'1', 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]
        .to_vec();
        assert_eq!(bytes, expected);
        assert_eq!(decode(&bytes), Ok(tx));
    }

    #[test]
    fn create_layer_has_the_hand_worked_bytes() {
        let mut tx = Transaction::new(1);
        tx.push(Change::Create(LayerId(1)));
        // header 17 bytes, then tag 0 and LayerId(1) as u32 little endian: 22 bytes in all.
        let expected: Vec<u8> = vec![
            b'B', b'L', b'T', b'1', 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0,
        ];
        assert_eq!(encode(&tx), expected);
        assert_eq!(decode(&expected), Ok(tx));
    }

    #[test]
    fn every_change_variant_round_trips() {
        let tx = tx_of(every_change());
        assert_eq!(tx.changes.len(), 16);
        roundtrip(&tx);
        roundtrip(&Transaction {
            seq: u64::MAX,
            changes: every_change(),
            implicit: None,
        });
    }

    #[test]
    fn every_value_kind_round_trips_through_set() {
        let values = [
            Value::Scalar(-1.25),
            Value::Point(Point { x: 3.0, y: -4.5 }),
            Value::Rect(Rect::new(1.0, 2.0, 30.0, 40.0)),
            Value::Transform(Affine {
                a: 0.5,
                b: 0.25,
                c: -0.25,
                d: 0.5,
                tx: 7.0,
                ty: 8.0,
            }),
            Value::Color(color(0.7)),
            Value::Shadow(shadow()),
        ];
        for v in values {
            roundtrip(&Transaction {
                seq: 1,
                changes: vec![Change::Set {
                    layer: LayerId(2),
                    key: Property::Bounds,
                    value: v,
                }],
                implicit: None,
            });
        }
    }

    #[test]
    fn every_property_code_round_trips_and_unknown_codes_are_refused() {
        let props = [
            Property::Bounds,
            Property::Position,
            Property::AnchorPoint,
            Property::Transform,
            Property::Opacity,
            Property::Hidden,
            Property::CornerRadius,
            Property::MasksToBounds,
            Property::BackgroundColor,
            Property::Border,
            Property::Shadow,
            Property::Material,
            Property::Contents,
            Property::ContentsGravity,
            Property::ZPosition,
        ];
        for (code, p) in props.iter().enumerate() {
            assert_eq!(property_code(*p), code as u8);
            assert_eq!(property_from_code(code as u8), Some(*p));
        }
        assert_eq!(property_from_code(15), None);
        let changes = props
            .iter()
            .map(|p| Change::Set {
                layer: LayerId(1),
                key: *p,
                value: Value::Scalar(1.0),
            })
            .collect();
        roundtrip(&Transaction {
            seq: 2,
            changes,
            implicit: None,
        });
    }

    #[test]
    fn every_animation_kind_round_trips_including_nested_groups() {
        let keyframes = Keyframes {
            values: vec![Value::Scalar(0.0), Value::Scalar(0.5), Value::Scalar(1.0)],
            key_times: vec![0.0, 0.25, 1.0],
            timing_functions: vec![TimingFunction::Linear, TimingFunction::EaseInEaseOut],
        };
        let spring = Spring {
            mass: 1.0,
            stiffness: 170.0,
            damping: 26.0,
            initial_velocity: 0.0,
        };
        let basic = Animation::Basic {
            id: AnimationId(1),
            key: Property::Opacity,
            from: Some(Value::Scalar(0.0)),
            to: Some(Value::Scalar(1.0)),
            timing: MediaTiming::default(),
            function: TimingFunction::Default,
            additive: true,
        };
        let kf = Animation::Keyframe {
            id: AnimationId(2),
            key: Property::Transform,
            keyframes,
            timing: timing(),
        };
        let sp = Animation::Spring {
            id: AnimationId(3),
            key: Property::Position,
            from: None,
            to: Some(Value::Point(Point { x: 5.0, y: 5.0 })),
            spring,
            timing: MediaTiming {
                begin_time: None,
                fill_mode: FillMode::Forwards,
                ..MediaTiming::default()
            },
        };
        let group = Animation::Group {
            id: AnimationId(4),
            animations: vec![basic.clone(), kf.clone(), sp.clone()],
            timing: timing(),
        };
        let nested = Animation::Group {
            id: AnimationId(5),
            animations: vec![
                group.clone(),
                Animation::Group {
                    id: AnimationId(6),
                    animations: vec![],
                    timing: MediaTiming::default(),
                },
            ],
            timing: MediaTiming::default(),
        };
        for a in [basic, kf, sp, group, nested] {
            roundtrip(&Transaction {
                seq: 3,
                changes: vec![Change::AddAnimation {
                    layer: LayerId(8),
                    animation: a,
                }],
                implicit: None,
            });
        }
    }

    #[test]
    fn every_timing_function_fill_mode_gravity_material_and_contents_round_trips() {
        let fns = [
            TimingFunction::Linear,
            TimingFunction::EaseIn,
            TimingFunction::EaseOut,
            TimingFunction::EaseInEaseOut,
            TimingFunction::Default,
            TimingFunction::CubicBezier(0.0, 0.5, 1.0, -0.25),
        ];
        let modes = [
            FillMode::Removed,
            FillMode::Forwards,
            FillMode::Backwards,
            FillMode::Both,
        ];
        let gravities = [
            ContentsGravity::Resize,
            ContentsGravity::ResizeAspect,
            ContentsGravity::ResizeAspectFill,
            ContentsGravity::Center,
            ContentsGravity::TopLeft,
        ];
        for f in fns {
            for m in modes {
                let a = Animation::Basic {
                    id: AnimationId(1),
                    key: Property::Bounds,
                    from: None,
                    to: None,
                    timing: MediaTiming {
                        fill_mode: m,
                        begin_time: None,
                        ..MediaTiming::default()
                    },
                    function: f,
                    additive: false,
                };
                roundtrip(&Transaction {
                    seq: 0,
                    changes: vec![Change::AddAnimation {
                        layer: LayerId(1),
                        animation: a,
                    }],
                    implicit: Some((0.0, f)),
                });
            }
        }
        for g in gravities {
            roundtrip(&Transaction {
                seq: 0,
                changes: vec![Change::SetContentsGravity(LayerId(1), g)],
                implicit: None,
            });
        }
        let materials = [
            Material::None,
            Material::Glass {
                blur: 0.0,
                tint: Color::CLEAR,
                saturation: 1.0,
            },
        ];
        for m in materials {
            roundtrip(&Transaction {
                seq: 0,
                changes: vec![Change::SetMaterial(LayerId(1), m)],
                implicit: None,
            });
        }
        for c in [
            LayerContents::None,
            LayerContents::Buffer(ContentsId(u32::MAX)),
        ] {
            roundtrip(&Transaction {
                seq: 0,
                changes: vec![Change::SetContents(LayerId(1), c)],
                implicit: None,
            });
        }
    }

    #[test]
    fn every_truncation_of_a_valid_blob_is_an_error() {
        let bytes = encode(&tx_of(every_change()));
        for len in 0..bytes.len() {
            assert!(
                decode(&bytes[..len]).is_err(),
                "prefix of {len} bytes decoded"
            );
        }
        assert!(decode(&bytes).is_ok());
    }

    #[test]
    fn hostile_inputs_are_refused_without_panicking() {
        // Bad magic.
        assert_eq!(
            decode(b"XXXX\0\0\0\0\0\0\0\0\0\0\0\0\0"),
            Err(DecodeError::BadMagic)
        );
        // Too long.
        assert_eq!(decode(&vec![0u8; MAX_LEN + 1]), Err(DecodeError::TooLong));
        // A count of u32::MAX with no payload is truncated, not an allocation.
        let mut huge = MAGIC.to_vec();
        huge.extend_from_slice(&[0u8; 8]);
        huge.push(0);
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&huge), Err(DecodeError::Truncated));
        // Trailing bytes.
        let mut bytes = encode(&Transaction::new(1));
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(DecodeError::BadValue));
        // Unknown change tag.
        let mut tx = Transaction::new(1);
        tx.push(Change::Create(LayerId(1)));
        let mut bytes = encode(&tx);
        bytes[17] = 0xEE;
        assert_eq!(decode(&bytes), Err(DecodeError::UnknownTag(0xEE)));
        // A bool byte that is neither 0 nor 1 (the last byte of SetHidden).
        let mut tx = Transaction::new(1);
        tx.push(Change::SetHidden(LayerId(1), true));
        let mut bytes = encode(&tx);
        let last = bytes.len() - 1;
        bytes[last] = 2;
        assert_eq!(decode(&bytes), Err(DecodeError::BadValue));
        // Layer id 0 is never used.
        let mut tx = Transaction::new(1);
        tx.push(Change::Create(LayerId(0)));
        assert_eq!(decode(&encode(&tx)), Err(DecodeError::BadValue));
        // A non-finite implicit duration.
        let tx = Transaction {
            seq: 1,
            changes: vec![],
            implicit: Some((f64::NAN, TimingFunction::Linear)),
        };
        assert_eq!(decode(&encode(&tx)), Err(DecodeError::BadValue));
        // A negative implicit duration.
        let tx = Transaction {
            seq: 1,
            changes: vec![],
            implicit: Some((-1.0, TimingFunction::Linear)),
        };
        assert_eq!(decode(&encode(&tx)), Err(DecodeError::BadValue));
    }

    #[test]
    fn deep_animation_groups_are_refused_not_recursed_forever() {
        let build = |depth: u32| {
            let mut a = Animation::Basic {
                id: AnimationId(1),
                key: Property::Opacity,
                from: None,
                to: None,
                timing: MediaTiming::default(),
                function: TimingFunction::Linear,
                additive: false,
            };
            for _ in 0..depth {
                a = Animation::Group {
                    id: AnimationId(1),
                    animations: vec![a],
                    timing: MediaTiming::default(),
                };
            }
            Transaction {
                seq: 0,
                changes: vec![Change::AddAnimation {
                    layer: LayerId(1),
                    animation: a,
                }],
                implicit: None,
            }
        };
        // Depth MAX_DEPTH groups around one leaf is accepted; one more is refused.
        roundtrip(&build(MAX_DEPTH));
        assert_eq!(
            decode(&encode(&build(MAX_DEPTH + 1))),
            Err(DecodeError::BadValue)
        );
        assert_eq!(decode(&encode(&build(200))), Err(DecodeError::BadValue));
    }

    #[test]
    fn garbage_never_panics() {
        // A small deterministic generator (LCG) so the test needs no crate.
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u8
        };
        for round in 0..2000 {
            let len = (round % 180) as usize;
            let junk: Vec<u8> = (0..len).map(|_| next()).collect();
            let _ = decode(&junk);
            // The same bytes behind a valid header reach the change decoder.
            let mut framed = MAGIC.to_vec();
            framed.extend_from_slice(&[0u8; 8]);
            framed.push((round % 2) as u8);
            framed.extend_from_slice(&junk);
            let _ = decode(&framed);
        }
    }

    #[test]
    fn every_single_byte_corruption_of_a_valid_blob_is_handled() {
        let good = encode(&tx_of(every_change()));
        for i in 0..good.len() {
            for flip in [0x01u8, 0x80, 0xff] {
                let mut bad = good.clone();
                bad[i] ^= flip;
                // Must return (Ok or Err) without panicking; a flipped byte inside a float may still decode.
                let _ = decode(&bad);
            }
        }
    }
}
