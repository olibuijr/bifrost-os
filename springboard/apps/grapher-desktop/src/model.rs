//! The graph document: the equations with their colours, the selected equation and the view. Pure: no drawing and no
//! shell calls. Saved as plain text, one equation per line.

use crate::equation::{parse_equation, Equation};
use crate::view::View;

/// The colours the equations take in turn, as (red, green, blue).
pub const PALETTE: [(u8, u8, u8); 6] = [
    (10, 132, 255),
    (255, 69, 58),
    (48, 209, 88),
    (255, 159, 10),
    (191, 90, 242),
    (100, 210, 255),
];

/// The equations a new document starts with.
pub const STARTER: [&str; 3] = ["y = sin(x)", "y = x^2 / 4 - 1", "r = 2 + cos(3t)"];

/// One equation as the user typed it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: u32,
    pub text: String,
    /// The index into `PALETTE`.
    pub colour: usize,
}

impl Row {
    pub fn rgb(&self) -> (u8, u8, u8) {
        PALETTE[self.colour % PALETTE.len()]
    }

    /// The equation, or why the text is not one.
    pub fn equation(&self) -> Result<Equation, String> {
        parse_equation(&self.text)
    }
}

/// The plotted points of one equation, in world units.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub rgb: (u8, u8, u8),
    pub lines: Vec<Vec<(f64, f64)>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    pub rows: Vec<Row>,
    pub selected: Option<u32>,
    pub view: View,
    next_id: u32,
}

impl Model {
    /// A document with the starter equations, the first one selected.
    pub fn starter() -> Model {
        let mut model = Model::default();
        for text in STARTER {
            model.add(text);
        }
        model.selected = model.rows.first().map(|r| r.id);
        model
    }

    /// Reads a saved document: one equation per line. Blank text gives an empty document.
    pub fn from_text(text: &str) -> Model {
        let mut model = Model::default();
        if text.trim().is_empty() {
            return model;
        }
        for line in text.lines() {
            model.add(line.trim_end_matches('\r'));
        }
        model
    }

    /// The document as saved: the equations in order, each on its own line.
    pub fn to_text(&self) -> String {
        self.rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    /// Appends an equation with the next colour, selects it and returns its id.
    pub fn add(&mut self, text: &str) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        let colour = self.rows.len() % PALETTE.len();
        self.rows.push(Row { id, text: text.to_string(), colour });
        self.selected = Some(id);
        id
    }

    /// Removes an equation. When it was selected, the equation after it (or else before it) is selected.
    pub fn remove(&mut self, id: u32) -> bool {
        let Some(position) = self.rows.iter().position(|r| r.id == id) else { return false };
        self.rows.remove(position);
        if self.selected == Some(id) {
            self.selected = self.rows.get(position).or_else(|| self.rows.last()).map(|r| r.id);
        }
        true
    }

    /// Selects an equation. True when the selection changed.
    pub fn select(&mut self, id: u32) -> bool {
        if self.selected == Some(id) || !self.rows.iter().any(|r| r.id == id) {
            return false;
        }
        self.selected = Some(id);
        true
    }

    pub fn selected_row(&self) -> Option<&Row> {
        let id = self.selected?;
        self.rows.iter().find(|r| r.id == id)
    }

    /// Types text into the selected equation. With nothing selected, starts a new equation. True when anything changed.
    pub fn type_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if self.selected.is_none() {
            self.add("");
        }
        let Some(id) = self.selected else { return false };
        match self.rows.iter_mut().find(|r| r.id == id) {
            Some(row) => {
                row.text.push_str(text);
                true
            }
            None => false,
        }
    }

    /// Deletes the last character of the selected equation. True when anything changed.
    pub fn backspace(&mut self) -> bool {
        let Some(id) = self.selected else { return false };
        match self.rows.iter_mut().find(|r| r.id == id) {
            Some(row) if !row.text.is_empty() => {
                row.text.pop();
                true
            }
            _ => false,
        }
    }

    /// The curves of the equations that parse, for a plot of `w` by `h` device pixels.
    pub fn curves(&self, w: f64, h: f64) -> Vec<Curve> {
        self.rows
            .iter()
            .filter_map(|row| {
                let equation = row.equation().ok()?;
                Some(Curve { rgb: row.rgb(), lines: equation.polylines(&self.view, w, h) })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_equations_take_the_palette_in_turn() {
        let mut model = Model::default();
        for _ in 0..(PALETTE.len() + 2) {
            model.add("y = x");
        }
        let colours: Vec<usize> = model.rows.iter().map(|r| r.colour).collect();
        assert_eq!(colours, vec![0, 1, 2, 3, 4, 5, 0, 1]);
    }

    #[test]
    fn adding_selects_the_new_equation() {
        let mut model = Model::default();
        let first = model.add("y = x");
        let second = model.add("y = -x");
        assert_ne!(first, second);
        assert_eq!(model.selected, Some(second));
    }

    #[test]
    fn removing_the_selected_equation_selects_its_neighbour() {
        let mut model = Model::default();
        let a = model.add("y = 1");
        let b = model.add("y = 2");
        let c = model.add("y = 3");
        model.select(b);
        assert!(model.remove(b));
        assert_eq!(model.selected, Some(c));
        assert!(model.remove(c));
        assert_eq!(model.selected, Some(a));
        assert!(model.remove(a));
        assert_eq!(model.selected, None);
        assert!(!model.remove(a));
    }

    #[test]
    fn selecting_an_unknown_id_changes_nothing() {
        let mut model = Model::default();
        let a = model.add("y = 1");
        assert!(!model.select(a));
        assert!(!model.select(999));
        assert_eq!(model.selected, Some(a));
    }

    #[test]
    fn typing_and_backspace_edit_the_selected_equation() {
        let mut model = Model::default();
        assert!(model.type_text("y = 2"));
        assert_eq!(model.selected_row().map(|r| r.text.as_str()), Some("y = 2"));
        assert!(model.backspace());
        assert_eq!(model.selected_row().map(|r| r.text.as_str()), Some("y = "));
        assert!(!model.type_text(""));
    }

    #[test]
    fn typing_with_nothing_selected_starts_an_equation() {
        let mut model = Model::default();
        assert!(model.type_text("x"));
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].text, "x");
    }

    #[test]
    fn backspace_on_an_empty_equation_changes_nothing() {
        let mut model = Model::default();
        model.add("");
        assert!(!model.backspace());
        let mut none = Model::default();
        assert!(!none.backspace());
    }

    #[test]
    fn the_document_round_trips_through_text() {
        let mut model = Model::default();
        model.add("y = sin(x)");
        model.add("");
        model.add("r = 1 + cos(t)");
        let text = model.to_text();
        assert_eq!(text, "y = sin(x)\n\nr = 1 + cos(t)");
        let back = Model::from_text(&text);
        assert_eq!(back.rows.len(), 3);
        assert_eq!(back.rows[1].text, "");
        assert_eq!(back.to_text(), text);
        assert!(Model::from_text("  \n ").rows.is_empty());
    }

    #[test]
    fn curves_skip_equations_that_do_not_parse() {
        let mut model = Model::default();
        model.add("y = (");
        model.add("y = x");
        let curves = model.curves(100.0, 100.0);
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0].rgb, PALETTE[1]);
        assert!(!curves[0].lines.is_empty());
    }

    #[test]
    fn the_starter_document_plots_three_curves() {
        let model = Model::starter();
        assert_eq!(model.rows.len(), 3);
        assert_eq!(model.curves(400.0, 300.0).len(), 3);
        assert_eq!(model.selected, Some(model.rows[0].id));
    }
}
