//! The `braille` service: text to six-dot Braille cells for apps and the screen reader, and the cells a display window
//! shows. The model is `bifrost_ui::braille` (Grade 1, one character per cell group, no contractions). The service
//! does no device I/O: a Braille display driver takes the cells it answers and sends them to the hardware, and none
//! is attached to the PC yet.
//!
//! Methods (JSON parameters):
//! - `translate {text}` answers `{cells, unicode}`: the cell values (bit n is dot n+1) and the same cells as Unicode
//!   Braille patterns. A character the translator does not cover is a service error, so nothing is shown wrongly.
//! - `window {text, width, offset}` answers `{cells, unicode, pages}`: the cells a display of `width` cells shows
//!   with the window panned to `offset` (clamped to the line), and how many pages the whole line takes.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::braille::{to_unicode, translate, BrailleError, Cell, Display};

/// The service name.
pub const SERVICE: &str = "braille";

/// The `braille` service.
pub struct BrailleText;

impl SystemService for BrailleText {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        answer(method, params)
    }
}

/// Runs one method. Pure: it reads only the parameters.
pub fn answer(method: &str, params: &Json) -> Result<Json, String> {
    match method {
        "translate" => {
            let cells = translate(&text_field(params)?).map_err(describe)?;
            Ok(cells_json(&cells))
        }
        "window" => {
            let cells = translate(&text_field(params)?).map_err(describe)?;
            let width = whole(params, "width")?;
            let offset = whole(params, "offset")?;
            let mut display = Display::new(width);
            display.pan(offset as isize, cells.len());
            let pages = display.pages(cells.len());
            let mut out = cells_json(display.window(&cells));
            if let Json::Obj(fields) = &mut out {
                fields.push(("pages".into(), Json::Num(pages as f64)));
            }
            Ok(out)
        }
        _ => Err(format!("braille has no method {method}")),
    }
}

fn text_field(params: &Json) -> Result<String, String> {
    params.str_field("text").ok_or_else(|| "text must be a string".to_string())
}

fn describe(e: BrailleError) -> String {
    match e {
        BrailleError::UnsupportedCharacter(c) => format!("no Braille cells for {c:?}"),
    }
}

fn whole(params: &Json, key: &str) -> Result<usize, String> {
    params
        .get(key)
        .and_then(Json::number)
        .filter(|n| *n >= 0.0 && n.fract() == 0.0)
        .map(|n| n as usize)
        .ok_or_else(|| format!("{key} must be a whole number"))
}

fn cells_json(cells: &[Cell]) -> Json {
    let list = cells.iter().map(|c| Json::Num(*c as f64)).collect();
    Json::Obj(vec![("cells".into(), Json::Arr(list)), ("unicode".into(), Json::Str(to_unicode(cells)))])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn translate_answers_cells_and_unicode() {
        let r = answer("translate", &obj(vec![("text", Json::Str("ab".into()))])).unwrap();
        assert_eq!(r.get("cells").and_then(Json::as_array).map(|a| a.len()), Some(2));
        assert_eq!(r.get("unicode").and_then(Json::as_str).map(|s| s.chars().count()), Some(2));
    }

    #[test]
    fn window_pans_and_reports_pages() {
        let p = obj(vec![("text", Json::Str("abcdef".into())), ("width", Json::Num(2.0)), ("offset", Json::Num(2.0))]);
        let r = answer("window", &p).unwrap();
        assert_eq!(r.get("cells").and_then(Json::as_array).map(|a| a.len()), Some(2));
        assert_eq!(r.get("pages").and_then(Json::number), Some(3.0));
    }

    #[test]
    fn unsupported_characters_and_bad_sizes_are_refused() {
        assert!(answer("translate", &obj(vec![("text", Json::Str("\u{1F600}".into()))])).is_err());
        let bad = obj(vec![("text", Json::Str("a".into())), ("width", Json::Num(-1.0)), ("offset", Json::Num(0.0))]);
        assert!(answer("window", &bad).is_err());
        assert!(answer("nope", &Json::Null).is_err());
    }
}
