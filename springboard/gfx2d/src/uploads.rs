//! Batched CPU-to-texture uploads. Glyphs, images and path masks queue their pixels here while a frame is walked;
//! `flush` writes them all into ONE staging buffer and records the copies into the frame's command buffer, so a
//! frame with 60 new glyphs is one buffer and zero GPU stalls (the old per-call path allocated a staging buffer,
//! a pool and a fence and waited for each one).

use bifrost_gpu::{BufferUsage, CommandBuffer, Device, Error, Region, Result, Texture};

struct Item {
    tex: Texture,
    region: Region,
    /// Offset of the tightly packed rows in `Uploads::data`.
    offset: usize,
    /// Bytes per row of the packed data.
    stride: usize,
}

/// A queue of pending texture uploads.
#[derive(Default)]
pub struct Uploads {
    items: Vec<Item>,
    data: Vec<u8>,
}

impl Uploads {
    pub fn new() -> Uploads {
        Uploads::default()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Queues `bytes` (rows `stride` bytes apart, `bpp` bytes per pixel) for `region` of `tex`. The rows are copied
    /// tightly packed, so the caller's buffer need not outlive the call.
    pub fn push(&mut self, tex: &Texture, region: Region, bytes: &[u8], stride: usize, bpp: usize) -> Result<()> {
        let row = (region.width as usize).checked_mul(bpp).ok_or_else(|| Error::Invalid("upload row overflows".into()))?;
        let rows = region.height as usize;
        if rows == 0 || row == 0 {
            return Err(Error::Invalid("empty upload region".into()));
        }
        let needed = stride.checked_mul(rows - 1).and_then(|n| n.checked_add(row)).ok_or_else(|| Error::Invalid("upload range overflows".into()))?;
        if stride < row || bytes.len() < needed {
            return Err(Error::Invalid(format!("upload needs {needed} bytes with rows of {row}, got {} with stride {stride}", bytes.len())));
        }
        // Offsets stay 4-byte aligned for the copy.
        let offset = (self.data.len() + 3) & !3;
        self.data.resize(offset, 0);
        for y in 0..rows {
            self.data.extend_from_slice(&bytes[y * stride..y * stride + row]);
        }
        self.items.push(Item { tex: tex.clone(), region, offset, stride: row });
        Ok(())
    }

    /// Moves everything queued in `other` to the end of this queue.
    pub fn append(&mut self, other: &mut Uploads) {
        let base = (self.data.len() + 3) & !3;
        self.data.resize(base, 0);
        self.data.extend_from_slice(&other.data);
        for mut it in other.items.drain(..) {
            it.offset += base;
            self.items.push(it);
        }
        other.data.clear();
    }

    /// Writes the queued pixels into one staging buffer and records the copies into `cb`. Call it outside a render
    /// pass. Returns how many copies were recorded; the queue is empty afterwards.
    pub fn flush(&mut self, device: &Device, cb: &mut CommandBuffer) -> Result<usize> {
        if self.items.is_empty() {
            return Ok(0);
        }
        let staging = device.new_buffer(self.data.len(), BufferUsage::STAGING)?;
        staging.write(0, &self.data)?;
        let n = self.items.len();
        let result = self.items.iter().try_for_each(|it| cb.copy_buffer_to_texture(&staging, it.offset, it.stride, &it.tex, it.region));
        self.items.clear();
        self.data.clear();
        result.map(|()| n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_queue_reports_so() {
        let u = Uploads::new();
        assert!(u.is_empty());
        assert_eq!(u.len(), 0);
        assert_eq!((5usize + 3) & !3, 8);
    }
}
