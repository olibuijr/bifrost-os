//! Animatable values: the closed set of kinds a key-path animation can interpolate (scalar, point, size, rect, colour,
//! transform), component access for sub-paths such as `position.x` or `transform.rotation.z`, and value functions
//! that turn plain numbers into transforms (rotate, scale, translate per axis).
//! Pure maths, no clock. The engine is in `keypath_anim`.

use crate::transform3d::Transform3D;
use crate::Rect;

/// What an animatable property holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimValue {
    Scalar(f32),
    Point(f32, f32),
    Size(f32, f32),
    Rect(Rect),
    /// Red, green, blue, alpha, each 0..=1 (not clamped while interpolating).
    Color([f32; 4]),
    Transform(Transform3D),
}

/// The kind of an [`AnimValue`], for checking that two values can be mixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Scalar,
    Point,
    Size,
    Rect,
    Color,
    Transform,
}

/// Something with named animatable properties: a view, a layer, a test double. `value` reads the model value of `key`
/// (`None` for an unknown key); `set_value` writes it and says whether the key and kind were accepted.
pub trait Animatable {
    fn value(&self, key: &str) -> Option<AnimValue>;
    fn set_value(&mut self, key: &str, value: AnimValue) -> bool;
}

/// Reads `key` from `target`, falling back to a component of a compound property (`position.x` is the `x` of
/// `position`; `frame.origin.x` is the `origin.x` of `frame`).
pub fn read_key(target: &dyn Animatable, key: &str) -> Option<AnimValue> {
    get_with(key, &|k| target.value(k))
}

/// Writes `value` to `key` of `target`, writing a component back into its compound property when `key` is a sub-path.
pub fn write_key(target: &mut dyn Animatable, key: &str, value: AnimValue) -> bool {
    if target.value(key).is_some() {
        return target.set_value(key, value);
    }
    let Some(s) = value.as_scalar() else { return false };
    for (d, _) in key.match_indices('.').rev() {
        let (parent, suffix) = (&key[..d], &key[d + 1..]);
        if let Some(pv) = read_key(&*target, parent) {
            if let Some(nv) = with_component(&pv, suffix, s) {
                return write_key(target, parent, nv);
            }
        }
    }
    false
}

/// Lookup with sub-path resolution over an arbitrary exact-key reader.
pub(crate) fn get_with(key: &str, exact: &dyn Fn(&str) -> Option<AnimValue>) -> Option<AnimValue> {
    if let Some(v) = exact(key) {
        return Some(v);
    }
    for (d, _) in key.match_indices('.').rev() {
        let (parent, suffix) = (&key[..d], &key[d + 1..]);
        if let Some(pv) = get_with(parent, exact) {
            if let Some(c) = component(&pv, suffix) {
                return Some(AnimValue::Scalar(c));
            }
        }
    }
    None
}

impl AnimValue {
    pub fn kind(&self) -> ValueKind {
        match self {
            AnimValue::Scalar(_) => ValueKind::Scalar,
            AnimValue::Point(..) => ValueKind::Point,
            AnimValue::Size(..) => ValueKind::Size,
            AnimValue::Rect(_) => ValueKind::Rect,
            AnimValue::Color(_) => ValueKind::Color,
            AnimValue::Transform(_) => ValueKind::Transform,
        }
    }

    pub fn as_scalar(&self) -> Option<f32> {
        match self {
            AnimValue::Scalar(v) => Some(*v),
            _ => None,
        }
    }

    /// The numbers the value is made of (a transform: its 16 matrix entries, row by row).
    pub fn components(&self) -> Vec<f32> {
        match self {
            AnimValue::Scalar(v) => vec![*v],
            AnimValue::Point(x, y) | AnimValue::Size(x, y) => vec![*x, *y],
            AnimValue::Rect(r) => vec![r.x, r.y, r.w, r.h],
            AnimValue::Color(c) => c.to_vec(),
            AnimValue::Transform(t) => t.m.iter().flatten().copied().collect(),
        }
    }

