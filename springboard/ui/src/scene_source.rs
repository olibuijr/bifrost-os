//! Scene asset import: reads Wavefront OBJ text into triangle meshes, one per object (`o`) in the file.
//! It is the text-mesh counterpart of the scene framework's scene source, which loads scene entries by identifier.
//! Pure: the caller passes the file text and the result owns its data. Nothing is read from disk here.
//!
//! Supported lines: `v` (position), `vt` (texture coordinate, `u v` with an optional `w`), `vn` (normal), `f` (a face
//! with three or more corners in `v`, `v/vt`, `v//vn` or `v/vt/vn` form) and `o` (starts a new object). Other lines,
//! including `g`, `s`, `mtllib` and `usemtl`, are read and ignored. Indices are 1-based; a negative index counts back
//! from the last vertex defined so far. Faces with more than three corners are fan-triangulated.
//! A corner without a normal takes the face normal (Newell's method), so such faces are flat-shaded. Each unique
//! (position, texture, normal) combination becomes one output vertex.

use crate::linalg::Vector3;
use std::collections::HashMap;

/// Why a file could not be imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// A number is malformed, a vertex line has too few coordinates, or a face has fewer than three corners.
    /// `line` is 1-based.
    Syntax { line: usize },
    /// A face refers to a position, texture coordinate or normal that does not exist, or uses index 0.
    Index { line: usize },
    /// The file has no faces, so there is nothing to import.
    NoFaces,
}

