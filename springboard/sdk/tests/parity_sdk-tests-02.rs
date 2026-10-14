//! Parity sdk-tests-02: headless tests of the spelling model (`bifrost_app::spellcheck`), the drawing model
//! (`bifrost_app::drawing_model`) and the disk session (`bifrost_app::disk_arbitration`). Nothing here opens a
//! window, a compositor or a disk; the checks run on plain values and in-memory stores.

mod spelling {
    use bifrost_app::spellcheck::{
        words, CheckingType, CheckingTypes, Dictionary, KeyValues, SpellChecker, WordList,
    };
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryStore(HashMap<String, String>);

    impl KeyValues for MemoryStore {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn put_string(&mut self, key: &str, value: &str) {
            self.0.insert(key.to_string(), value.to_string());
        }
    }

    fn english(words_in: &[&str]) -> SpellChecker {
        SpellChecker::new(Box::new(WordList::new("en", words_in.iter().copied())))
    }

    #[test]
    fn headless_words_split_on_spaces_and_punctuation() {
        let text = "Hello, world! 42 times.";
        let spans: Vec<&str> = words(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(spans, vec!["Hello", "world", "42", "times"]);
    }

    #[test]
    fn headless_words_keep_an_apostrophe_between_letters() {
        let text = "don't stop' they’re";
        let spans: Vec<&str> = words(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(spans, vec!["don't", "stop", "they’re"]);
    }

    #[test]
    fn headless_word_ranges_are_byte_offsets_on_char_boundaries() {
        let text = "héllo wörld";
        let ranges = words(text);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&text[ranges[0].clone()], "héllo");
        assert_eq!(&text[ranges[1].clone()], "wörld");
        assert_eq!(ranges[0].end, "héllo".len());
    }

    #[test]
    fn headless_a_word_list_lookup_ignores_case() {
        let list = WordList::new("en", ["Bifrost", "forest"]);
        assert!(list.contains("bifrost"));
        assert!(list.contains("FOREST"));
        assert!(!list.contains("forrest"));
        assert_eq!(list.language(), "en");
    }

    #[test]
    fn headless_suggestions_are_the_words_one_edit_away_in_order() {
        let list = WordList::new("en", ["cat", "cut", "bat", "cast", "dog"]);
        assert_eq!(list.suggest("cot"), vec!["cat", "cut"]);
    }

    #[test]
    fn headless_a_transposed_pair_is_one_edit_away() {
        let list = WordList::new("en", ["the", "tea", "ten"]);
        assert_eq!(list.suggest("teh"), vec!["tea", "ten", "the"]);
    }

    #[test]
    fn headless_check_spelling_finds_the_first_unknown_word_after_a_start() {
        let checker = english(&["hello", "there"]);
        let text = "hello wrold foo";
        assert_eq!(checker.check_spelling(text, 0), Some(6..11));
        assert_eq!(checker.check_spelling(text, 12), Some(12..15));
        assert_eq!(checker.check_spelling(text, 16), None);
    }

    #[test]
    fn headless_digit_runs_are_never_misspelled() {
        let checker = english(&["room"]);
        assert!(checker.check("room 404", CheckingTypes::SPELLING).is_empty());
    }

    #[test]
    fn headless_check_reports_each_unknown_word_as_a_spelling_result() {
        let checker = english(&["the", "quick"]);
        let results = checker.check("the quik fox", CheckingTypes::SPELLING);
        let kinds: Vec<CheckingType> = results.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, vec![CheckingType::Spelling, CheckingType::Spelling]);
        let spans: Vec<&str> = results.iter().map(|r| &"the quik fox"[r.range.clone()]).collect();
        assert_eq!(spans, vec!["quik", "fox"]);
    }

    #[test]
    fn headless_check_produces_nothing_without_the_spelling_type() {
        let checker = english(&["the"]);
        assert!(checker.check("zzz", CheckingTypes::GRAMMAR).is_empty());
        let both = CheckingTypes::GRAMMAR.union(CheckingTypes::SPELLING);
        assert_eq!(checker.check("zzz", both).len(), 1);
        assert!(both.contains(CheckingTypes::SPELLING));
        assert!(!CheckingTypes::LINK.contains(CheckingTypes::SPELLING));
    }

