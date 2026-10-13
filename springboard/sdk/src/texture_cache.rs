//! Texture cache for pixel buffers (the bookkeeping of Core Video's `CVOpenGLESTextureCache` and
//! `CVOpenGLESTextureCacheCreateTextureFromImage`, `CVOpenGLESTextureCacheFlush`, `CVOpenGLTextureGetName`).
//!
//! A [`TextureCache`] maps a caller-chosen key (a window, a layer, a decoder slot) to the GPU texture made from a
//! [`PixelBuffer`]. The texture is reused while the buffer's [`PixelBuffer::revision`] and its format and size are
//! unchanged. When the pixels change, or the buffer is replaced by another of a different shape, the cache asks the
//! caller for a new texture and reports the name of the one it replaced, so the caller can delete it.
//!
//! The cache holds no GPU handles itself. The caller passes a `make` function that creates the texture and returns
//! its name (for OpenGL, the texture object name). Nothing here reads pixels, so the model can be tested on the host.

use std::collections::HashMap;

use bifrost_render::pixel_format::FourCc;

use crate::pixel_buffer::PixelBuffer;

/// A texture made from a pixel buffer. `name` is the value the caller's `make` function returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureRef {
    pub name: u32,
    pub code: FourCc,
    pub width: usize,
    pub height: usize,
    /// The buffer revision the texture was made from.
    pub revision: u64,
}

/// The result of a lookup that may have made a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureUpdate {
    pub texture: TextureRef,
    /// The name of the texture this call replaced for the same key, which the caller should delete.
    pub replaced: Option<u32>,
}

/// Why a texture could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureError {
    /// The caller's `make` function returned no texture.
    CreateFailed,
}

#[derive(Debug, Default)]
pub struct TextureCache {
    entries: HashMap<u64, TextureRef>,
    hits: u64,
    misses: u64,
}

impl TextureCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The texture for `key` and `buffer`. A cached texture is returned as is when it was made from the same
    /// revision and has the same format and size. Otherwise `make` creates one; a texture it replaced for this key is
    /// reported in [`TextureUpdate::replaced`].
    pub fn texture_for(
        &mut self,
        key: u64,
        buffer: &PixelBuffer,
        make: impl FnOnce(&PixelBuffer) -> Option<u32>,
    ) -> Result<TextureUpdate, TextureError> {
        let (code, width, height, revision) = (buffer.code(), buffer.width(), buffer.height(), buffer.revision());
        if let Some(entry) = self.entries.get(&key) {
            if entry.revision == revision && entry.code == code && entry.width == width && entry.height == height {
                self.hits += 1;
                return Ok(TextureUpdate { texture: *entry, replaced: None });
            }
        }
        let name = make(buffer).ok_or(TextureError::CreateFailed)?;
        self.misses += 1;
        let texture = TextureRef { name, code, width, height, revision };
        let replaced = self.entries.insert(key, texture).map(|old| old.name);
        Ok(TextureUpdate { texture, replaced })
    }

    /// Forgets the texture for `key`, returning it so the caller can delete it.
    pub fn remove(&mut self, key: u64) -> Option<TextureRef> {
        self.entries.remove(&key)
    }

    /// Forgets every texture (`CVOpenGLESTextureCacheFlush`). Returns the names so the caller can delete them.
    pub fn flush(&mut self) -> Vec<u32> {
        let mut names: Vec<u32> = self.entries.drain().map(|(_, t)| t.name).collect();
        names.sort_unstable();
        names
    }

    /// Textures currently held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no texture is held.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Lookups answered from the cache.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Lookups that made a texture.
    pub fn misses(&self) -> u64 {
        self.misses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(s: &str) -> FourCc {
        FourCc::parse(s).unwrap()
    }

    fn buf(fmt: &str, w: usize, h: usize) -> PixelBuffer {
        PixelBuffer::new(code(fmt), w, h).unwrap()
    }

    fn make_counting(counter: &mut u32) -> impl FnMut(&PixelBuffer) -> Option<u32> + '_ {
        move |_| {
            *counter += 1;
            Some(*counter)
        }
    }

    #[test]
    fn same_revision_reuses_the_texture() {
        let mut cache = TextureCache::new();
        let mut made = 0;
        let b = buf("AR24", 4, 4);
        let first = cache.texture_for(1, &b, make_counting(&mut made)).unwrap();
        let second = cache.texture_for(1, &b, make_counting(&mut made)).unwrap();
        assert_eq!(made, 1);
        assert_eq!(first.texture, second.texture);
        assert_eq!(second.replaced, None);
        assert_eq!((cache.hits(), cache.misses()), (1, 1));
    }

    #[test]
    fn a_write_under_a_read_write_lock_makes_a_new_texture() {
        let mut cache = TextureCache::new();
        let mut made = 0;
        let mut b = buf("AR24", 4, 4);
        let first = cache.texture_for(7, &b, make_counting(&mut made)).unwrap();
        b.lock(false);
        b.base_address_mut(0).unwrap()[0] = 1;
        b.unlock(false).unwrap();
        let second = cache.texture_for(7, &b, make_counting(&mut made)).unwrap();
        assert_eq!(made, 2);
        assert_eq!(second.replaced, Some(first.texture.name));
        assert_eq!(second.texture.revision, 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_different_shape_under_the_same_key_replaces_the_texture() {
        let mut cache = TextureCache::new();
        let mut made = 0;
        cache.texture_for(3, &buf("AR24", 4, 4), make_counting(&mut made)).unwrap();
        let update = cache.texture_for(3, &buf("AR24", 8, 4), make_counting(&mut made)).unwrap();
        assert_eq!(made, 2);
        assert_eq!((update.texture.width, update.texture.height), (8, 4));
        assert!(update.replaced.is_some());
    }

    #[test]
    fn keys_are_independent() {
        let mut cache = TextureCache::new();
        let mut made = 0;
        let b = buf("NV12", 6, 4);
        let a = cache.texture_for(1, &b, make_counting(&mut made)).unwrap();
        let c = cache.texture_for(2, &b, make_counting(&mut made)).unwrap();
        assert_ne!(a.texture.name, c.texture.name);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn a_refused_texture_leaves_the_cache_unchanged() {
        let mut cache = TextureCache::new();
        let b = buf("AR24", 2, 2);
        assert_eq!(cache.texture_for(9, &b, |_| None), Err(TextureError::CreateFailed));
        assert!(cache.is_empty());
        assert_eq!(cache.misses(), 0);
    }

    #[test]
    fn remove_and_flush_return_the_names_to_delete() {
        let mut cache = TextureCache::new();
        let mut made = 0;
        let b = buf("AR24", 2, 2);
        cache.texture_for(1, &b, make_counting(&mut made)).unwrap();
        cache.texture_for(2, &b, make_counting(&mut made)).unwrap();
        cache.texture_for(3, &b, make_counting(&mut made)).unwrap();
        assert_eq!(cache.remove(2).map(|t| t.name), Some(2));
        assert_eq!(cache.remove(2), None);
        assert_eq!(cache.flush(), vec![1, 3]);
        assert!(cache.is_empty());
        assert_eq!(cache.flush(), Vec::<u32>::new());
    }
}
