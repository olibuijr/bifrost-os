//! Mesh files as plain data: Wavefront OBJ (read and write), STL (ASCII and binary) and PLY (ASCII).
//! Each reader returns a `PrimitiveMesh`, the same vertex data the built-in shapes use. Only triangles are kept: a
//! polygon is fan-triangulated. A file without normals gets area-weighted normals from `mesh_process::smooth_normals`
//! (STL keeps the facet normal the file declares, or the winding normal when that is zero). A file without texture
//! coordinates gets `[0, 0]`. The readers do not touch the filesystem; the caller passes the bytes or text.
//!
//! Not read: materials, groups, smoothing groups, curves, colours, STL solids with more than one body, PLY lists other
//! than the face list, and every other format. OBJ indices may be negative (relative to the vertices defined so far).

use std::collections::HashMap;
use std::fmt;

use crate::linalg::Vector3;
use crate::mesh_primitives::PrimitiveMesh;
use crate::mesh_process::smooth_normals;

/// Why a file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshFileError {
    /// The file defines no triangles.
    Empty,
    /// A line or record could not be parsed. `line` is 1-based for text; binary STL reports 0.
    Malformed { line: usize },
    /// A face refers to a position, texture coordinate or normal the file does not define yet.
    IndexOutOfRange { line: usize },
    /// The bytes end before the data the header promises.
    Truncated,
}

impl fmt::Display for MeshFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshFileError::Empty => write!(f, "the file defines no triangles"),
            MeshFileError::Malformed { line } => write!(f, "malformed data at line {}", line),
            MeshFileError::IndexOutOfRange { line } => write!(f, "index out of range at line {}", line),
            MeshFileError::Truncated => write!(f, "the file ends before its data"),
        }
    }
}

/// Parses every token on a line as `f32`; any token that is not a number is malformed.
fn floats(tokens: &[&str], line: usize) -> Result<Vec<f32>, MeshFileError> {
    tokens.iter().map(|t| t.parse::<f32>().map_err(|_| MeshFileError::Malformed { line })).collect()
}

/// Turns an OBJ index (1-based, or negative to count back from the newest) into a zero-based one. An empty field is
/// `None`, as in `1//3` where the texture coordinate is absent.
fn obj_index(field: &str, count: usize, line: usize) -> Result<Option<usize>, MeshFileError> {
    if field.is_empty() {
        return Ok(None);
    }
    let n: i64 = field.parse().map_err(|_| MeshFileError::Malformed { line })?;
    let zero = match n {
        0 => return Err(MeshFileError::Malformed { line }),
        n if n > 0 => n - 1,
        n => count as i64 + n,
    };
    if zero < 0 || zero >= count as i64 {
        return Err(MeshFileError::IndexOutOfRange { line });
    }
    Ok(Some(zero as usize))
}

/// Builds the mesh. `normals` of `None` means the file had none, so smooth normals are computed.
fn finish(
    positions: Vec<Vector3>,
    normals: Option<Vec<Vector3>>,
    uvs: Option<Vec<[f32; 2]>>,
    indices: Vec<u32>,
) -> Result<PrimitiveMesh, MeshFileError> {
    if indices.is_empty() {
        return Err(MeshFileError::Empty);
    }
    let texture_coordinates = uvs.unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
    let mut mesh = PrimitiveMesh { positions, normals: Vec::new(), texture_coordinates, indices };
    mesh.normals = match normals {
        Some(n) => n,
        None => smooth_normals(&mesh),
    };
    Ok(mesh)
}

