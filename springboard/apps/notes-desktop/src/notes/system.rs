//! Notes' links to the shell's system services: the `pasteboard` service (a copy of a selection carries rich text
//! with a plain text fallback), the `widgets_timeline` service (a widget with the most recent notes), and the
//! indexed text of each note for the `search-index` ranking (Core Search items, see `main.rs`).
//!
//! Every call here is made by the app on a real event (a copy, or a change to the recent notes); nothing runs per
//! frame. A reply that says no such service exists (an older shell) is logged by the app, not handled here.
use super::store::Note;
use bifrost_app::pasteboard_types::{self as pb, Board, Item};
use bifrost_app::widgets_timeline::{self as wt, Entry, ReloadPolicy};
use bifrost_app::Ctx;
use springboard::shell::md;

/// The kind of the Notes widget timeline.
pub const WIDGET_KIND: &str = "recent-notes";
/// How many notes the widget lists.
pub const RECENT_COUNT: usize = 3;
/// How much of a note's text is indexed for search (chars). The title is not part of this text.
pub const INDEX_BODY_CHARS: usize = 4000;

/// The copy of a Markdown selection as a pasteboard item with two representations: RTF (bold, italic, code and the
/// list markers) and the plain text (the markers stripped), which a reader without RTF takes.
pub fn copy_items(markdown: &str) -> Vec<Item> {
    vec![Item::default().with(pb::RTF, rtf(markdown).as_bytes()).with(pb::STRING, md::to_plain(markdown).as_bytes())]
}

/// Puts the selection on the general pasteboard. Returns the request id of the service call.
pub fn copy(ctx: &Ctx, markdown: &str) -> u32 {
    Board::general().write(ctx, &copy_items(markdown))
}

/// The RTF document of a Markdown text: one paragraph per line, with the same block and inline styles as the editor.
pub fn rtf(markdown: &str) -> String {
    let mut out = String::from("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Helvetica;}{\\f1 Menlo;}}\\fs24 ");
    let lines: Vec<String> = markdown.split('\n').map(line_rtf).collect();
    out.push_str(&lines.join("\\par\n"));
    out.push('}');
    out
}

/// One line as RTF runs. The block marker is written as text (a bullet, a number or a checkbox).
fn line_rtf(line: &str) -> String {
    let info = md::classify(line);
    let content: String = line.chars().skip(info.prefix).collect();
    let block_bold = matches!(info.block, md::Block::Title | md::Block::Heading | md::Block::Subheading);
    let mut out = String::new();
    match info.block {
        md::Block::Bullet => out.push_str(&escape("\u{2022} ")),
        md::Block::Numbered(n) => out.push_str(&escape(&format!("{n}. "))),
        md::Block::Check(done) => out.push_str(&escape(if done { "[x] " } else { "[ ] " })),
        _ => {}
    }
    for span in md::spans(&content, 0) {
        if span.marker {
            continue;
        }
        let text: String = content.chars().skip(span.range.start).take(span.range.end - span.range.start).collect();
        let bold = block_bold || span.style == md::Style::Bold;
        if bold {
            out.push_str("\\b ");
        }
        if span.style == md::Style::Italic {
            out.push_str("\\i ");
        }
        if span.style == md::Style::Code {
            out.push_str("\\f1 ");
        }
        out.push_str(&escape(&text));
        out.push_str("\\b0\\i0\\f0 ");
    }
    out
}

/// Escapes text for RTF: the control characters `\ { }`, a tab, and every non-ASCII char as a signed UTF-16 unit.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            c if c.is_control() => {}
            c if c.is_ascii() => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf).iter() {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    out
}

/// The titles of the newest notes (not in Recently Deleted), newest first, at most [`RECENT_COUNT`].
pub fn recent_titles(notes: &[(&Note, &str)]) -> Vec<String> {
    let mut sorted: Vec<&Note> = notes.iter().map(|(n, _)| *n).collect();
    sorted.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
    sorted.iter().take(RECENT_COUNT).map(|n| n.title().to_string()).collect()
}

/// The widget's timeline: one entry, shown from `now` (Unix seconds).
pub fn widget_entries(titles: &[String], now: f64) -> Vec<Entry> {
    if titles.is_empty() {
        return vec![Entry { date: now, title: "Notes".into(), text: "No notes yet".into() }];
    }
    vec![Entry { date: now, title: "Recent Notes".into(), text: titles.join("\n") }]
}

/// Gives the shell a new widget timeline. The policy is `Never`: the app reloads on a change of the recent notes
/// and when the shell asks for a reload, so the shell never has to poll.
pub fn reload_widget(ctx: &Ctx, titles: &[String], now: f64) -> u32 {
    wt::reload(ctx, WIDGET_KIND, &widget_entries(titles, now), ReloadPolicy::Never)
}

