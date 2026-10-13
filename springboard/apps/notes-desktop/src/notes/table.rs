//! Markdown pipe tables in a note: finding the table under the caret and editing it as text. Everything here is
//! pure text logic on char indices (the editor's unit), so it runs on the host without a window.
//!
//! A table is a block of consecutive lines that start and end with `|`, whose second line is the rule
//! (`| --- | --- |`). The rule is rebuilt from the column count whenever the table is written back.

use std::ops::Range;

/// What the table menu can do to the table under the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    RowAbove,
    RowBelow,
    ColumnLeft,
    ColumnRight,
    DeleteRow,
    DeleteColumn,
    /// Replace the table by plain lines, cells separated by tabs.
    ToText,
}

impl Op {
    pub const ALL: [Op; 7] = [Op::RowAbove, Op::RowBelow, Op::ColumnLeft, Op::ColumnRight, Op::DeleteRow, Op::DeleteColumn, Op::ToText];

    pub fn label(self) -> &'static str {
        match self {
            Op::RowAbove => "Add Row Above",
            Op::RowBelow => "Add Row Below",
            Op::ColumnLeft => "Add Column Left",
            Op::ColumnRight => "Add Column Right",
            Op::DeleteRow => "Delete Row",
            Op::DeleteColumn => "Delete Column",
            Op::ToText => "Convert Table to Text",
        }
    }
}

/// A table's cells: row 0 is the header, the rule line is not stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub rows: Vec<Vec<String>>,
}

/// A table found in a note: the char range of its lines (without the newline after the last one), the row and
/// column the caret is in, and the cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub range: Range<usize>,
    pub row: usize,
    pub col: usize,
    pub table: Table,
}

/// The replacement an operation makes: swap `range` of the note for `with` and put the caret at `caret`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub with: String,
    pub caret: usize,
}

/// A table row line: it starts and ends with `|` and holds at least one cell.
pub fn is_row(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 2 && t.starts_with('|') && t.ends_with('|')
}

/// A rule line: every cell is dashes, optionally with alignment colons.
pub fn is_rule(line: &str) -> bool {
    is_row(line) && cells(line).iter().all(|c| {
        let d = c.trim_matches(':');
        !d.is_empty() && d.chars().all(|ch| ch == '-')
    })
}

/// The trimmed cells of a row line.
pub fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let inner = t.strip_prefix('|').unwrap_or(t);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

impl Table {
    pub fn columns(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }

    /// The Markdown lines: the header, the rule, then the body rows.
    pub fn render(&self) -> String {
        let line = |cells: &[String]| format!("| {} |", cells.join(" | "));
        let rule = vec!["---".to_string(); self.columns()];
        let mut lines = vec![];
        for (i, row) in self.rows.iter().enumerate() {
            lines.push(line(row));
            if i == 0 {
                lines.push(line(&rule));
            }
        }
        lines.join("\n")
    }

    /// The table as text lines, cells separated by tabs (the header too).
    pub fn to_text(&self) -> String {
        self.rows.iter().map(|r| r.join("\t")).collect::<Vec<_>>().join("\n")
    }

    /// Adds an empty row next to row `at`. A row cannot go above the header: false.
    pub fn add_row(&mut self, at: usize, below: bool) -> bool {
        if at >= self.rows.len() || (!below && at == 0) {
            return false;
        }
        let pos = if below { at + 1 } else { at };
        self.rows.insert(pos, vec![String::new(); self.columns()]);
        true
    }

    /// Adds an empty column next to column `at`.
    pub fn add_column(&mut self, at: usize, right: bool) -> bool {
        if at >= self.columns() {
            return false;
        }
        let pos = if right { at + 1 } else { at };
        for row in &mut self.rows {
            row.insert(pos, String::new());
        }
        true
    }

    /// Removes body row `at`; the header stays.
    pub fn delete_row(&mut self, at: usize) -> bool {
        if at == 0 || at >= self.rows.len() {
            return false;
        }
        self.rows.remove(at);
        true
    }

    /// Removes column `at`; the last column stays.
    pub fn delete_column(&mut self, at: usize) -> bool {
        if self.columns() < 2 || at >= self.columns() {
            return false;
        }
        for row in &mut self.rows {
            row.remove(at);
        }
        true
    }
}

