//! The paper tape: the calculation history written out as a receipt. The oldest
//! calculation comes first; each one is its expression, and the line under it
//! is `= result`, right-aligned to `WIDTH` columns, the way an adding machine
//! prints a total. Results use the calculator's language (decimal comma when
//! Icelandic is chosen).

use crate::format::{localize, Locale};
use crate::history::Entry;

/// Columns the result lines are aligned to.
pub const WIDTH: usize = 32;

/// The title on the first line of every tape.
pub const TITLE: &str = "Calculator Tape";

/// The tape as plain text, one line per row, ending with a newline. `entries`
/// is newest first, as the history keeps it.
pub fn render(entries: &[Entry], locale: Locale) -> String {
    let mut lines = vec![TITLE.to_string()];
    for entry in entries.iter().rev() {
        lines.push(entry.expression.clone());
        let result = format!("= {}", localize(&entry.result, locale));
        lines.push(format!("{:>width$}", result, width = WIDTH));
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(expression: &str, result: &str) -> Entry {
        Entry { expression: expression.into(), result: result.into(), value: result.parse().unwrap_or(0.0) }
    }

    #[test]
    fn an_empty_history_gives_only_the_title() {
        assert_eq!(render(&[], Locale::En), "Calculator Tape\n");
    }

    #[test]
    fn the_tape_runs_oldest_first_with_results_right_aligned() {
        // Newest first in the history: 10/4 was typed after 2+3.
        let history = [entry("10/4", "2.5"), entry("2+3", "5")];
        // "= 5" is 3 columns, so 32 - 3 = 29 spaces come before it; "= 2.5" is 5, so 27.
        let expected = format!(
            "Calculator Tape\n2+3\n{}= 5\n10/4\n{}= 2.5\n",
            " ".repeat(29),
            " ".repeat(27)
        );
        assert_eq!(render(&history, Locale::En), expected);
    }

    #[test]
    fn result_lines_are_exactly_the_tape_width() {
        let history = [entry("1000*3", "3000"), entry("7-8", "-1")];
        let text = render(&history, Locale::En);
        let results: Vec<&str> = text.lines().filter(|l| l.contains('=')).collect();
        assert_eq!(results.len(), 2);
        for line in results {
            assert_eq!(line.chars().count(), WIDTH);
            assert!(line.ends_with(|c: char| c.is_ascii_digit()));
        }
    }

    #[test]
    fn icelandic_results_use_a_decimal_comma() {
        let text = render(&[entry("10/4", "2.5")], Locale::Is);
        // "= 2,5" is 5 columns, so 27 spaces come first.
        assert_eq!(text, format!("Calculator Tape\n10/4\n{}= 2,5\n", " ".repeat(27)));
    }

    #[test]
    fn a_long_expression_is_kept_whole_on_its_own_line() {
        let long = "1+".repeat(20) + "1";
        let text = render(&[entry(&long, "21")], Locale::En);
        assert_eq!(text.lines().nth(1), Some(long.as_str()));
    }
}
