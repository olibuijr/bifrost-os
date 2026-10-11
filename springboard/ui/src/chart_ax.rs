//! Accessible chart semantics: the descriptor a screen reader needs to speak a chart. It holds a title, a
//! summary, the axes and the data series, and produces three things from them: a spoken description of each
//! point with its units, a cursor that steps through the points the way a rotor does (point by point, or
//! series by series), and an audio graph, a sequence of tones where pitch follows the value, tone length
//! follows a second value and timbre follows the series. No speech or sound is produced here: the renderer
//! or the screen reader plays what these functions return.
//!
//! Output for numbers follows the locale's separators, so Icelandic reads "1.234,5" and English "1,234.5".

/// The decimal and grouping characters of a locale. `group: None` writes no grouping at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Separators {
    pub decimal: char,
    pub group: Option<char>,
}

impl Separators {
    pub const ENGLISH: Self = Self { decimal: '.', group: Some(',') };
    pub const ICELANDIC: Self = Self { decimal: ',', group: Some('.') };

    /// The separators for a language tag such as "is-IS" or "de". Languages not listed here use English.
    pub fn for_language(tag: &str) -> Self {
        let lang = tag.get(..2).unwrap_or("").to_ascii_lowercase();
        match lang.as_str() {
            "is" | "da" | "de" | "nl" | "it" | "es" | "pt" | "tr" => Self::ICELANDIC,
            _ => Self::ENGLISH,
        }
    }
}

/// Formats a finite number with `decimals` fraction digits and the locale's separators. Returns `None` for NaN
/// and the infinities, which have no spoken value.
pub fn format_number(value: f64, decimals: usize, sep: Separators) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let text = format!("{:.*}", decimals, value.abs());
    let (int, frac) = match text.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (text.as_str(), None),
    };
    let mut out = String::new();
    if value < 0.0 && text.bytes().any(|b| b.is_ascii_digit() && b != b'0') {
        out.push('-');
    }
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            if let Some(g) = sep.group {
                out.push(g);
            }
        }
        out.push(c);
    }
    if let Some(f) = frac {
        out.push(sep.decimal);
        out.push_str(f);
    }
    Some(out)
}

/// One axis of the chart. A categorical axis lists its categories in display order; a numeric axis gives the
/// range its values are scaled to and the unit and decimals spoken after each value.
#[derive(Clone, Debug, PartialEq)]
pub enum Axis {
    Categorical { title: String, order: Vec<String> },
    Numeric { title: String, range: (f64, f64), unit: String, decimals: usize },
}

impl Axis {
    pub fn title(&self) -> &str {
        match self {
            Axis::Categorical { title, .. } | Axis::Numeric { title, .. } => title,
        }
    }
}

/// One data point. `y` is NaN when the point has no value: it is spoken as "no value" and sounds as a rest.
/// `extra` holds further values; the first one sets the tone length in the audio graph.
#[derive(Clone, Debug, PartialEq)]
pub struct DataPoint {
    pub label: String,
    pub y: f64,
    pub extra: Vec<f64>,
}

/// A named run of points, drawn in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: String,
    pub is_continuous: bool,
    pub points: Vec<DataPoint>,
}

/// A position in the chart: which series and which point of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub series: usize,
    pub point: usize,
}

/// A rotor-style movement. `NextSeries` and `PreviousSeries` keep the point index, clamped to the new series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Next,
    Previous,
    First,
    Last,
    NextSeries,
    PreviousSeries,
}

/// One note of the audio graph. `hz` is 0 for a rest, which keeps the timing of a missing value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub hz: f64,
    pub millis: u32,
    pub timbre: u8,
}

/// The lowest and highest pitch of the audio graph, in hertz. Two octaves apart: 220 Hz to 1760 Hz.
const LOW_HZ: f64 = 220.0;
const HIGH_HZ: f64 = 1760.0;
/// Tone length range in milliseconds, used when the points carry an extra value.
const SHORT_MS: f64 = 100.0;
const LONG_MS: f64 = 800.0;
const DEFAULT_MS: u32 = 250;

