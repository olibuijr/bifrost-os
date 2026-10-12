//! Live glass: the blurred backdrop behind a surface's `ext-background-effect-v1` region, built every frame from what is
//! composited below it (wallpaper, windows, layers), the way a window server does it. The client draws its own tint
//! over the region with alpha. Dual-Kawase blur on a quarter-size copy, with a saturation lift like the shell's
//! materials (`bifrost_render::glass`).

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                texture::{TextureRenderBuffer, TextureRenderElement},
                Element, Id, Kind, RenderElement,
            },
            gles::{GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType},
            utils::{draw_render_elements, CommitCounter},
            Bind, Color32F, Frame, Offscreen, Renderer,
        },
    },
    utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform},
};

/// The backdrop is rendered at 1/DIV of the output, then halved PASSES times and doubled PASSES-1 times.
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
varying vec2 v_coords;
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
    gl_FragColor = c * alpha;
}"#;

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
    /// What the blur was built from; while it is unchanged the cached texture is reused (WM-1), so the buffer's damage
    /// stays empty on an idle frame and no page flip is queued.
    sig: Signature,
    src_size: (i32, i32),
}

/// The identity of a backdrop: output size and every element below with its commit and place.
#[derive(PartialEq)]
struct Signature {
    output: (i32, i32),
    below: Vec<(Id, CommitCounter, Rectangle<i32, Physical>)>,
}

fn signature<E: Element>(output: Size<i32, Physical>, below: &[E]) -> Signature {
    Signature {
        output: (output.w, output.h),
        below: below.iter().map(|e| (e.id().clone(), e.current_commit(), e.geometry(1.0f64.into()))).collect(),
    }
}

pub struct Glass {
    down: GlesTexProgram,
    up: GlesTexProgram,
    /// The last backdrop of each blurred surface (keyed by the caller), so its element keeps its id between frames.
    entries: HashMap<u64, Entry>,
}

impl Glass {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let down = renderer.compile_custom_texture_shader(DOWN, &[UniformName::new("halfpixel", UniformType::_2f)])?;
        let up = renderer.compile_custom_texture_shader(
            UP,
            &[UniformName::new("halfpixel", UniformType::_2f), UniformName::new("saturation", UniformType::_1f)],
        )?;
        Ok(Glass { down, up, entries: HashMap::new() })
    }

    /// The blurred backdrop for surface `key` whose blur `regions` (output logical px) lie over `below` (front-to-back
    /// render elements under the surface). One texture element per region rectangle, to be drawn right under the surface.
    pub fn backdrop<E: RenderElement<GlesRenderer>>(
        &mut self,
        renderer: &mut GlesRenderer,
        key: u64,
        output: Size<i32, Physical>,
        below: &[E],
        regions: &[Rectangle<i32, Logical>],
    ) -> Result<Vec<TextureRenderElement<GlesTexture>>, GlesError> {
        if regions.is_empty() {
            self.entries.remove(&key);
            return Ok(Vec::new());
        }
        let (bw, bh) = ((output.w / DIV).max(2), (output.h / DIV).max(2));
        let sig = signature(output, below);
        if let Some(entry) = self.entries.get(&key) {
            if entry.sig == sig {
                return Ok(Self::elements(entry, regions, entry.src_size, output));
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
                let sat = if i == last { SATURATION } else { 1.0 };
                let uniforms = [Uniform::new("halfpixel", half), Uniform::new("saturation", sat)];
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
                entry.buffer.update_from_texture(renderer, src, 1, Transform::Normal, None);
                entry.sig = sig;
                entry.src_size = src_size;
            }
            None => {
                let buffer = TextureRenderBuffer::from_texture(renderer, src, 1, Transform::Normal, None);
                self.entries.insert(key, Entry { buffer, sig, src_size });
            }
        }
        let entry = &self.entries[&key];
        Ok(Self::elements(entry, regions, src_size, output))
    }

    fn elements(
        entry: &Entry,
        regions: &[Rectangle<i32, Logical>],
        src_size: (i32, i32),
        output: Size<i32, Physical>,
    ) -> Vec<TextureRenderElement<GlesTexture>> {
        regions
            .iter()
            .map(|r| {
                let loc: Point<f64, Physical> = (r.loc.x as f64, r.loc.y as f64).into();
                TextureRenderElement::from_texture_render_buffer(
                    loc,
                    &entry.buffer,
                    None,
                    Some(source_in_texture(*r, src_size, (output.w, output.h))),
                    Some(r.size),
                    Kind::Unspecified,
                )
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