/// Finds the table that holds char `caret` of `text`; `None` when the caret is not in a table.
pub fn locate(text: &str, caret: usize) -> Option<Located> {
    let lines: Vec<&str> = text.split('\n').collect();
    // the char offset where each line starts
    let mut starts = Vec::with_capacity(lines.len());
    let mut at = 0;
    for l in &lines {
        starts.push(at);
        at += l.chars().count() + 1;
    }
    let line_of = |c: usize| starts.iter().rposition(|&s| s <= c).unwrap_or(0);
    let here = line_of(caret);
    if !is_row(lines[here]) {
        return None;
    }
    let mut first = here;
    while first > 0 && is_row(lines[first - 1]) {
        first -= 1;
    }
    let mut last = here;
    while last + 1 < lines.len() && is_row(lines[last + 1]) {
        last += 1;
    }
    if last == first || !is_rule(lines[first + 1]) {
        return None;
    }
    let width = cells(lines[first]).len();
    let mut rows = vec![];
    for (i, l) in lines[first..=last].iter().enumerate() {
        if i == 1 {
            continue;
        }
        let mut row = cells(l);
        row.resize(width, String::new());
        rows.push(row);
    }
    // the rule line counts with the header
    let rel = here - first;
    let row = if rel <= 1 { 0 } else { rel - 1 };
    let in_line = caret.saturating_sub(starts[here]);
    let bars = lines[here].trim_start().chars().take(in_line.saturating_sub(lines[here].chars().take_while(|c| c.is_whitespace()).count())).filter(|&c| c == '|').count();
    let col = bars.saturating_sub(1).min(width - 1);
    let end = starts[last] + lines[last].chars().count();
    Some(Located { range: starts[first]..end, row, col, table: Table { rows } })
}

/// Applies `op` to the table under the caret: the replacement to make, or `None` when the caret is not in a table
/// or the operation does not apply (a row above the header, deleting the header or the last column).
pub fn apply(text: &str, caret: usize, op: Op) -> Option<Edit> {
    let Located { range, row, col, mut table } = locate(text, caret)?;
    let with = match op {
        Op::RowAbove => table.add_row(row, false).then(|| table.render())?,
        Op::RowBelow => table.add_row(row, true).then(|| table.render())?,
        Op::ColumnLeft => table.add_column(col, false).then(|| table.render())?,
        Op::ColumnRight => table.add_column(col, true).then(|| table.render())?,
        Op::DeleteRow => table.delete_row(row).then(|| table.render())?,
        Op::DeleteColumn => table.delete_column(col).then(|| table.render())?,
        Op::ToText => table.to_text(),
    };
    let caret = range.start + with.chars().count();
    Some(Edit { range, with, caret })
}