/// Everything a screen reader needs for one chart.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartDescriptor {
    pub title: String,
    pub summary: String,
    pub x_axis: Axis,
    pub y_axis: Option<Axis>,
    pub series: Vec<Series>,
    pub separators: Separators,
}

impl ChartDescriptor {
    pub fn new(title: &str, summary: &str, x_axis: Axis, y_axis: Option<Axis>, series: Vec<Series>) -> Self {
        Self { title: title.into(), summary: summary.into(), x_axis, y_axis, series, separators: Separators::ENGLISH }
    }

    /// The chart's title, summary and size, spoken when the chart gets focus.
    pub fn overview(&self) -> String {
        let points: usize = self.series.iter().map(|s| s.points.len()).sum();
        let series = if self.series.len() == 1 { "1 series".to_string() } else { format!("{} series", self.series.len()) };
        let points = if points == 1 { "1 point".to_string() } else { format!("{points} points") };
        let mut out = String::new();
        for part in [self.title.as_str(), self.summary.as_str()] {
            if !part.is_empty() {
                out.push_str(part.trim_end_matches('.'));
                out.push_str(". ");
            }
        }
        out.push_str(&format!("{series}, {points}."));
        out
    }

    /// The spoken text of one point: its label, its series, its value with the unit, and its place among the
    /// categories when the x axis is categorical. Returns `None` for a cursor that points at nothing.
    pub fn describe(&self, at: Cursor) -> Option<String> {
        let series = self.series.get(at.series)?;
        let point = series.points.get(at.point)?;
        let mut parts = vec![point.label.clone()];
        if let Axis::Categorical { order, .. } = &self.x_axis {
            if let Some(pos) = order.iter().position(|c| *c == point.label) {
                parts[0] = format!("{} ({} of {})", point.label, pos + 1, order.len());
            }
        }
        if !series.name.is_empty() {
            parts.push(series.name.clone());
        }
        let value = match (&self.y_axis, format_number(point.y, self.decimals(), self.separators)) {
            (Some(Axis::Numeric { unit, .. }), Some(v)) if !unit.is_empty() => format!("{v} {unit}"),
            (_, Some(v)) => v,
            (_, None) => "no value".to_string(),
        };
        parts.push(value);
        Some(parts.join(", "))
    }

    /// Moves the cursor. Returns `None` at an edge, for an invalid cursor, or when the target series is empty.
    pub fn step(&self, at: Cursor, step: Step) -> Option<Cursor> {
        let series = self.series.get(at.series)?;
        let len = series.points.len();
        if at.point >= len {
            return None;
        }
        let across = |to: usize| -> Option<Cursor> {
            let target = self.series.get(to)?;
            let n = target.points.len();
            (n > 0).then(|| Cursor { series: to, point: at.point.min(n - 1) })
        };
        match step {
            Step::Next => (at.point + 1 < len).then(|| Cursor { point: at.point + 1, ..at }),
            Step::Previous => at.point.checked_sub(1).map(|point| Cursor { point, ..at }),
            Step::First => Some(Cursor { point: 0, ..at }),
            Step::Last => Some(Cursor { point: len - 1, ..at }),
            Step::NextSeries => across(at.series + 1),
            Step::PreviousSeries => at.series.checked_sub(1).and_then(across),
        }
    }

