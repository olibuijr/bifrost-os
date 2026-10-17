//! Vertex displacement from a height sample: the vertex-stage half of a displacement map, as plain std code. A vertex
//! moves along its unit normal by `height * scale + bias`, with the height sample clamped to 0..=1. This is the CPU
//! model for a displacement evaluation; it does not read textures.

use crate::linalg::Vector3;

/// The displaced position of one vertex. A zero normal leaves the vertex where it is.
pub fn displace(vertex: Vector3, normal: Vector3, height: f32, scale: f32, bias: f32) -> Vector3 {
    if normal.length() == 0.0 {
        return vertex;
    }
    let distance = height.clamp(0.0, 1.0) * scale + bias;
    vertex + normal.normalized() * distance
}

/// Displaces every vertex in place. Returns false, and changes nothing, when the three slices differ in length.
pub fn displace_all(vertices: &mut [Vector3], normals: &[Vector3], heights: &[f32], scale: f32, bias: f32) -> bool {
    if vertices.len() != normals.len() || vertices.len() != heights.len() {
        return false;
    }
    for ((v, n), h) in vertices.iter_mut().zip(normals).zip(heights) {
        *v = displace(*v, *n, *h, scale, bias);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn zero_height_and_zero_bias_keep_the_vertex() {
        let v = Vector3::new(1.0, 2.0, 3.0);
        assert_eq!(displace(v, Vector3::new(0.0, 1.0, 0.0), 0.0, 4.0, 0.0), v);
    }

    #[test]
    fn full_height_moves_along_the_unit_normal_by_scale_plus_bias() {
        // Normal (0, 0, 3) normalises to (0, 0, 1); height 1 * scale 2 + bias 0.5 = 2.5.
        let p = displace(Vector3::ZERO, Vector3::new(0.0, 0.0, 3.0), 1.0, 2.0, 0.5);
        assert!(near(p.x, 0.0) && near(p.y, 0.0) && near(p.z, 2.5));
    }

    #[test]
    fn heights_are_clamped_to_the_unit_range() {
        let p = displace(Vector3::ZERO, Vector3::new(1.0, 0.0, 0.0), 7.0, 1.0, 0.0);
        assert!(near(p.x, 1.0));
    }

    #[test]
    fn zero_normal_is_left_alone() {
        let v = Vector3::new(4.0, 5.0, 6.0);
        assert_eq!(displace(v, Vector3::ZERO, 1.0, 1.0, 1.0), v);
    }

    #[test]
    fn displace_all_refuses_mismatched_lengths() {
        let mut v = vec![Vector3::ZERO; 2];
        assert!(!displace_all(&mut v, &[Vector3::new(0.0, 0.0, 1.0)], &[1.0, 1.0], 1.0, 0.0));
        assert!(displace_all(&mut v, &[Vector3::new(0.0, 0.0, 1.0); 2], &[1.0, 0.0], 1.0, 0.0));
        assert!(near(v[0].z, 1.0) && near(v[1].z, 0.0));
    }
}
