#[test]
fn audio_unit_tables_declare_the_processors_an_app_ships() {
    let m = Manifest::parse("id = \"a.synth\"\nname = \"Synth\"\nexec = \"x\"\n\n[[audio_unit]]\nname = \"Pad\"\nkind = \"aumu\"\nsub_kind = \"pads\"\nmanufacturer = \"Exmp\"\noutputs = [2]\n\n[[audio_unit]]\nname = \"Broken\"\nkind = \"aufx\"\n").unwrap();
    assert_eq!(m.audio_units.len(), 1, "a table that fails the checks is left out");
    assert_eq!(m.audio_units[0].name, "Pad");
    assert_eq!(m.name, "Synth", "an audio unit table does not change the app keys");
    assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n").unwrap().audio_units.is_empty());
}
#[test]
fn package_types_are_declared_in_the_manifest() {
    let m = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\npackage_types = [\"rtfd\"]\n").unwrap();
    assert_eq!(m.package_types, vec!["rtfd".to_string()]);
    let none = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\n").unwrap();
    assert!(none.package_types.is_empty());
}
#[test]
fn records_audio_is_off_unless_declared() {
    let m = Manifest::parse("id = \"is.a.rec\"\nname = \"Rec\"\nexec = \"x\"\nrecords_audio = true\n").unwrap();
    assert!(m.records_audio);
    let no = Manifest::parse("id = \"is.a.rec\"\nname = \"Rec\"\nexec = \"x\"\n").unwrap();
    assert!(!no.records_audio);
}

#[test]
fn edit_keys_is_declared_by_text_editors_only() {
    let m = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\nedit_keys = true\n").unwrap();
    assert!(m.edit_keys);
    let no = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\nedit_keys = false\n").unwrap();
    assert!(!no.edit_keys);
    assert!(!Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\n").unwrap().edit_keys);
}
#[test]
fn file_manager_extensions_declare_the_folders_they_watch() {
    let m = Manifest::parse("id = \"is.a.cloud\"\nname = \"Cloud\"\nexec = \"x\"\nfile_manager_folders = [\"~/Cloud\", \"/mnt/sync\", \"~/\", \"/\", \"docs\", \"~/../etc\", \"/mnt/sync\"]\n").unwrap();
    assert_eq!(m.file_manager_folders.len(), 7);
    let home = Path::new("/home/ann");
    assert_eq!(m.file_manager_dirs(home), vec![PathBuf::from("/home/ann/Cloud"), PathBuf::from("/mnt/sync")]);
    let none = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\n").unwrap();
    assert!(none.file_manager_dirs(home).is_empty());
}
#[test]
fn quit_plan_kills_terminable_apps_and_gives_the_rest_a_deadline() {
    assert_eq!(super::quit_plan(true), super::QuitPlan::KillNow);
    assert_eq!(super::quit_plan(false), super::QuitPlan::QuitThenKill(super::QUIT_GRACE_MS));
}
#[test]
fn entity_tables_declare_the_types_search_indexes() {
    let m = Manifest::parse("id = \"a.b\"\nname = \"Notes\"\nexec = \"x\"\n\n[[entity]]\ntype = \"Note\"\n\n[[entity]]\ntype = \"Folder\"\nname = \"ignored\"\n\n[[intent]]\nid = \"open\"\ntitle = \"Open\"\n").unwrap();
    assert_eq!(m.entity_types, vec!["Note", "Folder"]);
    assert_eq!(m.name, "Notes", "an entity table does not change the app keys");
    assert_eq!(m.intents.len(), 1);
    assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n").unwrap().entity_types.is_empty());
}

#[test]
fn control_tables_reach_the_manifest_for_control_center() {
    let m = Manifest::parse("id = \"a.b\"\nname = \"Notes\"\nexec = \"x\"\n\n[[control]]\nkind = \"toggle\"\ntitle = \"Quiet Hours\"\nsymbol = \"moon\"\nintent = \"set-quiet\"\n").unwrap();
    assert_eq!(m.controls.len(), 1);
    assert_eq!((m.controls[0].title.as_str(), m.controls[0].intent.as_str()), ("Quiet Hours", "set-quiet"));
    assert_eq!(m.name, "Notes", "a control table does not change the app keys");
}

#[test]
fn intent_tables_do_not_leak_into_the_app_keys() {
    let m = Manifest::parse("id = \"a.b\"\nname = \"Notes\"\nexec = \"x\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\nshortcut = true\nparams = []\n\n[[intent]]\nid = \"find\"\ntitle = \"Find\"\nparams = [\"text:string\", \"due:date?\"]\n\n[other]\nname = \"ignored\"\n").unwrap();
    assert_eq!((m.id.as_str(), m.name.as_str()), ("a.b", "Notes"));
    assert_eq!(m.intents.len(), 2);
    assert!(m.intents[0].runnable());
    assert!(!m.intents[1].runnable());
    assert_eq!(m.intents[1].params, vec!["text:string", "due:date?"]);
}

use super::*;

#[test]
fn the_input_source_is_sent_on_first_contact_and_on_each_change() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    let next = |peer: &Channel| match peer.recv(false).unwrap() {
        sys::Recv::Packet(p, _) => Some(ToApp::decode(&p).unwrap()),
        _ => None,
    };
    host.sync_input_source(false);
    assert_eq!(next(&peer), Some(ToApp::InputSource { japanese: false }), "first contact sends the source");
    host.sync_input_source(false);
    assert_eq!(next(&peer), None, "unchanged: nothing sent");
    host.sync_input_source(true);
    assert_eq!(next(&peer), Some(ToApp::InputSource { japanese: true }));
}

