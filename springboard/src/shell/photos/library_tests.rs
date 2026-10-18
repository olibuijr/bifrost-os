//! Switching the library: the chosen folder is shown on its own, remembered for the next launch, and a folder that
//! cannot be made leaves the library as it was.

use super::library::tests::{picture, scratch};
use super::tests::{photos, pump_until};
use super::*;

#[test]
fn switching_the_library_shows_its_pictures_and_the_choice_is_remembered() {
    let (mut p, dir, ui) = photos("library-switch", 2);
    let pictures = dir.join("Pictures");
    assert_eq!(p.photos().len(), 2);

    let other = dir.join("Other Library");
    for (name, unix) in [("A.png", 1_790_000_000), ("B.png", 1_790_000_100), ("C.png", 1_790_000_200)] {
        picture(&other.join(name), unix);
    }
    assert!(p.switch_library(&ui, other.clone()));
    pump_until(&mut p, &ui, |p| !p.scanning);
    assert_eq!(p.photos().len(), 3, "only the new library is shown");
    assert_eq!(libraries::decode(&p.prefs), vec![other.clone()]);

    // switching back reorders the list and keeps both libraries
    assert!(p.switch_library(&ui, pictures.clone()));
    pump_until(&mut p, &ui, |p| !p.scanning);
    assert_eq!(p.photos().len(), 2);
    assert_eq!(libraries::decode(&p.prefs), vec![pictures.clone(), other.clone()]);
    assert_eq!(libraries::remembered(&p.prefs), Some(pictures.clone()));

    // the library in use opens at the next launch, even when the shell offers another default
    drop(p);
    let again = Photos::new(&ui, 1200, 1920, dir.clone(), vec![dir.join("Missing")], 0).unwrap();
    assert_eq!(again.pictures, pictures);
    assert_eq!(again.dirs, vec![pictures]);
}

#[test]
fn a_library_that_cannot_be_made_leaves_the_current_one_in_place() {
    let (mut p, dir, ui) = photos("library-refused", 1);
    let blocker = dir.join("notes.txt");
    std::fs::write(&blocker, b"not a folder").unwrap();

    assert!(!p.switch_library(&ui, blocker.join("Library")));
    assert_eq!(p.pictures, dir.join("Pictures"));
    assert!(libraries::decode(&p.prefs).is_empty(), "nothing is remembered for a refused folder");
}

#[test]
fn a_remembered_library_that_has_gone_is_not_used_at_launch() {
    let dir = scratch("library-gone");
    let ui = crate::shell::tests::ui();
    let gone = dir.join("Gone");
    std::fs::create_dir_all(&gone).unwrap();
    {
        let mut prefs = Prefs::load(dir.join(PREFS_FILE));
        prefs.set(libraries::CURRENT, &gone.display().to_string()).unwrap();
    }
    std::fs::remove_dir(&gone).unwrap();

    let p = Photos::new(&ui, 1200, 1920, dir.clone(), vec![dir.join("Pictures")], 0).unwrap();
    assert_eq!(p.pictures, dir.join("Pictures"));
}
