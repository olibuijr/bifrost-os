//! Texture bookkeeping that needs no GPU: id allocation, size tracking,
//! upload validation and source-rectangle to UV conversion.

use super::TextureId;
use crate::gfx::Image;
use crate::layout::Rect;
use std::collections::HashMap;

/// GLES guarantees at least 8192; refusing more keeps uploads sane.
pub const MAX_DIM: usize = 8192;

#[derive(Debug, PartialEq, Eq)]
pub enum UploadError {
    Empty,
    TooLarge,
    /// pixel buffer does not hold w*h pixels
    Mismatch,
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadError::Empty => write!(f, "image has no pixels"),
            UploadError::TooLarge => write!(f, "image exceeds {MAX_DIM} px"),
            UploadError::Mismatch => write!(f, "pixel buffer does not match the image size"),
        }
    }
}

/// `Image` is premultiplied RGBA, R in the low byte of each u32: byte for byte
/// GL's RGBA / UNSIGNED_BYTE, so the pixels upload without conversion.
pub fn validate(img: &Image) -> Result<(), UploadError> {
    if img.w == 0 || img.h == 0 {
        return Err(UploadError::Empty);
    }
    if img.w > MAX_DIM || img.h > MAX_DIM {
        return Err(UploadError::TooLarge);
    }
    if img.px.len() != img.w * img.h {
        return Err(UploadError::Mismatch);
    }
    Ok(())
}

/// `[u0, v0, u1, v1]` of `src` (image pixels) inside a `w` x `h` texture.
pub fn uv(src: Rect, w: usize, h: usize) -> [f32; 4] {
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    [src.x as f32 / w, src.y as f32 / h, (src.x + src.w) as f32 / w, (src.y + src.h) as f32 / h]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// GL texture name
    pub gl: u32,
    pub w: usize,
    pub h: usize,
}

#[derive(Default)]
pub struct Textures {
    next: u32,
    map: HashMap<u32, Entry>,
}

impl Textures {
    pub fn new() -> Self {
        Textures::default()
    }

    pub fn insert(&mut self, entry: Entry) -> TextureId {
        self.next += 1;
        self.map.insert(self.next, entry);
        TextureId(self.next)
    }

    pub fn get(&self, id: TextureId) -> Option<Entry> {
        self.map.get(&id.0).copied()
    }

    /// Forgets `id`; None for an unknown or already freed id.
    pub fn remove(&mut self, id: TextureId) -> Option<Entry> {
        self.map.remove(&id.0)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Bytes of GPU memory held (RGBA8, no mipmaps).
    pub fn bytes(&self) -> usize {
        self.map.values().map(|e| e.w * e.h * 4).sum()
    }

    /// Every live entry, for teardown.
    pub fn drain(&mut self) -> Vec<Entry> {
        self.map.drain().map(|(_, e)| e).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize) -> Image {
        Image { w, h, px: vec![0; w * h] }
    }

    #[test]
    fn validate_accepts_good_images() {
        assert_eq!(validate(&img(4, 3)), Ok(()));
        assert_eq!(validate(&img(MAX_DIM, 1)), Ok(()));
    }

    #[test]
    fn validate_rejects_bad_images() {
        assert_eq!(validate(&img(0, 3)), Err(UploadError::Empty));
        assert_eq!(validate(&img(3, 0)), Err(UploadError::Empty));
        assert_eq!(validate(&img(MAX_DIM + 1, 1)), Err(UploadError::TooLarge));
        assert_eq!(validate(&Image { w: 4, h: 4, px: vec![0; 15] }), Err(UploadError::Mismatch));
    }

    #[test]
    fn uv_normalises_the_source_rect() {
        assert_eq!(uv(Rect { x: 0, y: 0, w: 100, h: 50 }, 100, 50), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(uv(Rect { x: 25, y: 10, w: 50, h: 20 }, 100, 40), [0.25, 0.25, 0.75, 0.75]);
        assert_eq!(uv(Rect { x: 0, y: 0, w: 1, h: 1 }, 0, 0), [0.0, 0.0, 1.0, 1.0], "no division by zero");
    }

    #[test]
    fn ids_are_unique_and_never_reused() {
        let mut t = Textures::new();
        let a = t.insert(Entry { gl: 7, w: 2, h: 2 });
        let b = t.insert(Entry { gl: 8, w: 2, h: 2 });
        assert_ne!(a, b);
        t.remove(a);
        let c = t.insert(Entry { gl: 9, w: 2, h: 2 });
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn remove_is_explicit_and_idempotent() {
        let mut t = Textures::new();
        let a = t.insert(Entry { gl: 7, w: 10, h: 10 });
        assert_eq!(t.get(a).map(|e| e.gl), Some(7));
        assert_eq!(t.remove(a).map(|e| e.gl), Some(7));
        assert_eq!(t.remove(a), None);
        assert_eq!(t.get(a), None);
        assert!(t.is_empty());
    }

    #[test]
    fn bytes_and_drain() {
        let mut t = Textures::new();
        t.insert(Entry { gl: 1, w: 10, h: 10 });
        t.insert(Entry { gl: 2, w: 5, h: 2 });
        assert_eq!(t.len(), 2);
        assert_eq!(t.bytes(), 440);
        assert_eq!(t.drain().len(), 2);
        assert_eq!(t.bytes(), 0);
    }
}
