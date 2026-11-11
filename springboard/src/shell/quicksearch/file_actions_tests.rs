use super::*;
use crate::files::fs::{kind_of, Entry};

fn ui() -> Ui {
    super::super::tests::ui()
}

fn entry(dir: &str, name: &str) -> Entry {
    Entry { kind: kind_of(name, false), name: name.into(), path: PathBuf::from(dir).join(name), is_dir: false, size: 1, mtime: 0, items: None }
}

/// The Files and Show in Folder groups of `query` over `files`, merged the way the panel merges them.
fn file_groups(query: &str, files: &[crate::files::fs::Entry]) -> Vec<Group> {
    let folded: Vec<String> = files.iter().map(|e| fuzzy::fold(&e.name)).collect();
    let corpus = Corpus { deildu: &[], deildu_folded: &[], contacts: &[], contacts_folded: &[], files, files_folded: &folded, events: &[], wallpapers: &[] };
    let (tables, hits) = build_tables(&corpus);
    let mut run = search_rank::Query::start(query, &sources_of(tables), SYNC_DEADLINE);
    run.wait();
    let merged = run.merge(PER_GROUP, &search_rank::Usage::default(), 0, &search_rank::NoSemantic);
    split_merged(&merged, &hits, None).2
}

#[test]
fn groups_that_arrive_late_follow_the_ones_on_screen() {
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    q.shown = vec!["Applications", "Files"];
    let g = |title: &'static str| Group { title, hits: vec![] };
    let out = q.keep_shown_order(vec![g("Search in Apps"), g("Settings"), g("Applications"), g("Files")]);
    let titles: Vec<&str> = out.iter().map(|g| g.title).collect();
    assert_eq!(titles, vec!["Applications", "Files", "Settings", "Search in Apps"]);
}

