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
    let corpus = Corpus { deildu: &[], deildu_folded: &[], contacts: &[], contacts_folded: &[], files, files_folded: &folded, events: &[] };
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
    let corpus = Corpus { deildu: &[], deildu_folded: &[], contacts: &[], contacts_folded: &[], files: &[], files_folded: &[], events: &[] };
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