    /// A value of the same kind built from `c`; missing numbers read as zero.
    pub fn with_components(&self, c: &[f32]) -> AnimValue {
        let g = |i: usize| c.get(i).copied().unwrap_or(0.0);
        match self {
            AnimValue::Scalar(_) => AnimValue::Scalar(g(0)),
            AnimValue::Point(..) => AnimValue::Point(g(0), g(1)),
            AnimValue::Size(..) => AnimValue::Size(g(0), g(1)),
            AnimValue::Rect(_) => AnimValue::Rect(Rect { x: g(0), y: g(1), w: g(2), h: g(3) }),
            AnimValue::Color(_) => AnimValue::Color([g(0), g(1), g(2), g(3)]),
            AnimValue::Transform(_) => {
                let mut m = [[0.0f32; 4]; 4];
                for (i, v) in m.iter_mut().flatten().enumerate() {
                    *v = g(i);
                }
                AnimValue::Transform(Transform3D { m })
            }
        }
    }

    /// The zero of this kind (a transform: the identity, which is what an additive transform adds nothing with).
    pub fn zero_like(&self) -> AnimValue {
        match self {
            AnimValue::Transform(_) => AnimValue::Transform(Transform3D::IDENTITY),
            other => other.with_components(&[]),
        }
    }

    /// `self` at `t` between `self` (0) and `other` (1); `t` may leave 0..=1 for springs. `None` for different kinds.
    /// Transforms interpolate their decomposition (translation, scale, rotation) when both have one.
    pub fn lerp(&self, other: &AnimValue, t: f32) -> Option<AnimValue> {
        if self.kind() != other.kind() {
            return None;
        }
        if let (AnimValue::Transform(a), AnimValue::Transform(b)) = (self, other) {
            if let (Some(da), Some(db)) = (decompose(a), decompose(b)) {
                let mix = |x: f32, y: f32| x + (y - x) * t;
                let mut d = Decomp::default();
                for i in 0..3 {
                    d.t[i] = mix(da.t[i], db.t[i]);
                    d.s[i] = mix(da.s[i], db.s[i]);
                    d.r[i] = mix(da.r[i], db.r[i]);
                }
                return Some(AnimValue::Transform(compose(&d)));
            }
        }
        let (a, b) = (self.components(), other.components());
        let c: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x + (y - x) * t).collect();
        Some(self.with_components(&c))
    }

    /// `self + other`. A transform adds by applying `self` first and then `other`.
    pub fn add(&self, other: &AnimValue) -> Option<AnimValue> {
        if self.kind() != other.kind() {
            return None;
        }
        if let (AnimValue::Transform(a), AnimValue::Transform(b)) = (self, other) {
            return Some(AnimValue::Transform(a.concat(*b)));
        }
        let c: Vec<f32> = self.components().iter().zip(other.components()).map(|(x, y)| x + y).collect();
        Some(self.with_components(&c))
    }

    /// `self - other`. For transforms: `self` followed by the inverse of `other` (`None` when it has none).
    pub fn sub(&self, other: &AnimValue) -> Option<AnimValue> {
        if self.kind() != other.kind() {
            return None;
        }
        if let (AnimValue::Transform(a), AnimValue::Transform(b)) = (self, other) {
            return Some(AnimValue::Transform(a.concat(b.invert()?)));
        }
        let c: Vec<f32> = self.components().iter().zip(other.components()).map(|(x, y)| x - y).collect();
        Some(self.with_components(&c))
    }

    /// `self * k` (a transform: its decomposition scaled, so `k` of a rotation is a fraction of it).
    pub fn scaled(&self, k: f32) -> AnimValue {
        if let AnimValue::Transform(_) = self {
            return AnimValue::Transform(Transform3D::IDENTITY).lerp(self, k).unwrap_or(*self);
        }
        let c: Vec<f32> = self.components().iter().map(|x| x * k).collect();
        self.with_components(&c)
    }

    /// Euclidean distance between two values of one kind (zero for different kinds). Paced keyframes use it.
    pub fn distance(&self, other: &AnimValue) -> f32 {
        if self.kind() != other.kind() {
            return 0.0;
        }
        let s: f32 = self.components().iter().zip(other.components()).map(|(x, y)| (x - y) * (x - y)).sum();
        s.sqrt()
    }

    /// A premultiplied `0xAABBGGRR`-style word as the view layer stores it: channel `i` sits at bit `8 * i`.
    pub fn from_argb(word: u32) -> AnimValue {
        let ch = |s: u32| ((word >> s) & 255) as f32 / 255.0;
        AnimValue::Color([ch(0), ch(8), ch(16), ch(24)])
    }

    /// The inverse of [`AnimValue::from_argb`] for a colour (channels clamped and rounded); `None` for other kinds.
    pub fn to_argb(&self) -> Option<u32> {
        let AnimValue::Color(c) = self else { return None };
        let ch = |i: usize, s: u32| ((c[i].clamp(0.0, 1.0) * 255.0).round() as u32) << s;
        Some(ch(0, 0) | ch(1, 8) | ch(2, 16) | ch(3, 24))
    }
}

