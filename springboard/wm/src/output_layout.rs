//! Arrangement of several outputs and mirroring: the geometry behind Settings > Displays (Arrange, Mirror Displays).
//! Pure data, host-tested. The DRM backend drives one connector today (`drm::open_gpu`), so these placements are not
//! applied to hardware yet: a second output needs its own CRTC and `Output` before a layout can be committed.

/// One output and its current size in pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    pub w: i32,
    pub h: i32,
}

/// Where one output sits in the desktop, in global pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Places the outputs in a row from left to right, tops aligned, in the order given. The first one starts at the origin.
pub fn extend_row(outputs: &[Output]) -> Vec<Placement> {
    let mut x = 0;
    outputs
        .iter()
        .map(|o| {
            let p = Placement { name: o.name.clone(), x, y: 0, w: o.w, h: o.h };
            x += o.w;
            p
        })
        .collect()
}

/// Mirrors every output on the source's origin: all of them show the same content, each at its own size. Errors when
/// `source` is not one of the outputs.
pub fn mirror(outputs: &[Output], source: &str) -> Result<Vec<Placement>, String> {
    if !outputs.iter().any(|o| o.name == source) {
        return Err(format!("mirror: no output {source}"));
    }
    Ok(outputs.iter().map(|o| Placement { name: o.name.clone(), x: 0, y: 0, w: o.w, h: o.h }).collect())
}

/// Whether two placements share any pixel.
pub fn overlaps(a: &Placement, b: &Placement) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Checks an extended arrangement: every output listed once, with a size, and no two outputs overlapping.
pub fn check(placements: &[Placement]) -> Result<(), String> {
    for (i, a) in placements.iter().enumerate() {
        if a.w <= 0 || a.h <= 0 {
            return Err(format!("{}: no size", a.name));
        }
        if placements[..i].iter().any(|b| b.name == a.name) {
            return Err(format!("{}: listed twice", a.name));
        }
        if let Some(b) = placements[i + 1..].iter().find(|b| overlaps(a, b)) {
            return Err(format!("{} overlaps {}", a.name, b.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(name: &str, w: i32, h: i32) -> Output {
        Output { name: name.into(), w, h }
    }

    #[test]
    fn a_row_starts_at_the_origin_and_follows_the_order_given() {
        let row = extend_row(&[out("A", 1920, 1080), out("B", 1280, 800)]);
        assert_eq!((row[0].x, row[0].y), (0, 0));
        assert_eq!((row[1].x, row[1].y, row[1].w), (1920, 0, 1280));
        assert!(check(&row).is_ok());
    }

    #[test]
    fn overlapping_outputs_and_repeated_names_are_refused() {
        let a = Placement { name: "A".into(), x: 0, y: 0, w: 100, h: 100 };
        let b = Placement { name: "B".into(), x: 99, y: 50, w: 100, h: 100 };
        assert_eq!(check(&[a.clone(), b.clone()]), Err("A overlaps B".to_string()));
        let touching = Placement { name: "C".into(), x: 100, y: 0, w: 10, h: 10 };
        assert!(check(&[a.clone(), touching]).is_ok(), "edges that touch do not overlap");
        assert_eq!(check(&[a.clone(), Placement { name: "A".into(), x: 500, ..a.clone() }]), Err("A: listed twice".to_string()));
        assert_eq!(check(&[Placement { w: 0, ..a }]), Err("A: no size".to_string()));
    }

    #[test]
    fn mirroring_puts_every_output_on_the_origin_and_needs_a_known_source() {
        let outputs = [out("A", 1920, 1080), out("B", 1280, 800)];
        let m = mirror(&outputs, "B").unwrap();
        assert!(m.iter().all(|p| p.x == 0 && p.y == 0));
        assert_eq!((m[1].w, m[1].h), (1280, 800));
        assert_eq!(mirror(&outputs, "C"), Err("mirror: no output C".to_string()));
    }
}
