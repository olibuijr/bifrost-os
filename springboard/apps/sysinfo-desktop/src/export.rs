//! Saving a report in three forms and reading one back. Plain text and rich text are for people; the JSON report keeps
//! every category's items and facts, so File > Open can show a report from another machine in place of the live one.

use crate::model::{kv, Category, Content, Item, Row, Stored};
use bifrost_core::json::{self, Json};

/// The marker inside a JSON report, so another JSON file is not mistaken for one.
const FORMAT: &str = "bifrost-system-report";
const VERSION: f64 = 1.0;
/// Reading stops short of absurd files: the most items one category may hold.
const MAX_ITEMS: usize = 20_000;
/// The largest report File > Open reads, in bytes.
pub const MAX_REPORT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Rtf,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Text => "txt",
            Format::Json => "json",
            Format::Rtf => "rtf",
        }
    }

    /// The name the save panel suggests.
    pub fn suggested_name(self) -> String {
        format!("System Report.{}", self.extension())
    }

    /// The file type the save panel offers (a UTI).
    pub fn filter(self) -> &'static str {
        match self {
            Format::Text => "public.plain-text",
            Format::Json => "public.json",
            Format::Rtf => "public.rtf",
        }
    }
}

/// The format a file name asks for by its extension (`.txt`, `.json`, `.rtf`); None for any other name.
pub fn format_for(name: &str) -> Option<Format> {
    match name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).as_deref() {
        Some("json") => Some(Format::Json),
        Some("rtf") => Some(Format::Rtf),
        Some("txt") | Some("text") => Some(Format::Text),
        _ => None,
    }
}

/// The report in `format`.
pub fn render(contents: &[(Category, Content)], format: Format) -> Vec<u8> {
    match format {
        Format::Text => crate::model::text_of(contents).into_bytes(),
        Format::Json => to_json(contents).encode().into_bytes(),
        Format::Rtf => rtf(contents).into_bytes(),
    }
}

// ---------------------------------------------------------------- JSON

fn str_json(s: &str) -> Json {
    Json::Str(s.to_string())
}

fn to_json(contents: &[(Category, Content)]) -> Json {
    let categories = contents
        .iter()
        .map(|(c, content)| {
            let items = content
                .items
                .iter()
                .map(|i| {
                    let rows = i.detail.iter().map(|(k, v)| Json::Arr(vec![str_json(k), str_json(v)])).collect();
                    Json::Obj(vec![("title".to_string(), str_json(&i.title)), ("summary".to_string(), str_json(&i.summary)), ("rows".to_string(), Json::Arr(rows))])
                })
                .collect();
            Json::Obj(vec![
                ("title".to_string(), str_json(c.title())),
                ("section".to_string(), str_json(c.section().title())),
                ("list".to_string(), Json::Bool(content.list)),
                ("empty".to_string(), Json::Bool(content.empty)),
                ("items".to_string(), Json::Arr(items)),
            ])
        })
        .collect();
    Json::Obj(vec![("format".to_string(), str_json(FORMAT)), ("version".to_string(), Json::Num(VERSION)), ("categories".to_string(), Json::Arr(categories))])
}

/// Reads a JSON report. `name` is the file's name, shown in the window.
pub fn parse(bytes: &[u8], name: &str) -> Result<Stored, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "The file is not text".to_string())?;
    let j = json::parse(text).map_err(|_| "The file is not a system report".to_string())?;
    if j.str_field("format").as_deref() != Some(FORMAT) {
        return Err("The file is not a system report".to_string());
    }
    let categories = j.get("categories").and_then(Json::as_array).ok_or_else(|| "The report has no categories".to_string())?;
    let mut contents = Vec::new();
    for entry in categories {
        let Some(category) = entry.str_field("title").and_then(|t| Category::from_title(&t)) else { continue };
        let items: Vec<Item> = entry.get("items").and_then(Json::as_array).unwrap_or_default().iter().take(MAX_ITEMS).map(item_from).collect();
        if items.is_empty() {
            continue;
        }
        let flag = |key: &str| matches!(entry.get(key), Some(Json::Bool(true)));
        contents.push((category, Content { list: flag("list"), empty: flag("empty"), items }));
    }
    if contents.is_empty() {
        return Err("The report holds no categories this version knows".to_string());
    }
    Ok(Stored::of(name, contents))
}

fn item_from(j: &Json) -> Item {
    let detail: Vec<Row> = j
        .get("rows")
        .and_then(Json::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| match row.as_array()? {
            [k, v] => Some(kv(k.as_str()?, v.as_str()?)),
            _ => None,
        })
        .collect();
    Item { title: j.str_field("title").unwrap_or_default(), summary: j.str_field("summary").unwrap_or_default(), detail }
}

// ---------------------------------------------------------------- rich text

/// `s` as RTF text: control characters and braces escaped, characters beyond ASCII as `\uN?` (UTF-16 units).
pub fn rtf_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\line "),
            '\t' => out.push_str("\\tab "),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c if c.is_ascii() => out.push(' '),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units).iter() {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    out
}

