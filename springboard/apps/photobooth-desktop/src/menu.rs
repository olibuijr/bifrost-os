//! Photo Booth's menus in the menu bar: the items, their key equivalents, and the mapping from a chosen item id back
//! to the command it stands for. The window state they depend on is passed in as plain values.

use crate::backdrop::Backdrop;
use bifrost_app::{AppMenu, AppMenuItem};

const TAKE_PHOTO: u32 = 1;
const EXPORT: u32 = 2;
const SHOW_LIBRARY: u32 = 3;
const DELETE: u32 = 4;
const AUTO_FLIP: u32 = 5;
const FULL_SCREEN: u32 = 6;
const SLIDESHOW: u32 = 7;
const SCREEN_FLASH: u32 = 8;
const RETAKE_SCENE: u32 = 9;
const BACKDROP_BASE: u32 = 100;
const CAMERA_BASE: u32 = 200;
/// The most cameras the Camera menu lists.
pub const MAX_CAMERAS: usize = 32;

/// What a chosen menu item asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    TakePhoto,
    Export,
    ShowLibrary,
    Delete,
    AutoFlip,
    FullScreen,
    Slideshow,
    ScreenFlash,
    RetakeScene,
    Backdrop(Backdrop),
    Camera(usize),
}

/// The command behind menu item `id`, or none for an id the menus never hand out.
pub fn decode(id: u32) -> Option<Command> {
    Some(match id {
        TAKE_PHOTO => Command::TakePhoto,
        EXPORT => Command::Export,
        SHOW_LIBRARY => Command::ShowLibrary,
        DELETE => Command::Delete,
        AUTO_FLIP => Command::AutoFlip,
        FULL_SCREEN => Command::FullScreen,
        SLIDESHOW => Command::Slideshow,
        SCREEN_FLASH => Command::ScreenFlash,
        RETAKE_SCENE => Command::RetakeScene,
        n if (BACKDROP_BASE..BACKDROP_BASE + Backdrop::ALL.len() as u32).contains(&n) => Command::Backdrop(Backdrop::ALL[(n - BACKDROP_BASE) as usize]),
        n if (CAMERA_BASE..CAMERA_BASE + MAX_CAMERAS as u32).contains(&n) => Command::Camera((n - CAMERA_BASE) as usize),
        _ => return None,
    })
}

/// What the menus show of the window.
pub struct MenuState<'a> {
    /// the camera is streaming, so a photo can be taken
    pub live: bool,
    /// a capture is selected in the strip
    pub selected: bool,
    pub has_captures: bool,
    pub auto_flip: bool,
    pub screen_flash: bool,
    pub full_screen: bool,
    pub slideshow: bool,
    pub backdrop: Backdrop,
    /// the names of the cameras the system lists
    pub cameras: &'a [String],
    /// the index in `cameras` of the one in use
    pub current_camera: Option<usize>,
}

