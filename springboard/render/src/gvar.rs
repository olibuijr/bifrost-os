//! TrueType variations: `fvar` axes, `avar` axis maps and `gvar` glyph
//! deltas (tuple regions, packed point numbers and deltas, IUP for
//! unreferenced points). Phantom points carry the advance-width change, so
//! no `HVAR` is needed. This is what turns SF Pro's single variable file
//! into real weights (wght) and optical sizes (opsz).

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}
fn f2dot14(d: &[u8], o: usize) -> Option<f32> {
    i16_at(d, o).map(|v| v as f32 / 16384.0)
}
fn fixed(d: &[u8], o: usize) -> Option<f32> {
    u32_at(d, o).map(|v| v as i32 as f32 / 65536.0)
}

/// One `fvar` axis in user units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub tag: [u8; 4],
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// Parses the `fvar` table's axes.
pub fn axes(fvar: &[u8]) -> Option<Vec<Axis>> {
    let off = u16_at(fvar, 4)? as usize;
    let count = u16_at(fvar, 8)? as usize;
    let size = u16_at(fvar, 10)? as usize;
    (0..count)
        .map(|i| {
            let a = off + i * size;
            Some(Axis { tag: fvar.get(a..a + 4)?.try_into().ok()?, min: fixed(fvar, a + 4)?, default: fixed(fvar, a + 8)?, max: fixed(fvar, a + 12)? })
        })
        .collect()
}

/// Normalises user coordinates to -1..1 per axis (clamped), then applies
/// the `avar` segment maps when present. `coords` are (tag, value) pairs;
/// axes not mentioned sit at their default.
pub fn normalize(axes: &[Axis], avar: Option<&[u8]>, coords: &[([u8; 4], f32)]) -> Vec<f32> {
    let mut out: Vec<f32> = axes
        .iter()
        .map(|a| {
            let v = coords.iter().find(|(t, _)| *t == a.tag).map_or(a.default, |(_, v)| *v).clamp(a.min, a.max);
            if v < a.default && a.default > a.min {
                (v - a.default) / (a.default - a.min)
            } else if v > a.default && a.max > a.default {
                (v - a.default) / (a.max - a.default)
            } else {
                0.0
            }
        })
        .collect();
    if let Some(av) = avar {
        let mut p = 8;
        for n in out.iter_mut() {
            let Some(pairs) = u16_at(av, p) else { break };
            let pairs = pairs as usize;
            let map: Vec<(f32, f32)> = (0..pairs).filter_map(|i| Some((f2dot14(av, p + 2 + 4 * i)?, f2dot14(av, p + 4 + 4 * i)?))).collect();
            p += 2 + 4 * pairs;
            if let Some(i) = map.iter().position(|(from, _)| *from >= *n) {
                let (f1, t1) = map[i];
                *n = if i == 0 || f1 == *n {
                    t1
                } else {
                    let (f0, t0) = map[i - 1];
                    t0 + (t1 - t0) * (*n - f0) / (f1 - f0)
                };
            }
        }
    }
    out
}

/// Influence of a region (per-axis start/peak/end) at normalised `coords`.
pub fn scalar(region: &[(f32, f32, f32)], coords: &[f32]) -> f32 {
    let mut s = 1.0;
    for (&(lo, peak, hi), &v) in region.iter().zip(coords) {
        if peak == 0.0 {
            continue;
        }
        if lo > peak || peak > hi || (lo < 0.0 && hi > 0.0) || v == peak {
            continue;
        }
        if v <= lo || hi <= v {
            return 0.0;
        }
        s *= if v < peak { (v - lo) / (peak - lo) } else { (hi - v) / (hi - peak) };
    }
    s
}

