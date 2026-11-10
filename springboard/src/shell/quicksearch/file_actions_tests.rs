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
/// Up's Search section. Every section but Applications (and Deildu) is strict; an app's name stays fuzzy.
#[test]
fn only_app_names_match_by_scattered_letters() {
    let corpus = Corpus { deildu: &[], deildu_folded: &[], contacts: &[], contacts_folded: &[], files: &[], files_folded: &[], events: &[], wallpapers: &[] };
    let (tables, _) = build_tables(&corpus);
    for t in &tables {
        assert_eq!(t.strict, !matches!(t.name, "Applications" | "Deildu"), "{} strictness", t.name);
    }
    let mut hits = BTreeMap::new();
    let title = "Set Wallpaper to Gullfoss Canyon";
    let row = table_row(&mut hits, "w".into(), title, String::new(), search_rank::Kind::Settings, Hit::Terminal(String::new()), fuzzy::fold(title));
    let shortcuts = Table { name: "Shortcuts", rows: vec![row], prefixes: Vec::new(), phones: Vec::new(), strict: true };
    use search_rank::Source;
    assert!(shortcuts.search("AAPL").is_empty(), "scattered letters do not find a shortcut");
    assert_eq!(shortcuts.search("wallp").len(), 1, "a word start does");
    assert_eq!(shortcuts.search("gullfoss c").len(), 1, "a contiguous run does");
    let mut hits = BTreeMap::new();
    let title = "Open Passwords";
    let row = table_row(&mut hits, "p".into(), title, String::new(), search_rank::Kind::Settings, Hit::Terminal(String::new()), fuzzy::fold(title));
    let shortcuts = Table { name: "Shortcuts", rows: vec![row], prefixes: Vec::new(), phones: Vec::new(), strict: true };
    assert!(shortcuts.search("words").is_empty(), "a run inside a word does not find a shortcut (E2E13 4)");
    assert_eq!(shortcuts.search("passw").len(), 1, "a word start does");
    assert_eq!(shortcuts.search("open").len(), 1, "a prefix does");
    let apps = tables.iter().find(|t| t.name == "Applications").expect("the apps table");
    assert!(apps.rows.iter().any(|(_, f)| fuzzy::Matcher::new("stg").rank_folded(f).is_some()), "an app name still matches loosely (Settings)");
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
    let known = |p: &Path| (p == Path::new(note)).then_some(1_700_000_000);
    let docs = vec![Group { title: "Files", hits: vec![indexed(files_app, note, "note.txt"), indexed(files_app, "/home/me/gone.txt", "gone.txt")] }, Group { title: "Notes", hits: vec![indexed("is.olibuijr.notes", "/home/me/E2E4/note.txt", "note.txt")] }];
    let (documents, files) = fold_indexed_files(docs.clone(), vec![], None, known);
    let file = Hit::File { path: PathBuf::from(note), name: "note.txt".into(), modified: 1_700_000_000 };
    assert_eq!(files, vec![Group { title: "Files", hits: vec![file.clone()] }, Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me/E2E4"), name: "note.txt".into() }] }]);
    assert_eq!(documents[0].hits, vec![indexed(files_app, "/home/me/gone.txt", "gone.txt")], "an item that is no file Search may list stays as it was");
    assert_eq!(documents[1], docs[1], "another app's items are its own, whatever their ids look like");
    // the row reads kind, folder, date
    let (title, sub) = row_text(&file, |_| String::new(), (1_700_000_060, 0, true));
    assert_eq!(title, "note.txt");
    assert!(sub.starts_with("Plain Text \u{b7} E2E4 \u{b7} Today at "), "{sub}");

    // the walk lists the file already: no second row, and the emptied section goes
    let walked = vec![Group { title: "Files", hits: vec![file.clone()] }, Group { title: "Show in Folder", hits: vec![Hit::Reveal { folder: PathBuf::from("/home/me/E2E4"), name: "note.txt".into() }] }];
    let only = vec![Group { title: "Files", hits: vec![indexed(files_app, note, "note.txt")] }];
    let (documents, files) = fold_indexed_files(only.clone(), walked.clone(), None, known);
    assert!(documents.is_empty(), "{documents:?}");
    assert_eq!(files, walked);
    // the same when it is the Top Hit, which was taken out of its section
    let top = Group { title: "Top Hit", hits: vec![file.clone()] };
    let (documents, files) = fold_indexed_files(only.clone(), vec![], Some(&top), known);
    assert!(documents.is_empty() && files.is_empty(), "{documents:?} {files:?}");
    // another file joins the walk's rows, after them
    let other = Hit::File { path: PathBuf::from("/home/me/a/other.txt"), name: "other.txt".into(), modified: 5 };
    let (_, files) = fold_indexed_files(only, vec![Group { title: "Files", hits: vec![other.clone()] }], None, known);
    assert_eq!(files, vec![Group { title: "Files", hits: vec![other, file] }]);
    // a relative id is not a path
    let odd = vec![Group { title: "Files", hits: vec![indexed(files_app, "note.txt", "note.txt")] }];
    assert_eq!(fold_indexed_files(odd.clone(), vec![], None, |_| Some(1)), (odd, vec![]));
}

/// The folded row is one Cmd+Return shows in its folder.
#[test]
fn cmd_return_reveals_a_file_that_came_from_the_index() {
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    let (_, files) = fold_indexed_files(vec![Group { title: "Files", hits: vec![indexed(super::super::workspace::FILES_APP, "/home/me/E2E4/note.txt", "note.txt")] }], vec![], None, |_| Some(9));
    q.sections = files.into_iter().map(|g| Section { header: Image::new(1, 1), subs: g.hits.iter().map(|_| None).collect(), rows: g.hits.into_iter().map(|h| (h, u.text("x", 13.0, Weight::Regular, WHITE))).collect() }).collect();
    q.selected = Some((0, 0));
    assert_eq!(q.reveal_hit(), Some(Hit::Reveal { folder: PathBuf::from("/home/me/E2E4"), name: "note.txt".into() }));
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