/// Translation, scale and rotation (radians about x, y and z) of a transform.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Decomp {
    pub t: [f32; 3],
    pub s: [f32; 3],
    pub r: [f32; 3],
}

/// Splits a transform into translation, scale and rotation. Shear is dropped and `None` is returned for a transform
/// with perspective. Exact for any single-axis rotation combined with scale and translation.
pub fn decompose(t: &Transform3D) -> Option<Decomp> {
    let m = &t.m;
    if m[0][3].abs() > 1e-6 || m[1][3].abs() > 1e-6 || m[2][3].abs() > 1e-6 || (m[3][3] - 1.0).abs() > 1e-6 {
        return None;
    }
    let mut d = Decomp { t: [m[3][0], m[3][1], m[3][2]], ..Decomp::default() };
    let mut n = [[0.0f32; 3]; 3];
    for i in 0..3 {
        let len = (m[i][0] * m[i][0] + m[i][1] * m[i][1] + m[i][2] * m[i][2]).sqrt();
        d.s[i] = len;
        for j in 0..3 {
            n[i][j] = if len > 1e-9 { m[i][j] / len } else if i == j { 1.0 } else { 0.0 };
        }
    }
    d.r[2] = n[0][1].atan2(n[0][0]);
    d.r[1] = (-n[0][2]).clamp(-1.0, 1.0).asin();
    d.r[0] = n[1][2].atan2(n[2][2]);
    Some(d)
}

/// Builds the transform of a [`Decomp`]: scale, then rotate about x, y and z, then translate.
pub fn compose(d: &Decomp) -> Transform3D {
    Transform3D::make_scale(d.s[0], d.s[1], d.s[2])
        .rotated(d.r[0], 1.0, 0.0, 0.0)
        .rotated(d.r[1], 0.0, 1.0, 0.0)
        .rotated(d.r[2], 0.0, 0.0, 1.0)
        .translated(d.t[0], d.t[1], d.t[2])
}

/// One component of a compound value by name. Point: `x`, `y`. Size: `width`, `height`. Rect: `x`, `y`, `width`,
/// `height` (also as `origin.x`, `size.width`, ...). Colour: `red`, `green`, `blue`, `alpha`. Transform:
/// `translation.x|y|z`, `scale.x|y|z`, `scale` (the x scale), `rotation.x|y|z`, `rotation` (about z).
pub fn component(v: &AnimValue, name: &str) -> Option<f32> {
    match v {
        AnimValue::Scalar(_) => None,
        AnimValue::Point(x, y) => match name {
            "x" => Some(*x),
            "y" => Some(*y),
            _ => None,
        },
        AnimValue::Size(w, h) => match name {
            "width" => Some(*w),
            "height" => Some(*h),
            _ => None,
        },
        AnimValue::Rect(r) => match name {
            "x" | "origin.x" => Some(r.x),
            "y" | "origin.y" => Some(r.y),
            "width" | "size.width" => Some(r.w),
            "height" | "size.height" => Some(r.h),
            _ => None,
        },
        AnimValue::Color(c) => match name {
            "red" => Some(c[0]),
            "green" => Some(c[1]),
            "blue" => Some(c[2]),
            "alpha" => Some(c[3]),
            _ => None,
        },
        AnimValue::Transform(t) => {
            let d = decompose(t)?;
            Some(match name {
                "translation.x" => d.t[0],
                "translation.y" => d.t[1],
                "translation.z" => d.t[2],
                "scale.x" | "scale" => d.s[0],
                "scale.y" => d.s[1],
                "scale.z" => d.s[2],
                "rotation.x" => d.r[0],
                "rotation.y" => d.r[1],
                "rotation.z" | "rotation" => d.r[2],
                _ => return None,
            })
        }
    }
}