/// Reads packed point numbers; None means "all points".
fn point_numbers(d: &[u8], p: &mut usize) -> Option<Option<Vec<usize>>> {
    let b0 = *d.get(*p)? as usize;
    *p += 1;
    let count = if b0 & 0x80 != 0 {
        let c = (b0 & 0x7f) << 8 | *d.get(*p)? as usize;
        *p += 1;
        c
    } else {
        b0
    };
    if count == 0 {
        return Some(None);
    }
    let mut out = Vec::with_capacity(count);
    let mut v = 0usize;
    while out.len() < count {
        let run = *d.get(*p)?;
        *p += 1;
        let n = (run & 0x7f) as usize + 1;
        for _ in 0..n.min(count - out.len()) {
            let step = if run & 0x80 != 0 {
                let s = u16_at(d, *p)? as usize;
                *p += 2;
                s
            } else {
                let s = *d.get(*p)? as usize;
                *p += 1;
                s
            };
            v += step;
            out.push(v);
        }
    }
    Some(Some(out))
}

/// Reads `count` packed deltas.
fn deltas(d: &[u8], p: &mut usize, count: usize) -> Option<Vec<f32>> {
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let run = *d.get(*p)?;
        *p += 1;
        let n = ((run & 0x3f) as usize + 1).min(count - out.len());
        for _ in 0..n {
            out.push(if run & 0x80 != 0 {
                0.0
            } else if run & 0x40 != 0 {
                let v = i16_at(d, *p)? as f32;
                *p += 2;
                v
            } else {
                let v = *d.get(*p)? as i8 as f32;
                *p += 1;
                v
            });
        }
    }
    Some(out)
}

/// Interpolates deltas for unreferenced points of one contour along one
/// dimension (IUP), from the original coordinates `c` of its points.
fn iup(c: &[f32], delta: &mut [f32], referenced: &[bool]) {
    let n = c.len();
    if referenced.iter().all(|&r| !r) {
        return;
    }
    for i in 0..n {
        if referenced[i] {
            continue;
        }
        let prev = (1..n).map(|k| (i + n - k) % n).find(|&k| referenced[k]).unwrap();
        let next = (1..n).map(|k| (i + k) % n).find(|&k| referenced[k]).unwrap();
        let (c1, c2, d1, d2) = (c[prev], c[next], delta[prev], delta[next]);
        delta[i] = if c1 == c2 {
            if d1 == d2 {
                d1
            } else {
                0.0
            }
        } else {
            let (lo, hi, dlo, dhi) = if c1 < c2 { (c1, c2, d1, d2) } else { (c2, c1, d2, d1) };
            if c[i] <= lo {
                dlo
            } else if c[i] >= hi {
                dhi
            } else {
                dlo + (dhi - dlo) * (c[i] - lo) / (hi - lo)
            }
        };
    }
}

pub struct Gvar<'a> {
    data: &'a [u8],
    axis_count: usize,
    shared: usize,
    shared_count: usize,
    glyph_count: usize,
    long: bool,
    offsets: usize,
    base: usize,
}

