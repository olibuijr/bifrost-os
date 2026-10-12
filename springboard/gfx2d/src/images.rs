//! Image texture cache keyed by `ImageRef::id`, with byte-budget LRU eviction (default 64 MiB).
//!
//! Images are premultiplied RGBA8 (`bifrost_render::gfx::Image`, one little-endian `u32` per pixel with R in the low
//! byte), which is byte for byte the `Rgba8Unorm` layout, so an upload is a repack into little-endian bytes and one
//! `replace_region`. The LRU bookkeeping (`LruBook`) is pure and carries no GPU state; the cache drops a texture
//! handle when its entry is evicted, and the GPU memory goes with the last handle.
//!
//! Owner: wave G1 agent `gfx2d-images`.

use std::collections::{BTreeMap, HashMap};

use bifrost_gpu::{Device, Error, PixelFormat, Region, Result, Texture, TextureDescriptor, TextureUsage};
use bifrost_render::canvas::ImageRef;
use bifrost_render::gfx::Image;

/// The default byte budget of the image cache (64 MiB of decoded pixels).
pub const DEFAULT_BUDGET_BYTES: usize = 64 << 20;

/// Byte-budget LRU bookkeeping: which keys are resident, how many bytes each takes, and what to evict. Pure logic,
/// no GPU handles. Recency is a tick counter; `order` maps tick to key so the least recent entry is the first one.
#[derive(Debug, Clone)]
pub(crate) struct LruBook {
    budget: usize,
    used: usize,
    tick: u64,
    /// key -> (tick of last use, bytes)
    entries: HashMap<u64, (u64, usize)>,
    /// tick of last use -> key; the first entry is the least recently used.
    order: BTreeMap<u64, u64>,
}

impl LruBook {
    pub(crate) fn new(budget: usize) -> LruBook {
        LruBook { budget, used: 0, tick: 0, entries: HashMap::new(), order: BTreeMap::new() }
    }

    /// Marks `key` as the most recently used. Returns false when it is not resident.
    pub(crate) fn touch(&mut self, key: u64) -> bool {
        let Some(&(old, bytes)) = self.entries.get(&key) else {
            return false;
        };
        self.order.remove(&old);
        self.tick += 1;
        self.entries.insert(key, (self.tick, bytes));
        self.order.insert(self.tick, key);
        true
    }

    /// Makes `key` resident with `bytes` (replacing an older size of the same key) as the most recently used entry,
    /// then evicts the least recently used entries until the total fits the budget. The entry just inserted is never
    /// evicted, so one image larger than the whole budget stays alone. Returns the evicted keys, oldest first.
    pub(crate) fn insert(&mut self, key: u64, bytes: usize) -> Vec<u64> {
        self.remove(key);
        self.tick += 1;
        self.entries.insert(key, (self.tick, bytes));
        self.order.insert(self.tick, key);
        self.used += bytes;
        let mut evicted = Vec::new();
        while self.used > self.budget {
            let Some((tick, victim)) = self.order.iter().next().map(|(&t, &k)| (t, k)) else {
                break;
            };
            if victim == key {
                break;
            }
            self.order.remove(&tick);
            if let Some((_, b)) = self.entries.remove(&victim) {
                self.used -= b;
            }
            evicted.push(victim);
        }
        evicted
    }

    /// Forgets `key`, returning the bytes it held.
    pub(crate) fn remove(&mut self, key: u64) -> Option<usize> {
        let (tick, bytes) = self.entries.remove(&key)?;
        self.order.remove(&tick);
        self.used -= bytes;
        Some(bytes)
    }

    pub(crate) fn contains(&self, key: u64) -> bool {
        self.entries.contains_key(&key)
    }

