//! Volumetric voxel grids for 3D objects (`NativeVoxelArray`, `NativeVoxelIndex`, `NativeVoxelIndexExtent`).
//! A voxel array holds the solid volume of an object as a set of cubic units on an integer grid. Voxel `(i, j, k)`
//! occupies the cube whose minimum corner is `bounds.min + (i, j, k) * voxel_extent`. Solid operations (union,
//! intersection, difference) combine two arrays only when they share one grid. Shells, signed distance fields and
//! turning the volume into a mesh are not modelled here.

use crate::linalg::Vector3;
use std::collections::BTreeSet;

/// The position of one voxel on the grid (`NativeVoxelIndex`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoxelIndex {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl VoxelIndex {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        VoxelIndex { x, y, z }
    }
}

/// Inclusive corners of a box of voxels (`NativeVoxelIndexExtent`). The box is empty when a maximum is below its minimum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelIndexExtent {
    pub minimum: VoxelIndex,
    pub maximum: VoxelIndex,
}

impl VoxelIndexExtent {
    /// Number of grid cells in the box: 0 when it is empty.
    pub fn count(&self) -> usize {
        let axis = |lo: i32, hi: i32| if hi < lo { 0 } else { (i64::from(hi) - i64::from(lo) + 1) as usize };
        axis(self.minimum.x, self.maximum.x) * axis(self.minimum.y, self.maximum.y) * axis(self.minimum.z, self.maximum.z)
    }

    pub fn contains(&self, i: VoxelIndex) -> bool {
        i.x >= self.minimum.x
            && i.x <= self.maximum.x
            && i.y >= self.minimum.y
            && i.y <= self.maximum.y
            && i.z >= self.minimum.z
            && i.z <= self.maximum.z
    }
}

/// An axis-aligned box in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelBounds {
    pub min: Vector3,
    pub max: Vector3,
}

impl VoxelBounds {
    /// True when every minimum is at or below its maximum. NaN coordinates are not valid.
    pub fn is_valid(&self) -> bool {
        self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z
    }
}

/// A solid volume as a set of set voxels (`NativeVoxelArray`).
#[derive(Clone, Debug, PartialEq)]
pub struct VoxelArray {
    bounds: VoxelBounds,
    voxel_extent: f32,
    voxels: BTreeSet<VoxelIndex>,
}

impl VoxelArray {
    /// An empty array over `bounds` with cubic voxels of edge `voxel_extent`. `None` when the bounds are inverted or
    /// invalid, or the edge is not a positive finite number.
    pub fn new(bounds: VoxelBounds, voxel_extent: f32) -> Option<Self> {
        if !bounds.is_valid() || !voxel_extent.is_finite() || voxel_extent <= 0.0 {
            return None;
        }
        Some(VoxelArray { bounds, voxel_extent, voxels: BTreeSet::new() })
    }

    pub fn bounds(&self) -> VoxelBounds {
        self.bounds
    }

    pub fn voxel_extent(&self) -> f32 {
        self.voxel_extent
    }

    /// Number of voxels set.
    pub fn count(&self) -> usize {
        self.voxels.len()
    }

    pub fn voxel_exists(&self, i: VoxelIndex) -> bool {
        self.voxels.contains(&i)
    }

    /// Sets a voxel. Setting one that is already set changes nothing.
    pub fn set_voxel(&mut self, i: VoxelIndex) {
        self.voxels.insert(i);
    }

    /// Every set voxel, in index order (by x, then y, then z).
    pub fn voxel_indices(&self) -> Vec<VoxelIndex> {
        self.voxels.iter().copied().collect()
    }

    /// The set voxels inside `extent`, in index order.
    pub fn voxels_within(&self, extent: VoxelIndexExtent) -> Vec<VoxelIndex> {
        self.voxels.iter().copied().filter(|v| extent.contains(*v)).collect()
    }

    /// The smallest box of indices that holds every set voxel. `None` when no voxel is set.
    pub fn voxel_index_extent(&self) -> Option<VoxelIndexExtent> {
        let first = *self.voxels.iter().next()?;
        let mut ext = VoxelIndexExtent { minimum: first, maximum: first };
        for v in &self.voxels {
            ext.minimum = VoxelIndex::new(ext.minimum.x.min(v.x), ext.minimum.y.min(v.y), ext.minimum.z.min(v.z));
            ext.maximum = VoxelIndex::new(ext.maximum.x.max(v.x), ext.maximum.y.max(v.y), ext.maximum.z.max(v.z));
        }
        Some(ext)
    }