    #[test]
    fn headless_correction_is_the_first_guess_and_none_without_one() {
        let checker = english(&["cat", "cut"]);
        let text = "cot";
        assert_eq!(checker.correction(text, 0..3), Some("cat".to_string()));
        assert_eq!(checker.guesses(text, 0..3), vec!["cat", "cut"]);
        let unmatched = "zebra";
        assert_eq!(checker.correction(unmatched, 0..5), None);
    }

    #[test]
    fn headless_automatic_correction_is_off_by_default() {
        let mut checker = english(&[]);
        assert!(!checker.automatically_corrects_spelling);
        checker.automatically_corrects_spelling = true;
        assert!(checker.automatically_corrects_spelling);
    }

    #[test]
    fn headless_a_learned_word_is_kept_in_the_store_and_survives_a_reload() {
        let mut store = MemoryStore::default();
        let mut checker = english(&["hello"]);
        assert_eq!(checker.check_spelling("Bifrost", 0), Some(0..7));
        checker.learn_word("Bifrost", &mut store);
        assert!(checker.has_learned_word("bifrost"));
        assert_eq!(checker.check_spelling("Bifrost", 0), None);

        let mut reloaded = english(&["hello"]);
        assert_eq!(reloaded.check_spelling("Bifrost", 0), Some(0..7));
        reloaded.load(&store);
        assert!(reloaded.has_learned_word("BIFROST"));
        assert_eq!(reloaded.check_spelling("Bifrost", 0), None);
    }

    #[test]
    fn headless_unlearning_a_word_removes_it_from_the_store() {
        let mut store = MemoryStore::default();
        let mut checker = english(&[]);
        checker.learn_word("zork", &mut store);
        checker.unlearn_word("ZORK", &mut store);
        assert!(!checker.has_learned_word("zork"));

        let mut reloaded = english(&[]);
        reloaded.load(&store);
        assert!(!reloaded.has_learned_word("zork"));
        assert_eq!(reloaded.check_spelling("zork", 0), Some(0..4));
    }

    #[test]
    fn headless_an_empty_store_loads_no_learned_words() {
        let store = MemoryStore::default();
        let mut checker = english(&[]);
        checker.load(&store);
        assert!(!checker.has_learned_word("alpha"));
        assert_eq!(checker.check_spelling("alpha", 0), Some(0..5));
    }

    #[test]
    fn headless_an_ignored_word_lasts_only_for_its_checker() {
        let mut first = english(&[]);
        first.ignore_word("Gandalf");
        assert_eq!(first.check_spelling("Gandalf", 0), None);
        let second = english(&[]);
        assert_eq!(second.check_spelling("Gandalf", 0), Some(0..7));
    }

    #[test]
    fn headless_a_custom_dictionary_plugs_in_behind_the_trait() {
        struct Everything;
        impl Dictionary for Everything {
            fn language(&self) -> &str {
                "any"
            }
            fn contains(&self, _word: &str) -> bool {
                true
            }
            fn suggest(&self, _word: &str) -> Vec<String> {
                vec!["ok".to_string()]
            }
        }
        let checker = SpellChecker::new(Box::new(Everything));
        assert!(checker.check("anything at all", CheckingTypes::SPELLING).is_empty());
        assert_eq!(checker.available_languages(), vec!["any".to_string()]);
        assert_eq!(checker.correction("xyz", 0..3), Some("ok".to_string()));
    }

    #[test]
    fn headless_checking_types_combine_as_bit_sets() {
        let flags = CheckingTypes::DATE.union(CheckingTypes::ADDRESS).union(CheckingTypes::PHONE_NUMBER);
        assert_eq!(flags.0, 8 | 16 | 32);
        assert!(flags.contains(CheckingTypes::DATE));
        assert!(!flags.contains(CheckingTypes::LINK));
        assert_eq!(CheckingTypes::default(), CheckingTypes(0));
    }
}

mod drawing {
    use bifrost_app::drawing_model::{BezierPath, Gradient, LineCapStyle, LineJoinStyle, PageLayout, ShadowStyle, WindingRule};
    use bifrost_app::graphics_context::GraphicsContext;
    use bifrost_app::print::{Orientation, PaperSize, PrintInfo};

    const INK: u32 = 0xff12_3456;

    fn canvas(w: usize, h: usize) -> GraphicsContext {
        GraphicsContext::new(w, h)
    }