/// Reads Wavefront OBJ text. Positions, texture coordinates and normals are indexed separately in the file; each unique
/// `position/texture/normal` triple becomes one output vertex, so shared corners are emitted once. Faces with more than
/// three corners are fan-triangulated. Normals are used only when every corner has one.
pub fn read_obj(text: &str) -> Result<PrimitiveMesh, MeshFileError> {
    let mut positions: Vec<Vector3> = Vec::new();
    let mut uv_list: Vec<[f32; 2]> = Vec::new();
    let mut normal_list: Vec<Vector3> = Vec::new();
    let mut corner_ids: HashMap<(usize, Option<usize>, Option<usize>), u32> = HashMap::new();
    let mut out_positions: Vec<Vector3> = Vec::new();
    let mut out_uvs: Vec<[f32; 2]> = Vec::new();
    let mut out_normals: Vec<Vector3> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut all_normals = true;

    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let body = raw.split('#').next().unwrap_or("").trim();
        let tokens: Vec<&str> = body.split_whitespace().collect();
        match tokens.first().copied() {
            Some("v") => {
                let f = floats(&tokens[1..], line)?;
                if f.len() < 3 {
                    return Err(MeshFileError::Malformed { line });
                }
                positions.push(Vector3::new(f[0], f[1], f[2]));
            }
            Some("vt") => {
                let f = floats(&tokens[1..], line)?;
                if f.len() < 2 {
                    return Err(MeshFileError::Malformed { line });
                }
                uv_list.push([f[0], f[1]]);
            }
            Some("vn") => {
                let f = floats(&tokens[1..], line)?;
                if f.len() < 3 {
                    return Err(MeshFileError::Malformed { line });
                }
                normal_list.push(Vector3::new(f[0], f[1], f[2]));
            }
            Some("f") => {
                if tokens.len() < 4 {
                    return Err(MeshFileError::Malformed { line });
                }
                let mut corners: Vec<(usize, Option<usize>, Option<usize>)> = Vec::with_capacity(tokens.len() - 1);
                for token in &tokens[1..] {
                    let mut fields = token.split('/');
                    let p = obj_index(fields.next().unwrap_or(""), positions.len(), line)?;
                    let t = obj_index(fields.next().unwrap_or(""), uv_list.len(), line)?;
                    let n = obj_index(fields.next().unwrap_or(""), normal_list.len(), line)?;
                    let p = p.ok_or(MeshFileError::Malformed { line })?;
                    corners.push((p, t, n));
                }
                for k in 1..corners.len() - 1 {
                    for &corner in &[corners[0], corners[k], corners[k + 1]] {
                        let id = match corner_ids.get(&corner) {
                            Some(&id) => id,
                            None => {
                                let id = out_positions.len() as u32;
                                out_positions.push(positions[corner.0]);
                                out_uvs.push(corner.1.map_or([0.0, 0.0], |t| uv_list[t]));
                                match corner.2 {
                                    Some(n) => out_normals.push(normal_list[n]),
                                    None => {
                                        all_normals = false;
                                        out_normals.push(Vector3::ZERO);
                                    }
                                }
                                corner_ids.insert(corner, id);
                                id
                            }
                        };
                        indices.push(id);
                    }
                }
            }
            _ => {}
        }
    }
    let normals = if all_normals { Some(out_normals) } else { None };
    finish(out_positions, normals, Some(out_uvs), indices)
}

/// Writes a mesh as OBJ text: one `v`, `vt` and `vn` per vertex, and each triangle as `f a/a/a b/b/b c/c/c` with
/// one-based indices. The mesh's texture coordinate and normal arrays must have one entry per position; that is the
/// layout `mesh_primitives` and `read_obj` produce.
pub fn write_obj(mesh: &PrimitiveMesh) -> String {
    let mut out = String::new();
    for p in &mesh.positions {
        out.push_str(&format!("v {} {} {}\n", p.x, p.y, p.z));
    }
    for t in &mesh.texture_coordinates {
        out.push_str(&format!("vt {} {}\n", t[0], t[1]));
    }
    for n in &mesh.normals {
        out.push_str(&format!("vn {} {} {}\n", n.x, n.y, n.z));
    }
    for tri in mesh.indices.chunks_exact(3) {
        out.push('f');
        for &i in tri {
            let k = i as u64 + 1;
            out.push_str(&format!(" {}/{}/{}", k, k, k));
        }
        out.push('\n');
    }
    out
}

/// One triangle as read from an STL record: the declared facet normal and three corners.
type StlTriangle = (Vector3, [Vector3; 3]);