/// 0.10.39: Mail restored behind other windows at 2560x1309 was never configured past its launch size (only the app
/// in front was), so the shell showed black past its 1200x860 frame until it came to the front.
#[test]
fn a_background_app_is_configured_to_its_window_on_the_desk() {
    let fg = Configure { w: 800, h: 600, scale: 2., safe_top: 0., safe_bottom: 120., dark: true, text_scale: 1., reduce_motion: false };
    let cfg = back_configure(fg, (2560, 1274), true, (1200, 860)).expect("a new size is sent");
    assert_eq!((cfg.w, cfg.h), (2560, 1274));
    assert_eq!(cfg.safe_bottom, 0.0, "the keyboard inset is the app in front's");
    assert_eq!((cfg.scale, cfg.dark), (2., true), "the rest follows the shell's settings");
    assert_eq!(back_configure(fg, (2560, 1274), true, (2560, 1274)), None, "already that size: nothing sent");
    assert_eq!(back_configure(fg, (2560, 1274), false, (1200, 860)), None, "not running");
    assert_eq!(back_configure(fg, (0, 1274), true, (1200, 860)), None, "an empty body is no size");
    // and a host sends it: the app sees the new size
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, fg);
    host.configure(cfg);
    assert_eq!(host.size(), (2560, 1274));
    let got = match peer.recv(false).unwrap() {
        sys::Recv::Packet(p, _) => ToApp::decode(&p).ok(),
        _ => None,
    };
    assert!(matches!(got, Some(ToApp::Configure(c)) if (c.w, c.h) == (2560, 1274)), "{got:?}");
}

#[test]
fn the_pasteboard_reaches_an_app_once_per_change_and_not_back_to_its_writer() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    let next = |peer: &Channel| match peer.recv(false).unwrap() {
        sys::Recv::Packet(p, _) => Some(ToApp::decode(&p).unwrap()),
        _ => None,
    };
    let text = |s: &str| vec![("public.utf8-plain-text".to_string(), s.as_bytes().to_vec())];
    host.sync_pasteboard(0, &[]);
    assert_eq!(next(&peer), None, "an empty board at first contact sends nothing");
    host.sync_pasteboard(3, &text("from another app"));
    assert_eq!(next(&peer), Some(ToApp::Pasteboard { change_count: 3, items: text("from another app") }));
    assert_eq!(next(&peer), Some(ToApp::Clipboard { text: "from another app".into() }), "the text fields read the plain clipboard");
    host.sync_pasteboard(3, &text("from another app"));
    assert_eq!(next(&peer), None, "unchanged: nothing sent");
    host.pasteboard_known(4);
    host.sync_pasteboard(4, &text("what this app wrote"));
    assert_eq!(next(&peer), None, "the writer has its own board");
    host.sync_pasteboard(5, &[("public.png".to_string(), vec![1])]);
    assert!(matches!(next(&peer), Some(ToApp::Pasteboard { change_count: 5, .. })));
    assert_eq!(next(&peer), Some(ToApp::Clipboard { text: String::new() }), "a board without text clears the app's text");
}

