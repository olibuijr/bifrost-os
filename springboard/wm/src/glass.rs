//! Live glass: the blurred backdrop behind a surface's `ext-background-effect-v1` region, built every frame from what is
//! composited below it (wallpaper, windows, layers), the way a window server does it. The client draws its own tint
//! over the region with alpha. Dual-Kawase blur on a quarter-size copy, with a saturation lift like the shell's
//! materials (`bifrost_render::glass`). Each region is masked to its rounded rectangle (the window's corner radius the
//! client sends), so the blur never shows past a window's rounded corners.

use std::collections::HashMap;

use crate::material_filter::Tone;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                texture::{TextureRenderBuffer, TextureRenderElement},
                Element, Id, Kind, RenderElement,
            },
            gles::{element::PixelShaderElement, GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType},
            utils::{draw_render_elements, CommitCounter, DamageSet, OpaqueRegions},
            Bind, Color32F, Frame, Offscreen, Renderer,
        },
    },
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

/// The backdrop is rendered at 1/DIV of the output (the recipes' `backdropScale` 0.25 for every desktop material), then
/// halved PASSES times and doubled PASSES-1 times.
pub const DIV: i32 = 4;
pub const PASSES: usize = 3;
/// Saturation of the blurred backdrop (vibrancy); 1.0 keeps the colours.
pub const SATURATION: f32 = 1.8;

const DOWN: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision mediump float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
uniform vec2 halfpixel;
varying vec2 v_coords;
void main() {
    vec4 sum = texture2D(tex, v_coords) * 4.0;
    sum += texture2D(tex, v_coords - halfpixel);
    sum += texture2D(tex, v_coords + halfpixel);
    sum += texture2D(tex, v_coords + vec2(halfpixel.x, -halfpixel.y));
    sum += texture2D(tex, v_coords - vec2(halfpixel.x, -halfpixel.y));
    gl_FragColor = sum / 8.0 * alpha;
}"#;

