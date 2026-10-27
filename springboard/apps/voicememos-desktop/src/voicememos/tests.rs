use super::*;

fn app(name: &str) -> (VoiceMemos, Ui) {
    let ui = springboard::shell::tests::ui();
    let dir = std::env::temp_dir().join(format!("voicememos-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    (VoiceMemos::new(&ui, 1200, 1920, dir.clone(), Wall::default(), &dir), ui)
}

#[test]
fn the_sidebar_title_shrinks_before_it_truncates() {
    let (_, ui) = app("title");
    let name = VoiceMemos::place_title(&Scope::Deleted);
    // a column the full-size title overflows but a smaller style fits: the whole name, at a smaller size
    let narrow = ui.measure(&name, MAC_LARGE_TITLE_PT, Weight::Bold) - ui.pt(1.0);
    let (text, pt) = VoiceMemos::sidebar_title(&ui, &name, narrow);
    assert_eq!(text, name);
    assert!(pt < MAC_LARGE_TITLE_PT);
    // wide enough: the large title
    let (text, pt) = VoiceMemos::sidebar_title(&ui, &name, ui.pt(1000.0));
    assert_eq!((text.as_str(), pt), (name.as_str(), MAC_LARGE_TITLE_PT));
    // too narrow for any style: truncated at the smallest, and never wider than the column
    let tiny = ui.measure(&name, MAC_TITLE3_PT, Weight::Bold) / 2.0;
    let (text, pt) = VoiceMemos::sidebar_title(&ui, &name, tiny);
    assert!(text.ends_with('…') && ui.measure(&text, pt, Weight::Bold) <= tiny);
}

/// The accent is process-wide: this test changes it and restores it.
static ACCENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn the_volume_slider_is_filled_with_the_chosen_accent_not_system_blue() {
    use bifrost_app::kit::appearance::{accent, set_accent, Accent};
    let _g = ACCENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, ui) = app("accent");
    app.volume = 0.6;
    let keep = accent();
    set_accent(Accent::Green);
    let green = accent_now();
    let (w, h) = (1200, 1920);
    let mut px = vec![0u32; w * h];
    app.draw_volume(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui);
    set_accent(keep);
    assert!(px.contains(&green), "the filled part is the accent");
    assert!(!px.contains(&TINT), "no fixed systemBlue left");
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn opening_the_app_opens_no_device() {
    let (app, _) = app("idle");
    assert!(!app.recording && app.playing.is_none());
    assert!(app.next_wake(0.0).is_none(), "nothing polls until the user acts");
}

#[test]
fn rename_and_delete_follow_the_selection() {
    let (mut app, ui) = app("flow");
    let wav = springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &vec![0; 4000]);
    store::save(&app.dir, "20261004-120000", &wav, 100).unwrap();
    app.reload();
    assert_eq!(app.memos.len(), 1);
    // the places come first, then the "Recordings" header, then the recordings
    let r = app.sidebar().item_rect(app.places().len() + 1, app.column(&ui), 0.0);
    let (x, y) = (ui.pt(r.x + 10.0), ui.pt(r.y + 10.0));
    app.touch(&ui, Event::Down(x, y), 0.0);
    app.touch(&ui, Event::Up(x, y), 10.0);
    assert_eq!(app.selected, Some(0));
    app.begin_rename(&ui);
    app.editor.as_mut().unwrap().field.set_text(&ui, "Standup");
    app.end_rename(true);
    assert_eq!(app.memos[0].title, "Standup");
    app.activate(&ui, Target::Delete);
    assert!(app.confirm_delete && app.memos.len() == 1, "the first tap only asks");
    app.activate(&ui, Target::Delete);
    assert!(app.visible.is_empty(), "the recording left the list");
    assert!(app.memos.len() == 1 && app.memos[0].deleted, "it waits in Recently Deleted");
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn trim_saves_the_kept_part_as_a_new_memo() {
    let (mut app, ui) = app("trim");
    let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
    store::save(&app.dir, "20261004-120000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &samples), 100).unwrap();
    app.reload();
    app.selected = Some(0);
    app.activate(&ui, Target::Trim);
    assert!(app.trim.is_some(), "Trim opens the trim view");
    if let Some(edit) = app.trim.as_mut() {
        edit.range.set_start(0.25);
        edit.range.set_end(0.5);
    }
    app.activate(&ui, Target::Done);
    assert!(app.trim.is_none());
    assert_eq!(app.memos.len(), 2, "the original stays");
    let kept = &app.memos[app.selected.unwrap()];
    assert_eq!(kept.duration_ms, 250);
    assert!(kept.title.ends_with(" trimmed"), "{}", kept.title);
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn enhance_saves_a_gated_copy_and_selects_it() {
    let (mut app, ui) = app("enhance");
    let samples: Vec<i16> = (0..8000).map(|i| if i < 4000 { 40 } else { 12000 }).collect();
    store::save(&app.dir, "20261004-120000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &samples), 100).unwrap();
    app.reload();
    app.selected = Some(0);
    assert!(app.menu(&ui, MENU_ENHANCE));
    assert_eq!(app.memos.len(), 2, "the original and the enhanced copy");
    let copy = &app.memos[app.selected.unwrap()];
    assert!(copy.title.ends_with(" enhanced"), "{}", copy.title);
    assert_eq!(copy.duration_ms, 1000, "the length is kept");
    assert!(app.error.is_empty());
    assert!(app.notice.starts_with("Enhanced copy saved as"), "{}", app.notice);
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn split_at_the_playhead_writes_two_parts_and_selects_the_first() {
    let (mut app, ui) = app("split");
    let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
    store::save(&app.dir, "20261004-120000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &samples), 100).unwrap();
    app.reload();
    app.selected = Some(0);
    assert!(!app.can_split(), "a split needs the selected take to be playing");
    app.playing = Some(app.memos[0].path.clone());
    app.position_ms = 250.0;
    assert!(app.can_split());
    assert!(app.menu(&ui, MENU_SPLIT));
    assert!(app.playing.is_none(), "a split stops playback");
    let mut durations: Vec<u64> = app.memos.iter().map(|m| m.duration_ms).collect();
    durations.sort();
    assert_eq!(durations, vec![250, 750, 1000], "two parts and the original");
    let first = &app.memos[app.selected.unwrap()];
    assert!(first.title.ends_with(" part 1"), "{}", first.title);
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn skip_silence_is_a_menu_check_and_jumps_over_the_quiet_part() {
    let (mut app, ui) = app("skip");
    assert!(!app.skip_silence_on());
    assert!(app.menu(&ui, MENU_SKIP));
    assert!(app.skip_silence_on(), "the first pick turns it on");
    assert!(app.menu(&ui, MENU_SKIP));
    assert!(!app.skip_silence_on(), "the second pick turns it off");

    app.skip_silence = true;
    app.spans = vec![(500.0, 1000.0)];
    app.skip_silence_from(0.0);
    assert_eq!(app.position_ms, 500.0, "silence before the speech is skipped");
    assert_eq!(app.skip_guard, 500.0);
    app.skip_silence_from(100.0);
    assert_eq!(app.position_ms, 500.0, "a jump still in flight is waited for");
    app.skip_silence_from(600.0);
    assert_eq!(app.position_ms, 500.0, "inside speech nothing moves");
    app.skip_silence = false;
    app.skip_silence_from(1200.0);
    assert_eq!(app.position_ms, 500.0, "off means off");
}

#[test]
fn notes_save_beside_the_recording_and_the_menu_opens_them() {
    let (mut app, ui) = app("notes");
    store::save(&app.dir, "20261012-100000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &vec![0; 800]), 100).unwrap();
    app.reload();
    app.selected = Some(0);
    assert!(app.has_selection(), "a selected recording enables the menu");
    assert!(app.menu(&ui, MENU_NOTES));
    assert!(app.notes.is_some(), "Notes opens the notes card");
    app.key(&ui, Out::Text("Milk".into()), 0.0);
    app.key(&ui, Out::Enter, 0.0);
    app.key(&ui, Out::Text("Eggs".into()), 0.0);
    app.key(&ui, Out::Backspace, 0.0);
    app.key(&ui, Out::Text("s".into()), 0.0);
    app.activate(&ui, Target::Done);
    assert!(app.notes.is_none());
    assert_eq!(store::read_notes(&app.memos[0].path), "Milk\nEggs");
    assert_eq!(app.memos.len(), 1, "the notes file is not listed as a recording");
    app.menu(&ui, MENU_NOTES);
    assert_eq!(app.notes.as_ref().unwrap().view.text(), "Milk\nEggs");
    app.activate(&ui, Target::Cancel);
    assert!(app.notes.is_none());
    let _ = std::fs::remove_dir_all(&app.dir);
}

fn parse(text: &str) -> Json {
    bifrost_core::json::parse(text).unwrap()
}

fn one_recording(app: &mut VoiceMemos, samples: usize) {
    let wav = springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &vec![0; samples]);
    store::save(&app.dir, "20261004-120000", &wav, 100).unwrap();
    app.reload();
    app.selected = Some(0);
}

#[test]
fn a_new_app_asks_for_the_microphone_and_the_devices_and_opens_no_device() {
    let (mut app, _) = app("ask");
    assert!(matches!(app.take_wants().as_slice(), [Want::MicStatus, Want::ListDevices]));
    assert!(app.next_wake(0.0).is_none());
}

#[test]
fn a_denied_microphone_refuses_to_record_and_the_app_asks_again() {
    let (mut app, _) = app("denied");
    app.take_wants();
    app.reply(Ask::Mic, &Ok(parse(r#"{"media":"microphone","status":"denied"}"#)));
    app.start_recording();
    assert!(!app.recording, "no recording without the permission");
    assert!(app.error.contains("Microphone access is off"), "{}", app.error);
    assert!(matches!(app.take_wants().as_slice(), [Want::MicStatus]));
}

#[test]
fn a_missing_privacy_service_never_records() {
    let (mut app, _) = app("noservice");
    app.reply(Ask::Mic, &Err("no service privacy".into()));
    assert!(app.mic.is_none());
    app.take_wants();
    app.start_recording();
    assert!(!app.recording, "no answer, no capture");
    assert!(matches!(app.take_wants().as_slice(), [Want::MicRequest]));
    app.reply(Ask::Mic, &Err("no service privacy".into()));
    assert!(!app.recording && app.mic_wait.is_none());
    assert!(app.error.contains("could not be asked"), "{}", app.error);
}

#[test]
fn an_unanswered_microphone_asks_the_person_and_records_only_after_allow() {
    let (mut app, ui) = app("consent");
    app.reply(Ask::Mic, &Ok(parse(r#"{"media":"microphone","status":"not_determined"}"#)));
    app.take_wants();
    app.start_recording();
    assert!(!app.recording, "nothing is captured before consent");
    assert!(matches!(app.take_wants().as_slice(), [Want::MicRequest]), "the shell's Allow sheet is asked for");
    // the request reply: still open while the sheet is up
    app.reply(Ask::Mic, &Ok(parse(r#"{"media":"microphone","status":"not_determined"}"#)));
    assert!(!app.recording && app.mic_wait.is_some());
    // the model polls the status while it waits
    let due = app.next_wake(0.0).expect("a wake while waiting");
    app.tick(&ui, due);
    assert!(matches!(app.take_wants().as_slice(), [Want::MicStatus]));
    // the person allowed it: the take starts (the audio service may be absent in a test, which only sets an error)
    app.reply(Ask::Mic, &Ok(parse(r#"{"media":"microphone","status":"authorized"}"#)));
    assert!(app.mic_wait.is_none());
    assert_eq!(app.mic, Some(Status::Authorized));
    assert!(app.recording || app.error.contains("audio service"), "{}", app.error);
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn a_dont_allow_answer_refuses_the_waiting_take() {
    let (mut app, _) = app("dontallow");
    app.take_wants();
    app.start_recording();
    assert!(app.mic_wait.is_some());
    app.reply(Ask::Mic, &Ok(parse(r#"{"media":"microphone","status":"denied"}"#)));
    assert!(!app.recording && app.mic_wait.is_none());
    assert!(app.error.contains("not allowed"), "{}", app.error);
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn the_input_device_is_named_and_a_change_during_a_recording_is_noted() {
    let (mut app, _) = app("input");
    let listing = |input: &str| {
        parse(&format!(
            r#"{{"devices":[{{"id":"a","name":"Desk mic","input":true,"output":false,"sound_effects":false}},{{"id":"b","name":"USB mic","input":true,"output":false,"sound_effects":false}}],"defaults":{{"input":"{input}"}}}}"#
        ))
    };
    app.reply(Ask::Devices, &Ok(listing("a")));
    assert_eq!(app.input_name.as_deref(), Some("Desk mic"));
    app.recording = true;
    app.record_input = Some("a".into());
    app.reply(Ask::Devices, &Ok(listing("b")));
    assert_eq!(app.notice, "Input changed to USB mic");
    assert_eq!(app.input_name.as_deref(), Some("USB mic"));
    assert!(services::caption(app.mic, app.input_name.as_deref(), &app.notice).starts_with("Input changed"));
}

#[test]
fn a_route_or_device_change_asks_for_the_device_list_again() {
    let (mut app, _) = app("route");
    app.take_wants();
    app.devices_changed();
    assert!(matches!(app.take_wants().as_slice(), [Want::ListDevices]));
}

#[test]
fn a_playing_recording_is_published_and_withdrawn_when_playback_ends() {
    let (mut app, _) = app("now");
    one_recording(&mut app, 8000);
    app.take_wants();
    // the state play() sets, without a speaker
    let memo = app.memos[0].clone();
    app.playing = Some(memo.path.clone());
    if let Some(session) = app.session.as_mut() {
        session.set_content(content_for(&memo));
    }
    app.publish_playback();
    let body = app
        .take_wants()
        .into_iter()
        .find_map(|w| match w {
            Want::Publish(body) => Some(body),
            _ => None,
        })
        .expect("a publish");
    assert_eq!(body.str_field("state").as_deref(), Some("playing"));
    assert_eq!(body.str_field("title").as_deref(), Some(memo.title.as_str()));
    let cover = now_playing::ArtworkImage::from_json(body.get("artwork").expect("a cover")).expect("a readable cover");
    assert_eq!((cover.width(), cover.height()), (96, 96));
    assert!(cover.rgba().chunks_exact(4).any(|p| p[..3] == [(RED & 255) as u8, (RED >> 8 & 255) as u8, (RED >> 16 & 255) as u8]), "the waveform is in the app's red");
    assert_eq!(&cover.rgba()[..4], &[ARTWORK_BACKDROP[0], ARTWORK_BACKDROP[1], ARTWORK_BACKDROP[2], 255], "on the dark tile");
    assert!(body.encode().len() < 60_000, "a 96 px cover keeps the publish small");
    app.playing = None;
    app.end_playback();
    assert!(app.take_wants().iter().any(|w| matches!(w, Want::Withdraw)));
    let _ = std::fs::remove_dir_all(&app.dir);
}

/// W5-12: a recording that disappears from the library while it is the Now Playing session (deleted in Files, or
/// trashed) takes the session off the media tile on the next reload.
#[test]
fn a_recording_that_went_away_withdraws_its_now_playing_session() {
    let (mut app, _) = app("gone");
    one_recording(&mut app, 8000);
    let memo = app.memos[0].clone();
    if let Some(session) = app.session.as_mut() {
        session.set_content(content_for(&memo));
    }
    app.publish_playback();
    app.take_wants();
    // a reload with the recording still there keeps the session
    app.reload();
    assert!(!app.take_wants().iter().any(|w| matches!(w, Want::Withdraw)));
    std::fs::remove_file(&memo.path).unwrap();
    app.reload();
    assert!(app.take_wants().iter().any(|w| matches!(w, Want::Withdraw)), "the deleted memo's session is withdrawn");
    assert!(app.session.as_ref().is_some_and(|s| s.content().is_none()));
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn a_shell_seek_or_skip_moves_the_playhead_within_the_recording() {
    let (mut app, _) = app("seek");
    one_recording(&mut app, 4000); // 500 ms
    app.command(now_playing::Command::Seek(900.0));
    assert_eq!(app.position_ms, 500.0, "clamped to the length");
    app.command(now_playing::Command::SkipBackward);
    assert_eq!(app.position_ms, 0.0, "clamped at the start");
    app.command(now_playing::Command::Next);
    assert_eq!(app.position_ms, 0.0, "next is not declared, so it does nothing");
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn the_input_meter_reads_silence_as_the_floor_and_full_scale_at_the_top() {
    assert_eq!(meter_fraction(0.0), 0.0);
    assert_eq!(meter_title(0.0), "Silent");
    assert!((meter_fraction(1.0) - 1.0).abs() < 1e-6);
    assert_eq!(meter_title(1.0), "0 dB");
    // power 0.25 is amplitude 0.5, about -6.02 dB: (60 - 6.02) / 60 = 0.8997
    assert!((meter_fraction(0.25) - 0.8997).abs() < 0.001, "{}", meter_fraction(0.25));
    assert_eq!(meter_title(0.25), "-6 dB");
    // power 1e-7 is amplitude 3.2e-4, about -70 dB: below the floor, so the bar is empty and it reads Silent
    assert_eq!(meter_fraction(1e-7), 0.0);
    assert_eq!(meter_title(1e-7), "Silent");
}

#[test]
fn the_speed_control_steps_and_is_kept_with_the_recording() {
    let (mut app, ui) = app("speed");
    // the speed belongs to a recording, so there must be one to select
    one_recording(&mut app, 800);
    assert_eq!(app.rate_now(), 1.0, "normal speed until one is chosen");
    app.activate(&ui, Target::SpeedUp);
    assert_eq!(app.rate_now(), 1.5);
    app.activate(&ui, Target::SpeedUp);
    app.activate(&ui, Target::SpeedUp);
    assert_eq!(app.rate_now(), 2.0, "stops at the fastest");
    let path = app.memos[0].path.clone();
    assert_eq!(store::read_rate(&path), 2.0, "saved beside the recording");
    // a fresh read of the selection gets the saved speed back
    app.rate_of = None;
    app.rate = 1.0;
    assert_eq!(app.rate_now(), 2.0);
    app.activate(&ui, Target::SpeedDown);
    assert_eq!(store::read_rate(&path), 1.5);
    assert_eq!(speed_label(app.rate_now()), "1.5x");
    assert_eq!(speed_label(1.0), "1x");
    assert_eq!(speed_label(0.5), "0.5x");
}

#[test]
fn dragging_the_volume_slider_sets_the_level_and_clamps_at_the_ends() {
    let (mut app, ui) = app("volume");
    one_recording(&mut app, 8000);
    let r = app.volume_rect(&ui);
    let y = r.1 + r.3 / 2.0;
    // a press a quarter of the way along the track sets the level to 0.25
    app.touch(&ui, Event::Down(r.0 + r.2 * 0.25, y), 0.0);
    app.touch(&ui, Event::Up(r.0 + r.2 * 0.25, y), 0.0);
    assert!((app.volume - 0.25).abs() < 1e-3, "{}", app.volume);
    // a drag past the right end clamps to full
    app.touch(&ui, Event::Down(r.0 + r.2 * 0.5, y), 0.0);
    app.touch(&ui, Event::Move(r.0 + r.2 * 2.0, y), 0.0);
    app.touch(&ui, Event::Up(r.0 + r.2 * 2.0, y), 0.0);
    assert_eq!(app.volume, 1.0);
    assert!(!app.volume_drag, "the drag ends on release");
    let _ = std::fs::remove_dir_all(&app.dir);
}

fn two_recordings(app: &mut VoiceMemos) {
    let wav = springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &vec![0; 4000]);
    store::save(&app.dir, "20261004-120000", &wav, 100).unwrap();
    store::save(&app.dir, "20261005-120000", &wav, 200).unwrap();
    app.reload();
}

#[test]
fn favourites_narrow_the_list_and_unmarking_leaves_it() {
    let (mut app, ui) = app("favourites");
    two_recordings(&mut app);
    assert_eq!(app.visible.len(), 2);
    app.selected = Some(0);
    assert!(!app.selected_favorite());
    assert!(app.menu(&ui, MENU_FAVORITE));
    assert!(app.selected_favorite(), "the first pick marks it");
    assert!(app.memos[0].favorite);
    app.set_scope(Scope::Favorites);
    assert_eq!(app.visible.len(), 1, "only the favourite is listed");
    assert!(app.selected.is_none(), "another place starts with nothing selected");
    app.selected = Some(app.visible[0]);
    app.activate(&ui, Target::Favorite);
    assert!(app.visible.is_empty(), "an unmarked recording leaves Favourites");
    assert!(app.selected.is_none());
    assert_eq!(app.place_count(&Scope::All), 2);
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn typing_in_the_search_field_filters_by_title_and_notes() {
    let (mut app, ui) = app("search");
    two_recordings(&mut app);
    let first = app.memos[1].path.clone();
    store::write_notes(&first, "buy milk").unwrap();
    // a press on the field gives it the keyboard
    let field = app.search_rect(&ui);
    let (x, y) = (ui.pt(field.x + 40.0), ui.pt(field.y + field.h / 2.0));
    app.touch(&ui, Event::Down(x, y), 0.0);
    app.touch(&ui, Event::Up(x, y), 10.0);
    assert!(app.search_active());
    app.key(&ui, Out::Text("MILK".into()), 0.0);
    assert_eq!(app.visible.len(), 1, "the notes are searched");
    assert_eq!(app.memos[app.visible[0]].path, first);
    app.key(&ui, Out::Backspace, 0.0);
    app.key(&ui, Out::Backspace, 0.0);
    app.key(&ui, Out::Backspace, 0.0);
    app.key(&ui, Out::Backspace, 0.0);
    assert_eq!(app.visible.len(), 2, "an empty search lists everything");
    app.key(&ui, Out::Text("20261005".into()), 0.0);
    assert_eq!(app.visible.len(), 1, "titles are searched");
    app.key(&ui, Out::Text("zzz".into()), 0.0);
    assert!(app.visible.is_empty());
    // the clear button empties the field and keeps the keyboard
    app.activate(&ui, Target::SearchClear);
    assert_eq!(app.visible.len(), 2);
    assert!(app.search_active());
    app.key(&ui, Out::Named("Escape"), 0.0);
    assert!(!app.search_active(), "Escape gives the keyboard back");
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn a_search_that_hides_the_selection_clears_it() {
    let (mut app, ui) = app("search-hide");
    two_recordings(&mut app);
    app.selected = Some(0);
    app.activate(&ui, Target::Search);
    app.key(&ui, Out::Text("20261004".into()), 0.0);
    assert!(app.selected.is_none(), "the newest recording does not match");
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn the_search_field_is_published_with_a_label() {
    let (app, ui) = app("search-node");
    let nodes = app.controls(&ui);
    let field = nodes.iter().find(|n| n.role == Role::TextField).expect("a text field");
    assert_eq!(field.label, "Search Recordings");
}

#[test]
fn a_press_on_a_place_shows_it() {
    let (mut app, ui) = app("places");
    two_recordings(&mut app);
    let r = app.sidebar().item_rect(1, app.column(&ui), 0.0);
    let (x, y) = (ui.pt(r.x + 10.0), ui.pt(r.y + 10.0));
    app.touch(&ui, Event::Down(x, y), 0.0);
    app.touch(&ui, Event::Up(x, y), 10.0);
    assert_eq!(app.scope, Scope::Favorites, "the second place is Favourites");
    assert!(app.visible.is_empty());
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn delete_goes_to_recently_deleted_where_it_can_be_recovered_or_removed_for_good() {
    let (mut app, ui) = app("trash");
    one_recording(&mut app, 4000);
    app.activate(&ui, Target::Delete);
    app.activate(&ui, Target::Delete);
    assert!(app.visible.is_empty());
    let last = app.places().len() - 1;
    app.activate(&ui, Target::Place(last));
    assert_eq!(app.scope, Scope::Deleted);
    assert_eq!(app.visible.len(), 1);
    app.selected = Some(app.visible[0]);
    assert!(!app.has_selection(), "edit commands do not apply to a deleted recording");
    app.activate(&ui, Target::Recover);
    assert!(app.visible.is_empty() && !app.memos[0].deleted, "the recording is back");
    assert!(app.notice.starts_with("Recovered"), "{}", app.notice);
    app.set_scope(Scope::All);
    assert_eq!(app.visible.len(), 1);

    // and once more, this time removed for good
    app.selected = Some(0);
    app.activate(&ui, Target::Delete);
    app.activate(&ui, Target::Delete);
    app.set_scope(Scope::Deleted);
    app.selected = Some(app.visible[0]);
    app.activate(&ui, Target::Delete);
    assert!(app.confirm_delete && app.memos.len() == 1, "Delete Forever asks first");
    app.activate(&ui, Target::Delete);
    assert!(app.memos.is_empty(), "nothing is left");
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn a_new_folder_takes_recordings_and_deleting_it_moves_them_up() {
    let (mut app, ui) = app("folders");
    one_recording(&mut app, 4000);
    assert!(app.menu(&ui, MENU_NEW_FOLDER));
    assert!(app.editor.is_some(), "the name is asked for");
    app.editor.as_mut().unwrap().field.set_text(&ui, "Work");
    app.end_rename(true);
    assert_eq!(app.scope, Scope::Folder("Work".into()), "the new folder is shown");
    assert_eq!(app.folders().to_vec(), vec!["Work".to_string()]);
    assert_eq!(app.record_dir(), app.dir.join("Work"), "new recordings go into the folder shown");

    // a second folder of the same name is refused
    app.menu(&ui, MENU_NEW_FOLDER);
    app.editor.as_mut().unwrap().field.set_text(&ui, "work");
    app.end_rename(true);
    assert!(app.error.contains("Cannot create the folder"), "{}", app.error);

    app.set_scope(Scope::All);
    app.selected = Some(0);
    assert!(app.menu(&ui, MENU_MOVE + 1), "the first folder");
    assert_eq!(app.memos[0].folder, "Work");
    assert_eq!(app.selected_folder(), "Work");
    app.set_scope(Scope::Folder("Work".into()));
    assert_eq!(app.visible.len(), 1);
    assert!(app.in_folder());
    assert!(app.menu(&ui, MENU_DELETE_FOLDER));
    assert_eq!(app.scope, Scope::All);
    assert!(app.folders().is_empty());
    assert_eq!(app.memos.len(), 1);
    assert_eq!(app.memos[0].folder, "", "the recording moved up");
    assert_eq!(app.visible.len(), 1);
    let _ = std::fs::remove_dir_all(&app.dir);
}

fn write_part(path: &Path, samples: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &vec![7; samples])).unwrap();
}

fn open_take(app: &mut VoiceMemos, target: &Path, current: &Path, stop_for: Option<StopFor>, replace: Option<Replacement>) {
    app.recording = true;
    app.recording_path = Some(current.to_path_buf());
    app.take = Some(Take { target: target.to_path_buf(), parts: Vec::new(), current: current.to_path_buf(), stop_for, paused: false, replace });
}

#[test]
fn a_paused_take_stays_open_and_its_parts_join_when_it_ends() {
    let (mut app, ui) = app("pause");
    let target = app.dir.join("Voice Memo 20261013-120000.wav");
    let (p0, p1) = (store::part_path(&target, 0), store::part_path(&target, 1));
    write_part(&p0, 4000);
    open_take(&mut app, &target, &p0, Some(StopFor::Pause), None);
    app.part_done(500);
    assert!(app.is_paused() && app.is_recording(), "a paused take is still open");
    assert_eq!(app.rec_base_ms, 500, "the timer keeps what was recorded");
    assert!(app.recording_path.is_none());
    assert!(app.memos.is_empty(), "nothing is listed until the take ends");
    // there is no audio service here, so a resume cannot start and the take stays paused
    app.toggle_pause();
    assert!(app.error.contains("Cannot resume"), "{}", app.error);
    assert!(app.is_paused());
    assert!(!app.menu(&ui, 9999));

    // the second part, as a resume would have started it, and the end of the take
    write_part(&p1, 2000);
    if let Some(take) = app.take.as_mut() {
        take.current = p1.clone();
        take.paused = false;
        take.stop_for = Some(StopFor::Finish);
    }
    app.recording_path = Some(p1.clone());
    app.part_done(250);
    assert!(!app.recording && app.take.is_none());
    assert_eq!(app.memos.len(), 1);
    assert_eq!(app.memos[0].duration_ms, 750, "both parts, back to back");
    assert_eq!(app.memos[0].title, "Voice Memo 20261013-120000");
    assert!(!p0.exists() && !p1.exists(), "the parts are gone");
    assert_eq!(app.selected, Some(0), "the new recording is selected");
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn stopping_a_paused_take_saves_it_at_once() {
    let (mut app, _) = app("pause-stop");
    let target = app.dir.join("Voice Memo 20261013-130000.wav");
    let p0 = store::part_path(&target, 0);
    write_part(&p0, 4000);
    open_take(&mut app, &target, &p0, None, None);
    if let Some(take) = app.take.as_mut() {
        take.parts.push(p0.clone());
        take.paused = true;
    }
    app.finish_recording();
    assert!(!app.recording && app.take.is_none());
    assert_eq!(app.memos.len(), 1);
    assert_eq!(app.memos[0].duration_ms, 500);
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn a_take_that_failed_keeps_the_finished_parts() {
    let (mut app, _) = app("pause-abort");
    let target = app.dir.join("Voice Memo 20261013-140000.wav");
    let (p0, p1) = (store::part_path(&target, 0), store::part_path(&target, 1));
    write_part(&p0, 4000);
    write_part(&p1, 100);
    open_take(&mut app, &target, &p1, None, None);
    if let Some(take) = app.take.as_mut() {
        take.parts.push(p0.clone());
    }
    app.abort_take();
    assert_eq!(app.memos.len(), 1, "the part before the failure is saved");
    assert_eq!(app.memos[0].duration_ms, 500);
    assert!(!p1.exists(), "the part that failed is dropped");

    // a take that failed before anything was finished leaves nothing
    let target = app.dir.join("Voice Memo 20261013-150000.wav");
    let p0 = store::part_path(&target, 0);
    write_part(&p0, 100);
    open_take(&mut app, &target, &p0, None, None);
    app.abort_take();
    assert!(!app.recording && app.take.is_none() && !p0.exists());
    assert_eq!(app.memos.len(), 1);
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn replace_picks_a_span_then_splices_the_new_take_over_it() {
    let (mut app, ui) = app("replace");
    let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
    store::save(&app.dir, "20261013-100000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &samples), 100).unwrap();
    app.reload();
    app.selected = Some(0);
    assert!(app.menu(&ui, MENU_REPLACE));
    assert!(app.trim.as_ref().is_some_and(|t| t.replace), "Replace opens the span picker");
    if let Some(edit) = app.trim.as_mut() {
        edit.range.set_start(0.25);
        edit.range.set_end(0.5);
    }
    // the person allowed the microphone (without an answer Record waits for consent: `mic_wait`)
    app.reply(Ask::Mic, &Ok(parse(r#"{"media":"microphone","status":"authorized"}"#)));
    // Record starts the take; without an audio service it cannot, and says so
    app.activate(&ui, Target::Done);
    assert!(app.trim.is_none() && !app.recording);
    assert!(app.error.contains("audio service"), "{}", app.error);

    // the take as the service would have written it, ended
    let target = app.dir.join("Voice Memo 20261013-110000.wav");
    let p0 = store::part_path(&target, 0);
    write_part(&p0, 2000);
    let job = Replacement { memo: app.memos[0].path.clone(), start: 0.25, end: 0.5 };
    open_take(&mut app, &target, &p0, Some(StopFor::Finish), Some(job));
    app.part_done(250);
    assert_eq!(app.memos.len(), 2, "the original and the replaced copy");
    let copy = &app.memos[app.selected.unwrap()];
    assert!(copy.title.ends_with(" replaced"), "{}", copy.title);
    assert_eq!(copy.duration_ms, 1000, "a take as long as the span keeps the length");
    assert!(!p0.exists());
    let _ = std::fs::remove_dir_all(&app.dir);
}

#[test]
fn pause_only_applies_while_recording_and_the_input_menu_sets_the_default_input() {
    let (mut app, ui) = app("menus");
    app.take_wants();
    assert!(app.menu(&ui, MENU_PAUSE), "the item is declared");
    assert!(!app.is_paused() && !app.is_recording());

    app.inputs = vec![("usb".into(), "USB mic".into()), ("desk".into(), "Desk mic".into())];
    app.input_id = Some("desk".into());
    assert_eq!(app.input_choices(), vec![("USB mic".to_string(), false), ("Desk mic".to_string(), true)]);
    assert!(app.menu(&ui, MENU_INPUT));
    assert!(matches!(app.take_wants().as_slice(), [Want::SetInput(id)] if id == "usb"));
    assert_eq!(app.notice, "Input changed to USB mic");

    app.recording = true;
    app.menu(&ui, MENU_INPUT + 1);
    assert!(app.take_wants().is_empty(), "the input does not change during a take");
    assert!(app.error.contains("Stop the recording"), "{}", app.error);
}

#[test]
fn export_names_the_selected_recording() {
    let (mut app, _) = app("export");
    assert!(app.export_source().is_none(), "nothing selected");
    one_recording(&mut app, 800);
    let (name, path) = app.export_source().unwrap();
    assert_eq!(name, "Voice Memo 20261004-120000.wav");
    assert_eq!(path, app.memos[0].path);
    app.set_notice("Exported Voice Memo.wav");
    assert_eq!(app.notice, "Exported Voice Memo.wav");
    app.set_error("Cannot export: x");
    assert_eq!(app.error, "Cannot export: x");
    let _ = std::fs::remove_dir_all(&app.dir);
}
