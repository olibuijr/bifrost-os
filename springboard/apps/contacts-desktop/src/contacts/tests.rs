use super::*;

fn test_ui() -> Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}

/// A fresh contacts file in its own directory, holding `contacts`.
fn file_with(name: &str, contacts: &[Contact]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("contacts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("contacts.txt");
    store::save(&path, contacts).unwrap();
    path
}

fn contact(id: u64, name: &str) -> Contact {
    Contact { id, name: name.into(), ..Default::default() }
}

fn donated(key: &str, name: &str, sources: u32) -> Suggested {
    Suggested { key: key.into(), handle: bifrost_app::contacts::suggested::Handle::Email, value: key.into(), name: name.into(), sources }
}

#[test]
fn donated_people_are_offered_in_the_sidebar_accept_adds_a_contact_and_dismiss_blocks_it() {
    let mut anna = contact(1, "Anna As");
    anna.email = "Anna@Work.is".into();
    let path = file_with("suggestions", &[anna]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(!app.sidebar_rows().iter().any(|row| row.0 == LIST_SUGGESTIONS), "nothing donated yet");
    // Anna is in the book already (by email, in any case) and is left out
    app.set_suggestions(&ui, vec![donated("anna@work.is", "Anna", 1), donated("ada@example.com", "Ada Lovelace", 2), donated("bob@example.com", "", 1)]);
    assert!(app.sidebar_rows().iter().any(|row| row.0 == LIST_SUGGESTIONS && row.1 == "Suggested Contacts"));
    assert_eq!(app.suggestions.len(), 2);

    app.activate(&ui, Target::Sidebar(LIST_SUGGESTIONS), 0.0, 0.0, 0.0);
    assert_eq!(app.screen, Screen::Suggestions);
    assert!(app.describe().contains("contacts_screen=suggestions"));
    let l = app.suggest_layout();
    assert_eq!(l.rows.len(), 2);
    let (add, dismiss) = (l.rows[0].add, l.rows[1].dismiss);
    assert_eq!(app.hit(&ui, add.x + add.w / 2.0, add.y + add.h / 2.0), Some(Target::SuggestAdd(0)));
    assert_eq!(app.hit(&ui, dismiss.x + dismiss.w / 2.0, dismiss.y + dismiss.h / 2.0), Some(Target::SuggestDismiss(1)));
    assert!(app.controls(&ui).iter().any(|n| n.id == NODE_SUGGEST_ADD && n.label == "Add Ada Lovelace"));

    // Add: the person is a contact now, saved to the file, and leaves the list
    app.activate(&ui, Target::SuggestAdd(0), 0.0, 0.0, 0.0);
    assert!(app.listed().iter().any(|c| c.name == "Ada Lovelace" && c.email == "ada@example.com"));
    assert_eq!(app.status.as_deref(), Some("Added Ada Lovelace"));
    assert_eq!(app.suggestions.len(), 1);
    assert_eq!(app.screen, Screen::Suggestions, "the screen stays while people are left");
    assert!(Contacts::new(&ui, 1200, 780, path.clone()).listed().iter().any(|c| c.email == "ada@example.com"), "saved");

    // Dismiss the last one: the screen closes and the sidebar row goes
    app.activate(&ui, Target::SuggestDismiss(0), 0.0, 0.0, 0.0);
    assert_eq!(app.screen, Screen::List);
    assert!(!app.sidebar_rows().iter().any(|row| row.0 == LIST_SUGGESTIONS));
    let answers: Vec<(String, Feedback)> = app.take_suggestion_answers();
    assert_eq!(answers, vec![("ada@example.com".to_string(), Feedback::Accepted), ("bob@example.com".to_string(), Feedback::Dismissed)]);
    assert!(app.take_suggestion_answers().is_empty(), "answers go to the shell once");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_tap_on_a_phone_or_email_hands_it_off_and_other_values_copy() {
    let mut ann = contact(1, "Anna As");
    ann.phone = "555 0100".into();
    ann.email = "anna@work.is".into();
    let path = file_with("handoff", &[ann]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path);
    app.open_detail(&ui, 1);
    let rows = app.rows_of(app.selected().unwrap());
    let phone = rows.iter().position(|r| r.action == contact_card::Action::Call).unwrap();
    let email = rows.iter().position(|r| r.action == contact_card::Action::Email).unwrap();
    app.activate(&ui, Target::Use(phone), 0.0, 0.0, 0.0);
    assert_eq!(app.take_handoff(), Some(Handoff::Call("555 0100".into())));
    app.activate(&ui, Target::Use(email), 0.0, 0.0, 0.0);
    assert_eq!(app.take_handoff(), Some(Handoff::Mail("anna@work.is".into())));
    assert!(app.take_handoff().is_none(), "taken once");
}

#[test]
fn a_phone_value_becomes_a_tel_link_without_spaces_or_dashes() {
    assert_eq!(tel_url("+354 555-0100"), "tel:+3545550100");
}

#[test]
fn a_card_window_click_in_window_points_lands_under_the_pointer_at_a_non_1_scale() {
    use crate::card_windows::card_touch;
    use bifrost_app::TouchKind;
    let path = file_with("card-touch", &[contact(1, "Anna As")]);
    let ui = test_ui();
    assert!((ui.s - 1.5625).abs() < 1e-6, "Midget's scale");
    let mut app = Contacts::new(&ui, 1200, 780, path);
    let (w, h) = ((480.0 * ui.s) as usize, (560.0 * ui.s) as usize);
    let mut px = vec![0u32; w * h];
    app.draw_card(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, 1);
    // the middle of the card's Edit button, in window points, as the shell sends it (`WindowTouch`)
    let edit = app.card_edit_rect();
    let (x, y) = (edit.x + edit.w / 2.0, edit.y + edit.h / 2.0);
    let k = ui.pt(1.0);
    app.touch_card(&ui, 1, card_touch(TouchKind::Down, x, y, k), 0.0);
    app.touch_card(&ui, 1, card_touch(TouchKind::Up, x, y, k), 0.05);
    assert_eq!(app.take_card_edit(), Some(1), "Edit is under the pointer");
    // the old reading (points taken as px) put the same click at (x / s, y / s) pt, left of and above the button
    app.touch_card(&ui, 1, Event::Down(x, y), 0.1);
    app.touch_card(&ui, 1, Event::Up(x, y), 0.15);
    assert_eq!(app.take_card_edit(), None);
}

#[test]
fn copy_contact_queues_the_open_contact_once_from_the_card_and_the_accessibility_control() {
    let path = file_with("copy", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path);
    app.open_detail(&ui, 1);
    assert!(app.activate_control(&ui, NODE_COPY_CONTACT));
    assert_eq!(app.take_copy().map(|(c, _)| c.id), Some(1));
    assert!(app.take_copy().is_none(), "taken once");
    app.open_detail(&ui, 2);
    app.activate(&ui, Target::CopyContact, 0.0, 0.0, 0.0);
    assert_eq!(app.take_copy().map(|(c, _)| c.name), Some("Bo Berg".to_string()));
}

#[test]
fn list_is_sorted_by_name_with_one_section_per_letter() {
    let path = file_with("sorted", &[contact(1, "Bo Berg"), contact(2, "Anna As"), contact(3, "Anders Ek")]);
    let ui = test_ui();
    let app = Contacts::new(&ui, 1200, 780, path.clone());
    let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Anna As", "Bo Berg", "Anders Ek"], "sorted by family name");
    let sections: String = app.rows.iter().filter_map(|r| r.section).collect();
    assert_eq!(sections, "ABE");
    assert!(app.describe().contains("contacts_count=3\n"));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_family_name_of_several_words_sections_under_its_first_letter() {
    let path = file_with("family", &[contact(1, "QA Testperson edited"), contact(2, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    let sections: String = app.rows.iter().filter_map(|r| r.section).collect();
    assert_eq!(sections, "AE", "without a record the last word is the family name");
    let mut card = CardDetails::default();
    card.family = "Testperson edited".into();
    app.details.set(1, card);
    app.rebuild(&ui);
    let sections: String = app.rows.iter().filter_map(|r| r.section).collect();
    assert_eq!(sections, "AT", "the recorded family name gives the section");
    app.edit_contact(&ui, Some(1));
    assert_eq!((app.fields[0].trim(), app.fields[1].trim()), ("QA", "Testperson edited"), "the editor splits at the family name");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn favorites_filter_shows_only_starred_contacts() {
    let starred = Contact { favorite: true, ..contact(3, "Cara Cole") };
    let path = file_with("favorites", &[contact(1, "Anna As"), starred]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.filter = Filter::Favorites;
    app.rebuild(&ui);
    let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Cara Cole"]);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_group_is_made_filled_from_the_open_contact_and_saved_to_its_file() {
    let path = file_with("groups", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.open_detail(&ui, 1);
    app.new_group(&ui);
    assert_eq!(app.filter, Filter::Group(1));
    assert!(app.listed().is_empty(), "the new group has no members yet");
    app.open_detail(&ui, 1);
    app.toggle_group(&ui);
    let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Anna As"]);
    let (saved, ids) = group_file::load(&path.with_file_name(GROUPS_FILE)).unwrap();
    assert_eq!(saved.members(ids[0], false).unwrap(), vec![1]);
    app.open_detail(&ui, 1);
    app.toggle_group(&ui);
    assert!(app.listed().is_empty(), "a second press takes the contact out again");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_group_can_be_renamed_and_deleted_and_the_file_keeps_the_rest() {
    let path = file_with("rename", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.new_group(&ui);
    app.new_group(&ui);
    assert_eq!(app.group_ids, vec![1, 2]);
    app.filter = Filter::Group(1);
    app.start_rename(&ui);
    assert_eq!(app.renaming, Some(1));
    assert_eq!(app.rename.to_string(), "Group 1", "the name is typed into the field that replaces search");
    app.rename.clear();
    app.rename.push_str("Friends");
    app.finish_rename(&ui);
    assert_eq!(app.renaming, None);
    assert_eq!(app.groups.get(1).unwrap().name, "Friends");

    app.filter = Filter::Group(1);
    app.delete_group(&ui);
    assert_eq!(app.filter, Filter::All);
    assert_eq!(app.group_ids, vec![2], "the deleted group leaves the sidebar");
    let (back, ids) = group_file::load(&path.with_file_name(GROUPS_FILE)).unwrap();
    assert_eq!(ids.len(), 1, "the file keeps only the group that is left");
    assert_eq!(back.get(ids[0]).unwrap().name, "Group 2");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_people_picker_adds_each_contact_once() {
    let path = file_with("picker", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.new_group(&ui);
    app.start_picking(&ui);
    assert_eq!(app.picking, Some(1));
    assert_eq!(app.filter, Filter::All, "the picker lists every contact");
    app.pick_person(&ui, 0);
    assert_eq!(app.groups.members(1, false).unwrap(), vec![1]);
    assert!(app.status.as_deref().unwrap().contains("Added Anna As"), "{:?}", app.status);
    app.pick_person(&ui, 0);
    assert_eq!(app.groups.members(1, false).unwrap(), vec![1], "a member cannot be chosen twice");
    assert!(app.status.as_deref().unwrap().contains("already"), "{:?}", app.status);
    app.finish_picking(&ui);
    assert_eq!(app.picking, None);
    assert_eq!(app.filter, Filter::Group(1));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_card_shows_the_values_a_contact_has_under_their_labels() {
    let mut ann = contact(1, "Anna As");
    ann.phone = "555 0100".into();
    ann.labels = [2, 1, 0];
    ann.address = "Hverfisgata 1".into();
    let rows: Vec<(String, String)> = card_rows(&ann, &[]).into_iter().map(|r| (r.label, r.value)).collect();
    assert_eq!(
        rows,
        vec![("work".to_string(), "555 0100".to_string()), ("home".to_string(), "Hverfisgata 1".to_string())],
        "the empty email has no row; the phone is the third phone label"
    );
    let path = file_with("card-rows", &[ann]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.open_detail(&ui, 1);
    let slots: Vec<usize> = app.card_layout(&ui).values.iter().map(|v| v.0).collect();
    assert_eq!(slots, vec![0, 1], "one row for the phone and one for the address, so copy and layout use the same row numbers");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn extra_values_follow_the_store_values_on_the_card_and_copy_by_row() {
    let mut ann = contact(1, "Anna As");
    ann.phone = "555 0100".into();
    ann.labels = [2, 1, 0];
    let extra = [
        Extra { kind: ValueKind::Phone, label: "home".into(), value: "555 0199".into() },
        Extra { kind: ValueKind::Email, label: "work".into(), value: "anna@work.is".into() },
    ];
    let rows: Vec<(String, String)> = card_rows(&ann, &extra).into_iter().map(|r| (r.label, r.value)).collect();
    assert_eq!(
        rows,
        vec![
            ("work".to_string(), "555 0100".to_string()),
            ("home".to_string(), "555 0199".to_string()),
            ("work".to_string(), "anna@work.is".to_string()),
        ],
        "store phone, extra phone, then extra email: the card groups by kind"
    );
    assert_eq!(value_word(card_rows(&ann, &extra)[2].key), "email");
}

#[test]
fn a_search_matches_extra_values_as_well_as_store_fields() {
    let contacts = vec![contact(1, "Anna As"), contact(2, "Bo Berg")];
    let mut extras = Extras::default();
    assert!(extras.add(2, Extra { kind: ValueKind::Email, label: "work".into(), value: "bo@berg.is".into() }));
    assert_eq!(list_hits(&contacts, &extras, &Details::default(), "BO@BERG"), vec![1], "only Bo has that extra email");
    assert_eq!(list_hits(&contacts, &extras, &Details::default(), "anna"), vec![0], "the store's name still matches");
    assert_eq!(list_hits(&contacts, &extras, &Details::default(), "zzz"), Vec::<usize>::new());
    assert_eq!(list_hits(&contacts, &extras, &Details::default(), ""), vec![0, 1], "a blank search shows everyone");
}

#[test]
fn deleting_a_contact_drops_its_extra_values_from_the_file() {
    let path = file_with("extras-delete", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(app.extras.add(1, Extra { kind: ValueKind::Phone, label: "home".into(), value: "555 0199".into() }));
    assert!(app.extras.add(2, Extra { kind: ValueKind::Phone, label: "home".into(), value: "555 0299".into() }));
    extras::save(&path.with_file_name(VALUES_FILE), &app.extras).unwrap();
    app.open_detail(&ui, 1);
    app.remove_selected(&ui);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.extras.get(1).is_empty());
    let left = extras::load_checked(&path.with_file_name(VALUES_FILE)).unwrap();
    assert_eq!(left.get(2).len(), 1, "Bo keeps his value");
    assert!(left.get(1).is_empty());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_sort_order_decides_which_name_leads() {
    let zoe = contact(1, "Zoe Ann");
    let adam = contact(2, "Adam Zed");
    assert_eq!(compare_contacts(SortOrder::FamilyName, &zoe, &adam, None, None, "", ""), Ordering::Less, "Ann sorts before Zed");
    assert_eq!(compare_contacts(SortOrder::GivenName, &zoe, &adam, None, None, "", ""), Ordering::Greater, "Adam sorts before Zoe");
    assert_eq!(compare_contacts(SortOrder::None, &zoe, &adam, None, None, "", ""), Ordering::Less, "identifier order");
}

#[test]
fn a_company_sorts_by_its_company_name_in_the_list() {
    let acme = contact(3, "Jon Smith");
    let bravo = contact(4, "Ann Berg");
    assert_eq!(
        compare_contacts(SortOrder::FamilyName, &acme, &bravo, Some("Acme"), None, "", ""),
        Ordering::Less,
        "the company Acme sorts before the person Ann Berg, whose family name is Berg"
    );
    assert_eq!(
        compare_contacts(SortOrder::FamilyName, &acme, &bravo, None, None, "", ""),
        Ordering::Greater,
        "without the company flag Jon Smith sorts after Ann Berg"
    );
    assert_eq!(compare_contacts(SortOrder::FamilyName, &acme, &bravo, Some("Zeta"), None, "", ""), Ordering::Greater, "Zeta sorts after Berg");
}

#[test]
fn the_settings_row_cycles_the_sort_and_saves_it() {
    let path = file_with("settings", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.screen = Screen::Settings;
    assert_eq!(app.display.effective_sort_order(SortOrder::FamilyName), SortOrder::FamilyName);
    app.cycle_sort(&ui);
    assert_eq!(app.display.sort_order(), SortOrder::FamilyName);
    app.cycle_sort(&ui);
    assert_eq!(app.display.sort_order(), SortOrder::GivenName);
    let (saved, _, _) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
    assert_eq!(saved.sort_order(), SortOrder::GivenName);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_file_menu_and_view_menu_are_the_app_menus_and_name_order_is_saved() {
    let path = file_with("menus", &[contact(1, "Ada Lovelace")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    let menus = app.main_menu();
    assert_eq!(menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["File", "View", "Card"]);
    assert_eq!(
        menus[0].items.iter().filter(|i| !i.separator).count(),
        13,
        "new card and group, import, restore, new and edit smart list, export 3.0, 4.0 and archive, print, labels, PDF, refresh"
    );
    assert_eq!(menus[1].items[0].title, "Hide Sidebar", "the sidebar starts shown on a wide window");
    assert!(menus[1].items[2].checked, "first name first is the default");
    assert!(app.menu_action(&ui, MENU_LAST_FIRST));
    assert_eq!(app.name_order, names::NameOrder::LastFirst);
    let menus = app.main_menu();
    assert!(!menus[1].items[2].checked);
    assert!(menus[1].items[3].checked);
    let (_, saved, _) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
    assert_eq!(saved, names::NameOrder::LastFirst);
    assert!(!app.menu_action(&ui, 99), "an unknown id is not handled");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_vcard_import_folder_adds_new_people_and_skips_duplicates() {
    let path = file_with("import", &[contact(1, "Anna As")]);
    let folder = path.parent().unwrap().join("Import");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("people.vcf"),
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Anna As\r\nEMAIL;TYPE=\"home\":anna@example.test\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Bo Berg\r\nN:Berg;Bo;;;\r\nEND:VCARD\r\n",
    )
    .unwrap();
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.import_vcards(&ui);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.all_contacts().map(|all| all.len()), Some(2), "Anna is a duplicate by name; Bo is new");
    assert!(app.status.as_deref().unwrap().contains("Imported 1 contacts"), "{:?}", app.status);
    assert_eq!(store::load(&path).len(), 2);
    app.export_vcards(exchange::Version::V4);
    let exported = std::fs::read_to_string(path.parent().unwrap().join("Export/contacts-4.0.vcf")).unwrap();
    assert!(exported.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\n"), "{exported}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_duplicates_filter_lists_only_contacts_with_a_twin() {
    let twin = Contact { email: "anna@x.is".into(), ..contact(2, "A. As") };
    let path = file_with("dupes", &[Contact { email: "anna@x.is".into(), ..contact(1, "Anna As") }, twin, contact(3, "Cara Cole")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.filter = Filter::Duplicates;
    app.rebuild(&ui);
    let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["A. As", "Anna As"]);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_smart_group_row_lists_the_contacts_that_match_its_rule() {
    let path = file_with(
        "smart",
        &[Contact { email: "a@x.is".into(), ..contact(1, "Anna As") }, contact(2, "Bo Berg"), contact(3, "Cara Cole")],
    );
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    let with_email = LIST_SMART + crate::smart::Smart::WithEmail.index() as u32;
    assert!(sidebar_node(with_email as u64));
    app.filter = Filter::from_sidebar_id(with_email);
    assert_eq!(app.filter, Filter::Smart(crate::smart::Smart::WithEmail));
    assert_eq!(app.filter.sidebar_id(), with_email);
    app.rebuild(&ui);
    let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Anna As"]);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_new_contact_from_the_editor_is_saved_to_the_file() {
    let path = file_with("create", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.edit_contact(&ui, None);
    app.fields[0].push_str("Ada");
    app.fields[1].push_str("Lovelace");
    app.fields[4].push_str("ada@example.test");
    app.save_editor(&ui);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.all_contacts().map(|all| all.len()), Some(2));
    let saved = store::load(&path);
    assert!(saved.iter().any(|c| c.name == "Ada Lovelace" && c.email == "ada@example.test"));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn an_empty_form_is_refused_and_the_file_is_untouched() {
    let path = file_with("refuse", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.edit_contact(&ui, None);
    app.save_editor(&ui);
    assert_eq!(app.error.as_deref(), Some("Enter a name, company, phone number or email"));
    assert_eq!(store::load(&path).len(), 1);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn favourite_then_delete_persist_to_the_file() {
    let path = file_with("delete", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.open_detail(&ui, 2);
    app.toggle_favorite(&ui);
    assert!(store::load(&path).iter().any(|c| c.id == 2 && c.favorite));
    app.remove_selected(&ui);
    assert!(app.selected().is_none());
    let left = store::load(&path);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, 1);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn an_unreadable_file_gives_the_search_index_nothing_to_clear() {
    let ui = test_ui();
    let loaded = Contacts::new(&ui, 1200, 780, file_with("index-ok", &[contact(1, "Anna As")]));
    assert_eq!(loaded.all_contacts().map(|all| all.len()), Some(1));
    let dir = std::env::temp_dir().join(format!("contacts-index-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("contacts.txt");
    std::fs::write(&path, "not a contacts file\n").unwrap();
    let broken = Contacts::new(&ui, 1200, 780, path);
    assert!(broken.all_contacts().is_none(), "no contacts are offered to the index");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unreadable_file_blocks_every_write() {
    let dir = std::env::temp_dir().join(format!("contacts-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("contacts.txt");
    std::fs::write(&path, "not a contacts file\n").unwrap();
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(app.describe().contains("contacts_error=true"));
    app.edit_contact(&ui, None);
    assert!(matches!(app.screen, Screen::List), "the editor must not open");
    app.save_editor(&ui);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a contacts file\n");
    let _ = std::fs::remove_dir_all(&dir);
}

fn data_dir_of(path: &Path) -> PathBuf {
    path.parent().unwrap().to_path_buf()
}

fn put_import(path: &Path, name: &str, text: &str) {
    let folder = data_dir_of(path).join("Import");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(name), text).unwrap();
}

#[test]
fn the_edit_form_saves_card_fields_and_the_company_flag_and_the_card_shows_them() {
    let path = file_with("card-fields", &[Contact { company: "Acme".into(), ..contact(1, "Anna As") }]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.open_detail(&ui, 1);
    app.add_field(&ui, Detail::Nickname);
    assert!(app.editing(), "Add Field opens the form on the open card");
    assert_eq!(app.focus, Some(DETAIL_BASE + Detail::Nickname.index()));
    app.detail_fields[Detail::Nickname.index()].push_str("Annie");
    app.add_field(&ui, Detail::Birthday);
    app.detail_fields[Detail::Birthday.index()].push_str("19900517");
    assert!(app.activate_control(&ui, NODE_COMPANY), "the switch is a control for assistive technology");
    assert!(app.edit_company);
    app.save_editor(&ui);
    assert!(app.error.is_none(), "{:?}", app.error);
    let saved = details::load_checked(&path.with_file_name(DETAILS_FILE)).unwrap();
    assert_eq!(saved.of(1).get(Detail::Nickname), "Annie");
    assert_eq!(saved.of(1).get(Detail::Birthday), "1990-05-17");
    assert!(saved.of(1).company);
    let rows: Vec<(String, String)> = app.rows_of(app.selected().unwrap()).into_iter().map(|r| (r.label, r.value)).collect();
    assert_eq!(
        rows,
        vec![("Nickname".to_string(), "Annie".to_string()), ("Birthday".to_string(), "1990-05-17".to_string())],
        "the card fields follow the phones, emails and addresses"
    );
    assert_eq!(app.title_of(app.selected().unwrap()), "Acme", "a company card shows its company as the title");
    assert_eq!(app.card_layout(&ui).values.len(), 2);
    app.prefs.prefer_nicknames = true;
    app.toggle_company(&ui);
    assert_eq!(app.title_of(app.selected().unwrap()), "Annie", "with the company flag off, the nickname is preferred");
    let reloaded = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(!reloaded.details.is_company(1), "the toggle on the card is saved");
    assert_eq!(reloaded.details.nickname(1), "Annie");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_search_finds_a_contact_by_a_card_field_and_a_deleted_contact_loses_its_fields() {
    let path = file_with("card-search", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    let mut card = CardDetails::default();
    card.set(Detail::JobTitle, "Lighthouse keeper");
    app.details.set(2, card);
    app.write_details();
    app.query.push_str("lighthouse");
    app.rebuild(&ui);
    assert_eq!(app.listed().iter().map(|c| c.id).collect::<Vec<_>>(), vec![2]);
    app.open_detail(&ui, 2);
    app.remove_selected(&ui);
    assert!(details::load_checked(&path.with_file_name(DETAILS_FILE)).unwrap().of(2).is_empty());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn tab_visits_the_shown_card_fields_before_the_notes() {
    let path = file_with("card-tab", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.edit_contact(&ui, Some(1));
    app.detail_shown[Detail::Url.index()] = true;
    app.detail_shown[Detail::Birthday.index()] = true;
    assert_eq!(app.focus_order(), vec![0, 1, 2, DETAIL_BASE + Detail::Url.index(), DETAIL_BASE + Detail::Birthday.index(), NOTES]);
    app.focus_field(DETAIL_BASE + Detail::Url.index());
    app.key(&ui, Out::Named("Tab"), 0.0);
    assert_eq!(app.focus, Some(DETAIL_BASE + Detail::Birthday.index()));
    app.key(&ui, Out::Text("1990".into()), 0.0);
    assert_eq!(&*app.detail_fields[Detail::Birthday.index()], "1990");
    app.key(&ui, Out::Named("Tab"), 0.0);
    assert_eq!(app.focus, Some(NOTES));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn merging_duplicates_folds_the_cards_and_moves_groups_and_my_card() {
    let first = Contact { email: "anna@x.is".into(), ..contact(1, "Anna As") };
    let second = Contact { email: "ANNA@x.is".into(), phone: "555 0100".into(), notes: "Likes tea".into(), ..contact(2, "A. As") };
    let path = file_with("merge", &[first, second, contact(3, "Cara Cole")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.new_group(&ui);
    app.groups.add_member(1, 2).unwrap();
    app.prefs.me = Some(2);
    app.filter = Filter::Duplicates;
    app.rebuild(&ui);
    assert_eq!(app.listed().len(), 2);
    app.open_detail(&ui, 2);
    app.merge_duplicates(&ui, false);
    assert!(app.error.is_none(), "{:?}", app.error);
    let saved = store::load(&path);
    assert_eq!(saved.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 3]);
    let kept = saved.iter().find(|c| c.id == 1).unwrap();
    assert_eq!((kept.phone.as_str(), kept.notes.as_str()), ("555 0100", "Likes tea"));
    assert_eq!(app.groups.members(1, false).unwrap(), vec![1], "the group follows the merged card");
    assert_eq!(app.prefs.me, Some(1));
    assert_eq!(app.screen, Screen::Detail(1), "the open card that was merged away is replaced by the kept one");
    assert!(app.status.as_deref().unwrap().starts_with("Merged 1 duplicate"), "{:?}", app.status);
    assert!(app.listed().is_empty(), "nothing is a duplicate any more");
    app.merge_duplicates(&ui, true);
    assert_eq!(app.status.as_deref(), Some("There are no duplicates to merge"));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn merge_all_merges_every_group_in_one_save() {
    let list = [
        Contact { email: "a@x.is".into(), ..contact(1, "Anna As") },
        Contact { email: "a@x.is".into(), ..contact(2, "A. As") },
        Contact { phone: "555 0300".into(), ..contact(3, "Cara Cole") },
        Contact { phone: "555 0300".into(), ..contact(4, "C. Cole") },
        contact(5, "Dan Dahl"),
    ];
    let path = file_with("merge-all", &list);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.merge_duplicates(&ui, true);
    assert_eq!(store::load(&path).iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 3, 5]);
    assert!(app.status.as_deref().unwrap().starts_with("Merged 2 duplicate cards into 2 cards"), "{:?}", app.status);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_smart_list_is_made_in_the_editor_shown_in_the_sidebar_edited_and_deleted() {
    let path = file_with(
        "smart-ui",
        &[Contact { company: "Acme".into(), ..contact(1, "Anna As") }, contact(2, "Bo Berg"), Contact { company: "Acme Ltd".into(), ..contact(3, "Cara Cole") }],
    );
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(app.menu_action(&ui, MENU_NEW_LIST));
    assert_eq!(app.screen, Screen::SmartEditor(None));
    assert_eq!(app.focus, Some(SMART_NAME));
    assert!(app.editing(), "an open smart list editor counts as unsaved work");
    app.key(&ui, Out::Text("Acme people".into()), 0.0);
    app.activate(&ui, Target::SmartField(0), 0.0, 0.0, 0.0);
    assert_eq!(app.smart_rules[0].field, RuleField::Company);
    app.activate(&ui, Target::Field(SMART_VALUE), 0.0, 0.0, 0.0);
    assert_eq!(app.focus, Some(SMART_VALUE));
    app.key(&ui, Out::Text("acme".into()), 0.0);
    app.bar(&ui, BAR_DONE);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.screen, Screen::List);
    assert_eq!(app.filter, Filter::UserList(0));
    assert_eq!(app.listed().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Anna As", "Cara Cole"]);
    assert!(app.sidebar_rows().iter().any(|row| row.0 == LIST_USER && row.1 == "Acme people"));
    assert_eq!(Filter::from_sidebar_id(LIST_USER), Filter::UserList(0));
    assert!(sidebar_node(LIST_USER as u64));
    assert_eq!(group_file::load_lists(&path.with_file_name(SMART_FILE)).unwrap().len(), 1);
    assert_eq!(Contacts::new(&ui, 1200, 780, path.clone()).lists, app.lists, "the list is there after a restart");

    assert!(app.menu_action(&ui, MENU_EDIT_LIST));
    assert_eq!(app.screen, Screen::SmartEditor(Some(0)));
    assert_eq!(&*app.smart_name, "Acme people");
    assert_eq!(&*app.smart_rules[0].value, "acme");
    assert!(app.activate_control(&ui, NODE_SMART_ADD));
    assert_eq!(app.smart_rules.len(), 2);
    assert_eq!(app.focus, Some(SMART_VALUE + 1), "the new rule's value is ready to type");
    app.key(&ui, Out::Text("bo".into()), 0.0);
    assert!(app.activate_control(&ui, NODE_SMART_ANY));
    assert!(app.smart_any);
    app.bar(&ui, BAR_DONE);
    assert_eq!(app.listed().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Anna As", "Bo Berg", "Cara Cole"], "any rule");

    assert!(app.menu_action(&ui, MENU_EDIT_LIST));
    app.activate(&ui, Target::SmartRemove(1), 0.0, 0.0, 0.0);
    assert_eq!(app.smart_rules.len(), 1);
    app.activate(&ui, Target::SmartRemove(0), 0.0, 0.0, 0.0);
    assert_eq!(app.smart_rules.len(), 1, "a list keeps its last rule");
    app.activate(&ui, Target::SmartDelete, 0.0, 0.0, 0.0);
    assert!(app.confirm_delete, "the first press asks");
    assert_eq!(app.lists.len(), 1);
    app.activate(&ui, Target::SmartDelete, 0.0, 0.0, 0.0);
    assert!(app.lists.is_empty());
    assert_eq!(app.filter, Filter::All);
    assert_eq!(app.screen, Screen::List);
    assert!(group_file::load_lists(&path.with_file_name(SMART_FILE)).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_smart_list_without_a_name_or_a_value_is_refused_and_tab_skips_rules_without_a_value() {
    let path = file_with("smart-ui-bad", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.new_list(&ui);
    app.bar(&ui, BAR_DONE);
    assert!(app.error.as_deref().unwrap().contains("needs a name"), "{:?}", app.error);
    assert_eq!(app.screen, Screen::SmartEditor(None), "the editor stays open");
    app.key(&ui, Out::Text("Friends".into()), 0.0);
    app.bar(&ui, BAR_DONE);
    assert!(app.error.is_some(), "the rule has no value yet");
    app.add_rule(&ui);
    app.smart_rules[1].condition = Condition::IsSet;
    assert_eq!(app.smart_focus_order(), vec![SMART_NAME, SMART_VALUE], "a rule that needs no value is skipped");
    app.focus_field(SMART_NAME);
    app.key(&ui, Out::Named("Tab"), 0.0);
    assert_eq!(app.focus, Some(SMART_VALUE));
    app.key(&ui, Out::Text("an".into()), 0.0);
    app.key(&ui, Out::Named("Escape"), 0.0);
    assert_eq!(app.screen, Screen::List, "Escape leaves the editor without saving");
    assert!(app.lists.is_empty());
    assert!(!path.with_file_name(SMART_FILE).exists());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn selection_mode_marks_rows_instead_of_opening_them_and_makes_a_group() {
    let path = file_with("select", &[contact(1, "Anna As"), contact(2, "Bo Berg"), contact(3, "Cara Cole")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(app.menu_action(&ui, MENU_SELECT));
    assert!(app.selecting);
    app.activate(&ui, Target::Row(0), 0.0, 0.0, 0.0);
    app.activate(&ui, Target::Row(1), 0.0, 0.0, 0.0);
    assert_eq!(app.screen, Screen::List, "a press only marks the row");
    assert_eq!(app.selection, BTreeSet::from([1, 2]));
    assert_eq!(app.controls(&ui).iter().filter(|n| n.id >= ROW_NODE && n.value == "selected").count(), 2, "assistive technology sees the marked contacts");
    app.activate(&ui, Target::Row(1), 0.0, 0.0, 0.0);
    assert_eq!(app.selection, BTreeSet::from([1]), "a second press takes the mark off");
    app.activate(&ui, Target::Row(1), 0.0, 0.0, 0.0);
    let bar: Vec<String> = app.list_bar().items.iter().filter(|i| i.id != 0 && i.id != BAR_SIDEBAR).map(|i| i.label.clone()).collect();
    assert_eq!(bar, ["Group Selection", "Merge", "Done"]);
    assert!(app.menu_action(&ui, MENU_GROUP_SELECTION));
    assert!(!app.selecting && app.selection.is_empty());
    assert_eq!(app.filter, Filter::Group(1));
    assert_eq!(app.groups.members(1, false).unwrap(), vec![1, 2]);
    assert_eq!(app.status.as_deref(), Some("Made Group 1 from 2 contacts"));
    let (saved, ids) = group_file::load(&path.with_file_name(GROUPS_FILE)).unwrap();
    assert_eq!(saved.members(ids[0], false).unwrap(), vec![1, 2]);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn selected_cards_merge_into_the_first_and_escape_leaves_selection_mode() {
    let path = file_with(
        "select-merge",
        &[Contact { email: "a@x.is".into(), ..contact(1, "Anna As") }, Contact { phone: "555 0100".into(), ..contact(2, "Bo Berg") }, contact(3, "Cara Cole")],
    );
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.menu_action(&ui, MENU_SELECT);
    app.activate(&ui, Target::Row(0), 0.0, 0.0, 0.0);
    app.menu_action(&ui, MENU_MERGE_SELECTION);
    assert_eq!(app.status.as_deref(), Some("Choose at least two contacts to merge"));
    assert!(app.selecting, "the choice is kept");
    app.activate(&ui, Target::Row(1), 0.0, 0.0, 0.0);
    assert!(app.menu_action(&ui, MENU_MERGE_SELECTION));
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(!app.selecting);
    let saved = store::load(&path);
    assert_eq!(saved.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 3]);
    let kept = saved.iter().find(|c| c.id == 1).unwrap();
    assert_eq!((kept.name.as_str(), kept.phone.as_str(), kept.email.as_str()), ("Anna As", "555 0100", "a@x.is"));
    app.menu_action(&ui, MENU_SELECT);
    app.key(&ui, Out::Named("Escape"), 0.0);
    assert!(!app.selecting);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn my_card_is_marked_found_again_and_forgotten_when_deleted() {
    let path = file_with("my-card", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.go_to_my_card(&ui);
    assert!(app.status.as_deref().unwrap().contains("No card is My Card yet"), "{:?}", app.status);
    app.open_detail(&ui, 2);
    assert!(app.menu_action(&ui, MENU_MY_CARD));
    assert_eq!(app.prefs.me, Some(2));
    let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
    assert_eq!(saved.me, Some(2), "My Card is saved with the display settings");
    app.close_detail(&ui);
    app.query.push_str("zzz");
    app.rebuild(&ui);
    assert!(app.listed().is_empty());
    assert!(app.menu_action(&ui, MENU_GO_MY_CARD));
    assert_eq!(app.screen, Screen::Detail(2));
    assert_eq!(app.listed().len(), 2, "going to My Card clears the search");
    app.remove_selected(&ui);
    assert_eq!(app.prefs.me, None);
    let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
    assert_eq!(saved.me, None);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn recent_changes_are_kept_between_runs_and_forgotten_when_deleted() {
    let path = file_with("recent", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(app.recent_ids().is_empty(), "the contacts loaded at launch are not changes");
    app.open_detail(&ui, 2);
    app.toggle_favorite(&ui);
    assert_eq!(app.recent_ids(), BTreeSet::from([2]));
    let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
    assert_eq!(saved.recent, BTreeSet::from([2]), "the change is saved with the display settings");

    let next_run = Contacts::new(&ui, 1200, 780, path.clone());
    assert_eq!(next_run.recent_ids(), BTreeSet::from([2]), "the change is still listed in the next run");

    app.remove_selected(&ui);
    let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
    assert!(saved.recent.is_empty(), "a deleted contact leaves the recent changes");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn next_and_previous_card_step_along_the_list_and_stop_at_the_ends() {
    let path = file_with("step", &[contact(1, "Anna As"), contact(2, "Bo Berg"), contact(3, "Cara Cole")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(app.menu_action(&ui, MENU_NEXT));
    assert_eq!(app.screen, Screen::Detail(1), "with no card open, next is the first card");
    app.step_card(&ui, true);
    app.step_card(&ui, true);
    assert_eq!(app.screen, Screen::Detail(3));
    app.step_card(&ui, true);
    assert_eq!(app.screen, Screen::Detail(3), "the last card has no next card");
    assert!(app.menu_action(&ui, MENU_PREVIOUS));
    assert_eq!(app.screen, Screen::Detail(2));
    app.close_detail(&ui);
    app.step_card(&ui, false);
    assert_eq!(app.screen, Screen::Detail(3), "with no card open, previous is the last card");
    app.edit_contact(&ui, Some(3));
    app.step_card(&ui, false);
    assert!(app.editing(), "stepping does not leave an open form");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_list_scrolls_just_enough_to_show_a_row() {
    assert_eq!(reveal_offset(100.0, 44.0, 0.0, 400.0), 0.0, "already in view");
    assert_eq!(reveal_offset(500.0, 44.0, 0.0, 400.0), 144.0, "below the window: its bottom lines up with the window's");
    assert_eq!(reveal_offset(100.0, 44.0, 300.0, 400.0), 76.0, "above the window: its section letter shows too");
    assert_eq!(reveal_offset(10.0, 44.0, 300.0, 400.0), 0.0, "never before the top");
}

#[test]
fn a_table_in_the_import_folder_is_mapped_added_and_becomes_the_last_import_list() {
    let path = file_with("import-csv", &[contact(1, "Anna As")]);
    put_import(&path, "people.csv", "Name,Phone,Email,Fax\nBo Berg,555 0200,bo@x.is,555 9999\nCara Cole,,cara@x.is,\nAnna As,,,\n");
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(!app.sidebar_rows().iter().any(|row| row.0 == LIST_LAST_IMPORT), "no import yet");
    app.import_vcards(&ui);
    assert!(app.error.is_none(), "{:?}", app.error);
    let status = app.status.clone().unwrap();
    assert!(status.contains("Imported 2 contacts") && status.contains("skipped 1 duplicates"), "{status}");
    assert!(status.contains("3 columns mapped, ignored: Fax"), "{status}");
    assert_eq!(app.last_import_ids(), BTreeSet::from([2, 3]));
    assert!(app.sidebar_rows().iter().any(|row| row.0 == LIST_LAST_IMPORT && row.1 == "Last Import"));
    assert!(app.menu_action(&ui, MENU_LAST_IMPORT));
    assert_eq!(app.filter, Filter::LastImport);
    assert_eq!(app.listed().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Bo Berg", "Cara Cole"]);
    let again = Contacts::new(&ui, 1200, 780, path.clone());
    assert_eq!(again.last_import_ids(), BTreeSet::from([2, 3]), "the last import is remembered");
    app.close_detail(&ui);
    put_import(&path, "later.ldif", "dn: cn=Dan Dahl\nobjectClass: person\ncn: Dan Dahl\nmail: dan@x.is\n");
    std::fs::remove_file(data_dir_of(&path).join("Import/people.csv")).unwrap();
    app.import_vcards(&ui);
    assert_eq!(app.last_import_ids(), BTreeSet::from([4]), "a new import replaces the list");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn an_import_that_adds_nothing_keeps_the_last_import_list() {
    let path = file_with("import-same", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.prefs.last_import = BTreeSet::from([1]);
    put_import(&path, "same.csv", "Name\nAnna As\n");
    app.import_vcards(&ui);
    assert_eq!(app.last_import_ids(), BTreeSet::from([1]));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_archive_backs_up_every_data_file_and_restores_it_after_a_second_confirmation() {
    let path = file_with("archive", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.new_group(&ui);
    app.groups.add_member(1, 2).unwrap();
    app.save_groups();
    app.export_archive();
    assert!(app.error.is_none(), "{:?}", app.error);
    let archive_file = data_dir_of(&path).join("Export").join(ARCHIVE_FILE);
    let text = std::fs::read_to_string(&archive_file).unwrap();
    assert!(text.starts_with("bifrost-contacts-archive 1\nfile\tcontacts.txt\t"), "{text}");
    assert!(text.contains("file\tgroups.txt\t"), "{text}");

    app.open_detail(&ui, 1);
    app.remove_selected(&ui);
    app.filter = Filter::Group(1);
    app.delete_group(&ui);
    assert_eq!(app.contacts.len(), 1);

    app.restore_archive(&ui);
    assert!(app.status.as_deref().unwrap().contains("Put a Contacts archive"), "{:?}", app.status);
    std::fs::create_dir_all(data_dir_of(&path).join("Import")).unwrap();
    std::fs::copy(&archive_file, data_dir_of(&path).join("Import").join(ARCHIVE_FILE)).unwrap();
    assert!(app.menu_action(&ui, MENU_RESTORE_ARCHIVE));
    assert!(app.confirm_restore, "the first choice only asks");
    assert_eq!(app.contacts.len(), 1);
    assert!(app.status.as_deref().unwrap().contains("Choose Restore from Archive again"), "{:?}", app.status);
    assert!(app.menu_action(&ui, MENU_RESTORE_ARCHIVE));
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.contacts.len(), 2, "both contacts are back");
    assert_eq!(app.groups.members(1, false).unwrap(), vec![2], "and the group with its member");
    assert_eq!(app.status.as_deref(), Some("Restored 2 contacts from contacts.bcarchive"));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn choosing_something_else_between_the_two_restore_choices_cancels_the_restore() {
    let path = file_with("archive-cancel", &[contact(1, "Anna As")]);
    put_import(&path, ARCHIVE_FILE, &archive::pack(&[("contacts.txt".to_string(), "springboard-contacts-v1\n".to_string())]));
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.menu_action(&ui, MENU_RESTORE_ARCHIVE);
    assert!(app.confirm_restore);
    app.menu_action(&ui, MENU_SIDEBAR);
    assert!(!app.confirm_restore);
    assert_eq!(app.contacts.len(), 1);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_damaged_archive_is_refused_and_a_half_good_one_puts_the_old_data_back() {
    let path = file_with("archive-bad", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    put_import(&path, ARCHIVE_FILE, "not an archive\n");
    app.confirm_restore = true;
    app.restore_archive(&ui);
    assert_eq!(app.error.as_deref(), Some("This file is not a Contacts archive"));
    assert_eq!(app.contacts.len(), 1);

    // a contacts file that reads and a groups file that does not
    let files = vec![
        ("contacts.txt".to_string(), "springboard-contacts-v1\n7\tBo\t\t\t\t0\n".to_string()),
        ("groups.txt".to_string(), "garbage\n".to_string()),
    ];
    put_import(&path, ARCHIVE_FILE, &archive::pack(&files));
    app.error = None;
    app.confirm_restore = true;
    app.restore_archive(&ui);
    assert_eq!(app.error.as_deref(), Some("The archive could not be read, so nothing was restored"));
    assert_eq!(app.contacts.len(), 1);
    assert_eq!(store::load(&path)[0].name, "Anna As", "the old contacts file is back");
    assert!(!path.with_file_name(GROUPS_FILE).exists(), "the groups file the archive brought is gone again");

    put_import(&path, ARCHIVE_FILE, &archive::pack(&[("evil.sh".to_string(), "x".to_string())]));
    app.error = None;
    app.confirm_restore = true;
    app.restore_archive(&ui);
    assert!(app.error.as_deref().unwrap().contains("does not use: evil.sh"), "{:?}", app.error);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn export_as_pdf_writes_the_list_or_the_open_card_and_print_without_a_service_says_so() {
    let path = file_with("pdf", &[Contact { phone: "555 0100".into(), ..contact(1, "Anna As") }, contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.export_pdf();
    assert!(app.error.is_none(), "{:?}", app.error);
    let list = std::fs::read(data_dir_of(&path).join("Export/Contacts.pdf")).unwrap();
    assert!(list.starts_with(b"%PDF"), "a PDF file");
    app.open_detail(&ui, 1);
    assert!(app.menu_action(&ui, MENU_EXPORT_PDF));
    assert!(data_dir_of(&path).join("Export/Anna As.pdf").exists(), "the open card is saved under its title");
    let (text, title) = app.printable();
    assert_eq!(title, "Anna As");
    assert!(text.contains("555 0100"), "{text}");
    app.close_detail(&ui);
    app.print_labels();
    assert_eq!(app.status.as_deref(), Some("None of the contacts shown has an address"));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn file_names_for_exports_keep_letters_and_drop_the_rest() {
    assert_eq!(file_stem("Anna As"), "Anna As");
    assert_eq!(file_stem("A/B: C?"), "A_B_ C_");
    assert_eq!(file_stem("///"), "contact");
    assert_eq!(file_stem("   "), "contact");
    assert_eq!(file_stem(&"x".repeat(60)).len(), 40);
}

#[test]
fn the_settings_switches_are_saved_and_change_what_the_app_does() {
    let path = file_with("switches", &[Contact { notes: "private".into(), ..contact(1, "Anna As") }]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.screen = Screen::Settings;
    let rows = app.settings_layout().switches;
    assert_eq!(rows.len(), 2 + N_DETAILS);
    assert!(app.controls(&ui).iter().any(|n| n.id == NODE_SETTING + 1 && n.label == "Include Notes in vCards" && n.value == "on"));
    assert!(app.activate_control(&ui, NODE_SETTING + 1));
    assert!(!app.prefs.export_notes);
    app.export_vcards(exchange::Version::V3);
    let exported = std::fs::read_to_string(data_dir_of(&path).join("Export/contacts-3.0.vcf")).unwrap();
    assert!(!exported.contains("NOTE"), "{exported}");
    let birthday = 2 + Detail::Birthday.index();
    app.toggle_setting(Setting::Template(Detail::Birthday.index()));
    assert!(app.setting_on(Setting::Template(Detail::Birthday.index())));
    assert_eq!(Setting::all()[birthday], Setting::Template(Detail::Birthday.index()));
    app.toggle_setting(Setting::Nicknames);
    let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
    assert!(!saved.export_notes && saved.prefer_nicknames);
    assert_eq!(saved.template, BTreeSet::from([Detail::Birthday]));
    app.screen = Screen::List;
    app.edit_contact(&ui, None);
    assert!(app.detail_shown[Detail::Birthday.index()], "a new card shows the template's fields");
    assert!(!app.detail_shown[Detail::Url.index()]);
    app.cancel_editor(&ui);
    app.edit_contact(&ui, Some(1));
    assert!(!app.detail_shown[Detail::Birthday.index()], "an old card shows only the fields it has");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_settings_screen_scrolls_to_reach_every_switch() {
    let path = file_with("settings-scroll", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 480, path.clone());
    app.menu_action(&ui, MENU_SIDEBAR);
    app.bar(&ui, BAR_SETTINGS);
    assert_eq!(app.screen, Screen::Settings);
    let layout = app.settings_layout();
    assert!(layout.height > app.h, "eleven switches do not fit a short window");
    let last = layout.switches.last().unwrap().1;
    assert!(last.y + last.h > app.h);
    app.scroll.set_max(layout.height - app.h);
    app.scroll.jump_to(layout.height - app.h);
    let scrolled = app.settings_layout().switches.last().unwrap().1;
    assert!(scrolled.y + scrolled.h <= app.h, "after scrolling the last switch is in view");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_intents_open_a_card_create_a_contact_and_search() {
    let path = file_with("intents", &[Contact { phone: "555 0100".into(), ..contact(1, "Anna As") }, contact(2, "Bo Berg")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    assert_eq!(app.open_card(&ui, "bo berg").unwrap(), (2, "Bo Berg".to_string()));
    assert_eq!(app.screen, Screen::Detail(2));
    assert_eq!(app.open_card(&ui, "1").unwrap().0, 1, "an identifier works too");
    assert_eq!(app.open_card(&ui, "555 0100").unwrap().0, 1, "so does a search hit");
    assert!(app.open_card(&ui, "Nobody").unwrap_err().contains("No contact matches"));
    assert!(app.open_card(&ui, "  ").is_err());

    let id = app.create_contact(&ui, "Cara Cole", "555 0300", "", "Acme").unwrap();
    assert_eq!(id, 3);
    assert_eq!(app.screen, Screen::Detail(3), "the new card is opened");
    let saved = store::load(&path);
    assert!(saved.iter().any(|c| c.id == 3 && c.phone == "555 0300" && c.company == "Acme"));
    let twice = app.create_contact(&ui, "Cara Cole", "", "", "").unwrap_err();
    assert!(twice.contains("already in the contacts"), "{twice}");
    assert!(app.create_contact(&ui, "", "", "", "").is_err());

    let found = app.search_contacts(&ui, "berg");
    assert_eq!(found, vec![(2, "Bo Berg".to_string(), String::new(), String::new())]);
    assert_eq!(&*app.query, "berg", "the search field shows the query");
    assert_eq!(app.search_contacts(&ui, "").len(), 3);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn card_value_rows_past_the_tenth_get_their_own_control_ids() {
    assert_eq!(copy_node(0), NODE_COPY);
    assert_eq!(copy_node(9), NODE_COPY + 9);
    assert_eq!(copy_node(10), NODE_COPY_MORE + 10);
    let path = file_with("many-rows", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    let mut card = CardDetails::default();
    for detail in Detail::ALL {
        card.set(detail, "value");
    }
    app.details.set(1, card);
    let mut ann = contact(1, "Anna As");
    ann.phone = "1".into();
    ann.email = "a@x.is".into();
    ann.address = "Gata 1".into();
    app.contacts = vec![ann];
    for n in 0..extras::MAX_EXTRAS {
        assert!(app.extras.add(1, Extra { kind: ValueKind::Phone, label: "home".into(), value: format!("55{n}") }));
    }
    app.open_detail(&ui, 1);
    let rows = app.rows_of(app.selected().unwrap());
    assert!(rows.len() > 10, "{} rows", rows.len());
    let controls = app.controls(&ui);
    let copies: Vec<u64> = controls.iter().filter(|n| n.label.starts_with("Copy ") && n.id != NODE_COPY_CONTACT).map(|n| n.id).collect();
    assert_eq!(copies.len(), rows.len());
    let unique: BTreeSet<u64> = copies.iter().copied().collect();
    assert_eq!(unique.len(), copies.len(), "no two controls share an id");
    let last = *copies.last().unwrap();
    assert!(app.activate_control(&ui, last), "the last row's control is found and pressed");
    assert!(app.toast_until.is_some(), "its value was copied");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn new_contact_plus_is_always_on_the_list_bar_and_cmd_n_makes_a_card() {
    // E2E W2-19: "+" sat behind six other buttons and always overflowed, and there was no New Card command.
    let path = file_with("plus-visible", &[contact(1, "Anna As")]);
    let ui = test_ui();
    for (w, h) in [(1200, 780), (1920, 1080), (900, 600), (2560, 1440)] {
        let app = Contacts::new(&ui, w, h, path.clone());
        let ids: Vec<u32> = app.list_bar().items.iter().filter(|i| i.id != 0).map(|i| i.id).collect();
        assert_eq!(ids, [BAR_SIDEBAR, BAR_ADD], "the list bar is the sidebar toggle and New Contact, as the stock app");
        let [list, _] = app.bar_layouts(&ui);
        assert!(list.visible.iter().any(|(id, _)| *id == BAR_ADD), "\"+\" is shown at {w}x{h}");
        assert!(list.overflow_button.is_none(), "nothing overflows the list bar at {w}x{h}");
    }
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    let file = app.main_menu().into_iter().find(|m| m.title == "File").unwrap();
    let new_card = file.items.iter().find(|i| i.id == MENU_NEW_CARD).expect("File > New Card");
    assert_eq!((new_card.title.as_str(), new_card.key.as_str()), ("New Card", "cmd+n"));
    assert!(new_card.enabled);
    assert!(app.menu_action(&ui, MENU_NEW_CARD));
    assert_eq!(app.screen, Screen::Editor(None), "Cmd+N opens a new card");
    assert_eq!(app.focus, Some(0), "typing goes into the first name");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn group_actions_lead_the_detail_bar_and_file_has_one_print() {
    let path = file_with("group-actions", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let mut app = Contacts::new(&ui, 1200, 780, path.clone());
    app.filter = Filter::Duplicates;
    assert!(app.detail_bar().items.iter().any(|i| i.id == BAR_MERGE_ALL), "Merge All moved to the detail bar");
    assert!(!app.list_bar().items.iter().any(|i| i.id == BAR_MERGE_ALL));
    let file = app.main_menu().into_iter().find(|m| m.title == "File").unwrap();
    // the app's own Print… (Cmd+P) is declared as such, so the shell replaces its own row with it instead of adding
    // a second one; no other item takes Print's title or key
    let prints: Vec<_> = file.items.iter().filter(|i| i.title.starts_with("Print\u{2026}") || i.title == "Print..." || i.key == "cmd+p").collect();
    assert_eq!(prints.len(), 1, "one Print…");
    assert!(prints[0].prints && prints[0].id == MENU_PRINT && prints[0].key == "cmd+p" && prints[0].title == "Print\u{2026}");
    assert!(file.items.iter().any(|i| i.id == MENU_NEW_GROUP && i.key == "shift+cmd+n"));
    assert!(app.menu_action(&ui, MENU_SETTINGS));
    assert_eq!(app.screen, Screen::Settings, "View > Contacts Settings replaces the toolbar gear");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn sidebar_labels_fit_without_truncation() {
    // E2E W2-19: "Possible Duplica..." and "Suggested Cont..." were cut at 1200 px.
    let path = file_with("sidebar-fit", &[contact(1, "Anna As")]);
    let ui = test_ui();
    let app = Contacts::new(&ui, 1200, 780, path.clone());
    assert!(app.sidebar_shown);
    let mut labels: Vec<String> = app.sidebar_rows().into_iter().filter(|r| r.0 != 0).map(|r| r.1).collect();
    labels.extend(["Suggested Contacts".to_string(), "Last Import".to_string()]);
    // plate inset, symbol, gaps and trailing room as the SDK source list draws a row
    let chrome = 2.0 * (SPACE_S + SPACE_XS / 2.0) + 20.0 + 3.0 * SPACE_S;
    for label in labels {
        let need = ui.measure(&label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + chrome;
        assert!(need <= app.side(), "{label} needs {need} pt, the sidebar is {} pt", app.side());
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_list_count_is_singular_for_one_contact() {
    assert_eq!(count_label(0), "0 Contacts");
    assert_eq!(count_label(1), "1 Contact");
    assert_eq!(count_label(2), "2 Contacts");
}