/// `v` with the component `name` (see [`component`]) set to `x`.
pub fn with_component(v: &AnimValue, name: &str, x: f32) -> Option<AnimValue> {
    component(v, name)?;
    Some(match v {
        AnimValue::Scalar(_) => return None,
        AnimValue::Point(a, b) => if name == "x" { AnimValue::Point(x, *b) } else { AnimValue::Point(*a, x) },
        AnimValue::Size(a, b) => if name == "width" { AnimValue::Size(x, *b) } else { AnimValue::Size(*a, x) },
        AnimValue::Rect(r) => {
            let mut r = *r;
            match name {
                "x" | "origin.x" => r.x = x,
                "y" | "origin.y" => r.y = x,
                "width" | "size.width" => r.w = x,
                _ => r.h = x,
            }
            AnimValue::Rect(r)
        }
        AnimValue::Color(c) => {
            let mut c = *c;
            c[match name {
                "red" => 0,
                "green" => 1,
                "blue" => 2,
                _ => 3,
            }] = x;
            AnimValue::Color(c)
        }
        AnimValue::Transform(t) => {
            let mut d = decompose(t)?;
            match name {
                "translation.x" => d.t[0] = x,
                "translation.y" => d.t[1] = x,
                "translation.z" => d.t[2] = x,
                "scale.x" => d.s[0] = x,
                "scale" => {
                    d.s[0] = x;
                    d.s[1] = x;
                }
                "scale.y" => d.s[1] = x,
                "scale.z" => d.s[2] = x,
                "rotation.x" => d.r[0] = x,
                "rotation.y" => d.r[1] = x,
                _ => d.r[2] = x,
            }
            AnimValue::Transform(compose(&d))
        }
    })
}

/// Turns the numbers an animation produces into a transform (`valueFunction`): `RotateZ` takes an angle in radians,
/// `TranslateX` a distance, `ScaleY` a factor, and so on. `Scale` takes a factor or a point (x and y factors);
/// `Translate` takes a point or a distance along x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueFunction {
    RotateX,
    RotateY,
    RotateZ,
    ScaleX,
    ScaleY,
    ScaleZ,
    Scale,
    TranslateX,
    TranslateY,
    TranslateZ,
    Translate,
}

impl ValueFunction {
    /// `rotateX`, `scaleY`, `translate`, ... (case-insensitive).
    pub fn named(name: &str) -> Option<ValueFunction> {
        Some(match name.to_ascii_lowercase().as_str() {
            "rotatex" => ValueFunction::RotateX,
            "rotatey" => ValueFunction::RotateY,
            "rotatez" => ValueFunction::RotateZ,
            "scalex" => ValueFunction::ScaleX,
            "scaley" => ValueFunction::ScaleY,
            "scalez" => ValueFunction::ScaleZ,
            "scale" => ValueFunction::Scale,
            "translatex" => ValueFunction::TranslateX,
            "translatey" => ValueFunction::TranslateY,
            "translatez" => ValueFunction::TranslateZ,
            "translate" => ValueFunction::Translate,
            _ => return None,
        })
    }