    #[test]
    fn headless_a_rect_path_paints_exactly_its_pixels() {
        let mut ctx = canvas(8, 8);
        BezierPath::with_rect(2.0, 2.0, 4.0, 3.0).fill(&mut ctx, INK);
        assert_eq!(ctx.layer.get(2, 2), INK);
        assert_eq!(ctx.layer.get(5, 4), INK);
        assert_eq!(ctx.layer.get(6, 2), 0, "the pixel just past the right edge stays clear");
        assert_eq!(ctx.layer.get(1, 2), 0, "the pixel just before the left edge stays clear");
        assert_eq!(ctx.layer.get(3, 5), 0, "the pixel just below the bottom edge stays clear");
    }

    #[test]
    fn headless_even_odd_fill_leaves_a_hole_that_non_zero_fills() {
        let mut outer = BezierPath::with_rect(0.0, 0.0, 8.0, 8.0);
        outer.append_rect(2.0, 2.0, 4.0, 4.0);

        let mut non_zero = canvas(8, 8);
        outer.fill(&mut non_zero, INK);
        assert_eq!(non_zero.layer.get(4, 4), INK, "same winding direction fills the inner square");

        outer.winding_rule = WindingRule::EvenOdd;
        let mut even_odd = canvas(8, 8);
        outer.fill(&mut even_odd, INK);
        assert_eq!(even_odd.layer.get(4, 4), 0, "even-odd leaves the inner square open");
        assert_eq!(even_odd.layer.get(1, 1), INK);
    }

    #[test]
    fn headless_a_stroke_paints_the_edge_and_not_the_interior() {
        let mut ctx = canvas(8, 8);
        let mut path = BezierPath::with_rect(1.0, 1.0, 5.0, 5.0);
        path.line_width = 1.0;
        path.stroke(&mut ctx, INK);
        assert_ne!(ctx.layer.get(1, 3) >> 24, 0, "the left edge is painted");
        assert_eq!(ctx.layer.get(3, 3), 0, "the interior of a stroked rect is clear");
    }

    #[test]
    fn headless_a_clip_limits_later_fills_to_the_clip_path() {
        let mut ctx = canvas(8, 8);
        ctx.save();
        BezierPath::with_rect(0.0, 0.0, 4.0, 4.0).add_clip(&mut ctx);
        BezierPath::with_rect(0.0, 0.0, 8.0, 8.0).fill(&mut ctx, INK);
        ctx.restore();
        assert_eq!(ctx.layer.get(1, 1), INK);
        assert_eq!(ctx.layer.get(6, 6), 0, "the fill outside the clip is dropped");
    }

    #[test]
    fn headless_the_bounds_of_an_oval_match_its_frame() {
        let (x, y, w, h) = BezierPath::with_oval(10.0, 20.0, 30.0, 12.0).bounds();
        assert!((x - 10.0).abs() < 0.05 && (y - 20.0).abs() < 0.05, "origin {x},{y}");
        assert!((w - 30.0).abs() < 0.05 && (h - 12.0).abs() < 0.05, "size {w}x{h}");
    }

    #[test]
    fn headless_rounded_corners_are_outside_the_path() {
        let p = BezierPath::with_rounded_rect(0.0, 0.0, 40.0, 40.0, 10.0, 10.0);
        assert!(!p.contains((0.5, 0.5)), "the corner outside the arc is not in the path");
        assert!(p.contains((20.0, 20.0)));
        assert!(p.contains((0.5, 20.0)), "the straight edge is inside");
    }

    #[test]
    fn headless_line_style_defaults_and_dash_pattern_are_stored() {
        let mut p = BezierPath::default();
        assert_eq!(p.line_width, 1.0);
        assert_eq!(p.line_cap_style, LineCapStyle::Butt);
        assert_eq!(p.line_join_style, LineJoinStyle::Miter);
        assert_eq!(p.line_dash(), (&[] as &[f32], 0.0));
        p.set_line_dash(&[4.0, 2.0], 1.5);
        assert_eq!(p.line_dash(), (&[4.0, 2.0][..], 1.5));
        p.set_line_dash(&[], 0.0);
        assert!(p.line_dash().0.is_empty(), "an empty pattern is a solid line");
    }

