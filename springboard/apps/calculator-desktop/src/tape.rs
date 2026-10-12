//! The paper tape: the calculation history written out as a receipt. The oldest
//! calculation comes first; each one is its expression, and the line under it
//! is `= result`, right-aligned to `WIDTH` columns, the way an adding machine
//! prints a total. Below the calculations comes a statistics block: count, sum,
//! mean, sample standard deviation (two or more values), minimum and maximum,
//! computed by the SDK's `NumericSummary` model. Numbers use the calculator's
//! language (decimal comma when Icelandic is chosen).

use crate::format::{display, localize, Locale};
use crate::history::Entry;
use bifrost_app::controls::column_summary::NumericSummary;

/// Columns the result lines are aligned to.
pub const WIDTH: usize = 32;

/// The title on the first line of every tape.
pub const TITLE: &str = "Calculator Tape";

/// The heading of the statistics block.
pub const STATISTICS: &str = "Statistics";

/// The tape as plain text, one line per row, ending with a newline. `entries`
/// is newest first, as the history keeps it.
pub fn render(entries: &[Entry], locale: Locale) -> String {
    let mut lines = vec![TITLE.to_string()];
    for entry in entries.iter().rev() {
        lines.push(entry.expression.clone());
        let result = format!("= {}", localize(&entry.result, locale));
        lines.push(format!("{:>width$}", result, width = WIDTH));
    }
    let statistics = statistics_lines(entries, locale);
    if !statistics.is_empty() {
        lines.push(String::new());
        lines.extend(statistics);
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// The statistics block as lines: the heading, then each label with its value
/// right-aligned to `WIDTH`. Empty when there are no entries. The standard
/// deviation line is left out for a single value, where it is undefined.
pub fn statistics_lines(entries: &[Entry], locale: Locale) -> Vec<String> {
    if entries.is_empty() {
        return Vec::new();
    }
    let column: Vec<Option<f64>> = entries.iter().map(|e| Some(e.value)).collect();
    let summary = NumericSummary::new(&column);
    let sum: f64 = column.iter().flatten().filter(|v| !v.is_nan()).sum();
    let mut lines = vec![STATISTICS.to_string(), row("Count", &summary.some_count.to_string())];
    lines.push(row("Sum", &display(sum, locale)));
    if let Some(mean) = summary.mean {
        lines.push(row("Mean", &display(mean, locale)));
    }
    if let Some(sd) = summary.standard_deviation {
        lines.push(row("Std Dev", &display(sd, locale)));
    }
    if let Some(min) = summary.min {
        lines.push(row("Min", &display(min, locale)));
    }
    if let Some(max) = summary.max {
        lines.push(row("Max", &display(max, locale)));
    }
    lines
}

/// A label on the left and its value right-aligned, filling `WIDTH` columns.
fn row(label: &str, value: &str) -> String {
    format!("{label}{value:>width$}", width = WIDTH - label.len())
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
        assert!(statistics_lines(&[], Locale::En).is_empty());
    }

    #[test]
    fn the_tape_runs_oldest_first_with_results_right_aligned() {
        // Newest first in the history: 10/4 was typed after 2+3.
        let history = [entry("10/4", "2.5"), entry("2+3", "5")];
        // "= 5" is 3 columns, so 29 spaces come before it; "= 2.5" is 5, so 27.
        let expected = format!("Calculator Tape\n2+3\n{}= 5\n10/4\n{}= 2.5\n", " ".repeat(29), " ".repeat(27));
        assert!(render(&history, Locale::En).starts_with(&expected));
    }

    #[test]
    fn statistics_of_three_values_are_worked_out_by_hand() {
        // Values 1, 2, 3: sum 6, mean 2, deviations -1, 0, 1 give squares summing to 2;
        // the sample divisor is n - 1 = 2, so the standard deviation is sqrt(1) = 1.
        let history = [entry("3+0", "3"), entry("2+0", "2"), entry("1+0", "1")];
        let lines = statistics_lines(&history, Locale::En);
        assert_eq!(
            lines,
            vec![
                "Statistics".to_string(),
                format!("Count{}3", " ".repeat(26)),   // 27 - 1
                format!("Sum{}6", " ".repeat(28)),     // 29 - 1
                format!("Mean{}2", " ".repeat(27)),    // 28 - 1
                format!("Std Dev{}1", " ".repeat(24)), // 25 - 1
                format!("Min{}1", " ".repeat(28)),     // 29 - 1
                format!("Max{}3", " ".repeat(28)),     // 29 - 1
            ]
        );
        for line in &lines[1..] {
            assert_eq!(line.chars().count(), WIDTH);
        }
    }

    #[test]
    fn the_statistics_block_follows_the_entries_after_a_blank_line() {
        let text = render(&[entry("1+0", "1")], Locale::En);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Calculator Tape");
        assert_eq!(lines[1], "1+0");
        assert!(lines[2].ends_with("= 1"));
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], "Statistics");
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
    fn icelandic_results_and_statistics_use_a_decimal_comma() {
        let text = render(&[entry("10/4", "2.5")], Locale::Is);
        // "= 2,5" is 5 columns, so 27 spaces come first. One value has no standard deviation.
        let expected = format!(
            "Calculator Tape\n10/4\n{}= 2,5\n\nStatistics\nCount{}1\nSum{}2,5\nMean{}2,5\nMin{}2,5\nMax{}2,5\n",
            " ".repeat(27),
            " ".repeat(26),
            " ".repeat(26),
            " ".repeat(25),
            " ".repeat(26),
            " ".repeat(26),
        );
        assert_eq!(text, expected);
    }

    #[test]
    fn a_long_expression_is_kept_whole_on_its_own_line() {
        let long = "1+".repeat(20) + "1";
        let text = render(&[entry(&long, "21")], Locale::En);
        assert_eq!(text.lines().nth(1), Some(long.as_str()));
    }
}
