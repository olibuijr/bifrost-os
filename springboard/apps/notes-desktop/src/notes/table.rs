//! Pipe-table commands: add a row or column, delete one, or turn the table into tab-separated text. The table is
//! the run of lines starting with `|` around the caret; the second line is the header rule. Pure functions on the
//! note text, so the editor applies each as one undoable rewrite.

/// A table command (the editor menu shows them while the caret is in a table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    RowAbove,
    RowBelow,
    ColumnLeft,
    ColumnRight,
    DeleteRow,
    DeleteColumn,
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

fn is_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// The first and one-past-last line of the table containing `line`.
fn block(lines: &[&str], line: usize) -> Option<(usize, usize)> {
    if !is_row(lines.get(line)?) {
        return None;
    }
    let mut a = line;
    while a > 0 && is_row(lines[a - 1]) {
        a -= 1;
    }
    let mut b = line + 1;
    while b < lines.len() && is_row(lines[b]) {
        b += 1;
    }
    Some((a, b))
}

/// Whether `line` is inside a pipe table.
pub fn in_table(text: &str, line: usize) -> bool {
    let lines: Vec<&str> = text.split('\n').collect();
    block(&lines, line).is_some()
}

fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

fn render(cells: &[String]) -> String {
    format!("| {} |", cells.iter().map(|c| if c.is_empty() { " ".to_string() } else { c.clone() }).collect::<Vec<_>>().join(" | "))
}

fn is_rule(row: &[String]) -> bool {
    !row.is_empty() && row.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
}

/// The column index of char column `col` in `line`: the pipes before it, less the leading one.
fn column_at(line: &str, col: usize, count: usize) -> usize {
    let pipes = line.chars().take(col).filter(|&c| c == '|').count();
    pipes.saturating_sub(1).min(count.saturating_sub(1))
}

/// Applies `op` to the table at `line`, `col` (a char column); None when the caret is not in a table or the command
/// does not apply (adding above the header, deleting the header or the only column).
pub fn apply(text: &str, line: usize, col: usize, op: Op) -> Option<String> {
    let all: Vec<&str> = text.split('\n').collect();
    let (a, b) = block(&all, line)?;
    let mut rows: Vec<Vec<String>> = all[a..b].iter().map(|l| cells(l)).collect();
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for r in rows.iter_mut() {
        let fill = if is_rule(r) { "---" } else { "" };
        while r.len() < width {
            r.push(fill.to_string());
        }
    }
    let ri = line - a;
    let ci = column_at(all[line], col, width);
    let has_rule = rows.len() > 1 && is_rule(&rows[1]);
    let protected = |i: usize| i == 0 || (has_rule && i == 1);
    match op {
        Op::RowAbove => {
            if protected(ri) {
                return None;
            }
            rows.insert(ri, vec![String::new(); width]);
        }
        Op::RowBelow => {
            // below the header rule at the earliest
            let at = if has_rule && ri == 0 { 2 } else { ri + 1 };
            rows.insert(at, vec![String::new(); width]);
        }
        Op::DeleteRow => {
            if protected(ri) {
                return None;
            }
            rows.remove(ri);
        }
        Op::ColumnLeft | Op::ColumnRight => {
            let at = if op == Op::ColumnLeft { ci } else { ci + 1 };
            for r in rows.iter_mut() {
                let v = if is_rule(r) { "---" } else { "" };
                r.insert(at, v.to_string());
            }
        }
        Op::DeleteColumn => {
            if width <= 1 {
                return None;
            }
            for r in rows.iter_mut() {
                r.remove(ci);
            }
        }
        Op::ToText => {
            let out: Vec<String> = rows.iter().filter(|r| !is_rule(r)).map(|r| r.join("\t")).collect();
            return Some(splice(&all, a, b, out));
        }
    }
    Some(splice(&all, a, b, rows.iter().map(|r| render(r)).collect()))
}

fn splice(all: &[&str], a: usize, b: usize, mid: Vec<String>) -> String {
    all[..a].iter().map(|s| s.to_string()).chain(mid).chain(all[b..].iter().map(|s| s.to_string())).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "intro\n| A | B |\n| --- | --- |\n| 1 | 2 |\nafter";

    #[test]
    fn finds_the_table_around_a_line() {
        assert!(in_table(T, 1) && in_table(T, 3));
        assert!(!in_table(T, 0) && !in_table(T, 4));
    }

    #[test]
    fn adds_rows_but_not_above_the_header() {
        assert_eq!(apply(T, 3, 2, Op::RowBelow).unwrap(), "intro\n| A | B |\n| --- | --- |\n| 1 | 2 |\n|   |   |\nafter");
        assert_eq!(apply(T, 3, 2, Op::RowAbove).unwrap(), "intro\n| A | B |\n| --- | --- |\n|   |   |\n| 1 | 2 |\nafter");
        assert_eq!(apply(T, 1, 2, Op::RowAbove), None);
        assert_eq!(apply(T, 2, 2, Op::DeleteRow), None);
    }

    #[test]
    fn adds_and_deletes_columns_at_the_caret() {
        // the caret in the second cell of the body row (char column 8)
        assert_eq!(apply(T, 3, 8, Op::ColumnRight).unwrap(), "intro\n| A | B |   |\n| --- | --- | --- |\n| 1 | 2 |   |\nafter");
        assert_eq!(apply(T, 3, 2, Op::ColumnLeft).unwrap(), "intro\n|   | A | B |\n| --- | --- | --- |\n|   | 1 | 2 |\nafter");
        assert_eq!(apply(T, 3, 8, Op::DeleteColumn).unwrap(), "intro\n| A |\n| --- |\n| 1 |\nafter");
        assert_eq!(apply("| a |\n| - |", 0, 2, Op::DeleteColumn), None);
    }

    #[test]
    fn deletes_a_body_row_and_converts_to_text() {
        assert_eq!(apply(T, 3, 2, Op::DeleteRow).unwrap(), "intro\n| A | B |\n| --- | --- |\nafter");
        assert_eq!(apply(T, 3, 2, Op::ToText).unwrap(), "intro\nA\tB\n1\t2\nafter");
        assert_eq!(apply(T, 0, 0, Op::ToText), None);
    }
}