    #[test]
    fn headless_a_gradient_needs_at_least_one_colour() {
        assert!(Gradient::new(&[], None).is_none());
        let single = Gradient::new(&[INK], None).unwrap();
        assert_eq!(single.number_of_color_stops(), 1);
        assert_eq!(single.stop(0), Some((INK, 0.0)));
        assert_eq!(single.interpolated_color(0.7), INK);
    }

    #[test]
    fn headless_gradient_stops_spread_evenly_without_locations() {
        let g = Gradient::new(&[0xff00_0000, 0xffff_ffff, INK], None).unwrap();
        assert_eq!(g.number_of_color_stops(), 3);
        assert_eq!(g.stop(1), Some((0xffff_ffff, 0.5)));
        assert_eq!(g.stop(2), Some((INK, 1.0)));
        assert_eq!(g.stop(3), None);
    }

    #[test]
    fn headless_gradient_locations_are_clamped_and_sorted() {
        let g = Gradient::new(&[0xff00_0000, 0xffff_ffff], Some(&[1.5, -0.5])).unwrap();
        assert_eq!(g.stop(0), Some((0xffff_ffff, 0.0)));
        assert_eq!(g.stop(1), Some((0xff00_0000, 1.0)));
    }

    #[test]
    fn headless_a_gradient_mixes_each_channel_and_holds_its_ends() {
        let g = Gradient::new(&[0xff00_0000, 0xffff_ffff], None).unwrap();
        assert_eq!(g.interpolated_color(0.5), 0xff80_8080);
        assert_eq!(g.interpolated_color(-1.0), 0xff00_0000);
        assert_eq!(g.interpolated_color(2.0), 0xffff_ffff);
    }

    #[test]
    fn headless_the_gradient_offset_runs_across_a_rect_at_the_angle() {
        let g = Gradient::new(&[0xff00_0000, 0xffff_ffff], None).unwrap();
        let rect = (0.0, 0.0, 100.0, 50.0);
        assert!(g.linear_t(rect, 0.0, (0.0, 25.0)).abs() < 1e-5);
        assert!((g.linear_t(rect, 0.0, (100.0, 25.0)) - 1.0).abs() < 1e-5);
        assert!((g.linear_t(rect, 90.0, (50.0, 25.0)) - 0.5).abs() < 1e-5);
        assert!((g.radial_t((0.0, 0.0), 0.0, 10.0, (5.0, 0.0)) - 0.5).abs() < 1e-5);
        assert_eq!(g.radial_t((0.0, 0.0), 4.0, 4.0, (9.0, 9.0)), 0.0, "a zero span has no offset");
    }

    #[test]
    fn headless_a_shadow_grows_its_rect_by_offset_and_blur() {
        let s = ShadowStyle::new((3.0, 4.0), 2.0, INK);
        assert_eq!(s.extent((10.0, 10.0, 20.0, 20.0)), (11.0, 12.0, 24.0, 24.0));
        let flat = ShadowStyle::new((0.0, 0.0), -5.0, INK);
        assert_eq!(flat.blur_radius, 0.0, "a negative blur is clamped to zero");
        assert_eq!(flat.extent((1.0, 2.0, 3.0, 4.0)), (1.0, 2.0, 3.0, 4.0));
    }

    #[test]
    fn headless_page_layout_falls_back_to_scale_one_and_prints_at_least_one_page() {
        assert_eq!(PageLayout::new(-2.0).scaling, 1.0);
        let info = PrintInfo::default();
        let layout = PageLayout::new(1.0);
        assert_eq!(layout.page_grid(&info, (0.0, 0.0)), (1, 1), "an empty document still has one page");
        assert_eq!(layout.page_count(&info, (0.0, 0.0)), 1);
        assert_eq!(layout.page_rect(&info, (0.0, 0.0), 1), None);
    }

    #[test]
    fn headless_pages_are_numbered_in_reading_order_rows_first() {
        let mut info = PrintInfo::default();
        info.paper = PaperSize::Letter;
        let layout = PageLayout::new(1.0);
        let (w, h) = layout.content_size(&info);
        let doc = (w * 2.0, h * 2.0);
        assert_eq!(layout.page_grid(&info, doc), (2, 2));
        assert_eq!(layout.page_rect(&info, doc, 0), Some((0.0, 0.0, w, h)));
        assert_eq!(layout.page_rect(&info, doc, 1), Some((w, 0.0, w, h)));
        assert_eq!(layout.page_rect(&info, doc, 2), Some((0.0, h, w, h)));
        assert_eq!(layout.page_rect(&info, doc, 4), None);
    }