#[test]
fn a_large_pasteboard_reaches_the_app_in_front_through_a_descriptor_and_is_never_cut() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    let big = vec![("public.png".to_string(), vec![9u8; 200_000]), ("public.utf8-plain-text".to_string(), b"a picture".to_vec())];
    host.sync_pasteboard(5, &big);
    let (packet, mut fds) = match peer.recv(false).unwrap() {
        sys::Recv::Packet(p, fds) => (p, fds),
        _ => panic!("nothing sent"),
    };
    let Ok(ToApp::PasteboardBlob { change_count, types }) = ToApp::decode(&packet) else { panic!("a blob message") };
    assert_eq!(change_count, 5);
    let fd = fds.pop().expect("the descriptor travels with the message");
    assert_eq!(bifrost_proto::pbblob::read(&fd, &types).unwrap(), big, "every byte of every item");
    assert!(matches!(peer.recv(false).unwrap(), sys::Recv::Empty), "no cut text follows");
    host.sync_pasteboard(5, &big);
    assert!(matches!(peer.recv(false).unwrap(), sys::Recv::Empty), "unchanged: nothing sent");
}

#[test]
fn an_app_in_the_background_hears_the_count_and_never_the_contents() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    let next = |peer: &Channel| match peer.recv(false).unwrap() {
        sys::Recv::Packet(p, _) => Some(ToApp::decode(&p).unwrap()),
        _ => None,
    };
    host.notify_pasteboard(8);
    assert_eq!(next(&peer), Some(ToApp::PasteboardChanged { change_count: 8 }));
    host.notify_pasteboard(8);
    assert_eq!(next(&peer), None, "once per change");
    // coming to the front: the contents follow, and a board the app wrote itself needs no message at all
    host.sync_pasteboard(8, &[("public.utf8-plain-text".to_string(), b"copied elsewhere".to_vec())]);
    assert!(matches!(next(&peer), Some(ToApp::Pasteboard { change_count: 8, .. })));
    assert!(matches!(next(&peer), Some(ToApp::Clipboard { .. })));
    host.pasteboard_known(9);
    host.notify_pasteboard(9);
    assert_eq!(next(&peer), None, "the writer is not told about its own copy");
    // asking for the board again resets what the app is known to have
    host.pasteboard_wanted();
    host.sync_pasteboard(9, &[("public.utf8-plain-text".to_string(), b"again".to_vec())]);
    assert!(matches!(next(&peer), Some(ToApp::Pasteboard { change_count: 9, .. })));
}

#[test]
fn only_an_app_in_front_or_just_left_may_write_the_clipboard() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    assert!(host.may_write_clipboard(true, 1e6));
    assert!(!host.may_write_clipboard(false, 1e6), "an app that was never in front cannot");
    host.fg_seen_ms = 1e6;
    assert!(host.may_write_clipboard(false, 1e6 + PB_WRITE_GRACE_MS), "a copy and a quick switch still land");
    assert!(!host.may_write_clipboard(false, 1e6 + PB_WRITE_GRACE_MS + 1.0));
}