impl<'a> Gvar<'a> {
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        Some(Gvar {
            data,
            axis_count: u16_at(data, 4)? as usize,
            shared_count: u16_at(data, 6)? as usize,
            shared: u32_at(data, 8)? as usize,
            glyph_count: u16_at(data, 12)? as usize,
            long: u16_at(data, 14)? & 1 != 0,
            base: u32_at(data, 16)? as usize,
            offsets: 20,
        })
    }

    fn glyph_data(&self, gid: u16) -> Option<&'a [u8]> {
        let g = gid as usize;
        if g >= self.glyph_count {
            return None;
        }
        let d = self.data;
        let (a, b) = if self.long {
            (u32_at(d, self.offsets + 4 * g)? as usize, u32_at(d, self.offsets + 4 * g + 4)? as usize)
        } else {
            (u16_at(d, self.offsets + 2 * g)? as usize * 2, u16_at(d, self.offsets + 2 * g + 2)? as usize * 2)
        };
        (b > a).then(|| d.get(self.base + a..self.base + b)).flatten()
    }

    fn region(&self, index: usize) -> Option<Vec<(f32, f32, f32)>> {
        if index >= self.shared_count {
            return None;
        }
        let at = self.shared + index * 2 * self.axis_count;
        (0..self.axis_count).map(|i| f2dot14(self.data, at + 2 * i).map(|p| (p.min(0.0), p, p.max(0.0)))).collect()
    }

    /// Adds the variation deltas at normalised `coords` to `xs`/`ys`, which
    /// hold the glyph's points followed by its four phantom points.
    /// `ends` are the contour end indices (empty for composites, whose
    /// "points" are the component offsets).
    pub fn apply(&self, gid: u16, coords: &[f32], xs: &mut [f32], ys: &mut [f32], ends: &[usize]) -> Option<()> {
        let d = self.glyph_data(gid)?;
        let n = xs.len();
        let tuple_count = u16_at(d, 0)?;
        let mut data_p = u16_at(d, 2)? as usize;
        let mut header = 4;
        let shared_points = if tuple_count & 0x8000 != 0 { point_numbers(d, &mut data_p)? } else { None };
        let (orig_x, orig_y) = (xs.to_vec(), ys.to_vec());
        let (mut dx, mut dy) = (vec![0.0f32; n], vec![0.0f32; n]);
        for _ in 0..(tuple_count & 0xfff) {
            let size = u16_at(d, header)? as usize;
            let index = u16_at(d, header + 2)?;
            header += 4;
            let mut region = if index & 0x8000 != 0 {
                let r = (0..self.axis_count).map(|i| f2dot14(d, header + 2 * i).map(|p| (p.min(0.0), p, p.max(0.0)))).collect::<Option<Vec<_>>>()?;
                header += 2 * self.axis_count;
                r
            } else {
                self.region((index & 0xfff) as usize)?
            };
            if index & 0x4000 != 0 {
                for (i, r) in region.iter_mut().enumerate() {
                    r.0 = f2dot14(d, header + 2 * i)?;
                    r.2 = f2dot14(d, header + 2 * (self.axis_count + i))?;
                }
                header += 4 * self.axis_count;
            }
            let s = scalar(&region, coords);
            let mut p = data_p;
            data_p += size;
            if s == 0.0 {
                continue;
            }
            let points = if index & 0x2000 != 0 { point_numbers(d, &mut p)? } else { shared_points.clone() };
            let count = points.as_ref().map_or(n, |v| v.len());
            let tx = deltas(d, &mut p, count)?;
            let ty = deltas(d, &mut p, count)?;
            let (mut ex, mut ey) = (vec![0.0f32; n], vec![0.0f32; n]);
            let mut referenced = vec![false; n];
            match &points {
                None => {
                    ex.copy_from_slice(&tx[..n]);
                    ey.copy_from_slice(&ty[..n]);
                }
                Some(pts) => {
                    for (k, &i) in pts.iter().enumerate() {
                        if i < n {
                            ex[i] = tx[k];
                            ey[i] = ty[k];
                            referenced[i] = true;
                        }
                    }
                    let mut start = 0;
                    for &end in ends {
                        let r = start..end + 1;
                        iup(&orig_x[r.clone()], &mut ex[r.clone()], &referenced[r.clone()]);
                        iup(&orig_y[r.clone()], &mut ey[r.clone()], &referenced[r.clone()]);
                        start = end + 1;
                    }
                }
            }
            for i in 0..n {
                dx[i] += s * ex[i];
                dy[i] += s * ey[i];
            }
        }
        for i in 0..n {
            xs[i] += dx[i];
            ys[i] += dy[i];
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_point_numbers_and_deltas() {
        // count 3, one byte-run of 3 steps: 1, 2, 4 -> points 1, 3, 7
        let d = [3u8, 0x02, 1, 2, 4];
        let mut p = 0;
        assert_eq!(point_numbers(&d, &mut p).unwrap(), Some(vec![1, 3, 7]));
        assert_eq!(p, 5);
        // word run with a 300 step
        let d = [1u8, 0x80, 1, 44];
        let mut p = 0;
        assert_eq!(point_numbers(&d, &mut p).unwrap(), Some(vec![300]));
        assert_eq!(point_numbers(&[0u8], &mut 0).unwrap(), None, "zero count means all points");
        // deltas: 2 zeros, 2 bytes, 1 word
        let d = [0x81u8, 0x01, 5, 0xfe, 0x40, 0x01, 0x2c];
        let mut p = 0;
        assert_eq!(deltas(&d, &mut p, 5).unwrap(), vec![0.0, 0.0, 5.0, -2.0, 300.0]);
    }

    #[test]
    fn region_scalars() {
        let wght = [(0.0, 1.0, 1.0)];
        assert_eq!(scalar(&wght, &[0.0]), 0.0);
        assert_eq!(scalar(&wght, &[0.5]), 0.5);
        assert_eq!(scalar(&wght, &[1.0]), 1.0);
        assert_eq!(scalar(&wght, &[-0.5]), 0.0);
        let both = [(0.0, 1.0, 1.0), (-1.0, -1.0, 0.0)];
        assert_eq!(scalar(&both, &[0.5, -0.5]), 0.25);
        assert_eq!(scalar(&both, &[0.5, 0.5]), 0.0);
        let mid = [(0.0, 0.5, 1.0)];
        assert_eq!(scalar(&mid, &[0.25]), 0.5);
        assert_eq!(scalar(&mid, &[0.75]), 0.5);
        assert_eq!(scalar(&[(0.0, 0.0, 0.0)], &[0.7]), 1.0, "unused axes do not scale");
    }

    #[test]
    fn iup_interpolates_between_referenced_neighbours() {
        let c = [0.0, 10.0, 20.0, 30.0];
        let mut d = [0.0, 0.0, 0.0, 6.0];
        iup(&c, &mut d, &[true, false, false, true]);
        assert_eq!(d, [0.0, 2.0, 4.0, 6.0]);
        // outside the referenced span the nearer end's delta is used
        let c = [5.0, 0.0, 10.0, 50.0];
        let mut d = [0.0, 1.0, 3.0, 0.0];
        iup(&c, &mut d, &[false, true, true, false]);
        assert_eq!(d, [2.0, 1.0, 3.0, 3.0]);
        // equal coordinates with different deltas get none
        let mut d = [0.0, 1.0, 3.0];
        iup(&[7.0, 7.0, 7.0], &mut d, &[false, true, true]);
        assert_eq!(d[0], 0.0);
        let mut none = [1.0, 2.0];
        iup(&[0.0, 1.0], &mut none, &[false, false]);
        assert_eq!(none, [1.0, 2.0]);
    }

    #[test]
    fn normalisation_clamps_and_maps() {
        let axes = [Axis { tag: *b"wght", min: 1.0, default: 400.0, max: 1000.0 }, Axis { tag: *b"opsz", min: 17.0, default: 28.0, max: 96.0 }];
        assert_eq!(normalize(&axes, None, &[]), vec![0.0, 0.0]);
        assert_eq!(normalize(&axes, None, &[(*b"wght", 700.0)]), vec![0.5, 0.0]);
        assert_eq!(normalize(&axes, None, &[(*b"opsz", 17.0)]), vec![0.0, -1.0]);
        assert_eq!(normalize(&axes, None, &[(*b"opsz", 5.0), (*b"wght", 5000.0)]), vec![1.0, -1.0]);
        // avar: identity on wght, squashes opsz -1 to -0.5
        let mut avar = vec![0, 1, 0, 0, 0, 0, 0, 2];
        avar.extend([0, 3, 0xc0, 0, 0xc0, 0, 0, 0, 0, 0, 0x40, 0, 0x40, 0]);
        avar.extend([0, 3, 0xc0, 0, 0xe0, 0, 0, 0, 0, 0, 0x40, 0, 0x40, 0]);
        assert_eq!(normalize(&axes, Some(&avar), &[(*b"opsz", 17.0)]), vec![0.0, -0.5]);
        assert_eq!(normalize(&axes, Some(&avar), &[(*b"opsz", 22.5)]), vec![0.0, -0.25]);
    }
}