    /// The minimum corner of voxel `i` in world space.
    fn corner(&self, i: VoxelIndex) -> Vector3 {
        let m = self.bounds.min;
        let e = self.voxel_extent;
        Vector3::new(m.x + i.x as f32 * e, m.y + i.y as f32 * e, m.z + i.z as f32 * e)
    }

    /// The centre of voxel `i` in world space.
    pub fn spatial_location(&self, i: VoxelIndex) -> Vector3 {
        let c = self.corner(i);
        let h = self.voxel_extent * 0.5;
        Vector3::new(c.x + h, c.y + h, c.z + h)
    }

    /// The world-space cube of voxel `i`.
    pub fn voxel_bounding_box(&self, i: VoxelIndex) -> VoxelBounds {
        let lo = self.corner(i);
        let e = self.voxel_extent;
        VoxelBounds { min: lo, max: Vector3::new(lo.x + e, lo.y + e, lo.z + e) }
    }

    /// The grid cell that contains world point `p`. `None` when `p` is below the minimum or at or beyond the maximum
    /// of the bounds.
    pub fn index_of_location(&self, p: Vector3) -> Option<VoxelIndex> {
        let m = self.bounds.min;
        let mx = self.bounds.max;
        let inside = p.x >= m.x
            && p.x < mx.x
            && p.y >= m.y
            && p.y < mx.y
            && p.z >= m.z
            && p.z < mx.z;
        if !inside {
            return None;
        }
        let e = self.voxel_extent;
        let cell = |v: f32, lo: f32| ((v - lo) / e).floor() as i32;
        Some(VoxelIndex::new(cell(p.x, m.x), cell(p.y, m.y), cell(p.z, m.z)))
    }

    /// True when both arrays use the same cell size and origin, so their indices name the same cells.
    fn same_grid(&self, other: &VoxelArray) -> bool {
        self.voxel_extent == other.voxel_extent && self.bounds.min == other.bounds.min
    }

    /// Sets every voxel of `other` as well, and grows the bounds to cover `other`'s. Returns `false` and changes nothing
    /// when the grids differ.
    pub fn union(&mut self, other: &VoxelArray) -> bool {
        if !self.same_grid(other) {
            return false;
        }
        self.voxels.extend(other.voxels.iter().copied());
        let o = other.bounds.max;
        let s = self.bounds.max;
        self.bounds.max = Vector3::new(s.x.max(o.x), s.y.max(o.y), s.z.max(o.z));
        true
    }

    /// Keeps only the voxels that both arrays set. Returns `false` and changes nothing when the grids differ. The bounds
    /// are kept as they were.
    pub fn intersect(&mut self, other: &VoxelArray) -> bool {
        if !self.same_grid(other) {
            return false;
        }
        self.voxels.retain(|v| other.voxels.contains(v));
        true
    }

    /// Removes the voxels that `other` sets. Returns `false` and changes nothing when the grids differ. The bounds are
    /// kept as they were.
    pub fn difference(&mut self, other: &VoxelArray) -> bool {
        if !self.same_grid(other) {
            return false;
        }
        self.voxels.retain(|v| !other.voxels.contains(v));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: i32, y: i32, z: i32) -> VoxelIndex {
        VoxelIndex::new(x, y, z)
    }

    fn cube(min: f32, max: f32) -> VoxelBounds {
        VoxelBounds { min: Vector3::new(min, min, min), max: Vector3::new(max, max, max) }
    }

    fn array(max: f32, extent: f32, set: &[VoxelIndex]) -> VoxelArray {
        let mut a = VoxelArray::new(cube(0.0, max), extent).unwrap();
        for &i in set {
            a.set_voxel(i);
        }
        a
    }

    #[test]
    fn new_rejects_bad_bounds_and_edges() {
        assert!(VoxelArray::new(cube(0.0, 10.0), 0.0).is_none());
        assert!(VoxelArray::new(cube(0.0, 10.0), -1.0).is_none());
        assert!(VoxelArray::new(cube(0.0, 10.0), f32::NAN).is_none());
        assert!(VoxelArray::new(cube(5.0, 1.0), 1.0).is_none());
        assert!(VoxelArray::new(cube(0.0, 10.0), 1.0).is_some());
    }

    #[test]
    fn setting_a_voxel_twice_counts_once() {
        let a = array(10.0, 1.0, &[v(0, 0, 0), v(1, 0, 0), v(1, 0, 0)]);
        assert_eq!(a.count(), 2);
        assert!(a.voxel_exists(v(1, 0, 0)));
        assert!(!a.voxel_exists(v(0, 1, 0)));
    }

