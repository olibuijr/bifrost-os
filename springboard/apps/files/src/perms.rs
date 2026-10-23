//! Edit permissions in Get Info: the Read, Write and Execute grid for Owner, Group and Others. A click on a cell
//! changes that mode bit with `chmod`, only for a file the person owns (or as root), and one Undo puts the old mode back.
//! The owner and group stay read-only: changing them needs a privileged helper (`chown`), which Files does not have.
use crate::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// Whose permissions a row of the grid is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Who {
    Owner,
    Group,
    Others,
}

/// What a column of the grid allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
    Execute,
}

impl Who {
    pub(crate) const ALL: [Who; 3] = [Who::Owner, Who::Group, Who::Others];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Who::Owner => "Owner",
            Who::Group => "Group",
            Who::Others => "Everyone else",
        }
    }
}

impl Access {
    pub(crate) const ALL: [Access; 3] = [Access::Read, Access::Write, Access::Execute];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Access::Read => "Read",
            Access::Write => "Write",
            Access::Execute => "Execute",
        }
    }
}

/// The mode bit of one cell (for example Group + Write is 0o020).
pub(crate) fn bit(who: Who, access: Access) -> u32 {
    let class = match access {
        Access::Read => 0o4,
        Access::Write => 0o2,
        Access::Execute => 0o1,
    };
    class
        << match who {
            Who::Owner => 6,
            Who::Group => 3,
            Who::Others => 0,
        }
}

/// `mode` with one cell turned on or off. The set-user-ID, set-group-ID and sticky bits are kept; the file type bits
/// are dropped.
pub(crate) fn toggled(mode: u32, who: Who, access: Access) -> u32 {
    (mode & 0o7777) ^ bit(who, access)
}

/// True when the cell is on in `mode`.
pub(crate) fn allows(mode: u32, who: Who, access: Access) -> bool {
    mode & bit(who, access) != 0
}

/// The user id this process runs as (the owner of `/proc/self`).
pub(crate) fn current_uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// Whether the person may change the mode of a file owned by `file_uid`: its owner, or root.
pub(crate) fn can_edit(file_uid: u32, me: Option<u32>) -> bool {
    me.is_some_and(|me| me == 0 || me == file_uid)
}

/// The permission bits of `path` (the twelve mode bits), following links.
#[cfg(test)]
pub(crate) fn mode_of(path: &Path) -> std::io::Result<u32> {
    Ok(std::fs::metadata(path)?.permissions().mode() & 0o7777)
}

/// Sets the permission bits of `path` to `mode` (only the twelve mode bits are used).
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
}

/// The grid's geometry in points: a label column, three check columns, and a heading row above three rows.
pub(crate) const CELL_W: f32 = 76.;
pub(crate) const CELL_H: f32 = 26.;
const LABEL_W: f32 = 130.;

/// Total height of the grid (heading plus three rows).
pub(crate) const GRID_H: f32 = CELL_H * 4.;

/// Where cell (`who`, `access`) is, as (x, y, w, h), for a grid whose heading row starts at (`x`, `y`).
pub(crate) fn cell_rect(x: f32, y: f32, who: Who, access: Access) -> (f32, f32, f32, f32) {
    let row = Who::ALL.iter().position(|w| *w == who).unwrap_or(0) as f32;
    let col = Access::ALL.iter().position(|a| *a == access).unwrap_or(0) as f32;
    (x + LABEL_W + col * CELL_W, y + CELL_H + row * CELL_H, CELL_W, CELL_H)
}

/// The cell under the point (`px`, `py`) for a grid at (`x`, `y`), or None.
pub(crate) fn cell_at(x: f32, y: f32, px: f32, py: f32) -> Option<(Who, Access)> {
    for who in Who::ALL {
        for access in Access::ALL {
            let (cx, cy, cw, ch) = cell_rect(x, y, who, access);
            if px >= cx && px < cx + cw && py >= cy && py < cy + ch {
                return Some((who, access));
            }
        }
    }
    None
}

