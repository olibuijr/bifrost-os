//! Disk Utility as an installed BifrOSt app in the Golden Gate full-size window: the disks and volumes in a sidebar,
//! and an info pane with capacity, file system, mount point and SMART status. Mount, unmount and eject go through the
//! disk session in `bifrost-core`. The window is `window.rs`, painting is `draw.rs`, the actions are `actions.rs`, the
//! model and its parsers are `model.rs`, and the reads are `sysfs.rs` and `probe.rs`.

mod actions;
mod draw;
mod model;
mod probe;
mod sysfs;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind};
use crate::actions::Action;
use crate::model::Kind;
use bifrost_core::disk_format::Personality;
use window::{Ask, DiskUtility, Target, B_DETACH, REFRESH_S};

/// Menu item ids.
const MENU_DETACH: u32 = 1;
const MENU_MOUNT: u32 = 2;
const MENU_UNMOUNT: u32 = 3;
const MENU_EJECT: u32 = 4;
const MENU_INFO: u32 = 5;
const MENU_VOLUMES_ONLY: u32 = 6;
const MENU_ALL_DEVICES: u32 = 7;
const MENU_ERASE: u32 = 8;
const MENU_FIRST_AID: u32 = 9;
const MENU_FS_EXT4: u32 = 20;
const MENU_FS_EXFAT: u32 = 21;
const MENU_FS_FAT32: u32 = 22;
/// Partition into N equal parts: `MENU_PARTITION_BASE + N`.
const MENU_PARTITION_BASE: u32 = 30;

impl App for DiskUtility {
    fn id(&self) -> &str {
        "is.olibuijr.diskutil.desktop"
    }

    /// File, View, Disk and Image menus: the toolbar's actions with shortcuts, the sidebar mode, the erase format and
    /// the partition layouts.
    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let idle = self.job.is_none();
        let node = self.selected_node();
        let mountable = idle && node.is_some_and(|n| n.mount_point.is_none() && n.fs_type.is_some());
        let mounted = idle && node.is_some_and(|n| n.mount_point.is_some());
        let ejectable = idle && node.is_some_and(|n| n.kind == Kind::Disk && n.removable);
        let has = idle && node.is_some();
        let whole_disk = idle && node.is_some_and(|n| n.kind == Kind::Disk);
        let fs = self.erase_fs;
        let mut disk = vec![
            AppMenuItem::new(MENU_ERASE, "Erase\u{2026}").key("cmd+shift+e").enabled(has),
            AppMenuItem::new(MENU_FIRST_AID, "Run First Aid\u{2026}").key("cmd+shift+a").enabled(has),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_FS_EXT4, "Format as ext4").checked(fs == Personality::Ext4),
            AppMenuItem::new(MENU_FS_EXFAT, "Format as exFAT").checked(fs == Personality::ExFat),
            AppMenuItem::new(MENU_FS_FAT32, "Format as FAT32").checked(fs == Personality::Fat32),
            AppMenuItem::separator(),
        ];
        for n in 2..=4u32 {
            disk.push(AppMenuItem::new(MENU_PARTITION_BASE + n, &format!("Partition into {n} Equal Parts\u{2026}")).enabled(whole_disk));
        }
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_INFO, "Get Info").key("cmd+i").enabled(node.is_some()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_MOUNT, "Mount").key("cmd+m").enabled(mountable),
                    AppMenuItem::new(MENU_UNMOUNT, "Unmount").key("cmd+u").enabled(mounted),
                    AppMenuItem::new(MENU_EJECT, "Eject").key("cmd+e").enabled(ejectable),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_VOLUMES_ONLY, "Show Only Volumes").key("cmd+1").checked(!self.show_all),
                    AppMenuItem::new(MENU_ALL_DEVICES, "Show All Devices").key("cmd+2").checked(self.show_all),
                ],
            },
            AppMenu { title: "Disk".into(), items: disk },
            AppMenu { title: "Image".into(), items: vec![AppMenuItem::new(MENU_DETACH, "Detach Disk Image").enabled(self.selected_image())] },
        ]
    }

    fn menu_action(&mut self, _ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_DETACH => {
                let t = self.now;
                self.activate(Target::Toolbar(B_DETACH), t)
            }
            MENU_MOUNT => self.run_action(Action::Mount),
            MENU_UNMOUNT => self.run_action(Action::Unmount),
            MENU_EJECT => self.run_action(Action::Eject),
            MENU_INFO => self.ask_menu(Ask::Info),
            MENU_ERASE => self.ask_menu(Ask::Erase),
            MENU_FIRST_AID => self.ask_menu(Ask::FirstAid),
            MENU_VOLUMES_ONLY => self.set_show_all(false),
            MENU_ALL_DEVICES => self.set_show_all(true),
            MENU_FS_EXT4 => self.set_erase_fs(Personality::Ext4),
            MENU_FS_EXFAT => self.set_erase_fs(Personality::ExFat),
            MENU_FS_FAT32 => self.set_erase_fs(Personality::Fat32),
            n if (MENU_PARTITION_BASE + 2..=MENU_PARTITION_BASE + 4).contains(&n) => self.ask_menu(Ask::Partition((n - MENU_PARTITION_BASE) as usize)),
            _ => false,
        }
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state
        self.size = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        if self.last_refresh.is_infinite() {
            let t = self.now;
            self.refresh(t, true);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, t, .. } => {
                self.now = t;
                let scale = ctx.ui.pt(1.0);
                let (x, y) = (x / scale, y / scale);
                match kind {
                    TouchKind::Down => {
                        self.press = self.hit(&ctx.ui, x, y);
                        false
                    }
                    TouchKind::Up => {
                        let released = self.hit(&ctx.ui, x, y);
                        let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                        if released == pressed && released != Target::Nothing {
                            self.activate(released, t)
                        } else {
                            false
                        }
                    }
                    TouchKind::Move => false,
                    TouchKind::Cancel => {
                        self.press = Target::Nothing;
                        false
                    }
                }
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => {
                let t = self.now;
                self.activate(DiskUtility::target_of_id(id), t)
            }
            _ => false,
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(self, &ctx.ui)
    }

    /// Polls a running action and re-reads the disks every `REFRESH_S` seconds, so attached and removed devices show up.
    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.now = t;
        let finished = self.poll(t);
        if t - self.last_refresh >= REFRESH_S {
            self.refresh(t, false) || finished
        } else {
            finished
        }
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        if self.job.is_some() {
            return Some(t + 0.25);
        }
        if self.last_refresh.is_finite() {
            Some(self.last_refresh + REFRESH_S)
        } else {
            None
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.now = t;
        draw::paint(self, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(DiskUtility::new());
}