#[test]
fn a_file_match_comes_with_show_in_folder_for_the_top_match() {
    let files = vec![entry("/home/me/Docs", "Q1 report.pdf"), entry("/home/me/Pics", "Beach.png")];
    let groups = file_groups("report", &files);
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert_eq!(groups[0].title, "Files");
    assert_eq!(groups[0].hits, vec![Hit::File { path: PathBuf::from("/home/me/Docs/Q1 report.pdf"), name: "Q1 report.pdf".into(), modified: 0 }]);
    assert_eq!(groups[1], Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me/Docs"), name: "Q1 report.pdf".into() }] });
    assert!(file_groups("zzz", &files).is_empty());
    assert!(file_groups("  ", &files).is_empty());
}

#[test]
fn space_previews_an_arrow_selected_file_and_types_otherwise() {
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    let file = Hit::File { path: PathBuf::from("/home/me/a.txt"), name: "a.txt".into(), modified: 0 };
    let section = |u: &Ui| Section { header: u.text("Files", 20.0, Weight::Semibold, WHITE), rows: vec![(file.clone(), u.text("a.txt", 13.0, Weight::Regular, WHITE))], subs: vec![None] };
    q.sections = vec![section(&u)];
    assert_eq!(q.key(&u, 800, Out::Text(" ".into()), 0.0), None, "no row selected: the space is typed");
    assert_eq!(q.query(), " ");
    q.sections = vec![section(&u)];
    q.selected = Some((0, 0));
    assert_eq!(q.key(&u, 800, Out::Text(" ".into()), 0.0), Some(file), "Space previews the selected file");
    assert_eq!(q.query(), " ", "the query is untouched by the preview");
}

#[test]
fn space_on_a_definition_asks_for_the_whole_definition() {
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    let def = Hit::Definition { word: "weather".into(), dictionary: "Test".into(), preview: "n. air".into() };
    q.sections = vec![Section { header: u.text("Dictionary", 20.0, Weight::Semibold, WHITE), rows: vec![(def.clone(), u.text("weather", 13.0, Weight::Regular, WHITE))], subs: vec![None] }];
    q.selected = Some((0, 0));
    assert_eq!(q.key(&u, 800, Out::Text(" ".into()), 0.0), Some(def));
    assert!(q.space_preview, "the act shows the definition instead of opening Dictionary");
}

#[test]
fn cmd_return_reveals_the_selected_file_or_else_the_first_one() {
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    assert_eq!(q.reveal_hit(), None, "no file listed");
    let file = |p: &str| Hit::File { path: PathBuf::from(p), name: Path::new(p).file_name().unwrap().to_string_lossy().into_owned(), modified: 0 };
    let row = |h: Hit| (h, u.text("x", 13.0, Weight::Regular, WHITE));
    q.sections = vec![
        Section { header: Image::new(1, 1), rows: vec![row(Hit::App(AppId::Files))], subs: vec![None] },
        Section { header: Image::new(1, 1), rows: vec![row(file("/home/me/Docs/a.txt")), row(file("/home/me/Pics/b.png"))], subs: vec![None, None] },
    ];
    assert_eq!(q.reveal_hit(), Some(Hit::Reveal { folder: PathBuf::from("/home/me/Docs"), name: "a.txt".into() }), "the first file");
    q.selected = Some((1, 1));
    assert_eq!(q.reveal_hit(), Some(Hit::Reveal { folder: PathBuf::from("/home/me/Pics"), name: "b.png".into() }), "the selected file");
    q.selected = Some((0, 0));
    assert_eq!(q.reveal_hit(), Some(Hit::Reveal { folder: PathBuf::from("/home/me/Docs"), name: "a.txt".into() }), "an app row is not revealed");
}

#[test]
fn define_puts_a_dictionary_row_first_and_a_word_with_no_definition_gets_none() {
    // no dictionary defines this word: "define" still offers to look it up, first, so Return opens Dictionary
    let groups = results("define zqxjvw", &[]);
    assert_eq!(groups[0], Group { title: "Dictionary", hits: vec![Hit::Definition { word: "zqxjvw".into(), dictionary: String::new(), preview: String::new() }] });
    // a plain word is only defined when a dictionary has it
    assert!(results("zqxjvw", &[]).iter().all(|g| g.title != "Dictionary"));
    assert!(results("e2e", &[]).iter().all(|g| g.title != "Dictionary"));
}

/// E2E10: "lantern" found "Display & Brightness" through letters scattered over its keyword aliases.
#[test]
fn a_settings_pane_needs_a_contiguous_or_word_start_match() {
    let text = "display & brightness brightness dark mode appearance text size night shift";
    assert!(!strict_match("lantern", text), "scattered letters do not find a pane");
    assert!(strict_match("display", text), "a prefix does");
    assert!(strict_match("bright", text), "a word start does");
    assert!(strict_match("dark m", text), "a contiguous run over words does");
    assert!(!strict_match("ightn", text), "a run inside a word does not (E2E13 4)");
    assert!(strict_match("wi-f", "wi-fi network") && strict_match("fi", "wi-fi network"), "a word after punctuation starts a word");
    assert!(!strict_match("", text));
    let settings = |q: &str| results(q, &[]).into_iter().flat_map(|g| g.hits).filter(|h| matches!(h, Hit::Settings(_))).collect::<Vec<_>>();
    assert!(settings("lantern").is_empty(), "{:?}", settings("lantern"));
    assert!(settings("dark mode").contains(&Hit::Settings("Display & Brightness")), "an alias still finds its pane");
    assert_eq!(results("wi", &[])[0].hits, vec![Hit::Settings("Wi-Fi")]);
}

/// E2E12 1: "AAPL" listed "Set Wallpaper to ..." in Shortcuts through scattered letters, in Search and in Look
/// Up's Search section. Every section but Applications (and Deildu) is strict; an app's name is matched as a name
/// (`name_match`, see `an_app_name_matches_by_a_run_or_its_initials_not_by_scattered_letters`).
#[test]
fn only_app_names_match_by_scattered_letters() {
    let corpus = Corpus { deildu: &[], deildu_folded: &[], contacts: &[], contacts_folded: &[], files: &[], files_folded: &[], events: &[], wallpapers: &[] };
    let (tables, _) = build_tables(&corpus);
    for t in &tables {
        assert_eq!(t.strict, !matches!(t.name, "Applications" | "Deildu"), "{} strictness", t.name);
        assert_eq!(t.names, t.name == "Applications", "{} matches as a name", t.name);
    }
    let mut hits = BTreeMap::new();
    let title = "Set Wallpaper to Gullfoss Canyon";
    let row = table_row(&mut hits, "w".into(), title, String::new(), search_rank::Kind::Settings, Hit::Terminal(String::new()), fuzzy::fold(title));
    let shortcuts = Table { name: "Shortcuts", rows: vec![row], prefixes: Vec::new(), phones: Vec::new(), strict: true, names: false };
    use search_rank::Source;
    assert!(shortcuts.search("AAPL").is_empty(), "scattered letters do not find a shortcut");
    assert_eq!(shortcuts.search("wallp").len(), 1, "a word start does");
    assert_eq!(shortcuts.search("gullfoss c").len(), 1, "a contiguous run does");
    let mut hits = BTreeMap::new();
    let title = "Open Passwords";
    let row = table_row(&mut hits, "p".into(), title, String::new(), search_rank::Kind::Settings, Hit::Terminal(String::new()), fuzzy::fold(title));
    let shortcuts = Table { name: "Shortcuts", rows: vec![row], prefixes: Vec::new(), phones: Vec::new(), strict: true, names: false };
    assert!(shortcuts.search("words").is_empty(), "a run inside a word does not find a shortcut (E2E13 4)");
    assert_eq!(shortcuts.search("passw").len(), 1, "a word start does");
    assert_eq!(shortcuts.search("open").len(), 1, "a prefix does");
    let apps = tables.iter().find(|t| t.name == "Applications").expect("the apps table");
    assert!(apps.rows.iter().any(|(_, f)| fuzzy::Matcher::new("stg").rank_folded(f).is_some()), "the matcher itself still takes letters in order (Settings)");
    assert!(apps.search("stg").is_empty(), "but the apps section does not list by them");
    assert!(apps.search("sett").iter().any(|i| i.title == "Settings"));
}

/// E2E6 5: "tall" listed the app "Digital Color Meter" (digiTAL coLor). An app is found by a run of its name, a word
/// start, or its initials, and by its aliases as before.
#[test]
fn an_app_name_matches_by_a_run_or_its_initials_not_by_scattered_letters() {
    let name = fuzzy::fold("Digital Color Meter");
    assert!(!name_match("tall", &name), "letters scattered through the name");
    assert!(!name_match("dgtl", &name));
    assert!(name_match("dig", &name), "a prefix");
    assert!(name_match("color", &name) && name_match("met", &name), "a word start");
    assert!(name_match("color m", &name), "a run over words");
    assert!(name_match("gital", &name), "a run inside a word");
    assert!(name_match("dcm", &name) && name_match("dc", &name), "the initials, from the first word");
    assert!(!name_match("cm", &name) && !name_match("dm", &name), "not initials from the middle, or with a gap");
    assert!(!name_match("", &name));
    assert!(name_match("as", &fuzzy::fold("App Store")));
    assert!(!name_match("x", "term"));
    let mut hits = BTreeMap::new();
    let row = |hits: &mut BTreeMap<String, Hit>, title: &str| table_row(hits, title.into(), title, String::new(), search_rank::Kind::App, Hit::Terminal(String::new()), fuzzy::fold(title));
    let rows = vec![row(&mut hits, "Digital Color Meter"), row(&mut hits, "Tally")];
    let table = Table { name: "Applications", rows, prefixes: vec![fuzzy::fold("colour picker"), String::new()], phones: Vec::new(), strict: false, names: true };
    use search_rank::Source;
    let titles = |q: &str| table.search(q).into_iter().map(|i| i.title).collect::<Vec<_>>();
    assert_eq!(titles("tall"), vec!["Tally"]);
    assert_eq!(titles("pick"), vec!["Digital Color Meter"], "an alias still finds its app by a word prefix");
    // the whole query, end to end: every app listed has the query as a run of its name, its initials or an alias
    for q in ["tall", "stg", "se"] {
        for g in results(q, &[]) {
            for app in apps().iter().filter(|a| g.hits.contains(&Hit::App(a.id))) {
                let alias = starts_a_word(&fuzzy::fold(super::super::apps::search_aliases(app.icon)), q);
                assert!(name_match(q, &fuzzy::fold(app.name)) || alias, "{q:?} listed {}", app.name);
            }
        }
    }
    assert!(results("sett", &[]).iter().flat_map(|g| &g.hits).any(|h| *h == Hit::App(AppId::Settings)));
}

#[test]
fn an_exact_headword_ranks_above_a_loosely_matched_top_hit() {
    let def = |word: &str, dictionary: &str| Group { title: "Dictionary", hits: vec![Hit::Definition { word: word.into(), dictionary: dictionary.into(), preview: String::new() }] };
    let top = |h: Hit| Group { title: "Top Hit", hits: vec![h] };
    // a top hit matched only by scattered letters loses to the word itself
    assert!(headword_beats_top("lantern", &def("lantern", "WordNet"), Some(&top(Hit::Settings("Display & Brightness")))));
    assert!(headword_beats_top("Lantern ", &def("lantern", "WordNet"), Some(&top(Hit::City("Tokyo")))), "case and spaces are ignored");
    // a top hit whose title starts with the query keeps its place
    assert!(!headword_beats_top("wi", &def("wi", "WordNet"), Some(&top(Hit::Settings("Wi-Fi")))));
    assert!(!headword_beats_top("francisco", &def("francisco", "WordNet"), Some(&top(Hit::City("San Francisco")))), "a word start is strong too");
    // only an exact headword, and only a real definition, moves up
    assert!(!headword_beats_top("lant", &def("lantern", "WordNet"), Some(&top(Hit::Settings("Display & Brightness")))));
    assert!(!headword_beats_top("lantern", &def("lantern", ""), Some(&top(Hit::Settings("Display & Brightness")))));
    assert!(!headword_beats_top("lantern", &def("lantern", "WordNet"), None), "with no top hit the order stays");
    // a top hit found by an alias keeps its place: "terminal" is Term, "preview" is the Viewer
    assert!(!headword_beats_top("terminal", &def("terminal", "WordNet"), Some(&top(Hit::App(AppId::Terminal)))));
    assert!(!headword_beats_top("Terminal", &def("terminal", "WordNet"), Some(&top(Hit::App(AppId::Terminal)))), "case is ignored");
    // a run inside a word of the title is weak since 0.10.38 (strict matches start at a word start)
    assert!(headword_beats_top("isco", &def("isco", "WordNet"), Some(&top(Hit::City("San Francisco")))));
}

/// "Show in Folder" names the file itself, so Files selects it; a file that is gone, or a name that is not one plain
/// file name, leaves the folder.
#[test]
fn show_in_folder_hands_files_the_file_so_it_is_selected() {
    let dir = std::env::temp_dir().join(format!("bifrost-reveal-target-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("Q1 report.pdf"), b"x").unwrap();
    std::fs::write(dir.join("sub/inner.txt"), b"x").unwrap();
    assert_eq!(reveal_target(&dir, "Q1 report.pdf"), dir.join("Q1 report.pdf"));
    assert_eq!(reveal_target(&dir, "gone.pdf"), dir, "the file was removed after the search");
    for name in ["", "..", "sub/inner.txt", "../x"] {
        assert_eq!(reveal_target(&dir, name), dir, "{name:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn indexed(app_id: &str, unique_id: &str, title: &str) -> Hit {
    Hit::Indexed { app_id: app_id.into(), unique_id: unique_id.into(), domain: "documents".into(), title: title.into(), kind: "Plain Text".into(), thumb: String::new(), snippet: String::new() }
}

/// E2E5 2 to 4: a file the Files app had opened came back as that app's indexed document, a row with the kind alone
/// and the app's icon that Cmd+Return could not show in its folder. It is a file row now, with "Show in Folder".
#[test]
fn a_document_the_files_app_indexed_becomes_a_file_row() {
    let files_app = super::super::workspace::FILES_APP;
    let note = "/home/me/E2E4/note.txt";
    let known = |p: &Path| if p == Path::new(note) { OnDisk::File(1_700_000_000) } else { OnDisk::Unknown };
    let show = |folder: &str, name: &str| Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from(folder), name: name.into() }] };
    let docs = vec![Group { title: "Files", hits: vec![indexed(files_app, note, "note.txt"), indexed(files_app, "/home/me/Folder", "Folder")] }, Group { title: "Notes", hits: vec![indexed("is.olibuijr.notes", "/home/me/E2E4/note.txt", "note.txt")] }];
    let (documents, files, gone) = fold_indexed_files(docs.clone(), vec![], None, known);
    let file = Hit::File { path: PathBuf::from(note), name: "note.txt".into(), modified: 1_700_000_000 };
    assert_eq!(files, vec![Group { title: "Files", hits: vec![file.clone()] }, show("/home/me/E2E4", "note.txt")]);
    assert_eq!(documents[0].hits, vec![indexed(files_app, "/home/me/Folder", "Folder")], "an item Search cannot say more about stays as it was");
    assert_eq!(documents[1], docs[1], "another app's items are its own, whatever their ids look like");
    assert!(gone.is_empty());
    // the row reads kind, folder, date
    let (title, sub) = row_text(&file, |_| String::new(), (1_700_000_060, 0, true));
    assert_eq!(title, "note.txt");
    assert!(sub.starts_with("Plain Text \u{b7} E2E4 \u{b7} Today at "), "{sub}");

    // the walk lists the file already: no second row, and the emptied section goes
    let walked = vec![Group { title: "Files", hits: vec![file.clone()] }, show("/home/me/E2E4", "note.txt")];
    let only = vec![Group { title: "Files", hits: vec![indexed(files_app, note, "note.txt")] }];
    let (documents, files, _) = fold_indexed_files(only.clone(), walked.clone(), None, known);
    assert!(documents.is_empty(), "{documents:?}");
    assert_eq!(files, walked);
    // the same when it is the Top Hit, which was taken out of its section: no second row, and the action is the Top Hit's
    let top = Group { title: "Top Hit", hits: vec![file.clone()] };
    let (documents, files, _) = fold_indexed_files(only.clone(), vec![], Some(&top), known);
    assert!(documents.is_empty(), "{documents:?}");
    assert_eq!(files, vec![show("/home/me/E2E4", "note.txt")]);
    // another spelling of the same path is the same file
    let spelled = vec![Group { title: "Files", hits: vec![indexed(files_app, "/home/me//E2E4/./note.txt", "note.txt")] }];
    let (_, files, _) = fold_indexed_files(spelled, walked.clone(), None, |_| OnDisk::File(1_700_000_000));
    assert_eq!(files, walked);
    // another file joins the walk's rows, after them
    let other = Hit::File { path: PathBuf::from("/home/me/a/other.txt"), name: "other.txt".into(), modified: 5 };
    let (_, files, _) = fold_indexed_files(only, vec![Group { title: "Files", hits: vec![other.clone()] }], None, known);
    assert_eq!(files, vec![Group { title: "Files", hits: vec![other, file] }, show("/home/me/a", "other.txt")]);
    // a relative id is not a path
    let odd = vec![Group { title: "Files", hits: vec![indexed(files_app, "note.txt", "note.txt")] }];
    assert_eq!(fold_indexed_files(odd.clone(), vec![], None, |_| OnDisk::File(1)), (odd, vec![], vec![]));
}

/// E2E6 1 and 2: "note.txt" was listed twice (the file, and the index's leftover of a "note.txt" in a folder deleted
/// since), "pictures" gave "pic.png" twice and "bad-huge.gif", and "e2e7-notes.txt" and "e2e8-notes.txt" were listed
/// though the files were gone: a document whose file is gone stayed as a row of the Files app. It is not listed, and
/// its id is handed back for the index to drop.
#[test]
fn an_indexed_document_whose_file_is_gone_is_not_listed_and_is_dropped_from_the_index() {
    let files_app = super::super::workspace::FILES_APP;
    let live = "/home/me/E2E6/note.txt";
    let on_disk = |p: &Path| match p.to_str().unwrap() {
        "/home/me/E2E6/note.txt" | "/home/me/E2E6/late.txt" => OnDisk::File(1_700_000_000),
        "/home/me/Private/secret.txt" => OnDisk::Hidden,
        _ => OnDisk::Gone,
    };
    let file = Hit::File { path: PathBuf::from(live), name: "note.txt".into(), modified: 1_700_000_000 };
    let top = Group { title: "Top Hit", hits: vec![file.clone()] };
    // the index's rows as the tester saw them: the file itself, then three leftovers, and a live file behind them
    let docs = vec![
        Group {
            title: "Files",
            hits: vec![
                indexed(files_app, "/home/me/E2E5/note.txt", "note.txt"),
                indexed(files_app, "/home/me/E2E5/e2e8-notes.txt", "e2e8-notes.txt"),
                indexed(files_app, "/home/me/E2E4/e2e7-notes.txt", "e2e7-notes.txt"),
                indexed(files_app, live, "note.txt"),
                indexed(files_app, "/home/me/Private/secret.txt", "secret.txt"),
                indexed(files_app, "/home/me/E2E6/late.txt", "late.txt"),
            ],
        },
        Group { title: "Notes", hits: vec![indexed("is.olibuijr.notes", "/home/me/E2E5/note.txt", "note.txt")] },
    ];
    let (documents, files, gone) = fold_indexed_files(docs.clone(), vec![], Some(&top), on_disk);
    assert_eq!(documents, vec![docs[1].clone()], "no row of the Files app is left; another app's item with such an id is its own");
    let late = Hit::File { path: PathBuf::from("/home/me/E2E6/late.txt"), name: "late.txt".into(), modified: 1_700_000_000 };
    assert_eq!(
        files,
        vec![Group { title: "Files", hits: vec![late] }, Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me/E2E6"), name: "note.txt".into() }] }],
        "the Top Hit is the one row of note.txt; the live file behind the leftovers is listed; the hidden one is not"
    );
    assert_eq!(gone, vec!["/home/me/E2E5/note.txt".to_string(), "/home/me/E2E5/e2e8-notes.txt".into(), "/home/me/E2E4/e2e7-notes.txt".into()], "only the files that are gone leave the index");
    let all: Vec<&Hit> = documents.iter().chain(&files).flat_map(|g| &g.hits).collect();
    assert_eq!(all.iter().filter(|h| matches!(h, Hit::File { name, .. } if name == "note.txt")).count(), 0, "note.txt is the Top Hit alone");

    // each section is cut to three after the leftovers are out, not before
    let many: Vec<Hit> = (0..5).map(|i| indexed(files_app, &format!("/home/me/gone-{i}.txt"), "gone.txt")).chain((0..5).map(|i| indexed(files_app, &format!("/home/me/E2E6/f{i}.txt"), "f.txt"))).collect();
    let (documents, files, gone) = fold_indexed_files(vec![Group { title: "Files", hits: many }], vec![], None, |p| if p.starts_with("/home/me/E2E6") { OnDisk::File(7) } else { OnDisk::Gone });
    assert!(documents.is_empty());
    assert_eq!(files[0].hits.len(), PER_GROUP);
    assert_eq!(gone.len(), 5);
}

/// What is on the disk decides: a file, nothing, or something Search leaves alone.
#[test]
fn a_document_path_is_a_file_gone_or_left_alone() {
    let dir = std::env::temp_dir().join(format!("bifrost-indexed-on-disk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), b"x").unwrap();
    // the Files section may be switched off in the settings of the machine: then every file is hidden
    if super::super::search_prefs::current().source_on("Files") && !super::super::search_prefs::current().folder_excluded(&dir.join("a.txt")) {
        assert!(matches!(indexed_file_on_disk(&dir.join("a.txt")), OnDisk::File(at) if at >= 1));
        assert_eq!(indexed_file_on_disk(&dir.join("deleted.txt")), OnDisk::Gone);
        assert_eq!(indexed_file_on_disk(&dir.join("no-such-folder/a.txt")), OnDisk::Gone);
        assert_eq!(indexed_file_on_disk(&dir.join("sub")), OnDisk::Unknown, "a folder is not a document that is gone");
    }
    assert_eq!(plain_path(Path::new("/a//b/./c/")), PathBuf::from("/a/b/c"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// E2E6 3: no "Show ... in Folder" row was listed for a file found by its name, since the one file match was the Top
/// Hit and the action was built from the rows left in the Files section. The action follows the first file shown.
#[test]
fn show_in_folder_is_listed_for_a_file_that_is_the_top_hit() {
    let files = vec![entry("/home/me/E2E6", "note.txt"), entry("/home/me/Pics", "Beach.png")];
    let folded: Vec<String> = files.iter().map(|e| fuzzy::fold(&e.name)).collect();
    let corpus = Corpus { deildu: &[], deildu_folded: &[], contacts: &[], contacts_folded: &[], files: &files, files_folded: &folded, events: &[], wallpapers: &[] };
    let (tables, hits) = build_tables(&corpus);
    let mut run = search_rank::Query::start("note.txt", &sources_of(tables), SYNC_DEADLINE);
    run.wait();
    let merged = run.merge(PER_GROUP, &search_rank::Usage::default(), 0, &search_rank::NoSemantic);
    let note = Hit::File { path: PathBuf::from("/home/me/E2E6/note.txt"), name: "note.txt".into(), modified: 0 };
    let (top, _, files_groups) = split_merged(&merged, &hits, Some(note.clone()));
    assert_eq!(top, Some(Group { title: "Top Hit", hits: vec![note.clone()] }));
    let action = Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me/E2E6"), name: "note.txt".into() }] };
    assert_eq!(files_groups, vec![action.clone()], "the Files section is empty without its Top Hit, the action stays");
    // the whole list: the file, its action, the Terminal row
    let groups = compose("note.txt", None, top, vec![], vec![], files_groups);
    let titles: Vec<&str> = groups.iter().map(|g| g.title).collect();
    assert_eq!(titles, vec!["Top Hit", "Show in Folder", "Search in Apps"]);
    assert_eq!(chips_of(&groups), vec!["Files"], "the action is no kind of result");
    assert_eq!(filter_to_chip(groups.clone(), "Files"), vec![Group { title: "Files", hits: vec![note.clone()] }, action.clone()], "the Files chip keeps the action");
    assert!(filter_to_chip(groups, "Applications").is_empty());
    // the Top Hit is not a file: the action is for the first row of the Files section
    let other = Hit::File { path: PathBuf::from("/home/me/Pics/Beach.png"), name: "Beach.png".into(), modified: 0 };
    let app = Group { title: "Top Hit", hits: vec![Hit::App(AppId::Files)] };
    let shown = show_in_folder(Some(&app), vec![Group { title: "Files", hits: vec![other.clone(), note.clone()] }, action.clone()]);
    assert_eq!(shown[1], Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me/Pics"), name: "Beach.png".into() }] });
    assert_eq!(shown.len(), 2, "one action, the old one replaced");
    assert!(show_in_folder(Some(&app), vec![]).is_empty(), "no file, no action");
}

/// E2E6 4: "pictures" did not find the home folder "Pictures". A folder is a row of the Files section with the kind
/// "Folder"; Cmd+Return shows it in the folder that holds it.
#[test]
fn a_folder_is_found_by_name_as_a_folder_row() {
    let pictures = Entry { kind: kind_of("Pictures", true), name: "Pictures".into(), path: PathBuf::from("/home/me/Pictures"), is_dir: true, size: 0, mtime: 1_700_000_000, items: None };
    let files = vec![pictures, entry("/home/me/Pictures", "pic.png")];
    let groups = file_groups("pictures", &files);
    let folder = Hit::Folder { path: PathBuf::from("/home/me/Pictures"), name: "Pictures".into(), modified: 1_700_000_000 };
    assert_eq!(groups, vec![Group { title: "Files", hits: vec![folder.clone()] }, Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me"), name: "Pictures".into() }] }]);
    let (title, sub) = row_text(&folder, |_| String::new(), (1_700_000_060, 0, true));
    assert_eq!(title, "Pictures");
    assert!(sub.starts_with("Folder \u{b7} me \u{b7} Today at "), "{sub}");
    assert_eq!(kind_of_hit(&folder), Some("Files"));
    assert_eq!(hit_category(&folder), (Some(super::super::search_scope::Category::Files), Some(Path::new("/home/me/Pictures"))));
    // Cmd+Return, and the folder icon in place of a document icon
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    let row = |h: Hit| (h, u.text("x", 13.0, Weight::Regular, WHITE));
    let pic = Hit::File { path: PathBuf::from("/home/me/Pictures/pic.png"), name: "pic.png".into(), modified: 0 };
    q.sections = vec![Section { header: Image::new(1, 1), rows: vec![row(folder.clone()), row(pic)], subs: vec![None, None] }];
    assert_eq!(q.reveal_hit(), Some(Hit::Reveal { folder: PathBuf::from("/home/me"), name: "Pictures".into() }));
    let wanted = q.file_icons_wanted(&u);
    assert_eq!(wanted.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec![FOLDER_ICON, "pic.png"]);
    let px = wanted[0].1;
    q.set_file_icon(FOLDER_ICON, px, Image::new(px, px));
    assert!(q.file_icon(FOLDER_ICON, px).is_some());
    assert!(q.file_icon("Makefile", px).is_none(), "a file without an extension does not get the folder icon");
    assert_eq!(q.file_icons_wanted(&u).len(), 1);
    // Space does not preview a folder: it is typed
    q.selected = Some((0, 0));
    assert_eq!(q.key(&u, 800, Out::Text(" ".into()), 0.0), None);
}

/// The folded row is one Cmd+Return shows in its folder.
#[test]
fn cmd_return_reveals_a_file_that_came_from_the_index() {
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    let (_, files, _) = fold_indexed_files(vec![Group { title: "Files", hits: vec![indexed(super::super::workspace::FILES_APP, "/home/me/E2E4/note.txt", "note.txt")] }], vec![], None, |_| OnDisk::File(9));
    q.sections = files.into_iter().map(|g| Section { header: Image::new(1, 1), subs: g.hits.iter().map(|_| None).collect(), rows: g.hits.into_iter().map(|h| (h, u.text("x", 13.0, Weight::Regular, WHITE))).collect() }).collect();
    q.selected = Some((0, 0));
    assert_eq!(q.reveal_hit(), Some(Hit::Reveal { folder: PathBuf::from("/home/me/E2E4"), name: "note.txt".into() }));
}

/// No test walks the home folder of the machine it runs on: a panel has no home folder of its own, the look-up of the
/// account's gives none in tests, and so the scan Search starts when it opens reads the Files root it is given alone.
#[test]
fn a_panel_in_a_test_walks_only_the_root_it_is_given() {
    let root = std::env::temp_dir().join(format!("bifrost-qs-root-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Docs")).unwrap();
    std::fs::write(root.join("Docs/a.txt"), b"x").unwrap();
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    assert_eq!(q.home, None);
    assert_eq!(files_walk::search_home(), None);
    let nowhere = root.join("none");
    q.refresh_sources(&nowhere.join("contacts.tsv"), &nowhere, &nowhere.join("calendar"), root.clone());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !q.take_files() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let mut paths: Vec<PathBuf> = q.files.iter().map(|e| e.path.clone()).collect();
    paths.sort();
    assert_eq!(paths, vec![root.join("Docs"), root.join("Docs/a.txt")], "the root's own entries, and nothing from a home folder");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_file_row_asks_for_the_document_icon_of_its_type_once_per_kind() {
    assert_eq!(file_icon_key("Notes.TXT"), "txt");
    assert_eq!(file_icon_key("archive.tar.gz"), "gz");
    assert_eq!(file_icon_key("Makefile"), "");
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    let file = |p: &str| Hit::File { path: PathBuf::from(p), name: Path::new(p).file_name().unwrap().to_string_lossy().into_owned(), modified: 0 };
    let row = |h: Hit| (h, u.text("x", 13.0, Weight::Regular, WHITE));
    q.sections = vec![Section { header: Image::new(1, 1), rows: vec![row(Hit::App(AppId::Files)), row(file("/d/a.txt")), row(file("/d/b.TXT")), row(file("/d/c.pdf"))], subs: vec![None, None, None, None] }];
    let wanted = q.file_icons_wanted(&u);
    let names: Vec<&str> = wanted.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["a.txt", "c.pdf"], "one icon per kind of file, none for an app row");
    let px = wanted[0].1;
    assert!(px > 0);
    assert!(q.file_icon("b.TXT", px).is_none());
    q.set_file_icon("a.txt", px, Image::new(px, px));
    assert!(q.file_icon("b.TXT", px).is_some(), "files of one kind share the icon");
    assert!(q.file_icon("b.TXT", px + 1).is_none(), "another size is another icon");
    assert_eq!(q.file_icons_wanted(&u).iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["c.pdf"]);
}

#[test]
fn a_failed_file_action_is_said_with_the_file_name_and_the_reason() {
    assert_eq!(file_problem_banner(Path::new("/home/me/E2E4/note.txt"), "could not be shown in Files", "App not installed"), ("\u{201c}note.txt\u{201d} could not be shown in Files".to_string(), "App not installed".to_string()));
    assert_eq!(file_problem_banner(Path::new("/"), "could not be shown in Files", "x").0, "\u{201c}/\u{201d} could not be shown in Files");
}