#[test]
fn a_pasteboard_write_through_a_descriptor_becomes_an_ordinary_write_and_a_bad_one_is_dropped() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    host.pb_may_write = true;
    let items = vec![("public.png".to_string(), vec![3u8; 100_000]), ("org.nspasteboard.ConcealedType".to_string(), Vec::new())];
    let (fd, types) = bifrost_proto::pbblob::write(&items).unwrap();
    let mut pumped = Pumped::default();
    host.handle(ToShell::PasteboardBlob { types: types.clone() }, vec![fd], &mut pumped);
    assert_eq!(pumped.requests, vec![ToShell::PasteboardSet { items }]);
    // no descriptor, or one shorter than the layout says: nothing is written
    let mut none = Pumped::default();
    host.handle(ToShell::PasteboardBlob { types }, vec![], &mut none);
    assert!(none.requests.is_empty());
    let (short, _) = bifrost_proto::pbblob::write(&[("t".to_string(), vec![1u8; 4])]).unwrap();
    host.handle(ToShell::PasteboardBlob { types: vec![("t".to_string(), 4000)] }, vec![short], &mut none);
    assert!(none.requests.is_empty());
}

#[test]
fn a_background_apps_blob_is_not_read_and_only_one_blob_is_read_per_pump() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    let items = vec![("public.png".to_string(), vec![3u8; 100_000])];
    // an app that may not write: the descriptor is dropped unread, no request, no allocation for its layout
    let (fd, types) = bifrost_proto::pbblob::write(&items).unwrap();
    let mut pumped = Pumped::default();
    host.handle(ToShell::PasteboardBlob { types }, vec![fd], &mut pumped);
    assert!(pumped.requests.is_empty());
    assert!(!pumped.blob_read);
    // an app that may write: the first blob of a pump is read, the next ones are dropped
    host.pb_may_write = true;
    for _ in 0..3 {
        let (fd, types) = bifrost_proto::pbblob::write(&items).unwrap();
        host.handle(ToShell::PasteboardBlob { types }, vec![fd], &mut pumped);
    }
    assert_eq!(pumped.requests, vec![ToShell::PasteboardSet { items }]);
}

#[test]
fn a_blob_waits_for_the_backlog_and_a_full_socket_does_not_copy_the_board_again() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    let board = vec![("public.png".to_string(), vec![5u8; 100_000])];
    // fill the socket until it refuses
    for _ in 0..100_000 {
        if host.chan.try_send(&[0u8; 4000], &[]).is_err() {
            break;
        }
    }
    host.sync_pasteboard(3, &board);
    assert_eq!(host.pb_known(), None, "the socket was full: nothing was sent");
    let first = host.pb_blob.as_ref().map(|(c, fd, _)| (*c, fd.0)).expect("the copy is kept for the retry");
    host.sync_pasteboard(3, &board);
    assert_eq!(host.pb_blob.as_ref().map(|(c, fd, _)| (*c, fd.0)), Some(first), "the same descriptor, not another copy");
    // a queued older message is ahead of the blob: nothing is built while the backlog is not empty
    host.pb_blob = None;
    host.backlog.push_back(vec![0u8; 8]);
    host.sync_pasteboard(4, &board);
    assert!(host.pb_blob.is_none());
    assert_eq!(host.pb_known(), None);
    drop(peer);
}

#[test]
fn a_board_with_content_is_sent_on_first_contact() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    host.sync_pasteboard(7, &[("public.utf8-plain-text".to_string(), b"kept".to_vec())]);
    let first = match peer.recv(false).unwrap() {
        sys::Recv::Packet(p, _) => ToApp::decode(&p).unwrap(),
        _ => panic!("nothing sent"),
    };
    assert!(matches!(first, ToApp::Pasteboard { change_count: 7, .. }));
}

#[test]
fn appearance_is_sent_on_first_contact_and_only_when_it_changes() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    let next = |peer: &Channel| match peer.recv(false).unwrap() {
        sys::Recv::Packet(p, _) => Some(ToApp::decode(&p).unwrap()),
        _ => None,
    };
    host.sync_appearance(0);
    assert_eq!(next(&peer), Some(ToApp::Appearance { flags: 0 }), "first contact sends even all-off");
    host.sync_appearance(0);
    assert_eq!(next(&peer), None, "unchanged: nothing sent");
    host.sync_appearance(bifrost_proto::appearance::SHOW_BORDERS);
    assert_eq!(next(&peer), Some(ToApp::Appearance { flags: bifrost_proto::appearance::SHOW_BORDERS }));
}