    #[test]
    fn index_extent_is_the_box_around_the_set_voxels() {
        let a = array(10.0, 1.0, &[v(-1, 2, 0), v(3, 2, 1)]);
        let e = a.voxel_index_extent().unwrap();
        assert_eq!(e.minimum, v(-1, 2, 0));
        assert_eq!(e.maximum, v(3, 2, 1));
        // x spans 5 cells, y 1, z 2: 10 cells in the box.
        assert_eq!(e.count(), 10);
        assert!(array(10.0, 1.0, &[]).voxel_index_extent().is_none());
    }

    #[test]
    fn empty_extent_has_no_cells() {
        let e = VoxelIndexExtent { minimum: v(1, 0, 0), maximum: v(0, 0, 0) };
        assert_eq!(e.count(), 0);
        assert!(!e.contains(v(0, 0, 0)));
    }

    #[test]
    fn voxels_are_listed_in_index_order_and_filtered_by_extent() {
        let a = array(10.0, 1.0, &[v(2, 0, 0), v(0, 0, 0), v(0, 1, 0)]);
        assert_eq!(a.voxel_indices(), vec![v(0, 0, 0), v(0, 1, 0), v(2, 0, 0)]);
        let within = VoxelIndexExtent { minimum: v(0, 0, 0), maximum: v(0, 1, 0) };
        assert_eq!(a.voxels_within(within), vec![v(0, 0, 0), v(0, 1, 0)]);
    }

    #[test]
    fn spatial_location_is_the_cell_centre_and_the_box_is_the_cell() {
        // Origin at (0, 0, 0), cells 2 units wide. Index (1, 0, 0) spans x 2..4.
        let a = array(10.0, 2.0, &[]);
        assert_eq!(a.spatial_location(v(1, 0, 0)), Vector3::new(3.0, 1.0, 1.0));
        let b = a.voxel_bounding_box(v(1, 0, 0));
        assert_eq!(b.min, Vector3::new(2.0, 0.0, 0.0));
        assert_eq!(b.max, Vector3::new(4.0, 2.0, 2.0));
    }

    #[test]
    fn index_of_location_maps_points_to_cells_inside_the_bounds() {
        let a = array(10.0, 2.0, &[]);
        assert_eq!(a.index_of_location(Vector3::new(5.0, 0.5, 3.9)), Some(v(2, 0, 1)));
        assert_eq!(a.index_of_location(Vector3::new(0.0, 0.0, 0.0)), Some(v(0, 0, 0)));
        // The maximum face is outside, and so is anything below the minimum.
        assert_eq!(a.index_of_location(Vector3::new(10.0, 0.0, 0.0)), None);
        assert_eq!(a.index_of_location(Vector3::new(-0.5, 0.0, 0.0)), None);
    }

    #[test]
    fn union_adds_voxels_and_grows_the_bounds() {
        let mut a = array(10.0, 1.0, &[v(0, 0, 0)]);
        let b = {
            let mut b = VoxelArray::new(cube(0.0, 20.0), 1.0).unwrap();
            b.set_voxel(v(1, 0, 0));
            b
        };
        assert!(a.union(&b));
        assert_eq!(a.count(), 2);
        assert_eq!(a.bounds().max.x, 20.0);
    }

    #[test]
    fn different_grids_refuse_the_operation_and_change_nothing() {
        let mut a = array(10.0, 1.0, &[v(0, 0, 0)]);
        // Same cell size but a shifted origin, so index (0, 0, 0) names a different place.
        let shifted = VoxelArray::new(
            VoxelBounds { min: Vector3::new(1.0, 0.0, 0.0), max: Vector3::new(11.0, 10.0, 10.0) },
            1.0,
        )
        .unwrap();
        assert!(!a.union(&shifted));
        assert!(!a.intersect(&shifted));
        assert!(!a.difference(&shifted));
        assert_eq!(a.voxel_indices(), vec![v(0, 0, 0)]);
    }

    #[test]
    fn intersect_and_difference_split_the_set_voxels() {
        let a = array(10.0, 1.0, &[v(0, 0, 0), v(1, 0, 0), v(2, 0, 0)]);
        let b = array(10.0, 1.0, &[v(1, 0, 0), v(5, 5, 5)]);
        let mut both = a.clone();
        assert!(both.intersect(&b));
        assert_eq!(both.voxel_indices(), vec![v(1, 0, 0)]);
        let mut only_a = a.clone();
        assert!(only_a.difference(&b));
        assert_eq!(only_a.voxel_indices(), vec![v(0, 0, 0), v(2, 0, 0)]);
    }
}