/// One object's triangles: parallel vertex arrays, and indices in triples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedMesh {
    pub name: String,
    pub positions: Vec<Vector3>,
    pub normals: Vec<Vector3>,
    pub texture_coordinates: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl ImportedMesh {
    /// Number of triangles (the index count divided by three).
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A face corner: zero-based position index, texture index and normal index.
type Corner = (usize, Option<usize>, Option<usize>);

struct Object {
    name: String,
    faces: Vec<Vec<Corner>>,
}

/// Parses OBJ text into one mesh per object that has faces. Objects without faces are dropped.
pub fn import_obj(text: &str) -> Result<Vec<ImportedMesh>, ImportError> {
    let mut positions: Vec<Vector3> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut normals: Vec<Vector3> = Vec::new();
    let mut objects: Vec<Object> = Vec::new();
    let mut current = Object { name: String::new(), faces: Vec::new() };

    for (number, raw) in text.lines().enumerate() {
        let line = number + 1;
        let content = raw.split('#').next().unwrap_or("");
        let mut parts = content.split_whitespace();
        let Some(keyword) = parts.next() else {
            continue;
        };
        let rest: Vec<&str> = parts.collect();
        match keyword {
            "v" => {
                if rest.len() < 3 {
                    return Err(ImportError::Syntax { line });
                }
                positions.push(Vector3::new(number_at(rest[0], line)?, number_at(rest[1], line)?, number_at(rest[2], line)?));
            }
            "vt" => {
                if rest.len() < 2 {
                    return Err(ImportError::Syntax { line });
                }
                uvs.push([number_at(rest[0], line)?, number_at(rest[1], line)?]);
            }
            "vn" => {
                if rest.len() < 3 {
                    return Err(ImportError::Syntax { line });
                }
                normals.push(Vector3::new(number_at(rest[0], line)?, number_at(rest[1], line)?, number_at(rest[2], line)?));
            }
            "o" => {
                let finished = std::mem::replace(&mut current, Object { name: rest.join(" "), faces: Vec::new() });
                if !finished.faces.is_empty() {
                    objects.push(finished);
                }
            }
            "f" => {
                if rest.len() < 3 {
                    return Err(ImportError::Syntax { line });
                }
                let mut face = Vec::with_capacity(rest.len());
                for token in &rest {
                    face.push(corner(token, positions.len(), uvs.len(), normals.len(), line)?);
                }
                current.faces.push(face);
            }
            _ => {}
        }
    }
    if !current.faces.is_empty() {
        objects.push(current);
    }
    if objects.is_empty() {
        return Err(ImportError::NoFaces);
    }
    Ok(objects.iter().map(|object| build(object, &positions, &uvs, &normals)).collect())
}

fn number_at(token: &str, line: usize) -> Result<f32, ImportError> {
    token
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or(ImportError::Syntax { line })
}

fn corner(token: &str, positions: usize, uvs: usize, normals: usize, line: usize) -> Result<Corner, ImportError> {
    let mut fields = token.split('/');
    let v = resolve(fields.next().unwrap_or(""), positions, line)?;
    let t = match fields.next() {
        None | Some("") => None,
        Some(field) => Some(resolve(field, uvs, line)?),
    };
    let n = match fields.next() {
        None | Some("") => None,
        Some(field) => Some(resolve(field, normals, line)?),
    };
    Ok((v, t, n))
}

/// Turns a 1-based (or negative, counting back from the end) OBJ index into a zero-based one.
fn resolve(field: &str, count: usize, line: usize) -> Result<usize, ImportError> {
    let index: i64 = field.parse().map_err(|_| ImportError::Syntax { line })?;
    let count = count as i64;
    let zero_based = if index > 0 {
        index - 1
    } else if index < 0 {
        count + index
    } else {
        return Err(ImportError::Index { line });
    };
    if zero_based < 0 || zero_based >= count {
        return Err(ImportError::Index { line });
    }
    Ok(zero_based as usize)
}

/// Newell's method: the unit normal of a polygon, from the sum over its edges. Right-hand winding gives an outward normal.
fn newell_normal(face: &[Corner], positions: &[Vector3]) -> Vector3 {
    let mut n = Vector3::ZERO;
    for i in 0..face.len() {
        let p = positions[face[i].0];
        let q = positions[face[(i + 1) % face.len()].0];
        n.x += (p.y - q.y) * (p.z + q.z);
        n.y += (p.z - q.z) * (p.x + q.x);
        n.z += (p.x - q.x) * (p.y + q.y);
    }
    n.normalized()
}

fn build(object: &Object, positions: &[Vector3], uvs: &[[f32; 2]], normals: &[Vector3]) -> ImportedMesh {
    let mut mesh = ImportedMesh { name: object.name.clone(), ..ImportedMesh::default() };
    let mut seen: HashMap<(usize, Option<usize>, Option<usize>, usize), u32> = HashMap::new();
    for (face_number, face) in object.faces.iter().enumerate() {
        let face_normal = newell_normal(face, positions);
        let mut indices = Vec::with_capacity(face.len());
        for &(v, t, n) in face {
            // A corner without a normal is flat-shaded, so its key carries the face number and is never shared.
            let face_key = if n.is_none() { face_number } else { usize::MAX };
            let key = (v, t, n, face_key);
            let existing = seen.get(&key).copied();
            let index = match existing {
                Some(index) => index,
                None => {
                    let index = mesh.positions.len() as u32;
                    mesh.positions.push(positions[v]);
                    mesh.normals.push(match n {
                        Some(n) => normals[n],
                        None => face_normal,
                    });
                    mesh.texture_coordinates.push(match t {
                        Some(t) => uvs[t],
                        None => [0.0, 0.0],
                    });
                    seen.insert(key, index);
                    index
                }
            };
            indices.push(index);
        }
        // Faces have at least three corners (checked while parsing), so this range never underflows.
        for k in 1..indices.len() - 1 {
            mesh.indices.extend_from_slice(&[indices[0], indices[k], indices[k + 1]]);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vector3, b: Vector3) -> bool {
        (a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5 && (a.z - b.z).abs() < 1e-5
    }

    #[test]
    fn one_triangle_gets_a_flat_normal_from_its_winding() {
        let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let meshes = import_obj(text).unwrap();
        assert_eq!(meshes.len(), 1);
        let mesh = &meshes[0];
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert_eq!(mesh.triangle_count(), 1);
        // Counter-clockwise seen from +z, so the normal is +z.
        assert!(close(mesh.normals[0], Vector3::new(0.0, 0.0, 1.0)));
    }

    #[test]
    fn a_quad_is_fan_triangulated_into_two_triangles() {
        let text = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";
        let mesh = &import_obj(text).unwrap()[0];
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.triangle_count(), 2);
        assert!(close(mesh.normals[3], Vector3::new(0.0, 0.0, 1.0)));
    }

    #[test]
    fn negative_indices_count_back_from_the_last_vertex() {
        let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n";
        let mesh = &import_obj(text).unwrap()[0];
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert_eq!(mesh.positions[1], Vector3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn shared_corners_collapse_into_one_vertex_and_file_normals_are_kept() {
        let text = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
                    vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\n\
                    f 1/1/1 2/2/1 3/3/1\nf 1/1/1 3/3/1 4/4/1\n";
        let mesh = &import_obj(text).unwrap()[0];
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.texture_coordinates[2], [1.0, 1.0]);
        assert_eq!(mesh.normals[0], Vector3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn each_object_becomes_its_own_named_mesh() {
        let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\no first\nf 1 2 3\no second\nf 3 2 1\no empty\n";
        let meshes = import_obj(text).unwrap();
        assert_eq!(meshes.len(), 2);
        assert_eq!(meshes[0].name, "first");
        assert_eq!(meshes[1].name, "second");
        assert_eq!(meshes[1].indices, vec![0, 1, 2]);
    }

    #[test]
    fn malformed_and_out_of_range_input_is_refused_with_its_line() {
        assert_eq!(import_obj("v 0 0 0\nv 1 0 0\nf 1 2\n"), Err(ImportError::Syntax { line: 3 }));
        assert_eq!(import_obj("v 0 0 x\n"), Err(ImportError::Syntax { line: 1 }));
        assert_eq!(import_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n"), Err(ImportError::Index { line: 4 }));
        assert_eq!(import_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n"), Err(ImportError::Index { line: 4 }));
        assert_eq!(import_obj("v 0 0 0\n# only a comment\n"), Err(ImportError::NoFaces));
    }
}