pub fn menus(s: &MenuState) -> Vec<AppMenu> {
    let file = vec![
        AppMenuItem::new(TAKE_PHOTO, "Take Photo").key("cmd+t").enabled(s.live),
        AppMenuItem::new(EXPORT, "Export...").key("cmd+e").enabled(s.selected),
        AppMenuItem::separator(),
        AppMenuItem::new(SHOW_LIBRARY, "Show Photo Booth Library"),
    ];
    let edit = vec![
        AppMenuItem::new(DELETE, "Delete").key("cmd+backspace").enabled(s.selected),
        AppMenuItem::separator(),
        AppMenuItem::new(AUTO_FLIP, "Auto Flip").checked(s.auto_flip),
    ];
    let mut view = vec![
        AppMenuItem::new(SLIDESHOW, "Slideshow").enabled(s.has_captures).checked(s.slideshow),
        AppMenuItem::new(FULL_SCREEN, "Full Screen Viewfinder").checked(s.full_screen),
        AppMenuItem::new(SCREEN_FLASH, "Screen Flash").checked(s.screen_flash),
        AppMenuItem::separator(),
        AppMenuItem::submenu("Backdrop"),
    ];
    for (i, backdrop) in Backdrop::ALL.iter().enumerate() {
        view.push(AppMenuItem::new(BACKDROP_BASE + i as u32, backdrop.title()).checked(s.backdrop == *backdrop).depth(1));
    }
    view.push(AppMenuItem::new(RETAKE_SCENE, "Retake Empty Scene").enabled(s.backdrop != Backdrop::None && s.live));

    let mut camera = Vec::new();
    for (i, name) in s.cameras.iter().take(MAX_CAMERAS).enumerate() {
        camera.push(AppMenuItem::new(CAMERA_BASE + i as u32, name).checked(s.current_camera == Some(i)));
    }
    if camera.is_empty() {
        camera.push(AppMenuItem::new(0, "No Camera Found").enabled(false));
    }
    vec![
        AppMenu { title: "File".into(), items: file },
        AppMenu { title: "Edit".into(), items: edit },
        AppMenu { title: "View".into(), items: view },
        AppMenu { title: "Camera".into(), items: camera },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(cameras: &[String]) -> MenuState<'_> {
        MenuState {
            live: true,
            selected: false,
            has_captures: false,
            auto_flip: true,
            screen_flash: true,
            full_screen: false,
            slideshow: false,
            backdrop: Backdrop::None,
            cameras,
            current_camera: None,
        }
    }

    fn find<'a>(menus: &'a [AppMenu], title: &str) -> &'a AppMenuItem {
        menus.iter().flat_map(|m| m.items.iter()).find(|i| i.title == title).unwrap_or_else(|| panic!("no item {title}"))
    }

    #[test]
    fn every_menu_id_decodes_to_its_command_and_ids_are_unique() {
        let names = vec!["Front".to_string(), "Back".to_string()];
        let menus = menus(&state(&names));
        let mut seen = Vec::new();
        for item in menus.iter().flat_map(|m| m.items.iter()) {
            if item.id == 0 {
                continue;
            }
            assert!(decode(item.id).is_some(), "{} ({}) has no command", item.title, item.id);
            assert!(!seen.contains(&item.id), "{} repeats id {}", item.title, item.id);
            seen.push(item.id);
        }
        assert_eq!(decode(TAKE_PHOTO), Some(Command::TakePhoto));
        assert_eq!(decode(BACKDROP_BASE + 2), Some(Command::Backdrop(Backdrop::Ocean)));
        assert_eq!(decode(CAMERA_BASE + 1), Some(Command::Camera(1)));
    }

    #[test]
    fn ids_outside_the_menus_decode_to_nothing() {
        assert_eq!(decode(0), None);
        assert_eq!(decode(50), None);
        assert_eq!(decode(BACKDROP_BASE + Backdrop::ALL.len() as u32), None);
        assert_eq!(decode(CAMERA_BASE + MAX_CAMERAS as u32), None);
    }

    #[test]
    fn delete_and_export_need_a_selected_capture() {
        let none: Vec<String> = Vec::new();
        let mut s = state(&none);
        let m = menus(&s);
        assert!(!find(&m, "Delete").enabled);
        assert!(!find(&m, "Export...").enabled);
        s.selected = true;
        let m = menus(&s);
        assert!(find(&m, "Delete").enabled);
        assert!(find(&m, "Export...").enabled);
        assert_eq!(find(&m, "Delete").key, "cmd+backspace");
    }

    #[test]
    fn take_photo_needs_a_live_camera_and_the_slideshow_needs_captures() {
        let none: Vec<String> = Vec::new();
        let mut s = state(&none);
        s.live = false;
        let m = menus(&s);
        assert!(!find(&m, "Take Photo").enabled);
        assert!(!find(&m, "Slideshow").enabled);
        s.has_captures = true;
        assert!(find(&menus(&s), "Slideshow").enabled);
    }

    #[test]
    fn the_toggles_show_their_state() {
        let none: Vec<String> = Vec::new();
        let mut s = state(&none);
        s.auto_flip = false;
        s.screen_flash = true;
        s.full_screen = true;
        let m = menus(&s);
        assert!(!find(&m, "Auto Flip").checked);
        assert!(find(&m, "Screen Flash").checked);
        assert!(find(&m, "Full Screen Viewfinder").checked);
    }

    #[test]
    fn the_camera_menu_lists_every_device_and_checks_the_one_in_use() {
        let names = vec!["Front".to_string(), "Back".to_string()];
        let mut s = state(&names);
        s.current_camera = Some(1);
        let m = menus(&s);
        let camera = m.iter().find(|m| m.title == "Camera").unwrap();
        assert_eq!(camera.items.len(), 2);
        assert!(!camera.items[0].checked);
        assert!(camera.items[1].checked);
        assert_eq!(decode(camera.items[1].id), Some(Command::Camera(1)));
    }

    #[test]
    fn with_no_camera_the_menu_says_so_and_is_disabled() {
        let none: Vec<String> = Vec::new();
        let m = menus(&state(&none));
        let camera = m.iter().find(|m| m.title == "Camera").unwrap();
        assert_eq!(camera.items.len(), 1);
        assert!(!camera.items[0].enabled);
    }

    #[test]
    fn the_backdrop_submenu_sits_one_level_down_and_checks_the_current_one() {
        let none: Vec<String> = Vec::new();
        let mut s = state(&none);
        s.backdrop = Backdrop::Forest;
        let m = menus(&s);
        let view = m.iter().find(|m| m.title == "View").unwrap();
        let header = view.items.iter().position(|i| i.submenu && i.title == "Backdrop").unwrap();
        for (k, backdrop) in Backdrop::ALL.iter().enumerate() {
            let item = &view.items[header + 1 + k];
            assert_eq!((item.title.as_str(), item.depth, item.checked), (backdrop.title(), 1, *backdrop == Backdrop::Forest));
        }
        assert!(find(&m, "Retake Empty Scene").enabled);
    }

    #[test]
    fn retaking_the_empty_scene_needs_a_backdrop() {
        let none: Vec<String> = Vec::new();
        assert!(!find(&menus(&state(&none)), "Retake Empty Scene").enabled);
    }
}
