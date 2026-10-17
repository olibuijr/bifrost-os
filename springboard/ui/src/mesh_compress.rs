//! Mesh compression: a lossy, compact byte format for `PrimitiveMesh` data, for assets that are stored or sent
//! (the job of the reference OS's geometry compression component).
//!
//! What it does, in order of the bytes it writes:
//! - positions are quantized to `position_bits` (8 to 16) inside the mesh's bounding box and coded as zigzag varint
//!   deltas from the previous vertex, so neighbouring vertices cost a byte or two instead of twelve;
//! - normals are octahedral-encoded into two signed bytes (about one degree of angle error);
//! - texture coordinates are quantized to 16 bits inside their own range and delta coded the same way;
//! - indices are zigzag varint deltas from the previous index, which is small for meshes whose triangles share
//!   nearby vertices; the index values come back exactly;
//! - a 32-bit FNV-1a checksum of everything before it, so a damaged file is refused instead of decoded into garbage.
//!
//! Positions are exact to within half a quantization step, which [`max_position_error`] reports. Normals are decoded
//! unit length. Pure functions over plain vectors: no file access, no GPU. Not covered: vertex reordering for
//! cache locality, per-attribute bit depths other than positions, or streaming decode.

use crate::linalg::Vector3;
use crate::mesh_primitives::PrimitiveMesh;

const MAGIC: &[u8; 4] = b"BMC1";
/// The most vertices a stream may declare, so a damaged header cannot ask for gigabytes.
pub const MAX_VERTICES: usize = 1 << 24;
/// The most indices a stream may declare.
pub const MAX_INDICES: usize = 3 << 24;
/// The default bit depth of a position axis.
pub const DEFAULT_POSITION_BITS: u8 = 14;

/// Why a mesh could not be compressed or a stream could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshCompressError {
    /// The mesh is inconsistent: attribute counts differ, a value is not finite, an index is out of range, or the
    /// index count is not a multiple of three.
    InvalidMesh,
    /// `position_bits` is outside 8..=16.
    BadBits,
    /// The mesh has more vertices or indices than the format allows.
    TooLarge,
    /// The stream does not start with the format's magic bytes.
    BadMagic,
    /// The stream ends early, or a count in the header is larger than the stream can hold.
    Truncated,
    /// The checksum does not match: the stream is damaged.
    BadChecksum,
    /// A decoded index points past the vertices.
    IndexOutOfRange,
}

/// Compression settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Bits per position axis, 8 to 16. More bits mean less error and a larger stream.
    pub position_bits: u8,
}

impl Default for Options {
    fn default() -> Self {
        Options { position_bits: DEFAULT_POSITION_BITS }
    }
}

/// The largest distance a decoded position can be from the original, for a box of `extent` along its longest axis.
pub fn max_position_error(extent: f32, position_bits: u8) -> f32 {
    let steps = ((1u32 << position_bits) - 1) as f32;
    extent / steps / 2.0
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for &b in bytes {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], MeshCompressError> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len()).ok_or(MeshCompressError::Truncated)?;
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn byte(&mut self) -> Result<u8, MeshCompressError> {
        Ok(self.take(1)?[0])
    }

    fn f32(&mut self) -> Result<f32, MeshCompressError> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn varint(&mut self) -> Result<u64, MeshCompressError> {
        let mut v = 0u64;
        for shift in (0..70).step_by(7) {
            let b = self.byte()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(MeshCompressError::Truncated)
    }
}

fn quantize(v: f32, min: f32, extent: f32, max_q: u32) -> u32 {
    if extent <= 0.0 {
        return 0;
    }
    (((v - min) / extent) * max_q as f32).round().clamp(0.0, max_q as f32) as u32
}

fn dequantize(q: u32, min: f32, extent: f32, max_q: u32) -> f32 {
    if extent <= 0.0 {
        return min;
    }
    min + (q as f32 / max_q as f32) * extent
}