    /// The audio graph of one series: pitch follows the value on the numeric y axis (a logarithmic scale, so
    /// equal steps sound as equal intervals), tone length follows the first extra value, and timbre is the
    /// series index. Returns an empty list for an unknown series.
    pub fn audio_graph(&self, series: usize) -> Vec<Tone> {
        let Some(s) = self.series.get(series) else {
            return vec![];
        };
        let (y_lo, y_hi) = self.y_range();
        let (e_lo, e_hi) = self.extra_range();
        s.points
            .iter()
            .map(|p| {
                let hz = if p.y.is_finite() { LOW_HZ * (HIGH_HZ / LOW_HZ).powf(unit(p.y, y_lo, y_hi)) } else { 0.0 };
                let millis = match p.extra.first() {
                    Some(e) if e.is_finite() => {
                        (SHORT_MS + (LONG_MS - SHORT_MS) * unit(*e, e_lo, e_hi)).round() as u32
                    }
                    _ => DEFAULT_MS,
                };
                Tone { hz, millis, timbre: series.min(255) as u8 }
            })
            .collect()
    }

    fn decimals(&self) -> usize {
        match &self.y_axis {
            Some(Axis::Numeric { decimals, .. }) => *decimals,
            _ => 0,
        }
    }

    /// The y scale: the numeric axis range when there is one, otherwise the finite values of all series.
    fn y_range(&self) -> (f64, f64) {
        if let Some(Axis::Numeric { range, .. }) = &self.y_axis {
            return *range;
        }
        finite_range(self.series.iter().flat_map(|s| s.points.iter().map(|p| p.y)))
    }

    fn extra_range(&self) -> (f64, f64) {
        finite_range(self.series.iter().flat_map(|s| s.points.iter().filter_map(|p| p.extra.first().copied())))
    }
}

/// Where `value` sits between `lo` and `hi`, clamped to 0..=1. A degenerate range gives the middle.
fn unit(value: f64, lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        return 0.5;
    }
    ((value - lo) / (hi - lo)).clamp(0.0, 1.0)
}

