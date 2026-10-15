//! Sprite batching model: quads are appended to one vertex list, and consecutive quads that sample the same
//! texture share a run, which the caller draws with one call. Solid quads sample no texture and join the run
//! they follow. A batch holds at most `MAX_QUADS` quads; the caller flushes it before pushing more.

/// Vertices in one quad: two triangles.
pub const VERTICES_PER_QUAD: usize = 6;
/// Quads one batch holds before the caller must flush it.
pub const MAX_QUADS: usize = 4096;

/// One vertex of a sprite: a position in points, a texture coordinate and a straight (non-premultiplied) colour.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// A span of vertices drawn with one texture binding (`None` for solid quads only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub texture: Option<u32>,
    pub start: usize,
    pub count: usize,
}

/// Why a quad was not added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuadError {
    /// The area has a zero or negative width or height, so it covers no pixel.
    EmptyArea,
    /// The batch already holds `MAX_QUADS` quads; flush it first.
    Full,
}

impl std::fmt::Display for QuadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuadError::EmptyArea => write!(f, "a sprite needs a width and height above zero"),
            QuadError::Full => write!(f, "the sprite batch is full; flush it before adding more"),
        }
    }
}

impl std::error::Error for QuadError {}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpriteBatch {
    verts: Vec<Vertex>,
    runs: Vec<Run>,
}

impl SpriteBatch {
    pub fn new() -> Self {
        SpriteBatch::default()
    }

    /// Empties the batch for the next frame.
    pub fn clear(&mut self) {
        self.verts.clear();
        self.runs.clear();
    }

    pub fn quads(&self) -> usize {
        self.verts.len() / VERTICES_PER_QUAD
    }

    pub fn is_full(&self) -> bool {
        self.quads() >= MAX_QUADS
    }

    pub fn vertices(&self) -> &[Vertex] {
        &self.verts
    }

    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Adds a quad covering `area` (`[x, y, width, height]`), sampling `uv` (`[u0, v0, u1, v1]`) from `texture`.
    pub fn push_quad(&mut self, area: [f32; 4], uv: [f32; 4], color: [f32; 4], texture: Option<u32>) -> Result<(), QuadError> {
        let [x, y, w, h] = area;
        if w <= 0.0 || h <= 0.0 {
            return Err(QuadError::EmptyArea);
        }
        if self.is_full() {
            return Err(QuadError::Full);
        }
        let corner = |px: f32, py: f32, u: f32, v: f32| Vertex { pos: [px, py], uv: [u, v], color };
        let tl = corner(x, y, uv[0], uv[1]);
        let tr = corner(x + w, y, uv[2], uv[1]);
        let bl = corner(x, y + h, uv[0], uv[3]);
        let br = corner(x + w, y + h, uv[2], uv[3]);
        self.append(texture, [tl, tr, bl, tr, br, bl]);
        Ok(())
    }

    fn append(&mut self, texture: Option<u32>, quad: [Vertex; VERTICES_PER_QUAD]) {
        let needs_run = match (self.runs.last(), texture) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(last), Some(t)) => last.texture.is_some() && last.texture != Some(t),
        };
        let start = self.verts.len();
        if needs_run {
            self.runs.push(Run { texture, start, count: 0 });
        } else if let (Some(last), Some(t)) = (self.runs.last_mut(), texture) {
            // a run that began with solid quads adopts the first texture it meets
            if last.texture.is_none() {
                last.texture = Some(t);
            }
        }
        self.verts.extend_from_slice(&quad);
        if let Some(last) = self.runs.last_mut() {
            last.count += VERTICES_PER_QUAD;
        }
    }
}