/// The text a note is indexed with: its plain text after the title line, one line per row joined by spaces, capped.
pub fn index_body(text: &str) -> String {
    let plain = md::to_plain(text);
    let body = plain.lines().map(str::trim).filter(|l| !l.is_empty()).skip(1).collect::<Vec<_>>().join(" ");
    body.chars().take(INDEX_BODY_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::proto::{Configure, ToShell};
    use bifrost_core::json::{self, Json};

    fn test_ctx() -> Ctx {
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(crate::test_ui(), cfg)
    }

    fn note(id: &str, text: &str, modified: i64) -> Note {
        Note { id: id.into(), text: text.into(), modified }
    }

    #[test]
    fn rtf_escapes_controls_and_keeps_non_ascii_as_unicode() {
        let r = rtf("a {b} \\ \u{e9}");
        assert!(r.starts_with("{\\rtf1"), "{r}");
        assert!(r.contains("a \\{b\\} \\\\ \\u233?"), "{r}");
        assert!(r.ends_with('}'));
    }

    #[test]
    fn rtf_carries_bold_italic_code_and_list_markers() {
        let r = rtf("**bold** and *it*\n- item\n# Head");
        assert!(r.contains("\\b bold\\b0"), "{r}");
        assert!(r.contains("\\i it\\i0"), "{r}");
        assert!(r.contains("\\u8226? item"), "bullet marker: {r}");
        assert!(r.contains("\\b Head"), "titles are bold: {r}");
        assert!(r.contains("\\par\n"), "one paragraph per line");
    }

    #[test]
    fn a_copy_puts_rtf_and_the_plain_fallback_on_the_general_board() {
        let ctx = test_ctx();
        let id = copy(&ctx, "**bold** text");
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("no service call: {sent:?}") };
        assert_eq!(*request, id);
        assert_eq!(service, "pasteboard");
        assert_eq!(method, "write");
        let parsed = json::parse(body).unwrap();
        assert_eq!(parsed.get("board").and_then(Json::as_str), Some("general"));
        let reps = parsed.get("items").and_then(Json::as_array).unwrap()[0].get("reps").and_then(Json::as_array).unwrap();
        let uti = |i: usize| reps[i].get("uti").and_then(Json::as_str).unwrap_or_default().to_string();
        assert_eq!(uti(0), "public.rtf");
        assert_eq!(uti(1), "public.utf8-plain-text");
        assert_eq!(reps[1].get("text").and_then(Json::as_str), Some("bold text"), "the plain fallback has no markers");
    }

    #[test]
    fn the_widget_lists_the_three_newest_notes_by_modification() {
        let a = note("a", "Alpha\nbody", 10);
        let b = note("b", "Beta", 30);
        let c = note("c", "Gamma", 20);
        let d = note("d", "Delta", 5);
        let list = [(&a, ""), (&b, ""), (&c, "Work"), (&d, "")];
        assert_eq!(recent_titles(&list), vec!["Beta", "Gamma", "Alpha"]);
        assert!(recent_titles(&[]).is_empty());
    }

    #[test]
    fn the_widget_entry_says_no_notes_when_there_are_none() {
        let empty = widget_entries(&[], 100.0);
        assert_eq!(empty[0].text, "No notes yet");
        let some = widget_entries(&["Beta".into(), "Gamma".into()], 100.0);
        assert_eq!(some[0].title, "Recent Notes");
        assert_eq!(some[0].text, "Beta\nGamma");
        assert_eq!(some[0].date, 100.0);
    }

    #[test]
    fn a_reload_goes_to_the_widgets_service_with_the_recent_notes_kind() {
        let ctx = test_ctx();
        reload_widget(&ctx, &["Beta".into()], 1.0);
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { service, method, body, .. } = &sent[0] else { panic!("no service call") };
        assert_eq!(service, "widgets_timeline");
        assert_eq!(method, "reload");
        let parsed = json::parse(body).unwrap();
        assert_eq!(parsed.get("kind").and_then(Json::as_str), Some(WIDGET_KIND));
        assert_eq!(parsed.get("policy").and_then(Json::as_str), Some("never"));
    }

    #[test]
    fn the_index_text_is_the_body_after_the_title_and_is_capped() {
        assert_eq!(index_body("# Title\n\nFirst line\n- item"), "First line \u{2022} item");
        assert!(!index_body("Only a title").contains("Only"));
        let long = format!("T\n{}", "x".repeat(INDEX_BODY_CHARS + 50));
        assert_eq!(index_body(&long).chars().count(), INDEX_BODY_CHARS);
    }
}