#[test]
fn the_light_scheme_is_a_flag_and_the_config_dark_is_its_inverse() {
    let _lock = crate::test_support::global_lock();
    use bifrost_proto::appearance::LIGHT;
    bifrost_render::appearance::jump_to(true);
    assert_ne!(appearance_flags() & LIGHT, 0, "light scheme sends the LIGHT bit");
    bifrost_render::appearance::jump_to(false);
    assert_eq!(appearance_flags() & LIGHT, 0, "dark scheme clears it");
}

#[test]
fn highlight_sidebar_icons_and_scroll_bars_ride_in_the_appearance_flags() {
    let _lock = crate::test_support::global_lock();
    use bifrost_proto::appearance::*;
    use bifrost_render::ui;
    let keep = (ui::highlight_index(), ui::sidebar_icon_size(), ui::scroll_bars_mode());
    ui::set_highlight(4);
    ui::set_sidebar_icon_size(2);
    ui::set_scroll_bars(2);
    let f = appearance_flags();
    assert_eq!(field(f, HIGHLIGHT_MASK, HIGHLIGHT_SHIFT), Some(4));
    assert_eq!(field(f, SIDEBAR_ICON_MASK, SIDEBAR_ICON_SHIFT), Some(2));
    assert_eq!(field(f, SCROLL_BARS_MASK, SCROLL_BARS_SHIFT), Some(2));
    ui::set_highlight(0);
    ui::set_sidebar_icon_size(0);
    ui::set_scroll_bars(0);
    let f = appearance_flags();
    assert_eq!(field(f, HIGHLIGHT_MASK, HIGHLIGHT_SHIFT), Some(0), "follow the accent is sent, not left unsaid");
    assert_eq!(field(f, SIDEBAR_ICON_MASK, SIDEBAR_ICON_SHIFT), Some(0));
    assert_eq!(field(f, SCROLL_BARS_MASK, SCROLL_BARS_SHIFT), Some(0));
    assert_eq!(field(0, HIGHLIGHT_MASK, HIGHLIGHT_SHIFT), None, "an older shell says nothing");
    ui::set_highlight(keep.0);
    ui::set_sidebar_icon_size(keep.1);
    ui::set_scroll_bars(keep.2);
}

#[test]
fn the_accent_colour_rides_in_the_appearance_flags() {
    let _lock = crate::test_support::global_lock();
    use bifrost_proto::appearance::{ACCENT_MASK, ACCENT_SHIFT};
    use bifrost_render::appearance::{accent, set_accent, Accent};
    let keep = accent();
    for a in Accent::ALL {
        set_accent(a);
        assert_eq!(Accent::from_wire((appearance_flags() & ACCENT_MASK) >> ACCENT_SHIFT), Some(a));
    }
    set_accent(keep);
}

#[test]
fn only_the_keyboard_system_app_may_type_into_the_focused_field() {
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let typed = ToShell::KeyboardOut(bifrost_proto::KeyboardOut::Text("x".into()));
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut other = AppHost::with_channel("is.olibuijr.calculator", channel, None, cfg.clone());
    let mut pumped = Pumped::default();
    other.handle(typed.clone(), vec![], &mut pumped);
    assert!(pumped.requests.is_empty(), "an ordinary app cannot inject keystrokes");
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut keyboard = AppHost::with_channel(KEYBOARD_APP_ID, channel, None, cfg);
    keyboard.handle(typed.clone(), vec![], &mut pumped);
    assert_eq!(pumped.requests, vec![typed]);
}

#[test]
fn a_hello_from_another_protocol_marks_the_host_incompatible_instead_of_closing_it() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    let mut pumped = Pumped::default();
    host.handle(ToShell::Hello { sdk_version: SDK_VERSION + 1, app_id: "fixture".into() }, vec![], &mut pumped);
    assert!(host.incompatible && !host.hello);
    assert!(!pumped.died, "the window stays up to show the message");
}