/// The operations that apply to the table under the caret (empty when the caret is not in a table).
pub fn available(text: &str, caret: usize) -> Vec<Op> {
    Op::ALL.into_iter().filter(|&op| apply(text, caret, op).is_some()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "Notes\n| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\nafter";

    fn at(text: &str, needle: &str) -> usize {
        let b = text.find(needle).unwrap();
        text[..b].chars().count()
    }

    fn run(text: &str, caret: usize, op: Op) -> String {
        let e = apply(text, caret, op).unwrap();
        let mut chars: Vec<char> = text.chars().collect();
        drop(chars.splice(e.range.clone(), e.with.chars()));
        chars.into_iter().collect()
    }

    #[test]
    fn rows_and_rules_are_told_apart() {
        assert!(is_row("| a | b |"));
        assert!(is_row("  |a|  "));
        assert!(!is_row("a | b"));
        assert!(!is_row("|"));
        assert!(is_rule("| --- | :---: |"));
        assert!(!is_rule("| a | --- |"));
        assert_eq!(cells("| a |  | c d |"), vec!["a", "", "c d"]);
    }

    #[test]
    fn the_table_under_the_caret_is_found_with_its_row_and_column() {
        let l = locate(T, at(T, "| 3")).unwrap();
        assert_eq!(l.table.rows, vec![vec!["A", "B"], vec!["1", "2"], vec!["3", "4"]]);
        assert_eq!(l.row, 2);
        assert_eq!(l.col, 0);
        assert_eq!(l.range, at(T, "| A")..at(T, "\nafter"));
        // a caret in the second cell of the first body row
        let l = locate(T, at(T, "2 |")).unwrap();
        assert_eq!((l.row, l.col), (1, 1));
        // the rule line belongs to the header row
        assert_eq!(locate(T, at(T, "--- |")).unwrap().row, 0);
        assert!(locate(T, 0).is_none(), "the title line is not in the table");
        assert!(locate(T, at(T, "after")).is_none());
        assert!(locate("| a | b |\n| c | d |", 3).is_none(), "no rule line, no table");
    }

    #[test]
    fn rows_are_added_above_and_below_but_never_above_the_header() {
        let out = run(T, at(T, "| 1"), Op::RowAbove);
        assert_eq!(out, "Notes\n| A | B |\n| --- | --- |\n|  |  |\n| 1 | 2 |\n| 3 | 4 |\nafter");
        let out = run(T, at(T, "| 1"), Op::RowBelow);
        assert_eq!(out, "Notes\n| A | B |\n| --- | --- |\n| 1 | 2 |\n|  |  |\n| 3 | 4 |\nafter");
        assert!(apply(T, at(T, "| A"), Op::RowAbove).is_none());
        let out = run(T, at(T, "| A"), Op::RowBelow);
        assert_eq!(out, "Notes\n| A | B |\n| --- | --- |\n|  |  |\n| 1 | 2 |\n| 3 | 4 |\nafter");
    }

    #[test]
    fn columns_are_added_left_and_right_of_the_caret_cell() {
        let out = run(T, at(T, "2 |"), Op::ColumnLeft);
        assert_eq!(out, "Notes\n| A |  | B |\n| --- | --- | --- |\n| 1 |  | 2 |\n| 3 |  | 4 |\nafter");
        let out = run(T, at(T, "| 1"), Op::ColumnRight);
        assert_eq!(out, "Notes\n| A |  | B |\n| --- | --- | --- |\n| 1 |  | 2 |\n| 3 |  | 4 |\nafter");
    }

    #[test]
    fn rows_and_columns_are_deleted_but_not_the_header_or_the_last_column() {
        let out = run(T, at(T, "| 1"), Op::DeleteRow);
        assert_eq!(out, "Notes\n| A | B |\n| --- | --- |\n| 3 | 4 |\nafter");
        assert!(apply(T, at(T, "| A"), Op::DeleteRow).is_none());
        let out = run(T, at(T, "2 |"), Op::DeleteColumn);
        assert_eq!(out, "Notes\n| A |\n| --- |\n| 1 |\n| 3 |\nafter");
        let one = "| A |\n| --- |\n| 1 |";
        assert!(apply(one, 2, Op::DeleteColumn).is_none(), "the last column stays");
    }

    #[test]
    fn converting_to_text_separates_cells_with_tabs_and_drops_the_rule() {
        let out = run(T, at(T, "| 1"), Op::ToText);
        assert_eq!(out, "Notes\nA\tB\n1\t2\n3\t4\nafter");
        let e = apply(T, at(T, "| 1"), Op::ToText).unwrap();
        assert_eq!(e.caret, e.range.start + "A\tB\n1\t2\n3\t4".chars().count());
    }

    #[test]
    fn a_ragged_row_is_padded_and_the_menu_lists_only_what_applies() {
        let ragged = "| A | B |\n| --- | --- |\n| 1 |";
        let l = locate(ragged, 24).unwrap();
        assert_eq!(l.table.rows[1], vec!["1", ""]);
        assert!(available("plain text", 2).is_empty());
        let header = available(T, at(T, "| A"));
        assert!(!header.contains(&Op::RowAbove) && !header.contains(&Op::DeleteRow));
        assert!(header.contains(&Op::RowBelow) && header.contains(&Op::ToText));
        assert_eq!(available(T, at(T, "| 1")).len(), Op::ALL.len());
        assert_eq!(Op::ToText.label(), "Convert Table to Text");
    }
}