const UP: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision mediump float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
uniform vec2 halfpixel;
uniform float saturation;
uniform float brightness;
uniform float lum_amount;
uniform vec4 lum_values;
varying vec2 v_coords;
// the luminance curve: the cubic Bezier through four control values (material_filter::curve)
float lum_curve(float x) {
    float u = 1.0 - x;
    return u * u * u * lum_values.x + 3.0 * u * u * x * lum_values.y + 3.0 * u * x * x * lum_values.z + x * x * x * lum_values.w;
}
void main() {
    vec4 sum = texture2D(tex, v_coords + vec2(-halfpixel.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(-halfpixel.x, halfpixel.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(0.0, halfpixel.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(halfpixel.x, halfpixel.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(halfpixel.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(halfpixel.x, -halfpixel.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(0.0, -halfpixel.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(-halfpixel.x, -halfpixel.y)) * 2.0;
    vec4 c = sum / 12.0;
    float l = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
    c.rgb = clamp(mix(vec3(l), c.rgb, saturation), 0.0, c.a);
    if (lum_amount != 0.0) {
        float m = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
        c.rgb = clamp(c.rgb + lum_amount * (lum_curve(clamp(m, 0.0, 1.0)) - m), 0.0, c.a);
    }
    c.rgb = clamp(c.rgb + brightness, 0.0, c.a);
    gl_FragColor = c * alpha;
}"#;

/// The blurred backdrop of one region, masked to the region's rounded rectangle. `v_coords` is the texture coordinate
/// (the blur covers the whole output, so `v_coords * out_size` is the position in region space, flipped back when the
/// texture is upside down); `rect` is the region and `radius` its corner radius in the same units.
const MASK: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
uniform vec2 out_size;
uniform vec4 rect;
uniform float radius;
uniform float flip;
varying vec2 v_coords;
void main() {
    vec2 t = vec2(v_coords.x, mix(v_coords.y, 1.0 - v_coords.y, flip));
    vec2 extent = rect.zw * 0.5;
    float r = min(radius, min(extent.x, extent.y));
    vec2 q = abs(t * out_size - rect.xy - extent) - (extent - vec2(r));
    float d = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
    gl_FragColor = texture2D(tex, v_coords) * (clamp(0.5 - d, 0.0, 1.0) * alpha);
}"#;

/// The flat plate of a material region that shows no blur (an inactive window's sidebar and toolbar): the recipe's
/// opaque colour (premultiplied, so the anti-aliased rim of a rounded corner blends) in a rounded rectangle. A pixel
/// shader of the element's own size: `v_coords` runs 0..1 over it and `size` is its size in logical px.
const PLATE: &str = r#"
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform vec4 color;
uniform float radius;
void main() {
    vec2 extent = size * 0.5;
    float r = min(radius, min(extent.x, extent.y));
    vec2 q = abs(v_coords * size - extent) - (extent - vec2(r));
    float d = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
    gl_FragColor = color * (clamp(0.5 - d, 0.0, 1.0) * alpha);
}"#;

/// One flat plate to draw under a surface: its rectangle on the output (logical px), its premultiplied colour and the
/// radius of its corners (0 for square ones).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plate {
    pub rect: Rectangle<i32, Logical>,
    pub color: [f32; 4],
    pub radius: f32,
}

/// One blur region on the output (logical px) and the radius of its corners (0 for square ones).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub rect: Rectangle<i32, Logical>,
    pub radius: f32,
}

/// One region's part of the blurred backdrop, drawn through the `MASK` shader. Its id is kept per surface and region
/// index between frames (`Entry::ids`), so an unchanged backdrop adds no damage, unlike smithay's
/// `TextureShaderElement`, which takes a new id each time it is made.
pub struct GlassElement {
    inner: TextureRenderElement<GlesTexture>,
    texture: GlesTexture,
    program: GlesTexProgram,
    id: Id,
    uniforms: Vec<Uniform<'static>>,
}

impl Element for GlassElement {
    fn id(&self) -> &Id {
        &self.id
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }
    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // the rounded corners are see-through
        OpaqueRegions::default()
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }
}

impl RenderElement<GlesRenderer> for GlassElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.render_texture_from_to(&self.texture, src, dst, damage, opaque_regions, self.transform(), self.alpha(), Some(&self.program), &self.uniforms)
    }
}

/// A plate's element, kept between frames so its id stays, and the colour and radius it was last given (as bits).
struct PlateSlot {
    element: PixelShaderElement,
    look: ([u32; 4], u32),
}

/// The sizes of the blur pyramid for a `w` x `h` base: PASSES halvings, then PASSES-1 doublings back up.
pub fn pyramid(w: i32, h: i32) -> Vec<(i32, i32)> {
    let mut down = Vec::with_capacity(PASSES);
    let (mut cw, mut ch) = (w, h);
    for _ in 0..PASSES {
        cw = (cw / 2).max(1);
        ch = (ch / 2).max(1);
        down.push((cw, ch));
    }
    let up: Vec<(i32, i32)> = down.iter().rev().skip(1).copied().collect();
    down.into_iter().chain(up).collect()
}

/// Where `rect` (output logical px) lies in the blurred texture of `tex` px covering an output of `out` logical px.
pub fn source_in_texture(rect: Rectangle<i32, Logical>, tex: (i32, i32), out: (i32, i32)) -> Rectangle<f64, Logical> {
    let (fx, fy) = (tex.0 as f64 / out.0.max(1) as f64, tex.1 as f64 / out.1.max(1) as f64);
    Rectangle::new(
        (rect.loc.x as f64 * fx, rect.loc.y as f64 * fy).into(),
        (rect.size.w as f64 * fx, rect.size.h as f64 * fy).into(),
    )
}

struct Entry {
    buffer: TextureRenderBuffer<GlesTexture>,
    /// The blurred texture the buffer holds, for the masked draw.
    texture: GlesTexture,
    /// One element id per region index, kept between frames.
    ids: Vec<Id>,
    /// What the blur was built from; while it is unchanged the cached texture is reused (WM-1), so the buffer's damage
    /// stays empty on an idle frame and no page flip is queued.
    sig: Signature,
    src_size: (i32, i32),
}

/// The identity of a backdrop: output size and every element below with its commit and place.
#[derive(PartialEq)]
struct Signature {
    output: (i32, i32),
    /// The recipe's saturation, as bits (a recipe change rebuilds the backdrop).
    saturation: u32,
    /// The recipe's tone, as bits.
    tone: [u32; 6],
    below: Vec<(Id, CommitCounter, Rectangle<i32, Physical>)>,
}

fn signature<E: Element>(output: Size<i32, Physical>, below: &[E], saturation: f32, tone: Tone) -> Signature {
    Signature {
        output: (output.w, output.h),
        saturation: saturation.to_bits(),
        tone: [
            tone.brightness.to_bits(),
            tone.luminance_amount.to_bits(),
            tone.luminance_values[0].to_bits(),
            tone.luminance_values[1].to_bits(),
            tone.luminance_values[2].to_bits(),
            tone.luminance_values[3].to_bits(),
        ],
        below: below.iter().map(|e| (e.id().clone(), e.current_commit(), e.geometry(1.0f64.into()))).collect(),
    }
}

pub struct Glass {
    down: GlesTexProgram,
    up: GlesTexProgram,
    /// The rounded-rect mask over each region's blur.
    mask: GlesTexProgram,
    /// The plate shader; `None` when it did not compile (the plates are then not drawn, and the log says so).
    plate: Option<GlesPixelProgram>,
    /// The last backdrop of each blurred surface (keyed by the caller), so its element keeps its id between frames.
    entries: HashMap<u64, Entry>,
    /// The plate elements of each surface, by the surface's key and the plate's index.
    plates: HashMap<(u64, usize), PlateSlot>,
    /// The plates asked for since the last `retain_plates`.
    plates_seen: Vec<(u64, usize)>,
}

impl Glass {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let down = renderer.compile_custom_texture_shader(DOWN, &[UniformName::new("halfpixel", UniformType::_2f)])?;
        let up = renderer.compile_custom_texture_shader(
            UP,
            &[
                UniformName::new("halfpixel", UniformType::_2f),
                UniformName::new("saturation", UniformType::_1f),
                UniformName::new("brightness", UniformType::_1f),
                UniformName::new("lum_amount", UniformType::_1f),
                UniformName::new("lum_values", UniformType::_4f),
            ],
        )?;
        let mask = renderer.compile_custom_texture_shader(
            MASK,
            &[
                UniformName::new("out_size", UniformType::_2f),
                UniformName::new("rect", UniformType::_4f),
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("flip", UniformType::_1f),
            ],
        )?;
        // a shader that fails to compile must not stop the window server: the plates are left out instead
        let plate = match renderer.compile_custom_pixel_shader(
            PLATE,
            &[UniformName::new("color", UniformType::_4f), UniformName::new("radius", UniformType::_1f)],
        ) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("bifrost-wm: plate shader: {e}");
                None
            }
        };
        Ok(Glass { down, up, mask, plate, entries: HashMap::new(), plates: HashMap::new(), plates_seen: Vec::new() })
    }

    /// One element per plate of surface `key`, to be drawn right under the surface (next to its `backdrop`). The
    /// elements keep their ids between frames and change their commit only when a plate's rectangle, colour or radius
    /// changes, so a frame in which nothing moved has no damage.
    pub fn plates(&mut self, key: u64, plates: &[Plate]) -> Vec<PixelShaderElement> {
        let Some(program) = &self.plate else { return Vec::new() };
        let mut out = Vec::with_capacity(plates.len());
        for (i, p) in plates.iter().enumerate() {
            let look = (p.color.map(f32::to_bits), p.radius.to_bits());
            let uniforms = || vec![Uniform::new("color", p.color), Uniform::new("radius", p.radius)];
            self.plates_seen.push((key, i));
            let slot = self.plates.entry((key, i)).or_insert_with(|| PlateSlot {
                element: PixelShaderElement::new(program.clone(), p.rect, None, 1.0, uniforms(), Kind::Unspecified),
                look,
            });
            slot.element.resize(p.rect, None);
            if slot.look != look {
                slot.element.update_uniforms(uniforms());
                slot.look = look;
            }
            out.push(slot.element.clone());
        }
        out
    }

    /// One element per plate for a one-off render (a window capture): fresh elements that touch none of the scanout's
    /// cached ones, so a capture cannot disturb the frame cache. Empty when the plate shader did not compile.
    pub fn plates_once(&self, plates: &[Plate]) -> Vec<PixelShaderElement> {
        let Some(program) = &self.plate else { return Vec::new() };
        plates
            .iter()
            .map(|p| {
                let uniforms = vec![Uniform::new("color", p.color), Uniform::new("radius", p.radius)];
                PixelShaderElement::new(program.clone(), p.rect, None, 1.0, uniforms, Kind::Unspecified)
            })
            .collect()
    }

    /// Forgets the plates that were not asked for since the last call (a surface that went away or lost a region).
    pub fn retain_plates(&mut self) {
        let seen = std::mem::take(&mut self.plates_seen);
        self.plates.retain(|k, _| seen.contains(k));
    }

    /// The blurred backdrop for surface `key` whose blur `regions` (output logical px) lie over `below` (front-to-back
    /// render elements under the surface). One element per region, masked to its rounded rectangle, to be drawn right
    /// under the surface.
    pub fn backdrop<E: RenderElement<GlesRenderer>>(
        &mut self,
        renderer: &mut GlesRenderer,
        key: u64,
        output: Size<i32, Physical>,
        below: &[E],
        regions: &[Region],
        saturation: f32,
        tone: Tone,
    ) -> Result<Vec<GlassElement>, GlesError> {
        if regions.is_empty() {
            self.entries.remove(&key);
            return Ok(Vec::new());
        }
        let (bw, bh) = ((output.w / DIV).max(2), (output.h / DIV).max(2));
        let sig = signature(output, below, saturation, tone);
        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.sig == sig {
                return Ok(Self::elements(&self.mask, entry, regions, output));
            }
        }
        // 1. what lies under the surface, at a quarter of the output size
        let mut base: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (bw, bh).into())?;
        {
            let mut fb = renderer.bind(&mut base)?;
            let size: Size<i32, Physical> = (bw, bh).into();
            let mut frame = renderer.render(&mut fb, size, Transform::Normal)?;
            let full = [Rectangle::from_size(size)];
            frame.clear(Color32F::new(0.0, 0.0, 0.0, 1.0), &full)?;
            draw_render_elements::<GlesRenderer, _, E>(&mut frame, 1.0 / DIV as f64, below, &full)?;
            let _ = frame.finish()?;
        }
        // 2. dual Kawase: down, then up with the saturation lift on the last pass
        let mut src = base;
        let mut src_size = (bw, bh);
        let sizes = pyramid(bw, bh);
        let last = sizes.len().saturating_sub(1);
        for (i, &(w, h)) in sizes.iter().enumerate() {
            let mut dst: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (w, h).into())?;
            let going_down = w < src_size.0 || h < src_size.1;
            {
                let mut fb = renderer.bind(&mut dst)?;
                let size: Size<i32, Physical> = (w, h).into();
                let mut frame = renderer.render(&mut fb, size, Transform::Normal)?;
                let full = [Rectangle::from_size(size)];
                let half = [0.5 / w as f32, 0.5 / h as f32];
                let src_rect: Rectangle<f64, Buffer> = Rectangle::from_size((src_size.0 as f64, src_size.1 as f64).into());
                let sat = if i == last { saturation } else { 1.0 };
                // the tone applies on the last pass only, with the saturation
                let t = if i == last { tone } else { Tone::NONE };
                let uniforms = [
                    Uniform::new("halfpixel", half),
                    Uniform::new("saturation", sat),
                    Uniform::new("brightness", t.brightness),
                    Uniform::new("lum_amount", t.luminance_amount),
                    Uniform::new("lum_values", t.luminance_values),
                ];
                let (program, uniforms) = if going_down { (&self.down, &uniforms[..1]) } else { (&self.up, &uniforms[..]) };
                frame.render_texture_from_to(&src, src_rect, full[0], &full, &[], Transform::Normal, 1.0, Some(program), uniforms)?;
                let _ = frame.finish()?;
            }
            src = dst;
            src_size = (w, h);
        }
        // 3. one element per region rectangle, sampling its part of the blurred texture
        match self.entries.get_mut(&key) {
            Some(entry) => {
                entry.buffer.update_from_texture(renderer, src.clone(), 1, Transform::Normal, None);
                entry.texture = src;
                entry.sig = sig;
                entry.src_size = src_size;
            }
            None => {
                let buffer = TextureRenderBuffer::from_texture(renderer, src.clone(), 1, Transform::Normal, None);
                self.entries.insert(key, Entry { buffer, texture: src, ids: Vec::new(), sig, src_size });
            }
        }
        let entry = self.entries.get_mut(&key).expect("inserted above");
        Ok(Self::elements(&self.mask, entry, regions, output))
    }

    fn elements(mask: &GlesTexProgram, entry: &mut Entry, regions: &[Region], output: Size<i32, Physical>) -> Vec<GlassElement> {
        while entry.ids.len() < regions.len() {
            entry.ids.push(Id::new());
        }
        let flip = if entry.texture.is_y_inverted() { 1.0f32 } else { 0.0 };
        regions
            .iter()
            .zip(&entry.ids)
            .map(|(region, id)| {
                let r = region.rect;
                let loc: Point<f64, Physical> = (r.loc.x as f64, r.loc.y as f64).into();
                let inner = TextureRenderElement::from_texture_render_buffer(
                    loc,
                    &entry.buffer,
                    None,
                    Some(source_in_texture(r, entry.src_size, (output.w, output.h))),
                    Some(r.size),
                    Kind::Unspecified,
                );
                let uniforms = vec![
                    Uniform::new("out_size", [output.w as f32, output.h as f32]),
                    Uniform::new("rect", [r.loc.x as f32, r.loc.y as f32, r.size.w as f32, r.size.h as f32]),
                    Uniform::new("radius", region.radius.max(0.0)),
                    Uniform::new("flip", flip),
                ];
                GlassElement { inner, texture: entry.texture.clone(), program: mask.clone(), id: id.clone(), uniforms }
            })
            .collect()
    }

    /// Forgets surfaces that no longer ask for blur.
    pub fn retain(&mut self, keys: &[u64]) {
        self.entries.retain(|k, _| keys.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pyramid_halves_then_doubles_back_short_of_the_base() {
        assert_eq!(pyramid(640, 360), vec![(320, 180), (160, 90), (80, 45), (160, 90), (320, 180)]);
    }

    #[test]
    fn a_tiny_base_never_reaches_zero() {
        assert!(pyramid(4, 3).iter().all(|&(w, h)| w >= 1 && h >= 1));
    }

    #[test]
    fn a_region_maps_to_the_same_fraction_of_the_texture() {
        // a 400x300 sidebar at (100, 50) on a 2560x1440 output, blurred into a 320x180 texture
        let src = source_in_texture(Rectangle::new((100, 50).into(), (400, 300).into()), (320, 180), (2560, 1440));
        assert_eq!((src.loc.x, src.loc.y, src.size.w, src.size.h), (12.5, 6.25, 50.0, 37.5));
    }
}