impl Files {
    /// Top-left of the permissions grid in the Get Info panel (below the info rows), when the item has a mode.
    fn perm_grid_origin(&self) -> Option<(f32, f32)> {
        let info = self.info.as_ref()?;
        info.mode?;
        let (x, y) = self.info_origin();
        Some((x + 20., y + 56. + info.rows.len() as f32 * 26. + 14.))
    }
    /// A click while Get Info is open: a cell of the permissions grid changes the mode (true); anything else is not ours.
    pub(crate) fn info_click(&mut self, x: f32, y: f32) -> bool {
        let Some((gx, gy)) = self.perm_grid_origin() else { return false };
        if cell_at(gx, gy, x, y).is_none() {
            return false;
        }
        let (who, access) = cell_at(gx, gy, x, y).unwrap_or((Who::Owner, Access::Read));
        let Some(info) = self.info.as_ref() else { return false };
        if !info.editable {
            self.notice = Some("Only the owner can change the permissions".into());
            self.revision += 1;
            return true;
        }
        let (path, old) = (info.path.clone(), info.mode.unwrap_or(0));
        match set_mode(&path, toggled(old, who, access)) {
            Ok(()) => {
                self.push_undo(Undo::Mode(vec![(path.clone(), old)]));
                self.refresh_info_perms(&path);
            }
            Err(e) => self.notice = Some(batch::friendly(&e)),
        }
        self.revision += 1;
        true
    }
    /// Re-reads the mode of the open Get Info item after a change (or an undo), so the grid and the Permissions row follow.
    pub(crate) fn refresh_info_perms(&mut self, path: &Path) {
        let Some(info) = self.info.as_mut() else { return };
        if info.path != path {
            return;
        }
        if let Ok(st) = bifrost_app::file_stat::stat(path) {
            let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
            let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
            for (label, value) in permission_rows(&st, &passwd, &group) {
                if let Some(row) = info.rows.iter_mut().find(|r| r.0 == label) {
                    row.1 = value;
                }
            }
            info.mode = Some(st.permissions().bits() & 0o7777);
        }
    }
    /// Paints the grid under the Get Info rows: column headings, a label per row, and a check box per cell.
    pub(crate) fn paint_perm_grid(&self, ui: &Ui, s: &mut Surface) {
        let (Some(info), Some((gx, gy))) = (self.info.as_ref(), self.perm_grid_origin()) else { return };
        let Some(mode) = info.mode else { return };
        for (c, access) in Access::ALL.into_iter().enumerate() {
            text(s, ui, access.label(), 12., LABEL_2.now(), gx + LABEL_W + c as f32 * CELL_W, gy + 4., CELL_W);
        }
        let on = if info.editable { TINT } else { LABEL_2.now() };
        for (r, who) in Who::ALL.into_iter().enumerate() {
            text(s, ui, who.label(), 12., LABEL_2.now(), gx, gy + CELL_H + r as f32 * CELL_H + 4., LABEL_W - 8.);
            for access in Access::ALL {
                let (bx, by, ..) = cell_rect(gx, gy, who, access);
                let by = by + 4.;
                s.fill_rrect(ui.pt(bx), ui.pt(by), ui.pt(SPACE_XL), ui.pt(SPACE_XL), ui.pt(SPACE_XS), SEPARATOR_C.now());
                if allows(mode, who, access) {
                    s.fill_rrect(ui.pt(bx + 2.), ui.pt(by + 2.), ui.pt(SPACE_XL - 4.), ui.pt(SPACE_XL - 4.), ui.pt(SPACE_XS), on);
                } else {
                    s.fill_rrect(ui.pt(bx + 1.), ui.pt(by + 1.), ui.pt(SPACE_XL - 2.), ui.pt(SPACE_XL - 2.), ui.pt(SPACE_XS), CARD_BG.now());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-perms-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn each_cell_has_its_own_bit() {
        assert_eq!(bit(Who::Owner, Access::Read), 0o400);
        assert_eq!(bit(Who::Owner, Access::Write), 0o200);
        assert_eq!(bit(Who::Group, Access::Execute), 0o010);
        assert_eq!(bit(Who::Others, Access::Read), 0o004);
        let all: u32 = Who::ALL.iter().flat_map(|w| Access::ALL.iter().map(move |a| bit(*w, *a))).sum();
        assert_eq!(all, 0o777, "nine distinct bits");
    }

    #[test]
    fn toggling_twice_restores_the_mode_and_keeps_special_bits() {
        let mode = 0o4755;
        let once = toggled(mode, Who::Group, Access::Write);
        assert_eq!(once, 0o4775);
        assert_eq!(toggled(once, Who::Group, Access::Write), mode);
        assert_eq!(toggled(0o100000 | 0o1777, Who::Others, Access::Write), 0o1775, "file type bits never leak into the mode");
        assert!(allows(0o640, Who::Owner, Access::Write));
        assert!(!allows(0o640, Who::Others, Access::Read));
    }

    #[test]
    fn only_the_owner_or_root_may_edit() {
        assert!(can_edit(1000, Some(1000)));
        assert!(can_edit(1000, Some(0)));
        assert!(!can_edit(1000, Some(1001)));
        assert!(!can_edit(1000, None));
    }

    #[test]
    fn the_grid_hit_test_finds_each_cell_and_rejects_the_margins() {
        for who in Who::ALL {
            for access in Access::ALL {
                let (x, y, w, h) = cell_rect(20., 100., who, access);
                assert_eq!(cell_at(20., 100., x + w / 2., y + h / 2.), Some((who, access)));
            }
        }
        assert_eq!(cell_at(20., 100., 21., 101. + CELL_H), None, "the label column");
        assert_eq!(cell_at(20., 100., 20. + LABEL_W + 5., 100. + 5.), None, "the heading row");
        assert_eq!(cell_at(20., 100., 20. + LABEL_W + 3. * CELL_W + 1., 100. + CELL_H + 1.), None, "right of the grid");
        assert_eq!(cell_at(20., 100., 20. + LABEL_W + 1., 100. + GRID_H + 1.), None, "below the grid");
    }

    #[test]
    fn set_mode_writes_the_bits_and_undo_restores_them() {
        let d = scratch("chmod");
        let f = d.join("f");
        std::fs::write(&f, "x").unwrap();
        set_mode(&f, 0o640).unwrap();
        assert_eq!(mode_of(&f).unwrap(), 0o640);
        let old = mode_of(&f).unwrap();
        set_mode(&f, toggled(old, Who::Others, Access::Read)).unwrap();
        assert_eq!(mode_of(&f).unwrap(), 0o644);
        undo_step(&Undo::Mode(vec![(f.clone(), 0o640)]), &d).unwrap();
        assert_eq!(mode_of(&f).unwrap(), 0o640);
        assert!(undo_step(&Undo::Mode(vec![(d.join("gone"), 0o600)]), &d).is_err());
    }

    #[test]
    fn the_current_user_owns_a_file_it_just_made() {
        let d = scratch("uid");
        let f = d.join("f");
        std::fs::write(&f, "x").unwrap();
        assert!(can_edit(std::fs::metadata(&f).unwrap().uid(), current_uid()));
    }
}