fn sign(v: f32) -> f32 {
    if v >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

fn oct_encode(n: Vector3) -> [i8; 2] {
    let l1 = n.x.abs() + n.y.abs() + n.z.abs();
    if l1 <= 0.0 || !l1.is_finite() {
        return [0, 0];
    }
    let (mut u, mut v) = (n.x / l1, n.y / l1);
    if n.z < 0.0 {
        let (ou, ov) = (u, v);
        u = (1.0 - ov.abs()) * sign(ou);
        v = (1.0 - ou.abs()) * sign(ov);
    }
    [(u * 127.0).round().clamp(-127.0, 127.0) as i8, (v * 127.0).round().clamp(-127.0, 127.0) as i8]
}

fn oct_decode(e: [i8; 2]) -> Vector3 {
    let (mut u, mut v) = (f32::from(e[0]) / 127.0, f32::from(e[1]) / 127.0);
    let z = 1.0 - u.abs() - v.abs();
    if z < 0.0 {
        let (ou, ov) = (u, v);
        u = (1.0 - ov.abs()) * sign(ou);
        v = (1.0 - ou.abs()) * sign(ov);
    }
    let n = Vector3::new(u, v, z);
    if n.length() > 0.0 {
        n.normalized()
    } else {
        Vector3::new(0.0, 0.0, 1.0)
    }
}

/// Compresses a mesh. Normals and texture coordinates are kept only when the mesh has one per vertex; a mesh with
/// some other count of either is refused.
pub fn compress(mesh: &PrimitiveMesh, options: &Options) -> Result<Vec<u8>, MeshCompressError> {
    if !(8..=16).contains(&options.position_bits) {
        return Err(MeshCompressError::BadBits);
    }
    let n = mesh.positions.len();
    if n > MAX_VERTICES || mesh.indices.len() > MAX_INDICES {
        return Err(MeshCompressError::TooLarge);
    }
    let has_normals = !mesh.normals.is_empty();
    let has_uvs = !mesh.texture_coordinates.is_empty();
    if (has_normals && mesh.normals.len() != n) || (has_uvs && mesh.texture_coordinates.len() != n) {
        return Err(MeshCompressError::InvalidMesh);
    }
    if mesh.indices.len() % 3 != 0 || mesh.indices.iter().any(|&i| i as usize >= n) {
        return Err(MeshCompressError::InvalidMesh);
    }
    let finite3 = |v: &Vector3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    if !mesh.positions.iter().all(finite3) || !mesh.normals.iter().all(finite3) || !mesh.texture_coordinates.iter().all(|t| t[0].is_finite() && t[1].is_finite()) {
        return Err(MeshCompressError::InvalidMesh);
    }

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_varint(&mut out, n as u64);
    put_varint(&mut out, mesh.indices.len() as u64);
    out.push(u8::from(has_normals) | (u8::from(has_uvs) << 1));
    out.push(options.position_bits);

    // positions
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &mesh.positions {
        for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
            lo[k] = lo[k].min(c);
            hi[k] = hi[k].max(c);
        }
    }
    if n == 0 {
        lo = [0.0; 3];
        hi = [0.0; 3];
    }
    for c in lo.iter().chain(hi.iter()) {
        out.extend_from_slice(&c.to_le_bytes());
    }
    let max_q = (1u32 << options.position_bits) - 1;
    let mut prev = [0i64; 3];
    for p in &mesh.positions {
        for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
            let q = i64::from(quantize(c, lo[k], hi[k] - lo[k], max_q));
            put_varint(&mut out, zigzag(q - prev[k]));
            prev[k] = q;
        }
    }

    // normals
    if has_normals {
        for nrm in &mesh.normals {
            let e = oct_encode(*nrm);
            out.push(e[0] as u8);
            out.push(e[1] as u8);
        }
    }

    // texture coordinates
    if has_uvs {
        let (mut ulo, mut uhi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for t in &mesh.texture_coordinates {
            for k in 0..2 {
                ulo[k] = ulo[k].min(t[k]);
                uhi[k] = uhi[k].max(t[k]);
            }
        }
        for c in ulo.iter().chain(uhi.iter()) {
            out.extend_from_slice(&c.to_le_bytes());
        }
        let mut prev = [0i64; 2];
        for t in &mesh.texture_coordinates {
            for k in 0..2 {
                let q = i64::from(quantize(t[k], ulo[k], uhi[k] - ulo[k], 0xffff));
                put_varint(&mut out, zigzag(q - prev[k]));
                prev[k] = q;
            }
        }
    }

    // indices
    let mut last = 0i64;
    for &i in &mesh.indices {
        put_varint(&mut out, zigzag(i64::from(i) - last));
        last = i64::from(i);
    }

    let sum = fnv1a(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    Ok(out)
}

/// Reads a stream made by [`compress`].
pub fn decompress(bytes: &[u8]) -> Result<PrimitiveMesh, MeshCompressError> {
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Err(MeshCompressError::BadMagic);
    }
    if bytes.len() < MAGIC.len() + 4 {
        return Err(MeshCompressError::Truncated);
    }
    let (body, tail) = bytes.split_at(bytes.len() - 4);
    if fnv1a(body).to_le_bytes() != tail {
        return Err(MeshCompressError::BadChecksum);
    }
    let mut r = Reader { data: body, at: MAGIC.len() };
    let n = r.varint()? as usize;
    let index_count = r.varint()? as usize;
    if n > MAX_VERTICES || index_count > MAX_INDICES || index_count % 3 != 0 {
        return Err(MeshCompressError::TooLarge);
    }
    // every vertex costs at least three bytes of position and every index at least one byte
    if n.saturating_mul(3).saturating_add(index_count) > body.len() {
        return Err(MeshCompressError::Truncated);
    }
    let flags = r.byte()?;
    let bits = r.byte()?;
    if !(8..=16).contains(&bits) {
        return Err(MeshCompressError::BadBits);
    }
    let mut lo = [0f32; 3];
    let mut hi = [0f32; 3];
    for c in lo.iter_mut().chain(hi.iter_mut()) {
        *c = r.f32()?;
    }
    let max_q = (1u32 << bits) - 1;
    let mut positions = Vec::with_capacity(n);
    let mut prev = [0i64; 3];
    for _ in 0..n {
        let mut p = [0f32; 3];
        for k in 0..3 {
            prev[k] += unzigzag(r.varint()?);
            let q = u32::try_from(prev[k]).map_err(|_| MeshCompressError::Truncated)?;
            p[k] = dequantize(q.min(max_q), lo[k], hi[k] - lo[k], max_q);
        }
        positions.push(Vector3::new(p[0], p[1], p[2]));
    }
    let mut normals = Vec::new();
    if flags & 1 != 0 {
        for _ in 0..n {
            let b = r.take(2)?;
            normals.push(oct_decode([b[0] as i8, b[1] as i8]));
        }
    }
    let mut texture_coordinates = Vec::new();
    if flags & 2 != 0 {
        let mut ulo = [0f32; 2];
        let mut uhi = [0f32; 2];
        for c in ulo.iter_mut().chain(uhi.iter_mut()) {
            *c = r.f32()?;
        }
        let mut prev = [0i64; 2];
        for _ in 0..n {
            let mut t = [0f32; 2];
            for k in 0..2 {
                prev[k] += unzigzag(r.varint()?);
                let q = u32::try_from(prev[k]).map_err(|_| MeshCompressError::Truncated)?;
                t[k] = dequantize(q.min(0xffff), ulo[k], uhi[k] - ulo[k], 0xffff);
            }
            texture_coordinates.push(t);
        }
    }
    let mut indices = Vec::with_capacity(index_count);
    let mut last = 0i64;
    for _ in 0..index_count {
        last += unzigzag(r.varint()?);
        let i = u32::try_from(last).map_err(|_| MeshCompressError::IndexOutOfRange)?;
        if i as usize >= n {
            return Err(MeshCompressError::IndexOutOfRange);
        }
        indices.push(i);
    }
    if r.at != body.len() {
        return Err(MeshCompressError::Truncated);
    }
    Ok(PrimitiveMesh { positions, normals, texture_coordinates, indices })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_primitives::{box_mesh, sphere_mesh};

    fn raw_size(m: &PrimitiveMesh) -> usize {
        m.positions.len() * 12 + m.normals.len() * 12 + m.texture_coordinates.len() * 8 + m.indices.len() * 4
    }

    #[test]
    fn varints_and_zigzag_round_trip() {
        for v in [0i64, 1, -1, 63, -64, 64, 1 << 40, -(1 << 40), i64::from(i32::MAX), i64::from(i32::MIN)] {
            assert_eq!(unzigzag(zigzag(v)), v);
            let mut buf = Vec::new();
            put_varint(&mut buf, zigzag(v));
            assert_eq!(Reader { data: &buf, at: 0 }.varint().unwrap(), zigzag(v));
        }
        assert_eq!(zigzag(0), 0);
        assert_eq!(zigzag(-1), 1);
        assert_eq!(zigzag(1), 2);
    }

    #[test]
    fn a_sphere_round_trips_within_the_quantization_step_and_gets_smaller() {
        let m = sphere_mesh(2.0, 24).unwrap();
        let bytes = compress(&m, &Options::default()).unwrap();
        assert!(bytes.len() * 3 < raw_size(&m) * 2, "{} of {}", bytes.len(), raw_size(&m));
        let back = decompress(&bytes).unwrap();
        assert_eq!(back.indices, m.indices, "indices come back exactly");
        assert_eq!(back.positions.len(), m.positions.len());
        let bound = max_position_error(4.0, DEFAULT_POSITION_BITS) * 1.01;
        for (a, b) in m.positions.iter().zip(&back.positions) {
            for (x, y) in [(a.x, b.x), (a.y, b.y), (a.z, b.z)] {
                assert!((x - y).abs() <= bound, "{x} vs {y}");
            }
        }
        for (a, b) in m.normals.iter().zip(&back.normals) {
            let dot = (a.x * b.x + a.y * b.y + a.z * b.z).clamp(-1.0, 1.0);
            assert!(dot.acos().to_degrees() < 2.0, "normal off by {} degrees", dot.acos().to_degrees());
            assert!((b.length() - 1.0).abs() < 1e-4);
        }
        for (a, b) in m.texture_coordinates.iter().zip(&back.texture_coordinates) {
            assert!((a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4);
        }
    }

    #[test]
    fn more_position_bits_mean_less_error_and_more_bytes() {
        let m = box_mesh(3.0, 2.0, 1.0, 4, 4, 4).unwrap();
        let low = compress(&m, &Options { position_bits: 8 }).unwrap();
        let high = compress(&m, &Options { position_bits: 16 }).unwrap();
        assert!(low.len() < high.len());
        let err = |bytes: &[u8]| {
            let back = decompress(bytes).unwrap();
            m.positions.iter().zip(&back.positions).map(|(a, b)| (a.x - b.x).abs().max((a.y - b.y).abs()).max((a.z - b.z).abs())).fold(0.0f32, f32::max)
        };
        assert!(err(&high) < err(&low));
        assert!(err(&low) <= max_position_error(3.0, 8) * 1.01);
    }

    #[test]
    fn a_mesh_without_normals_or_coordinates_and_an_empty_mesh_round_trip() {
        let m = PrimitiveMesh {
            positions: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0), Vector3::new(0.0, 1.0, 0.0)],
            normals: vec![],
            texture_coordinates: vec![],
            indices: vec![0, 1, 2],
        };
        let back = decompress(&compress(&m, &Options::default()).unwrap()).unwrap();
        assert_eq!(back, m, "corner values of the box are exact");
        let empty = PrimitiveMesh { positions: vec![], normals: vec![], texture_coordinates: vec![], indices: vec![] };
        assert_eq!(decompress(&compress(&empty, &Options::default()).unwrap()).unwrap(), empty);
    }

    #[test]
    fn a_flat_axis_costs_nothing_and_decodes_to_the_same_value() {
        let m = PrimitiveMesh {
            positions: vec![Vector3::new(5.0, 7.0, 0.0), Vector3::new(6.0, 7.0, 0.0), Vector3::new(5.0, 7.0, 1.0)],
            normals: vec![],
            texture_coordinates: vec![],
            indices: vec![0, 1, 2],
        };
        let back = decompress(&compress(&m, &Options::default()).unwrap()).unwrap();
        assert!(back.positions.iter().all(|p| p.y == 7.0));
    }

    #[test]
    fn octahedral_normals_cover_every_hemisphere() {
        for n in [Vector3::new(0.0, 0.0, 1.0), Vector3::new(0.0, 0.0, -1.0), Vector3::new(1.0, 0.0, 0.0), Vector3::new(-1.0, -1.0, -1.0).normalized(), Vector3::new(0.3, -0.5, -0.8).normalized()] {
            let d = oct_decode(oct_encode(n));
            let dot = n.x * d.x + n.y * d.y + n.z * d.z;
            assert!(dot > 0.9995, "{n:?} came back as {d:?}");
        }
        assert_eq!(oct_decode(oct_encode(Vector3::ZERO)), Vector3::new(0.0, 0.0, 1.0), "a zero normal decodes to a unit one");
    }

    #[test]
    fn bad_meshes_and_bad_settings_are_refused() {
        let ok = box_mesh(1.0, 1.0, 1.0, 1, 1, 1).unwrap();
        assert_eq!(compress(&ok, &Options { position_bits: 7 }), Err(MeshCompressError::BadBits));
        assert_eq!(compress(&ok, &Options { position_bits: 17 }), Err(MeshCompressError::BadBits));
        let mut m = ok.clone();
        m.indices.push(0);
        assert_eq!(compress(&m, &Options::default()), Err(MeshCompressError::InvalidMesh), "not a multiple of three");
        let mut m = ok.clone();
        m.indices[0] = 9999;
        assert_eq!(compress(&m, &Options::default()), Err(MeshCompressError::InvalidMesh));
        let mut m = ok.clone();
        m.normals.pop();
        assert_eq!(compress(&m, &Options::default()), Err(MeshCompressError::InvalidMesh));
        let mut m = ok;
        m.positions[0].x = f32::NAN;
        assert_eq!(compress(&m, &Options::default()), Err(MeshCompressError::InvalidMesh));
    }

    #[test]
    fn damaged_streams_are_refused() {
        let bytes = compress(&sphere_mesh(1.0, 8).unwrap(), &Options::default()).unwrap();
        assert_eq!(decompress(b"nope"), Err(MeshCompressError::BadMagic));
        assert_eq!(decompress(&bytes[..6]), Err(MeshCompressError::Truncated));
        assert_eq!(decompress(&bytes[..4]), Err(MeshCompressError::Truncated));
        assert_eq!(decompress(&bytes[..12]), Err(MeshCompressError::BadChecksum));
        let mut flipped = bytes.clone();
        flipped[bytes.len() / 2] ^= 0x10;
        assert_eq!(decompress(&flipped), Err(MeshCompressError::BadChecksum));
        let mut cut = bytes.clone();
        cut.truncate(bytes.len() - 9);
        assert!(decompress(&cut).is_err());
        // a header that claims far more vertices than the stream holds, with a valid checksum
        let mut forged = MAGIC.to_vec();
        put_varint(&mut forged, 1_000_000);
        put_varint(&mut forged, 3);
        forged.extend_from_slice(&[0, 14]);
        let sum = fnv1a(&forged);
        forged.extend_from_slice(&sum.to_le_bytes());
        assert_eq!(decompress(&forged), Err(MeshCompressError::Truncated));
    }

    #[test]
    fn an_index_past_the_vertices_is_refused_even_with_a_valid_checksum() {
        let m = PrimitiveMesh { positions: vec![Vector3::ZERO], normals: vec![], texture_coordinates: vec![], indices: vec![0, 0, 0] };
        let mut bytes = compress(&m, &Options::default()).unwrap();
        // the three indices are the last three bytes before the checksum; point the last one at vertex 1
        let n = bytes.len();
        bytes[n - 5] = 2; // zigzag(+1) is 2: the third index becomes 1
        let sum = fnv1a(&bytes[..n - 4]);
        bytes[n - 4..].copy_from_slice(&sum.to_le_bytes());
        assert_eq!(decompress(&bytes), Err(MeshCompressError::IndexOutOfRange));
    }
}
