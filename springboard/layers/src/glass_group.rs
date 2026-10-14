//! Glass grouping for a glass effect container (`NSGlassEffectContainerView`): glass shapes that sit within a spacing of
//! each other join one group, so the container draws them as one material and they blend where they meet. Two shapes
//! are near when the larger of their horizontal and vertical gaps is at most the spacing; overlapping shapes have no
//! gap, and joining is transitive. Pure geometry.

use crate::geometry::Rect;

/// Groups `shapes` by proximity. Each group lists indices into `shapes` in ascending order, and the groups are listed by
/// their lowest index. A shape with a non-finite frame stands alone. A negative or non-finite spacing counts as zero.
pub fn group_shapes(shapes: &[Rect], spacing: f32) -> Vec<Vec<usize>> {
    let spacing = if spacing.is_finite() { spacing.max(0.0) } else { 0.0 };
    let mut assigned = vec![false; shapes.len()];
    let mut groups = Vec::new();
    for start in 0..shapes.len() {
        if assigned[start] {
            continue;
        }
        assigned[start] = true;
        let mut members = vec![start];
        let mut next = 0;
        while next < members.len() {
            let current = members[next];
            next += 1;
            for other in 0..shapes.len() {
                if !assigned[other] && joins(&shapes[current], &shapes[other], spacing) {
                    assigned[other] = true;
                    members.push(other);
                }
            }
        }
        members.sort_unstable();
        groups.push(members);
    }
    groups
}

fn joins(a: &Rect, b: &Rect, spacing: f32) -> bool {
    finite(a) && finite(b) && gap(a, b) <= spacing
}

fn finite(r: &Rect) -> bool {
    [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite())
}

/// The larger of the horizontal and vertical distances between two rectangles; zero where they overlap on that axis.
fn gap(a: &Rect, b: &Rect) -> f32 {
    let gap_x = (b.x - a.max_x()).max(a.x - b.max_x()).max(0.0);
    let gap_y = (b.y - a.max_y()).max(a.y - b.max_y()).max(0.0);
    gap_x.max(gap_y)
}