/// Reads STL. A file whose length is exactly 84 bytes plus 50 per triangle (the count is the little-endian `u32` at
/// byte 80) is binary; anything else is read as ASCII. Triangles are not shared, so every corner gets its facet normal.
pub fn read_stl(bytes: &[u8]) -> Result<PrimitiveMesh, MeshFileError> {
    if is_binary_stl(bytes) {
        read_stl_binary(bytes)
    } else {
        read_stl_ascii(bytes)
    }
}

fn is_binary_stl(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    bytes.len() == 84 + 50 * count
}

fn read_stl_binary(bytes: &[u8]) -> Result<PrimitiveMesh, MeshFileError> {
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    let mut tris: Vec<StlTriangle> = Vec::with_capacity(count);
    for t in 0..count {
        let start = 84 + 50 * t;
        let rec = bytes.get(start..start + 50).ok_or(MeshFileError::Truncated)?;
        let f = |k: usize| f32::from_le_bytes([rec[4 * k], rec[4 * k + 1], rec[4 * k + 2], rec[4 * k + 3]]);
        let normal = Vector3::new(f(0), f(1), f(2));
        let corner = |k: usize| Vector3::new(f(k), f(k + 1), f(k + 2));
        tris.push((normal, [corner(3), corner(6), corner(9)]));
    }
    flat_triangles(&tris)
}

fn read_stl_ascii(bytes: &[u8]) -> Result<PrimitiveMesh, MeshFileError> {
    let text = std::str::from_utf8(bytes).map_err(|_| MeshFileError::Malformed { line: 0 })?;
    let mut tris: Vec<StlTriangle> = Vec::new();
    let mut declared = Vector3::ZERO;
    let mut corners: Vec<Vector3> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        match tokens.first().copied() {
            Some("facet") => {
                if tokens.len() < 5 {
                    return Err(MeshFileError::Malformed { line });
                }
                let f = floats(&tokens[2..5], line)?;
                declared = Vector3::new(f[0], f[1], f[2]);
                corners.clear();
            }
            Some("vertex") => {
                if tokens.len() < 4 {
                    return Err(MeshFileError::Malformed { line });
                }
                let f = floats(&tokens[1..4], line)?;
                corners.push(Vector3::new(f[0], f[1], f[2]));
            }
            Some("endfacet") => {
                if corners.len() != 3 {
                    return Err(MeshFileError::Malformed { line });
                }
                tris.push((declared, [corners[0], corners[1], corners[2]]));
                corners.clear();
            }
            _ => {}
        }
    }
    flat_triangles(&tris)
}

/// Builds an unshared mesh from STL triangles. A declared normal of zero (common in exporters) is replaced by the
/// normal of the winding, so the faces still shade.
fn flat_triangles(tris: &[StlTriangle]) -> Result<PrimitiveMesh, MeshFileError> {
    let mut positions: Vec<Vector3> = Vec::with_capacity(tris.len() * 3);
    let mut normals: Vec<Vector3> = Vec::with_capacity(tris.len() * 3);
    let mut indices: Vec<u32> = Vec::with_capacity(tris.len() * 3);
    for (declared, corners) in tris {
        let [a, b, c] = *corners;
        let winding = (b - a).cross(c - a).normalized();
        let n = if declared.length() > 0.0 { declared.normalized() } else { winding };
        for p in corners {
            indices.push(positions.len() as u32);
            positions.push(*p);
            normals.push(n);
        }
    }
    finish(positions, Some(normals), None, indices)
}