    /// The transform for `v`, as an [`AnimValue::Transform`]. `None` when `v` is not a kind the function takes.
    pub fn apply(&self, v: &AnimValue) -> Option<AnimValue> {
        let t = match (self, v) {
            (ValueFunction::RotateX, AnimValue::Scalar(a)) => Transform3D::make_rotation(*a, 1.0, 0.0, 0.0),
            (ValueFunction::RotateY, AnimValue::Scalar(a)) => Transform3D::make_rotation(*a, 0.0, 1.0, 0.0),
            (ValueFunction::RotateZ, AnimValue::Scalar(a)) => Transform3D::make_rotation(*a, 0.0, 0.0, 1.0),
            (ValueFunction::ScaleX, AnimValue::Scalar(s)) => Transform3D::make_scale(*s, 1.0, 1.0),
            (ValueFunction::ScaleY, AnimValue::Scalar(s)) => Transform3D::make_scale(1.0, *s, 1.0),
            (ValueFunction::ScaleZ, AnimValue::Scalar(s)) => Transform3D::make_scale(1.0, 1.0, *s),
            (ValueFunction::Scale, AnimValue::Scalar(s)) => Transform3D::make_scale(*s, *s, 1.0),
            (ValueFunction::Scale, AnimValue::Point(x, y)) | (ValueFunction::Scale, AnimValue::Size(x, y)) => Transform3D::make_scale(*x, *y, 1.0),
            (ValueFunction::TranslateX, AnimValue::Scalar(d)) => Transform3D::make_translation(*d, 0.0, 0.0),
            (ValueFunction::TranslateY, AnimValue::Scalar(d)) => Transform3D::make_translation(0.0, *d, 0.0),
            (ValueFunction::TranslateZ, AnimValue::Scalar(d)) => Transform3D::make_translation(0.0, 0.0, *d),
            (ValueFunction::Translate, AnimValue::Scalar(d)) => Transform3D::make_translation(*d, 0.0, 0.0),
            (ValueFunction::Translate, AnimValue::Point(x, y)) | (ValueFunction::Translate, AnimValue::Size(x, y)) => Transform3D::make_translation(*x, *y, 0.0),
            _ => return None,
        };
        Some(AnimValue::Transform(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn lerp_add_sub_and_kind_checks() {
        let a = AnimValue::Point(0.0, 10.0);
        let b = AnimValue::Point(10.0, 30.0);
        assert_eq!(a.lerp(&b, 0.5), Some(AnimValue::Point(5.0, 20.0)));
        assert_eq!(a.lerp(&b, 2.0), Some(AnimValue::Point(20.0, 50.0)));
        assert_eq!(a.add(&b), Some(AnimValue::Point(10.0, 40.0)));
        assert_eq!(b.sub(&a), Some(AnimValue::Point(10.0, 20.0)));
        assert_eq!(a.lerp(&AnimValue::Scalar(1.0), 0.5), None);
        assert_eq!(a.distance(&b), (100.0f32 + 400.0).sqrt());
        assert_eq!(a.zero_like(), AnimValue::Point(0.0, 0.0));
    }

    #[test]
    fn colour_round_trips_through_the_stored_word() {
        let w = 0x80_40_20_10u32;
        assert_eq!(AnimValue::from_argb(w).to_argb(), Some(w));
        assert_eq!(AnimValue::Scalar(1.0).to_argb(), None);
        assert_eq!(AnimValue::Color([2.0, -1.0, 0.0, 1.0]).to_argb(), Some(0xFF_00_00_FF));
    }

    #[test]
    fn component_access_reads_and_writes_sub_paths() {
        let r = AnimValue::Rect(Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 });
        assert_eq!(component(&r, "size.width"), Some(3.0));
        assert_eq!(with_component(&r, "origin.y", 9.0), Some(AnimValue::Rect(Rect { x: 1.0, y: 9.0, w: 3.0, h: 4.0 })));
        assert_eq!(component(&r, "bogus"), None);
        let p = AnimValue::Point(1.0, 2.0);
        assert_eq!(with_component(&p, "y", 5.0), Some(AnimValue::Point(1.0, 5.0)));
        let c = AnimValue::Color([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(component(&with_component(&c, "alpha", 0.5).unwrap(), "alpha"), Some(0.5));
    }

    #[test]
    fn transform_components_round_trip() {
        let t = AnimValue::Transform(Transform3D::IDENTITY);
        let t = with_component(&t, "rotation.z", 0.5).unwrap();
        let t = with_component(&t, "translation.x", 7.0).unwrap();
        let t = with_component(&t, "scale", 2.0).unwrap();
        assert!(close(component(&t, "rotation.z").unwrap(), 0.5));
        assert!(close(component(&t, "translation.x").unwrap(), 7.0));
        assert!(close(component(&t, "scale.x").unwrap(), 2.0));
        assert!(close(component(&t, "scale.y").unwrap(), 2.0), "scale sets x and y");
        let x = AnimValue::Transform(Transform3D::make_rotation(0.4, 1.0, 0.0, 0.0));
        assert!(close(component(&x, "rotation.x").unwrap(), 0.4));
        let y = AnimValue::Transform(Transform3D::make_rotation(0.4, 0.0, 1.0, 0.0));
        assert!(close(component(&y, "rotation.y").unwrap(), 0.4));
    }

    #[test]
    fn transform_lerp_rotates_through_the_middle_angle() {
        let a = AnimValue::Transform(Transform3D::IDENTITY);
        let b = AnimValue::Transform(Transform3D::make_rotation(1.0, 0.0, 0.0, 1.0));
        let m = a.lerp(&b, 0.5).unwrap();
        assert!(close(component(&m, "rotation.z").unwrap(), 0.5));
        assert!(close(component(&m, "scale.x").unwrap(), 1.0), "a component-wise mix would shrink");
    }

    #[test]
    fn a_perspective_transform_has_no_decomposition_and_falls_back_to_components() {
        let p = Transform3D::make_perspective(500.0);
        assert!(decompose(&p).is_none());
        let a = AnimValue::Transform(Transform3D::IDENTITY);
        assert!(a.lerp(&AnimValue::Transform(p), 0.5).is_some());
    }

    #[test]
    fn value_functions_build_transforms() {
        let v = ValueFunction::RotateZ.apply(&AnimValue::Scalar(0.3)).unwrap();
        assert!(close(component(&v, "rotation.z").unwrap(), 0.3));
        let s = ValueFunction::Scale.apply(&AnimValue::Point(2.0, 3.0)).unwrap();
        assert!(close(component(&s, "scale.y").unwrap(), 3.0));
        let t = ValueFunction::Translate.apply(&AnimValue::Point(4.0, 5.0)).unwrap();
        assert!(close(component(&t, "translation.y").unwrap(), 5.0));
        assert_eq!(ValueFunction::RotateZ.apply(&AnimValue::Point(1.0, 1.0)), None);
        assert_eq!(ValueFunction::named("TranslateX"), Some(ValueFunction::TranslateX));
        assert_eq!(ValueFunction::named("nope"), None);
    }

    struct Bag {
        pos: (f32, f32),
    }
    impl Animatable for Bag {
        fn value(&self, key: &str) -> Option<AnimValue> {
            (key == "position").then(|| AnimValue::Point(self.pos.0, self.pos.1))
        }
        fn set_value(&mut self, key: &str, v: AnimValue) -> bool {
            match (key, v) {
                ("position", AnimValue::Point(x, y)) => {
                    self.pos = (x, y);
                    true
                }
                _ => false,
            }
        }
    }

    #[test]
    fn sub_paths_resolve_through_the_compound_property() {
        let mut b = Bag { pos: (1.0, 2.0) };
        assert_eq!(read_key(&b, "position.y"), Some(AnimValue::Scalar(2.0)));
        assert!(write_key(&mut b, "position.x", AnimValue::Scalar(9.0)));
        assert_eq!(b.pos, (9.0, 2.0));
        assert!(!write_key(&mut b, "nothing.x", AnimValue::Scalar(1.0)));
        assert_eq!(read_key(&b, "position.z"), None);
    }
}