    #[test]
    fn headless_scaling_up_packs_fewer_pages_and_orientation_swaps_the_content_box() {
        let portrait = PrintInfo::default();
        let layout = PageLayout::new(2.0);
        let doc = (400.0, 400.0);
        assert_eq!(layout.page_count(&portrait, doc), 4, "half-size content on a doubled document");
        let landscape = bifrost_app::drawing_model::with_orientation(PrintInfo::default(), Orientation::Landscape);
        assert_eq!(landscape.orientation, Orientation::Landscape);
        let (pw, ph) = PageLayout::new(1.0).content_size(&portrait);
        let (lw, lh) = PageLayout::new(1.0).content_size(&landscape);
        assert!((pw - lh).abs() < 1e-3 && (ph - lw).abs() < 1e-3, "landscape swaps the axes");
    }
}

mod disks {
    use bifrost_app::disk_arbitration::{
        plan_from_reply, DiskSession, PlannedCommand, DESCRIPTION_DEVICE_MODEL_KEY, DESCRIPTION_MEDIA_BSD_NAME_KEY,
        DESCRIPTION_MEDIA_REMOVABLE_KEY, DESCRIPTION_MEDIA_SIZE_KEY, DESCRIPTION_MEDIA_WRITABLE_KEY,
        DESCRIPTION_VOLUME_PATH_KEY,
    };
    use bifrost_core::disk_events::{Disk, DiskEvent};
    use bifrost_core::disk_wire;
    use bifrost_core::json::Json;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<String>>>;

    fn usb(name: &str, mount: Option<&str>) -> Disk {
        Disk {
            bsd_name: name.into(),
            size: 4_000_000_000,
            removable: true,
            read_only: false,
            model: "Stick ".into(),
            mount_point: mount.map(String::from),
        }
    }

    fn list_reply(disks: &[Disk]) -> Result<Json, String> {
        Ok(Json::Obj(vec![("disks".into(), Json::Arr(disks.iter().map(disk_wire::encode_disk).collect()))]))
    }

    fn deliver(session: &mut DiskSession, e: DiskEvent) -> bool {
        let (name, body) = disk_wire::encode_event(&e);
        session.handle_event(disk_wire::SERVICE, name, &body)
    }

    fn recording_session() -> (DiskSession, Log) {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let mut session = DiskSession::create();
        let (a, d) = (log.clone(), log.clone());
        session.register_disk_appeared_callback(move |disk| a.lock().unwrap().push(format!("+{}", disk.bsd_name)));
        session.register_disk_disappeared_callback(move |name| d.lock().unwrap().push(format!("-{name}")));
        (session, log)
    }

    #[test]
    fn headless_a_new_session_knows_no_disks() {
        let session = DiskSession::create();
        assert!(session.disks().is_empty());
        assert!(session.disk_from_bsd_name("disk2").is_none());
        assert!(session.disk_from_volume_path("/media/usb").is_none());
    }

    #[test]
    fn headless_a_list_reply_reports_each_connected_disk_as_appeared() {
        let (mut session, log) = recording_session();
        let reply = list_reply(&[usb("sdb", Some("/media/usb")), usb("sdc", None)]);
        assert!(session.on_list_reply(&reply));
        let mut seen = log.lock().unwrap().clone();
        seen.sort();
        assert_eq!(seen, vec!["+sdb", "+sdc"]);
        assert_eq!(session.disks().len(), 2);
        assert_eq!(session.disk_from_bsd_name("sdc").map(|d| d.size), Some(4_000_000_000));
    }