fn rtf(contents: &[(Category, Content)]) -> String {
    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0\\fswiss Helvetica;}}\\f0\\fs22\n");
    out.push_str("{\\b\\fs32 System Information Report}\\par\n");
    for (c, content) in contents {
        out.push_str(&format!("\\par{{\\b\\fs26 {} ({})}}\\par\n", rtf_escape(c.title()), rtf_escape(c.section().title())));
        if content.list {
            for item in &content.items {
                out.push_str(&format!("{{\\b {}}}: {}\\par\n", rtf_escape(item.title.trim_start()), rtf_escape(&item.summary)));
                for (k, v) in &item.detail {
                    out.push_str(&format!("\\tab {}: {}\\par\n", rtf_escape(k), rtf_escape(v)));
                }
            }
        } else {
            for (k, v) in content.summary_rows() {
                out.push_str(&format!("{{\\b {}}}: {}\\par\n", rtf_escape(&k), rtf_escape(&v)));
            }
        }
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<(Category, Content)> {
        vec![
            (Category::Overview, Content::card(vec![kv("Model", "Laptop \"13\""), kv("Memory", "16 GB")])),
            (
                Category::Usb,
                Content::list(vec![
                    Item { title: "Host".into(), summary: "Acme · High speed".into(), detail: vec![kv("Port", "usb1")] },
                    Item { title: "    Hub".into(), summary: "Acme".into(), detail: vec![kv("Port", "1-1"), kv("Bus", "1")] },
                ]),
            ),
            (Category::Audio, Content::card(Vec::new())),
        ]
    }

    #[test]
    fn the_format_follows_the_file_extension() {
        assert_eq!(format_for("System Report.json"), Some(Format::Json));
        assert_eq!(format_for("report.RTF"), Some(Format::Rtf));
        assert_eq!(format_for("report.txt"), Some(Format::Text));
        assert_eq!(format_for("report"), None);
        assert_eq!(format_for("report.docx"), None);
        assert_eq!(Format::Json.extension(), "json");
        assert_eq!(Format::Rtf.suggested_name(), "System Report.rtf");
        assert_eq!(Format::Text.filter(), "public.plain-text");
    }

    #[test]
    fn a_json_report_reads_back_as_the_same_content() {
        let bytes = render(&sample(), Format::Json);
        let stored = parse(&bytes, "r.json").expect("a report we wrote reads back");
        assert_eq!(stored.name, "r.json");
        assert_eq!(stored.contents, sample());
    }

    #[test]
    fn files_that_are_not_reports_are_refused() {
        assert!(parse(b"not json", "x").is_err());
        assert!(parse(b"{\"format\":\"other\"}", "x").is_err());
        assert!(parse(&[0xff, 0xfe], "x").is_err(), "not UTF-8");
        assert!(parse(b"{\"format\":\"bifrost-system-report\",\"categories\":[]}", "x").is_err(), "no categories");
        assert!(parse(b"{\"format\":\"bifrost-system-report\"}", "x").is_err());
    }

    #[test]
    fn unknown_categories_and_malformed_rows_are_skipped() {
        let text = r#"{"format":"bifrost-system-report","version":1,"categories":[
            {"title":"Overview","list":false,"items":[{"title":"","summary":"","rows":[["Model","X"],["bad"],[1,2],["A","B"]]}]},
            {"title":"Future Thing","items":[{"title":"t","summary":"s","rows":[]}]}]}"#;
        let stored = parse(text.as_bytes(), "x").unwrap();
        assert_eq!(stored.contents.len(), 1);
        assert_eq!(stored.contents[0].1.summary_rows(), vec![kv("Model", "X"), kv("A", "B")]);
    }

    #[test]
    fn text_output_matches_the_text_report() {
        assert_eq!(render(&sample(), Format::Text), crate::model::text_of(&sample()).into_bytes());
    }

    #[test]
    fn rich_text_escapes_braces_backslashes_and_non_ascii() {
        assert_eq!(rtf_escape("a{b}\\c"), "a\\{b\\}\\\\c");
        assert_eq!(rtf_escape("Ólafur"), "\\u211?lafur");
        assert_eq!(rtf_escape("a\nb"), "a\\line b");
        assert_eq!(rtf_escape("\u{1f600}"), "\\u-10179?\\u-8704?", "a character beyond the BMP is two units");
        assert_eq!(rtf_escape("x\u{7}y"), "x y");
    }

    #[test]
    fn rich_text_has_a_header_titles_and_rows() {
        let text = String::from_utf8(render(&sample(), Format::Rtf)).unwrap();
        assert!(text.starts_with("{\\rtf1"));
        assert!(text.contains("Overview (Hardware)"));
        assert!(text.contains("{\\b Model}: Laptop \"13\"\\par"));
        assert!(text.contains("\\tab Port: usb1\\par"));
        assert!(text.ends_with('}'));
    }
}