#[test]
fn an_incompatible_binary_is_remembered_so_the_shell_never_respawns_it() {
    let exec = PathBuf::from("/var/lib/springboard/apps/loop-test/1.4.1/bin");
    assert!(!is_known_incompatible("loop-test", &exec));
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("loop-test", channel, None, cfg);
    host.exec = exec.clone();
    host.handle(ToShell::Hello { sdk_version: SDK_VERSION + 1, app_id: "loop-test".into() }, vec![], &mut Pumped::default());
    assert!(is_known_incompatible("loop-test", &exec));
    assert!(!is_known_incompatible("loop-test", Path::new("/var/lib/springboard/apps/loop-test/1.5.0/bin")), "a new version gets a fresh try");
}

#[test]
fn a_host_is_outdated_only_when_it_runs_a_different_binary_than_the_installed_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("1.7.0")).unwrap();
    std::fs::write(dir.path().join("1.7.0/bin"), b"").unwrap();
    let new = std::fs::canonicalize(dir.path().join("1.7.0/bin")).unwrap();
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("fixture", channel, None, cfg);
    assert!(!host.is_outdated(&new), "a test host has no binary of record, so it is never outdated");
    host.exec = dir.path().join("1.6.0/bin");
    assert!(host.is_outdated(&new), "the installed version moved on");
    host.exec = new.clone();
    assert!(!host.is_outdated(&new), "same binary: just focus it");
}

#[test]
fn a_dangling_current_during_the_flip_is_not_an_update() {
    // the update replaces `current` (remove, then link) or links it before the binary is in place: while the
    // installed path does not resolve, the running version stays the one of record and nothing is launched
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("apps/x");
    std::fs::create_dir_all(app.join("1.0.0")).unwrap();
    std::fs::write(app.join("1.0.0/bin"), b"").unwrap();
    std::os::unix::fs::symlink("1.0.0", app.join("current")).unwrap();
    let installed = app.join("current/bin");
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("x", channel, None, cfg);
    host.exec = resolved(&installed);
    std::fs::remove_file(app.join("current")).unwrap();
    assert!(!host.is_outdated(&installed), "no current link at all: not outdated");
    std::os::unix::fs::symlink("1.1.0", app.join("current")).unwrap();
    assert!(!host.is_outdated(&installed), "current names a folder not there yet: not outdated");
    std::fs::create_dir_all(app.join("1.1.0")).unwrap();
    assert!(!host.is_outdated(&installed), "the folder is there but the binary is not: still not outdated");
    std::fs::write(app.join("1.1.0/bin"), b"").unwrap();
    assert!(host.is_outdated(&installed), "the new binary is in place: now it is");
}

#[test]
fn an_update_that_flips_current_makes_the_running_host_outdated() {
    // the registry names `current/<exec>` before and after an update; the host keeps the version folder it started from
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("apps/x");
    for v in ["1.0.0", "1.1.0"] {
        std::fs::create_dir_all(app.join(v)).unwrap();
        std::fs::write(app.join(v).join("bin"), b"").unwrap();
    }
    std::os::unix::fs::symlink("1.0.0", app.join("current")).unwrap();
    let installed = app.join("current/bin");
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = AppHost::with_channel("x", channel, None, cfg);
    host.exec = resolved(&installed);
    assert!(!host.is_outdated(&installed), "still the installed version");
    std::fs::remove_file(app.join("current")).unwrap();
    std::os::unix::fs::symlink("1.1.0", app.join("current")).unwrap();
    assert!(host.is_outdated(&installed), "current moved to 1.1.0 while 1.0.0 runs");
}