    pub(crate) fn used(&self) -> usize {
        self.used
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Texture cache for `ImageRef`s. One per renderer.
pub struct ImageCache {
    pub(crate) device: Device,
    pub(crate) budget: usize,
    book: LruBook,
    textures: HashMap<u64, Texture>,
}

impl ImageCache {
    /// A cache with `budget_bytes` of texture memory. A zero budget is refused.
    pub fn new(device: &Device, budget_bytes: usize) -> Result<ImageCache> {
        check_budget(budget_bytes)?;
        Ok(ImageCache { device: device.clone(), budget: budget_bytes, book: LruBook::new(budget_bytes), textures: HashMap::new() })
    }

    /// The texture for `image`, uploading it on first use (or when its id now names an image of another size). A
    /// resident image is marked as used. The caller must use a new id when the pixels change; `update` refreshes a
    /// region of an image under the same id.
    pub fn get(&mut self, image: &ImageRef) -> Result<Texture> {
        let (w, h) = check_image(image.image)?;
        if let Some(tex) = self.textures.get(&image.id) {
            if tex.width() == w && tex.height() == h {
                self.book.touch(image.id);
                return Ok(tex.clone());
            }
        }
        self.forget(image.id);
        let tex = self.device.new_texture(&TextureDescriptor {
            width: w,
            height: h,
            format: PixelFormat::Rgba8Unorm,
            usage: TextureUsage::SAMPLED | TextureUsage::COPY_DST,
            exportable: false,
            modifiers: Vec::new(),
        })?;
        let full = Region { x: 0, y: 0, width: w, height: h };
        let bytes = pixel_bytes(&image.image.px, w as usize, full);
        tex.replace_region(full, &bytes, w as usize * 4)?;
        let cost = w as usize * h as usize * 4;
        for victim in self.book.insert(image.id, cost) {
            self.textures.remove(&victim);
        }
        self.textures.insert(image.id, tex.clone());
        Ok(tex)
    }

    /// Uploads only `region` of `image` into its resident texture (through `replace_region`). When the image is not
    /// resident yet, the whole image is uploaded instead.
    pub fn update(&mut self, image: &ImageRef, region: Region) -> Result<()> {
        let (w, h) = check_image(image.image)?;
        let resident = matches!(self.textures.get(&image.id), Some(t) if t.width() == w && t.height() == h);
        if !resident {
            self.get(image)?;
            return Ok(());
        }
        check_region(region, w, h)?;
        let bytes = pixel_bytes(&image.image.px, w as usize, region);
        if let Some(tex) = self.textures.get(&image.id) {
            tex.replace_region(region, &bytes, region.width as usize * 4)?;
        }
        self.book.touch(image.id);
        Ok(())
    }

    /// Drops the texture of `id` now. Returns whether it was resident.
    pub fn remove(&mut self, id: u64) -> bool {
        self.forget(id)
    }

    pub fn contains(&self, id: u64) -> bool {
        self.book.contains(id)
    }

    /// Bytes of texture memory the resident images take (by their RGBA8 size).
    pub fn used_bytes(&self) -> usize {
        self.book.used()
    }

    pub fn len(&self) -> usize {
        self.book.len()
    }

    pub fn is_empty(&self) -> bool {
        self.book.len() == 0
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    fn forget(&mut self, id: u64) -> bool {
        self.textures.remove(&id);
        self.book.remove(id).is_some()
    }
}

/// Rejects a zero budget (nothing could ever be cached).
pub(crate) fn check_budget(budget_bytes: usize) -> Result<()> {
    if budget_bytes == 0 {
        return Err(Error::Invalid("image cache budget is zero".into()));
    }
    Ok(())
}

/// The size of a usable image: non-empty, its pixel count matching `px`. Returns the dimensions as texture sizes.
pub(crate) fn check_image(image: &Image) -> Result<(u32, u32)> {
    if image.w == 0 || image.h == 0 {
        return Err(Error::Invalid("image has no pixels".into()));
    }
    let (w, h) = (u32::try_from(image.w), u32::try_from(image.h));
    let (Ok(w), Ok(h)) = (w, h) else {
        return Err(Error::Invalid("image is larger than a texture".into()));
    };
    if image.w.checked_mul(image.h) != Some(image.px.len()) {
        return Err(Error::Invalid(format!("image pixel count {} does not match {}x{}", image.px.len(), image.w, image.h)));
    }
    Ok((w, h))
}

/// A region must be non-empty and lie inside a `w` x `h` image.
pub(crate) fn check_region(r: Region, w: u32, h: u32) -> Result<()> {
    let right = r.x as u64 + r.width as u64;
    let bottom = r.y as u64 + r.height as u64;
    if r.width == 0 || r.height == 0 || right > w as u64 || bottom > h as u64 {
        return Err(Error::Invalid(format!("region {:?} is outside the {w}x{h} image", r)));
    }
    Ok(())
}

/// Packs `region` of premultiplied pixels (rows `stride_px` pixels apart) into tightly packed RGBA8 bytes. Each
/// `u32` is little endian with R in the low byte, so the bytes come out R, G, B, A.
pub(crate) fn pixel_bytes(px: &[u32], stride_px: usize, region: Region) -> Vec<u8> {
    let mut out = Vec::with_capacity(region.width as usize * region.height as usize * 4);
    for row in region.y..region.y + region.height {
        let base = row as usize * stride_px;
        for col in region.x..region.x + region.width {
            out.extend_from_slice(&px[base + col as usize].to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(x: u32, y: u32, width: u32, height: u32) -> Region {
        Region { x, y, width, height }
    }

    #[test]
    fn lru_keeps_everything_under_budget() {
        let mut b = LruBook::new(100);
        assert!(b.insert(1, 30).is_empty());
        assert!(b.insert(2, 30).is_empty());
        assert!(b.insert(3, 40).is_empty());
        assert_eq!(b.used(), 100);
        assert_eq!(b.len(), 3);
        assert!(b.contains(1) && b.contains(2) && b.contains(3));
    }

    #[test]
    fn lru_evicts_least_recently_used_first() {
        let mut b = LruBook::new(10);
        b.insert(1, 4);
        b.insert(2, 4);
        assert_eq!(b.used(), 8);
        let evicted = b.insert(3, 4);
        assert_eq!(evicted, vec![1]);
        assert_eq!(b.used(), 8);
        assert!(!b.contains(1) && b.contains(2) && b.contains(3));
    }

    #[test]
    fn touch_protects_an_entry_from_eviction() {
        let mut b = LruBook::new(10);
        b.insert(1, 4);
        b.insert(2, 4);
        assert!(b.touch(1));
        let evicted = b.insert(3, 4);
        assert_eq!(evicted, vec![2]);
        assert!(b.contains(1) && b.contains(3));
    }

    #[test]
    fn touch_of_a_missing_key_is_false_and_changes_nothing() {
        let mut b = LruBook::new(10);
        b.insert(1, 4);
        assert!(!b.touch(9));
        assert_eq!(b.used(), 4);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn reinserting_a_key_replaces_its_size_without_double_counting() {
        let mut b = LruBook::new(100);
        b.insert(1, 30);
        b.insert(1, 50);
        assert_eq!(b.used(), 50);
        assert_eq!(b.len(), 1);
        assert_eq!(b.remove(1), Some(50));
        assert_eq!(b.used(), 0);
    }

    #[test]
    fn oversized_entry_stays_alone_and_evicts_the_rest() {
        let mut b = LruBook::new(10);
        b.insert(1, 4);
        let evicted = b.insert(2, 20);
        assert_eq!(evicted, vec![1]);
        assert!(b.contains(2));
        assert_eq!(b.used(), 20);
        // The next insert pushes the oversized one out, since it is now the least recent.
        let evicted = b.insert(3, 4);
        assert_eq!(evicted, vec![2]);
        assert_eq!(b.used(), 4);
    }

    #[test]
    fn remove_frees_its_bytes() {
        let mut b = LruBook::new(10);
        b.insert(1, 6);
        assert_eq!(b.remove(1), Some(6));
        assert_eq!(b.remove(1), None);
        assert_eq!(b.used(), 0);
        assert!(b.insert(2, 10).is_empty());
    }

    #[test]
    fn pixel_bytes_is_rgba_little_endian() {
        let bytes = pixel_bytes(&[0xAABB_CCDD], 1, region(0, 0, 1, 1));
        assert_eq!(bytes, vec![0xDD, 0xCC, 0xBB, 0xAA]);
    }

    #[test]
    fn pixel_bytes_copies_only_the_region_row_by_row() {
        // 3x3 image, pixel value = y * 3 + x.
        let px: Vec<u32> = (0..9).collect();
        let bytes = pixel_bytes(&px, 3, region(1, 1, 2, 2));
        let got: Vec<u32> = bytes.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        assert_eq!(got, vec![4, 5, 7, 8]);
        assert_eq!(bytes.len(), 2 * 2 * 4);
    }

    #[test]
    fn check_image_accepts_matching_pixel_count_and_rejects_the_rest() {
        let ok = Image { w: 2, h: 2, px: vec![0; 4] };
        assert_eq!(check_image(&ok).unwrap(), (2, 2));
        let short = Image { w: 2, h: 2, px: vec![0; 3] };
        assert!(check_image(&short).is_err());
        let empty = Image { w: 0, h: 4, px: vec![] };
        assert!(check_image(&empty).is_err());
    }

    #[test]
    fn check_region_rejects_empty_and_out_of_bounds() {
        assert!(check_region(region(0, 0, 4, 4), 4, 4).is_ok());
        assert!(check_region(region(1, 0, 4, 4), 4, 4).is_err());
        assert!(check_region(region(0, 0, 0, 1), 4, 4).is_err());
        assert!(check_region(region(u32::MAX, 0, 2, 1), 4, 4).is_err());
    }

    #[test]
    fn zero_budget_is_refused() {
        assert!(check_budget(0).is_err());
        assert!(check_budget(DEFAULT_BUDGET_BYTES).is_ok());
    }
}