fn finite_range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.filter(|v| v.is_finite()).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn point(label: &str, y: f64) -> DataPoint {
        DataPoint { label: label.into(), y, extra: vec![] }
    }

    fn chart() -> ChartDescriptor {
        let days = vec!["Mon".to_string(), "Tue".to_string(), "Wed".to_string()];
        let x = Axis::Categorical { title: "Day".into(), order: days };
        let y = Axis::Numeric { title: "Sales".into(), range: (0.0, 100.0), unit: "kg".into(), decimals: 1 };
        let a = Series {
            name: "Fish".into(),
            is_continuous: false,
            points: vec![
                DataPoint { label: "Mon".into(), y: 0.0, extra: vec![1.0] },
                DataPoint { label: "Tue".into(), y: f64::NAN, extra: vec![] },
                DataPoint { label: "Wed".into(), y: 100.0, extra: vec![3.0] },
            ],
        };
        let b = Series { name: "Lamb".into(), is_continuous: true, points: vec![point("Mon", 50.0)] };
        ChartDescriptor::new("Catch", "Sales peaked on Wednesday.", x, Some(y), vec![a, b])
    }

    #[test]
    fn numbers_follow_the_locale_separators() {
        assert_eq!(format_number(1234.5, 1, Separators::ENGLISH).as_deref(), Some("1,234.5"));
        assert_eq!(format_number(1234.5, 1, Separators::ICELANDIC).as_deref(), Some("1.234,5"));
        assert_eq!(format_number(-9.0, 0, Separators::ICELANDIC).as_deref(), Some("-9"));
        assert_eq!(format_number(-0.0001, 2, Separators::ENGLISH).as_deref(), Some("0.00"), "a rounded zero is not negative");
        assert_eq!(format_number(f64::NAN, 1, Separators::ENGLISH), None);
        assert_eq!(Separators::for_language("is-IS"), Separators::ICELANDIC);
        assert_eq!(Separators::for_language("en"), Separators::ENGLISH);
    }

    #[test]
    fn a_point_is_spoken_with_its_unit_and_category_place() {
        let c = chart();
        assert_eq!(c.describe(Cursor { series: 0, point: 0 }).as_deref(), Some("Mon (1 of 3), Fish, 0.0 kg"));
        assert_eq!(c.describe(Cursor { series: 0, point: 1 }).as_deref(), Some("Tue (2 of 3), Fish, no value"), "a missing value is not read as zero");
        assert_eq!(c.describe(Cursor { series: 9, point: 0 }), None, "an unknown series speaks nothing");
    }

    #[test]
    fn the_overview_names_the_chart_and_its_size() {
        assert_eq!(chart().overview(), "Catch. Sales peaked on Wednesday. 2 series, 4 points.");
        let mut one = chart();
        one.series.truncate(1);
        one.series[0].points.truncate(1);
        one.title.clear();
        one.summary.clear();
        assert_eq!(one.overview(), "1 series, 1 point.");
    }

    #[test]
    fn the_cursor_steps_point_by_point_and_stops_at_the_edges() {
        let c = chart();
        let start = Cursor { series: 0, point: 0 };
        let second = c.step(start, Step::Next).unwrap();
        assert_eq!(second, Cursor { series: 0, point: 1 });
        assert_eq!(c.step(start, Step::Previous), None, "no point before the first");
        assert_eq!(c.step(second, Step::Last), Some(Cursor { series: 0, point: 2 }));
        assert_eq!(c.step(Cursor { series: 0, point: 2 }, Step::Next), None, "no point after the last");
        assert_eq!(c.step(Cursor { series: 0, point: 2 }, Step::First), Some(start));
    }

    #[test]
    fn the_cursor_moves_between_series_and_clamps_the_point() {
        let c = chart();
        let last = Cursor { series: 0, point: 2 };
        assert_eq!(c.step(last, Step::NextSeries), Some(Cursor { series: 1, point: 0 }), "the point index is clamped to the shorter series");
        assert_eq!(c.step(Cursor { series: 1, point: 0 }, Step::PreviousSeries), Some(Cursor { series: 0, point: 0 }));
        assert_eq!(c.step(Cursor { series: 1, point: 0 }, Step::NextSeries), None);
        assert_eq!(c.step(Cursor { series: 5, point: 0 }, Step::Next), None, "an invalid cursor cannot move");
    }

    #[test]
    fn pitch_is_logarithmic_across_the_numeric_range() {
        let c = chart();
        let tones = c.audio_graph(0);
        assert_eq!(tones.len(), 3);
        assert!(close(tones[0].hz, 220.0), "the range minimum is the low note");
        assert!(close(tones[2].hz, 1760.0), "the range maximum is the high note");
        let mid = LOW_HZ * (HIGH_HZ / LOW_HZ).sqrt();
        let mut half = chart();
        half.y_axis = Some(Axis::Numeric { title: "x".into(), range: (0.0, 100.0), unit: String::new(), decimals: 0 });
        half.series[0].points[0].y = 50.0;
        assert!(close(half.audio_graph(0)[0].hz, mid), "half the range is the geometric midpoint of the two notes");
    }

    #[test]
    fn a_missing_value_keeps_its_place_as_a_rest() {
        let tones = chart().audio_graph(0);
        assert_eq!(tones[1].hz, 0.0, "a point without a value is a rest");
        assert_eq!(tones[1].millis, DEFAULT_MS, "and it keeps a normal length");
        assert_eq!(tones[0].millis, 100, "the smallest extra value gives the shortest tone");
        assert_eq!(tones[2].millis, 800, "the largest extra value gives the longest tone");
        assert!(tones.iter().all(|t| t.timbre == 0));
    }

    #[test]
    fn an_unknown_series_has_no_audio_graph() {
        assert!(chart().audio_graph(7).is_empty());
        assert_eq!(chart().audio_graph(1)[0].timbre, 1, "each series has its own timbre");
    }

    #[test]
    fn separators_skip_grouping_when_the_locale_has_none() {
        let plain = Separators { decimal: '.', group: None };
        assert_eq!(format_number(1234567.0, 0, plain).as_deref(), Some("1234567"));
    }
}
