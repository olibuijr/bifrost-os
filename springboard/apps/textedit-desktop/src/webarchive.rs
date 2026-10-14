//! Web archives: a page kept as a binary property list whose main resource holds the page's HTML as data. The reader
//! takes the HTML of the main resource and reads it as HTML; the writer makes an archive with the document's HTML and no
//! subresources. The codec is the SDK's binary property list.

use crate::html;
use crate::properties::Properties;
use crate::text::Cell;
use bifrost_app::binary_plist::{self, BValue};

/// The type of a web archive document (its uniform type identifier).
pub const TYPE: &str = "bifrost.web-archive";

/// The main resource's data and its MIME type, when the bytes are a property list that has one.
fn main_resource(bytes: &[u8]) -> Option<(Vec<u8>, String)> {
    let root = binary_plist::read(bytes).ok()?;
    let main = root.get("WebMainResource")?;
    let data = match main.get("WebResourceData")? {
        BValue::Data(data) => data.clone(),
        _ => return None,
    };
    let mime = match main.get("WebResourceMIMEType") {
        Some(BValue::Str(mime)) => mime.clone(),
        _ => String::new(),
    };
    Some((data, mime))
}

/// Whether the bytes are a web archive whose main resource is an HTML page.
pub fn is_webarchive(bytes: &[u8]) -> bool {
    main_resource(bytes).is_some_and(|(data, mime)| mime == "text/html" || html::is_html(&data))
}

/// The cells, the tab stop distance and the properties of the page in a web archive.
pub fn read(bytes: &[u8]) -> Option<(Vec<Cell>, f32, Properties)> {
    let (data, _) = main_resource(bytes)?;
    let (cells, tab) = html::read(&data);
    Some((cells, tab, html::read_properties(&data)))
}

/// The cells as a web archive: the HTML of the page as the main resource, with no subresources.
pub fn write(cells: &[Cell], tab: f32, props: &Properties) -> Vec<u8> {
    let main = BValue::Dict(vec![
        ("WebResourceData".to_string(), BValue::Data(html::write_with(cells, tab, props))),
        ("WebResourceMIMEType".to_string(), BValue::Str("text/html".to_string())),
        ("WebResourceTextEncodingName".to_string(), BValue::Str("UTF-8".to_string())),
        ("WebResourceFrameName".to_string(), BValue::Str(String::new())),
    ]);
    binary_plist::write(&BValue::Dict(vec![
        ("WebMainResource".to_string(), main),
        ("WebSubresources".to_string(), BValue::Array(Vec::new())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Style;

    #[test]
    fn a_web_archive_keeps_the_page_its_formatting_and_its_properties() {
        let bold = Style { bold: true, ..Style::default() };
        let cells = vec![Cell { ch: 'h', style: bold }, Cell { ch: 'i', style: Style::default() }, Cell { ch: '\n', style: Style::default() }];
        let props = Properties { title: "Page".into(), author: "Ada".into(), ..Properties::default() };
        let bytes = write(&cells, 48.0, &props);
        assert!(is_webarchive(&bytes));
        assert!(!is_webarchive(b"plain text"));
        let (back, tab, back_props) = read(&bytes).unwrap();
        assert_eq!(back, cells);
        assert_eq!(tab, 48.0);
        assert_eq!(back_props, props);
    }

    #[test]
    fn a_property_list_without_a_page_is_not_a_web_archive() {
        let other = binary_plist::write(&BValue::Dict(vec![("Name".to_string(), BValue::Str("x".to_string()))]));
        assert!(!is_webarchive(&other));
        assert!(read(&other).is_none());
    }
}