#[test]
fn resize_keeps_old_buffer_dimensions_until_replacement_arrives() {
    let (channel,_peer)=sys::socketpair_pair().unwrap();
    let cfg=Configure {w:4,h:3,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false};
    let mut host=AppHost::with_channel("fixture",channel,None,cfg.clone());
    let mut pumped=Pumped::default();
    let buffers=|| vec![sys::shm_create("resize-a",48).unwrap(),sys::shm_create("resize-b",48).unwrap()];
    host.handle(ToShell::Buffers{w:4,h:3,generation:1},buffers(),&mut pumped);
    host.handle(ToShell::Frame{buf:0,damage:vec![[0,0,4,3]]},vec![],&mut pumped);
    let mut resized=cfg; resized.w=8; resized.h=6;
    host.configure(resized);
    assert_eq!(host.size(),(8,6));
    assert_eq!(host.frame().map(|(_,w,h)|(w,h)),Some((4,3)));
    let mut pixels=vec![0;8*6];
    host.blit(&mut Surface{w:8,h:6,stride:8,px:&mut pixels,y0:0},true);
    host.handle(ToShell::Buffers{w:8,h:6,generation:2},vec![sys::shm_create("resize-c",192).unwrap(),sys::shm_create("resize-d",192).unwrap()],&mut pumped);
    host.handle(ToShell::Frame{buf:0,damage:vec![[0,0,8,6]]},vec![],&mut pumped);
    assert_eq!(host.frame().map(|(_,w,h)|(w,h)),Some((8,6)));
}

#[test]
fn a_web_link_comes_only_from_the_app_in_front_once_a_second() {
    let mut last = None;
    assert!(!open_url_allowed(false, &mut last, 0.), "a background app cannot open a page");
    assert_eq!(last, None, "a refused request does not start the wait");
    assert!(open_url_allowed(true, &mut last, 5000.));
    assert!(!open_url_allowed(true, &mut last, 5500.), "a second request within a second is dropped");
    assert!(open_url_allowed(true, &mut last, 6000.));
}

#[test]
fn a_buffer_is_sealed_before_its_size_is_trusted() {
    let fd = sys::shm_create("seal-order", 4096).unwrap();
    assert!(sealed_len_at_least(&fd, 4096));
    assert!(sys::shm_resize(&fd, 0).is_err(), "the check sealed it: the app can no longer shrink it under the mapping");
    let short = sys::shm_create("seal-short", 64).unwrap();
    assert!(!sealed_len_at_least(&short, 4096), "a file shorter than the claim is refused");
    assert!(sys::shm_resize(&short, 0).is_err(), "and it stays sealed");
}

#[test]
fn manifest_animation_behavior_parses_and_defaults() {
    let base = "id = \"a.b\"\nname = \"A\"\nexec = \"a\"\n";
    use super::super::window_fx::AnimationBehavior as B;
    assert_eq!(Manifest::parse(base).unwrap().animation_behavior(), B::Default);
    assert_eq!(Manifest::parse(&format!("{base}animation_behavior = \"none\"\n")).unwrap().animation_behavior(), B::None);
}

#[test]
fn manifest_window_size_keys_give_fixed_initial_min_and_max_sizes() {
    let base = "id = \"a.b\"\nname = \"A\"\nexec = \"a\"\n";
    let fixed = Manifest::parse(&format!("{base}window_size = \"fixed:320x520\"\n")).unwrap().size_limits();
    assert_eq!(fixed, SizeLimits { initial: Some((320.0, 520.0)), min: Some((320.0, 520.0)), max: Some((320.0, 520.0)), fixed: true });
    let free = Manifest::parse(&format!("{base}window_size = \"800x600\"\nmin_size = \"400x300\"\nmax_size = \"1200 x 900\"\n")).unwrap().size_limits();
    assert_eq!(free, SizeLimits { initial: Some((800.0, 600.0)), min: Some((400.0, 300.0)), max: Some((1200.0, 900.0)), fixed: false });
    let bad = Manifest::parse(&format!("{base}window_size = \"fixed:0x10\"\nmin_size = \"wide\"\n")).unwrap().size_limits();
    assert_eq!(bad, SizeLimits::default(), "malformed sizes are ignored");
}