    #[test]
    fn headless_a_reply_that_is_not_a_disk_list_is_refused() {
        let (mut session, log) = recording_session();
        assert!(!session.on_list_reply(&Err("no shell".into())));
        assert!(!session.on_list_reply(&Ok(Json::Null)));
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn headless_a_disappearance_is_reported_once_by_bsd_name() {
        let (mut session, log) = recording_session();
        assert!(session.on_list_reply(&list_reply(&[usb("sdb", None)])));
        assert!(deliver(&mut session, DiskEvent::Disappeared("sdb".into())));
        assert!(deliver(&mut session, DiskEvent::Disappeared("sdb".into())));
        let events: Vec<String> = log.lock().unwrap().iter().filter(|e| e.starts_with('-')).cloned().collect();
        assert_eq!(events, vec!["-sdb"]);
        assert!(session.disks().is_empty());
    }

    #[test]
    fn headless_a_disk_that_was_never_seen_leaving_is_a_disk_event_with_no_callback() {
        let (mut session, log) = recording_session();
        assert!(deliver(&mut session, DiskEvent::Disappeared("sdz".into())));
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn headless_a_repeated_appearance_is_not_reported_twice() {
        let (mut session, log) = recording_session();
        assert!(deliver(&mut session, DiskEvent::Appeared(usb("sdb", None))));
        assert!(deliver(&mut session, DiskEvent::Appeared(usb("sdb", None))));
        assert_eq!(*log.lock().unwrap(), vec!["+sdb"]);
        assert_eq!(session.disks().len(), 1);
    }

    #[test]
    fn headless_a_description_change_hands_over_the_disk_before_and_after() {
        let seen: Arc<Mutex<Vec<(Option<String>, Option<String>)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut session = DiskSession::create();
        let sink = seen.clone();
        session.register_disk_description_changed_callback(move |before, after| {
            sink.lock().unwrap().push((before.mount_point.clone(), after.mount_point.clone()));
        });
        let unmounted = usb("sdb", None);
        assert!(deliver(&mut session, DiskEvent::Appeared(unmounted.clone())));
        let mounted = usb("sdb", Some("/media/usb"));
        assert!(deliver(&mut session, DiskEvent::DescriptionChanged { before: unmounted, after: mounted.clone() }));
        assert_eq!(*seen.lock().unwrap(), vec![(None, Some("/media/usb".to_string()))]);
        assert_eq!(session.disk_from_volume_path("/media/usb"), Some(&mounted));
        assert_eq!(session.disks(), std::slice::from_ref(&mounted));
    }

    #[test]
    fn headless_handle_event_declines_events_from_other_services() {
        let mut session = DiskSession::create();
        assert!(!session.handle_event("network", "appeared", &Json::Null));
        assert!(session.disks().is_empty());
    }

    #[test]
    fn headless_the_description_carries_the_media_keys() {
        let mounted = DiskSession::description(&usb("sdb", Some("/media/usb")));
        assert_eq!(mounted[DESCRIPTION_MEDIA_BSD_NAME_KEY], "sdb");
        assert_eq!(mounted[DESCRIPTION_MEDIA_SIZE_KEY], "4000000000");
        assert_eq!(mounted[DESCRIPTION_MEDIA_REMOVABLE_KEY], "true");
        assert_eq!(mounted[DESCRIPTION_MEDIA_WRITABLE_KEY], "true");
        assert_eq!(mounted[DESCRIPTION_DEVICE_MODEL_KEY], "Stick", "the model is trimmed");
        assert_eq!(mounted[DESCRIPTION_VOLUME_PATH_KEY], "/media/usb");
    }

    #[test]
    fn headless_an_unmounted_read_only_disk_has_no_volume_path() {
        let mut disk = usb("sdc", None);
        disk.read_only = true;
        disk.model = "   ".into();
        let map = DiskSession::description(&disk);
        assert_eq!(map[DESCRIPTION_MEDIA_WRITABLE_KEY], "false");
        assert!(!map.contains_key(DESCRIPTION_VOLUME_PATH_KEY));
        assert!(!map.contains_key(DESCRIPTION_DEVICE_MODEL_KEY));
    }

    #[test]
    fn headless_a_plan_reply_yields_the_commands_in_order() {
        let command = Json::Obj(vec![
            ("program".into(), Json::Str("umount".into())),
            ("args".into(), Json::Arr(vec![Json::Str("/dev/sdb1".into())])),
        ]);
        let reply = Ok(Json::Obj(vec![("commands".into(), Json::Arr(vec![command]))]));
        assert_eq!(
            plan_from_reply(&reply),
            Ok(vec![PlannedCommand { program: "umount".into(), args: vec!["/dev/sdb1".into()] }])
        );
    }

    #[test]
    fn headless_a_refused_plan_carries_the_shell_sentence() {
        assert_eq!(plan_from_reply(&Err("refused: disk busy".into())), Err("refused: disk busy".to_string()));
        let empty = Ok(Json::Obj(vec![]));
        assert_eq!(plan_from_reply(&empty), Err("disks: the reply has no commands".to_string()));
    }
}

mod artwork {
    use bifrost_app::kit::{Font, Surface, Ui};
    use bifrost_app::widgets::{player, Player, PlayerLayout, PLAYER_PT};
    use bifrost_ui::Rect;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(
            std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice(),
        );
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn music() -> Player<'static> {
        Player { title: "Song", subtitle: "Artist", playing: true, progress: Some(0.5), skips: None }
    }

    #[test]
    fn headless_the_artwork_slot_is_a_square_inset_in_the_capsule() {
        let l = PlayerLayout::new(Rect { x: 10.0, y: 0.0, w: 400.0, h: 80.0 }, &music());
        assert_eq!(l.capsule.h, PLAYER_PT);
        assert_eq!(l.artwork.w, l.artwork.h, "the artwork slot is square");
        assert_eq!(l.artwork.x, l.capsule.x + 6.0, "six points of padding on the left");
        assert_eq!(l.artwork.y, l.capsule.y + 6.0, "six points of padding on top");
        assert!(l.artwork.bottom() <= l.capsule.bottom());
    }

    #[test]
    fn headless_the_artwork_slot_shrinks_with_a_shorter_bounds() {
        let l = PlayerLayout::new(Rect { x: 10.0, y: 0.0, w: 400.0, h: 40.0 }, &music());
        assert_eq!(l.capsule.h, 40.0, "the capsule is at most the bounds height");
        assert_eq!(l.artwork.w, 28.0, "the slot is the capsule height less the padding");
        assert_eq!(l.artwork.y, 6.0);
    }

    #[test]
    fn headless_the_title_text_sits_between_the_artwork_and_the_transport() {
        let with_skips = Player { skips: Some(("gobackward.15", "goforward.30")), ..music() };
        let l = PlayerLayout::new(Rect { x: 0.0, y: 0.0, w: 400.0, h: 52.0 }, &with_skips);
        assert!(l.text.x > l.artwork.right(), "the text starts after the artwork");
        assert!(l.text.right() <= l.back.unwrap().x, "the text ends before the back control");
        assert!(l.back.unwrap().right() <= l.play.x && l.play.right() <= l.forward.unwrap().x);
    }

    #[test]
    fn headless_a_live_stream_without_progress_has_no_progress_track() {
        let live = Player { progress: None, ..music() };
        let l = PlayerLayout::new(Rect { x: 0.0, y: 0.0, w: 300.0, h: 52.0 }, &live);
        assert!(l.progress.is_none());
        let with_progress = PlayerLayout::new(Rect { x: 0.0, y: 0.0, w: 300.0, h: 52.0 }, &music());
        let track = with_progress.progress.expect("a progress track under the text");
        assert_eq!(track.x, with_progress.text.x);
        assert!(track.bottom() <= with_progress.text.bottom());
    }

    #[test]
    fn headless_the_drawn_capsule_leaves_the_pixels_outside_its_bounds_clear() {
        let bounds = Rect { x: 10.0, y: 0.0, w: 400.0, h: 80.0 };
        let ui = ui();
        let mut px = vec![0u32; 420 * 90];
        let layout = player(&mut Surface { w: 420, h: 90, stride: 420, px: &mut px, y0: 0 }, &ui, bounds, &music(), None);
        assert_eq!(layout, PlayerLayout::new(bounds, &music()));
        assert_eq!(px[0], 0, "the top-left corner is outside the player");
        assert_eq!(px[20 * 420 + 5], 0, "the pixel left of the capsule is clear");
    }

    #[test]
    fn headless_the_artwork_slot_is_painted_under_the_player_artwork() {
        let bounds = Rect { x: 10.0, y: 0.0, w: 400.0, h: 80.0 };
        let ui = ui();
        let mut px = vec![0u32; 420 * 90];
        let layout = player(&mut Surface { w: 420, h: 90, stride: 420, px: &mut px, y0: 0 }, &ui, bounds, &music(), None);
        let cx = (layout.artwork.x + layout.artwork.w / 2.0) as usize;
        let cy = (layout.artwork.y + layout.artwork.h / 2.0) as usize;
        assert_ne!(px[cy * 420 + cx], 0, "the artwork slot is drawn at its centre");
    }
}