/// Reads ASCII PLY. The header must say `format ascii`; vertex `x y z` is required, with `nx ny nz` and `s t` (or
/// `u v`, `texture_u texture_v`) used when present. Faces are a list of vertex indices and are fan-triangulated.
pub fn read_ply(text: &str) -> Result<PrimitiveMesh, MeshFileError> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, "ply")) => {}
        _ => return Err(MeshFileError::Malformed { line: 1 }),
    }
    let mut format_ok = false;
    let mut ended = false;
    let mut current = "";
    let mut vertex_count = 0usize;
    let mut face_count = 0usize;
    let mut props: Vec<String> = Vec::new();
    while let Some((i, raw)) = lines.next() {
        let line = i + 1;
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        match tokens.as_slice() {
            ["end_header"] => {
                ended = true;
                break;
            }
            ["format", fmt, ..] => {
                if *fmt != "ascii" {
                    return Err(MeshFileError::Malformed { line });
                }
                format_ok = true;
            }
            ["element", name, count] => {
                let n: usize = count.parse().map_err(|_| MeshFileError::Malformed { line })?;
                match *name {
                    "vertex" => vertex_count = n,
                    "face" => face_count = n,
                    _ => {}
                }
                current = *name;
            }
            ["property", _, name] => {
                if current == "vertex" {
                    props.push(name.to_string());
                }
            }
            _ => {}
        }
    }
    if !format_ok {
        return Err(MeshFileError::Malformed { line: 1 });
    }
    if !ended {
        return Err(MeshFileError::Truncated);
    }
    let find = |name: &str| props.iter().position(|p| p == name);
    let (ix, iy, iz) = match (find("x"), find("y"), find("z")) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        _ => return Err(MeshFileError::Malformed { line: 1 }),
    };
    let normal_at = match (find("nx"), find("ny"), find("nz")) {
        (Some(a), Some(b), Some(c)) => Some((a, b, c)),
        _ => None,
    };
    let uv_at = match (
        find("s").or_else(|| find("u")).or_else(|| find("texture_u")),
        find("t").or_else(|| find("v")).or_else(|| find("texture_v")),
    ) {
        (Some(u), Some(v)) => Some((u, v)),
        _ => None,
    };

    let mut positions: Vec<Vector3> = Vec::with_capacity(vertex_count);
    let mut normals: Vec<Vector3> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    for _ in 0..vertex_count {
        let (i, raw) = lines.next().ok_or(MeshFileError::Truncated)?;
        let line = i + 1;
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        let f = floats(&tokens, line)?;
        let at = |k: usize| f.get(k).copied().ok_or(MeshFileError::Malformed { line });
        positions.push(Vector3::new(at(ix)?, at(iy)?, at(iz)?));
        if let Some((a, b, c)) = normal_at {
            normals.push(Vector3::new(at(a)?, at(b)?, at(c)?));
        }
        if let Some((u, v)) = uv_at {
            uvs.push([at(u)?, at(v)?]);
        }
    }

    let mut indices: Vec<u32> = Vec::new();
    for _ in 0..face_count {
        let (i, raw) = lines.next().ok_or(MeshFileError::Truncated)?;
        let line = i + 1;
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        let nums: Vec<usize> = tokens
            .iter()
            .map(|t| t.parse::<usize>().map_err(|_| MeshFileError::Malformed { line }))
            .collect::<Result<_, _>>()?;
        let count = *nums.first().ok_or(MeshFileError::Malformed { line })?;
        if count < 3 || nums.len() < 1 + count {
            return Err(MeshFileError::Malformed { line });
        }
        let corners = &nums[1..1 + count];
        if corners.iter().any(|&c| c >= positions.len()) {
            return Err(MeshFileError::IndexOutOfRange { line });
        }
        for k in 1..count - 1 {
            for &c in &[corners[0], corners[k], corners[k + 1]] {
                indices.push(c as u32);
            }
        }
    }
    let normals = if normal_at.is_some() { Some(normals) } else { None };
    let uvs = if uv_at.is_some() { Some(uvs) } else { None };
    finish(positions, normals, uvs, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn obj_quad_is_fan_triangulated_with_smooth_normals() {
        let text = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";
        let m = read_obj(text).unwrap();
        assert_eq!(m.positions.len(), 4);
        assert_eq!(m.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(m.texture_coordinates, vec![[0.0, 0.0]; 4]);
        // No vn in the file, so the normals come from the faces: all point to +z.
        for n in &m.normals {
            assert!(close(n.z, 1.0) && close(n.x, 0.0), "{:?}", n);
        }
    }

    #[test]
    fn obj_negative_indices_and_uvs_and_normals() {
        let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\n\
                    f -3/-3/-1 -2/-2/-1 -1/-1/-1\n";
        let m = read_obj(text).unwrap();
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert_eq!(m.texture_coordinates[1], [1.0, 0.0]);
        assert_eq!(m.texture_coordinates[2], [0.0, 1.0]);
        assert_eq!(m.normals, vec![Vector3::new(0.0, 0.0, 1.0); 3]);
    }

    #[test]
    fn obj_shared_corners_are_emitted_once() {
        let m = read_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3\nf 1 3 4\n").unwrap();
        assert_eq!(m.positions.len(), 4);
        assert_eq!(m.indices, vec![0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn obj_face_before_its_vertex_is_out_of_range() {
        let err = read_obj("v 0 0 0\nf 1 2 3\n").unwrap_err();
        assert_eq!(err, MeshFileError::IndexOutOfRange { line: 2 });
        assert_eq!(read_obj("v 0 0 0\n").unwrap_err(), MeshFileError::Empty);
    }

    #[test]
    fn obj_write_then_read_gives_the_same_mesh() {
        let original = PrimitiveMesh {
            positions: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(2.0, 0.0, 0.0), Vector3::new(0.0, 2.0, 0.0)],
            normals: vec![Vector3::new(0.0, 0.0, 1.0); 3],
            texture_coordinates: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
        };
        let text = write_obj(&original);
        assert!(text.contains("f 1/1/1 2/2/2 3/3/3"));
        assert_eq!(read_obj(&text).unwrap(), original);
    }

    fn stl_record(out: &mut Vec<u8>, n: [f32; 3], v: [[f32; 3]; 3]) {
        for x in n.iter().chain(v.iter().flatten()) {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }

    #[test]
    fn binary_stl_reads_one_triangle_with_its_declared_normal() {
        let mut bytes = vec![0u8; 80];
        bytes.extend_from_slice(&1u32.to_le_bytes());
        stl_record(&mut bytes, [0.0, 0.0, 1.0], [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert_eq!(bytes.len(), 134);
        let m = read_stl(&bytes).unwrap();
        assert_eq!(m.positions.len(), 3);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert_eq!(m.positions[1], Vector3::new(1.0, 0.0, 0.0));
        assert_eq!(m.normals, vec![Vector3::new(0.0, 0.0, 1.0); 3]);
    }

    #[test]
    fn ascii_stl_with_zero_normal_uses_the_winding() {
        let text = "solid t\nfacet normal 0 0 0\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\n\
                    endloop\nendfacet\nendsolid t\n";
        let m = read_stl(text.as_bytes()).unwrap();
        assert_eq!(m.indices, vec![0, 1, 2]);
        for n in &m.normals {
            assert!(close(n.z, 1.0) && close(n.x, 0.0), "{:?}", n);
        }
    }

    #[test]
    fn ascii_stl_with_a_short_facet_is_malformed() {
        let text = "solid t\nfacet normal 0 0 1\nvertex 0 0 0\nvertex 1 0 0\nendfacet\n";
        assert_eq!(read_stl(text.as_bytes()).unwrap_err(), MeshFileError::Malformed { line: 5 });
    }

    #[test]
    fn ply_quad_with_uvs_is_fan_triangulated() {
        let text = "ply\nformat ascii 1.0\ncomment square\nelement vertex 4\nproperty float x\nproperty float y\n\
                    property float z\nproperty float s\nproperty float t\nelement face 1\n\
                    property list uchar int vertex_indices\nend_header\n\
                    0 0 0 0 0\n1 0 0 1 0\n1 1 0 1 1\n0 1 0 0 1\n4 0 1 2 3\n";
        let m = read_ply(text).unwrap();
        assert_eq!(m.positions.len(), 4);
        assert_eq!(m.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(m.texture_coordinates[2], [1.0, 1.0]);
        assert_eq!(m.texture_coordinates[3], [0.0, 1.0]);
    }

    #[test]
    fn ply_rejects_binary_and_bad_face_indices() {
        let binary = "ply\nformat binary_little_endian 1.0\nend_header\n";
        assert_eq!(read_ply(binary).unwrap_err(), MeshFileError::Malformed { line: 2 });
        let bad = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\n\
                   element face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 5\n";
        assert_eq!(read_ply(bad).unwrap_err(), MeshFileError::IndexOutOfRange { line: 13 });
    }
}