#[test]
fn content_limits_from_the_app_replace_the_manifest_ones_except_on_a_fixed_window() {
    let base = "id = \"a.b\"\nname = \"A\"\nexec = \"a\"\n";
    let free = Manifest::parse(&format!("{base}window_size = \"800x600\"\nmin_size = \"400x300\"\nmax_size = \"1200x900\"\n")).unwrap().size_limits();
    let ours = free.with_content_limits(Some((560.0, 500.0)), None);
    assert_eq!(ours, SizeLimits { initial: Some((800.0, 600.0)), min: Some((560.0, 500.0)), max: Some((1200.0, 900.0)), fixed: false });
    assert_eq!(free.with_content_limits(None, None), free, "no limit of its own keeps the manifest's");
    let fixed = Manifest::parse(&format!("{base}window_size = \"fixed:320x520\"\n")).unwrap().size_limits();
    assert_eq!(fixed.with_content_limits(Some((10.0, 10.0)), Some((2000.0, 2000.0))), fixed);
}

#[test]
fn manifest_parses_and_rejects_unsafe_paths() {
    let m = Manifest::parse("# c\nid = \"is.olibuijr.calculator\"\nname = \"Calculator\"\nversion = \"1.0.0\"\nexec = \"calculator\"\nicon = \"icon.png\"\npermissions = [\"storage\", \"audio\"]\n").unwrap();
    assert_eq!((m.id.as_str(), m.exec.as_str(), m.permissions.len()), ("is.olibuijr.calculator", "calculator", 2));
    assert!(Manifest::parse("id = \"a/b\"\nname = \"x\"\nexec = \"x\"").is_none());
    assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"../x\"").is_none());
    assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"/bin/sh\"").is_none());
    assert!(Manifest::parse("id = \"a.b\"\nexec = \"x\"").is_none());
    assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"").is_none(), "an app needs an exec");
    let sys = Manifest::parse("id = \"a.b\"\nname = \"x\"\nkind = \"system\"\nprovides = [\"keyboard\"]").unwrap();
    assert!(sys.is_system() && sys.exec.is_empty() && sys.provides == ["keyboard"]);
    assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nkind = \"system\"\nexec = \"../x\"").is_none());
}

#[test]
fn the_window_style_comes_from_the_manifest_beside_the_executable() {
    let root = std::env::temp_dir().join(format!("system-pkgs-window-style-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let write = |dir: &str, extra: &str| {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.toml"), format!("id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n{extra}")).unwrap();
        d.join("x")
    };
    assert_eq!(manifest_window_style(&write("full", "window = \"full-size\"\n")).as_deref(), Some("full-size"));
    assert_eq!(manifest_window_style(&write("classic", "")).as_deref(), Some(""), "a classic window is told apart from an unknown one");
    assert_eq!(manifest_window_style(&root.join("none/x")), None, "no manifest: the app keeps the full-size layout");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn privacy_location_purpose_is_read_from_its_own_table() {
    let text = "id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n[privacy]\nlocation = \"Finna næstu stöð\"\n[[intent]]\nid = \"i\"\nlocation = \"not this\"\n";
    let m = Manifest::parse(text).unwrap();
    assert_eq!(m.location_purpose, "Finna næstu stöð");
    assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"x\"").unwrap().location_purpose.is_empty());
    let after = Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n[privacy]\n[window]\nlocation = \"other table\"\n").unwrap();
    assert!(after.location_purpose.is_empty(), "a later table ends [privacy]");
}

#[test]
fn scan_hides_system_packages_from_the_home_grid() {
    let root = std::env::temp_dir().join(format!("system-pkgs-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mk = |id: &str, manifest: &str, exec: bool| {
        let d = root.join("apps").join(id).join("current");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.toml"), manifest).unwrap();
        if exec {
            std::fs::write(d.join("x"), b"ELF").unwrap();
        }
    };
    mk("is.o.calc", "id = \"is.o.calc\"\nname = \"Calc\"\nexec = \"x\"\n", true);
    mk("is.o.kb", "id = \"is.o.kb\"\nname = \"Keyboard\"\nkind = \"system\"\nprovides = [\"keyboard\"]\n", false);
    mk("is.o.kb2", "id = \"is.o.kb2\"\nname = \"Kb2\"\nkind = \"system\"\nexec = \"x\"\n", true);
    let ids: Vec<String> = scan(&root).into_iter().map(|a| a.manifest.id).collect();
    assert_eq!(ids, ["is.o.calc"]);
    std::fs::remove_dir_all(&root).unwrap();
}
