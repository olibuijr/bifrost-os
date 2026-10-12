//! Model-quality evaluation: samples, scored metrics, summary statistics and tool-call trajectory checks.
//! Pure logic with no model or I/O. The caller supplies the responder that produces each response, so any model
//! (on-device, remote or a test double) is measured the same way.

/// One input to measure. `expected` is the ground truth when the sample has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub input: String,
    pub expected: Option<String>,
}

impl Sample {
    pub fn new(input: impl Into<String>) -> Self {
        Sample { input: input.into(), expected: None }
    }

    pub fn expecting(input: impl Into<String>, expected: impl Into<String>) -> Self {
        Sample { input: input.into(), expected: Some(expected.into()) }
    }
}

/// A named score for one sample. A pass or fail check scores 1.0 or 0.0, so the mean of its values is the pass rate.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    pub name: String,
    pub value: f64,
}

/// Scores a response to a sample. Closure-based: the score function sees the sample and the response.
pub struct Evaluator {
    name: String,
    score: Box<dyn Fn(&Sample, &str) -> f64>,
}

impl Evaluator {
    pub fn new(name: impl Into<String>, score: impl Fn(&Sample, &str) -> f64 + 'static) -> Self {
        Evaluator { name: name.into(), score: Box::new(score) }
    }

    /// A check that scores 1.0 when `pass` holds and 0.0 otherwise.
    pub fn check(name: impl Into<String>, pass: impl Fn(&Sample, &str) -> bool + 'static) -> Self {
        Self::new(name, move |sample, response| if pass(sample, response) { 1.0 } else { 0.0 })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Statistics over the values of one metric across the dataset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub mean: f64,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    /// Population variance: the squared distances are divided by `count`.
    pub variance: f64,
    pub std_dev: f64,
}

/// Computes the aggregate statistics for a metric's values.
pub struct MetricsAggregator;

impl MetricsAggregator {
    /// `None` when there are no values, so an empty dataset never reports a made-up mean.
    pub fn summarize(values: &[f64]) -> Option<Summary> {
        if values.is_empty() {
            return None;
        }
        let count = values.len();
        let n = count as f64;
        let mean = values.iter().sum::<f64>() / n;
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let median = if count % 2 == 1 {
            sorted[count / 2]
        } else {
            (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
        };
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        Some(Summary {
            count,
            mean,
            median,
            min: sorted[0],
            max: sorted[count - 1],
            variance,
            std_dev: variance.sqrt(),
        })
    }
}

/// A row that failed to run: the response for `sample` (its index in the dataset) returned an error.
#[derive(Clone, Debug, PartialEq)]
pub struct RunError {
    pub sample: usize,
    pub message: String,
}

/// One answered sample with every evaluator's score, in evaluator order.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub input: String,
    pub response: String,
    pub expected: Option<String>,
    pub metrics: Vec<Metric>,
}

/// A column of the result table: the input, the response, the expected value or one metric.
#[derive(Clone, Debug, PartialEq)]
pub enum Column {
    Input,
    Response,
    Expected,
    Metric(String),
}

/// A named set of evaluators, run against a dataset.
pub struct Evaluation {
    name: String,
    evaluators: Vec<Evaluator>,
}

impl Evaluation {
    pub fn new(name: impl Into<String>, evaluators: Vec<Evaluator>) -> Self {
        Evaluation { name: name.into(), evaluators }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Runs every sample through `respond` and scores each response with every evaluator. A sample whose response
    /// fails is recorded in `errors` and leaves no row, so one bad response does not stop the run.
    pub fn run(
        &self,
        samples: &[Sample],
        mut respond: impl FnMut(&Sample) -> Result<String, String>,
    ) -> EvaluationResult {
        let mut rows = Vec::new();
        let mut errors = Vec::new();
        for (index, sample) in samples.iter().enumerate() {
            match respond(sample) {
                Err(message) => errors.push(RunError { sample: index, message }),
                Ok(response) => {
                    let metrics = self
                        .evaluators
                        .iter()
                        .map(|e| Metric { name: e.name.clone(), value: (e.score)(sample, &response) })
                        .collect();
                    rows.push(Row {
                        input: sample.input.clone(),
                        response,
                        expected: sample.expected.clone(),
                        metrics,
                    });
                }
            }
        }
        EvaluationResult { name: self.name.clone(), rows, errors }
    }
}

/// The outcome of one run: the answered rows and the samples that failed.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationResult {
    name: String,
    rows: Vec<Row>,
    errors: Vec<RunError>,
}

impl EvaluationResult {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn errors(&self) -> &[RunError] {
        &self.errors
    }

    /// Every value a metric took, in row order.
    pub fn metric_values(&self, name: &str) -> Vec<f64> {
        self.rows
            .iter()
            .flat_map(|row| row.metrics.iter())
            .filter(|m| m.name == name)
            .map(|m| m.value)
            .collect()
    }

    /// The text of one column for each row, in row order. A missing expected value is an empty string.
    pub fn column(&self, column: &Column) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| match column {
                Column::Input => row.input.clone(),
                Column::Response => row.response.clone(),
                Column::Expected => row.expected.clone().unwrap_or_default(),
                Column::Metric(name) => row
                    .metrics
                    .iter()
                    .find(|m| &m.name == name)
                    .map(|m| m.value.to_string())
                    .unwrap_or_default(),
            })
            .collect()
    }

    /// Summary statistics for each metric, in the order the metrics first appear.
    pub fn summary(&self) -> Vec<(String, Summary)> {
        let mut names: Vec<String> = Vec::new();
        for row in &self.rows {
            for m in &row.metrics {
                if !names.contains(&m.name) {
                    names.push(m.name.clone());
                }
            }
        }
        names
            .into_iter()
            .filter_map(|name| {
                let summary = MetricsAggregator::summarize(&self.metric_values(&name))?;
                Some((name, summary))
            })
            .collect()
    }
}

/// A tool call the model made: the tool name and its arguments as key and value text.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Vec<(String, String)>,
}

impl ToolCall {
    pub fn new(name: &str, arguments: &[(&str, &str)]) -> Self {
        ToolCall {
            name: name.into(),
            arguments: arguments.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    fn argument(&self, key: &str) -> Option<&str> {
        self.arguments.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// How one argument of an expected call is validated. Regex and semantic matching are not implemented here; they
/// need a regex engine or a model.
#[derive(Clone, Debug, PartialEq)]
pub enum ArgumentMatcher {
    /// The argument is present with this exact value.
    Exact { name: String, value: String },
    /// The argument is present with any value.
    KeyOnly { name: String },
    /// The argument is present and its value is one of the allowed values.
    OneOf { name: String, allowed: Vec<String> },
    /// The argument is present and its value parses as a number within the range (inclusive).
    Range { name: String, minimum: f64, maximum: f64 },
    Contains { name: String, substring: String },
    HasPrefix { name: String, prefix: String },
    HasSuffix { name: String, suffix: String },
}

impl ArgumentMatcher {
    fn name(&self) -> &str {
        match self {
            Self::Exact { name, .. }
            | Self::KeyOnly { name }
            | Self::OneOf { name, .. }
            | Self::Range { name, .. }
            | Self::Contains { name, .. }
            | Self::HasPrefix { name, .. }
            | Self::HasSuffix { name, .. } => name,
        }
    }

    /// True when the call has the argument and the argument satisfies this rule.
    pub fn matches(&self, call: &ToolCall) -> bool {
        let Some(actual) = call.argument(self.name()) else { return false };
        match self {
            Self::KeyOnly { .. } => true,
            Self::Exact { value, .. } => actual == value,
            Self::OneOf { allowed, .. } => allowed.iter().any(|a| a == actual),
            Self::Range { minimum, maximum, .. } => {
                actual.trim().parse::<f64>().is_ok_and(|v| v >= *minimum && v <= *maximum)
            }
            Self::Contains { substring, .. } => actual.contains(substring.as_str()),
            Self::HasPrefix { prefix, .. } => actual.starts_with(prefix.as_str()),
            Self::HasSuffix { suffix, .. } => actual.ends_with(suffix.as_str()),
        }
    }
}

/// One expected tool call: its name and the rules every argument must satisfy.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolExpectation {
    pub name: String,
    pub arguments: Vec<ArgumentMatcher>,
}

impl ToolExpectation {
    pub fn new(name: &str, arguments: Vec<ArgumentMatcher>) -> Self {
        ToolExpectation { name: name.into(), arguments }
    }

    pub fn matches(&self, call: &ToolCall) -> bool {
        call.name == self.name && self.arguments.iter().all(|rule| rule.matches(call))
    }
}

/// The tool calls a run should make: `ordered` steps in sequence, `unordered` calls anywhere, calls that must never
/// happen, and whether calls outside the expectations are allowed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrajectoryExpectation {
    pub ordered: Vec<ToolExpectation>,
    pub unordered: Vec<ToolExpectation>,
    pub disallowed: Vec<String>,
    pub allows_additional_calls: bool,
}

impl TrajectoryExpectation {
    pub fn ordered(steps: Vec<ToolExpectation>) -> Self {
        TrajectoryExpectation { ordered: steps, ..Self::default() }
    }

    pub fn allowing_additional_calls(mut self) -> Self {
        self.allows_additional_calls = true;
        self
    }

    /// Checks the calls the model made. Returns the problems found; an empty list means the trajectory passed.
    /// Ordered steps are matched greedily, each at the earliest call after the previous step's call.
    pub fn check(&self, calls: &[ToolCall]) -> Vec<String> {
        let mut problems = Vec::new();
        for name in &self.disallowed {
            if calls.iter().any(|c| &c.name == name) {
                problems.push(format!("disallowed call {name}"));
            }
        }
        let mut used = vec![false; calls.len()];
        let mut cursor = 0;
        for (i, step) in self.ordered.iter().enumerate() {
            match (cursor..calls.len()).find(|&j| step.matches(&calls[j])) {
                Some(j) => {
                    used[j] = true;
                    cursor = j + 1;
                }
                None => problems.push(format!("ordered step {} ({}) not called in order", i + 1, step.name)),
            }
        }
        for step in &self.unordered {
            match (0..calls.len()).find(|&j| !used[j] && step.matches(&calls[j])) {
                Some(j) => used[j] = true,
                None => problems.push(format!("required call {} not made", step.name)),
            }
        }
        if !self.allows_additional_calls {
            for (j, call) in calls.iter().enumerate() {
                if !used[j] {
                    problems.push(format!("unexpected call {}", call.name));
                }
            }
        }
        problems
    }
}

/// One rubric dimension a judge scores on its own scale, from `min` to `max`. `weight` is its share of the overall
/// score. A dimension is valid when its name is not blank, its weight is finite and above zero, and `min < max`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreDimension {
    pub name: String,
    pub weight: f64,
    pub min: f64,
    pub max: f64,
}

impl ScoreDimension {
    pub fn new(name: impl Into<String>, weight: f64, min: f64, max: f64) -> Result<Self, JudgeError> {
        let dim = ScoreDimension { name: name.into(), weight, min, max };
        if dim.is_valid() {
            Ok(dim)
        } else {
            Err(JudgeError::InvalidDimension(dim.name))
        }
    }

    fn is_valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.weight.is_finite()
            && self.weight > 0.0
            && self.min.is_finite()
            && self.max.is_finite()
            && self.min < self.max
    }
}

/// Why a model judge could not be built.
#[derive(Clone, Debug, PartialEq)]
pub enum JudgeError {
    /// The judge has no dimension to score.
    NoDimensions,
    /// A dimension has a blank name, a weight that is not positive, or a scale that is empty.
    InvalidDimension(String),
}

/// The text sent to the judge for one dimension. The placeholders are `{input}`, `{expected}`, `{response}`,
/// `{dimension}`, `{min}` and `{max}`. Filled values are never scanned again, so an input that contains a
/// placeholder keeps it as text.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelJudgePrompt {
    template: String,
}

impl ModelJudgePrompt {
    pub fn new(template: impl Into<String>) -> Self {
        ModelJudgePrompt { template: template.into() }
    }

    pub fn render(&self, sample: &Sample, response: &str, dimension: &ScoreDimension) -> String {
        let values = [
            ("{input}", sample.input.clone()),
            ("{expected}", sample.expected.clone().unwrap_or_default()),
            ("{response}", response.to_string()),
            ("{dimension}", dimension.name.clone()),
            ("{min}", dimension.min.to_string()),
            ("{max}", dimension.max.to_string()),
        ];
        fill(&self.template, &values)
    }
}

/// One pass over the template: a known placeholder is replaced, anything else is copied as it is.
fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        match values.iter().find(|(key, _)| tail.starts_with(key)) {
            Some((key, value)) => {
                out.push_str(value);
                rest = &tail[key.len()..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The score a judge reply gives `dimension`: the first line `<dimension>: <number>`, with the name matched without
/// case. The number must be finite and inside the dimension's scale. Any other reply gives None.
pub fn parse_score(reply: &str, dimension: &ScoreDimension) -> Option<f64> {
    let key = dimension.name.trim().to_ascii_lowercase();
    reply.lines().find_map(|line| {
        let (name, value) = line.trim().split_once(':')?;
        if name.trim().to_ascii_lowercase() != key {
            return None;
        }
        let v: f64 = value.split_whitespace().next()?.parse().ok()?;
        (v.is_finite() && v >= dimension.min && v <= dimension.max).then_some(v)
    })
}

/// An evaluator that asks a model judge to score each response on every dimension. `judge` gets the rendered prompt
/// and returns the judge's reply, or an error. The score is the weighted mean of the normalized dimension scores,
/// from 0.0 (the bottom of every scale) to 1.0 (the top). A judge error or an unreadable reply scores NaN, so check
/// the metric with `is_nan()` before averaging it.
pub fn model_judge(
    name: impl Into<String>,
    dimensions: Vec<ScoreDimension>,
    prompt: ModelJudgePrompt,
    judge: impl Fn(&str) -> Result<String, String> + 'static,
) -> Result<Evaluator, JudgeError> {
    if dimensions.is_empty() {
        return Err(JudgeError::NoDimensions);
    }
    if let Some(bad) = dimensions.iter().find(|d| !d.is_valid()) {
        return Err(JudgeError::InvalidDimension(bad.name.clone()));
    }
    let total: f64 = dimensions.iter().map(|d| d.weight).sum();
    Ok(Evaluator::new(name, move |sample: &Sample, response: &str| {
        let mut weighted = 0.0;
        for dim in &dimensions {
            let reply = match judge(&prompt.render(sample, response, dim)) {
                Ok(reply) => reply,
                Err(_) => return f64::NAN,
            };
            match parse_score(&reply, dim) {
                Some(v) => weighted += dim.weight * (v - dim.min) / (dim.max - dim.min),
                None => return f64::NAN,
            }
        }
        weighted / total
    }))
}

#[cfg(test)]
mod judge_tests {
    use super::*;

    fn accuracy() -> ScoreDimension {
        ScoreDimension::new("Accuracy", 3.0, 1.0, 5.0).unwrap()
    }

    #[test]
    fn a_reply_gives_the_score_of_the_named_dimension_inside_its_scale() {
        let dim = accuracy();
        assert_eq!(parse_score("Accuracy: 4", &dim), Some(4.0));
        assert_eq!(parse_score("accuracy:   5 , well done", &dim), Some(5.0));
        assert_eq!(parse_score("Accuracy: 9", &dim), None, "above the scale");
        assert_eq!(parse_score("Accuracy: 0.5", &dim), None, "below the scale");
        assert_eq!(parse_score("Accuracy: high", &dim), None);
        assert_eq!(parse_score("Clarity: 4", &dim), None, "another dimension");
    }

    #[test]
    fn the_matching_line_is_found_among_others() {
        let reply = "Clarity: 3\nReasoning: good\nAccuracy: 2\nAccuracy: 5";
        assert_eq!(parse_score(reply, &accuracy()), Some(2.0), "the first readable line wins");
        let skipped = "Accuracy: 9\nAccuracy: 3";
        assert_eq!(parse_score(skipped, &accuracy()), Some(3.0), "an out of range line is skipped");
    }

    #[test]
    fn the_prompt_fills_placeholders_once_and_leaves_filled_text_alone() {
        let prompt = ModelJudgePrompt::new("Rate {dimension} of {response} for {input} (scale {min} to {max}).");
        let dim = accuracy();
        let sample = Sample::new("{max} is a placeholder");
        let text = prompt.render(&sample, "a reply", &dim);
        assert_eq!(text, "Rate Accuracy of a reply for {max} is a placeholder (scale 1 to 5).");
        let unknown = ModelJudgePrompt::new("{other} {expected}").render(&Sample::expecting("q", "gold"), "r", &dim);
        assert_eq!(unknown, "{other} gold");
    }

    #[test]
    fn the_overall_score_is_the_weighted_mean_of_normalized_dimensions() {
        let dims = vec![accuracy(), ScoreDimension::new("Clarity", 1.0, 0.0, 10.0).unwrap()];
        let judge = |text: &str| -> Result<String, String> {
            if text.contains("Rate Accuracy") {
                Ok("Accuracy: 5".to_string())
            } else {
                Ok("Clarity: 5".to_string())
            }
        };
        let ev = model_judge("quality", dims, ModelJudgePrompt::new("Rate {dimension}: {response}"), judge).unwrap();
        // Accuracy (5 - 1) / 4 = 1.0 with weight 3; Clarity (5 - 0) / 10 = 0.5 with weight 1; (3 + 0.5) / 4.
        let score = (ev.score)(&Sample::new("q"), "answer");
        assert!((score - 0.875).abs() < 1e-12, "got {score}");
        assert_eq!(ev.name(), "quality");
    }

    #[test]
    fn a_judge_failure_or_an_unreadable_reply_scores_nan() {
        let dims = vec![accuracy()];
        let out_of_scale = model_judge("q", dims.clone(), ModelJudgePrompt::new("{dimension}"), |_| {
            Ok("Accuracy: 9".to_string())
        })
        .unwrap();
        assert!((out_of_scale.score)(&Sample::new("x"), "y").is_nan());
        let failing = model_judge("q", dims, ModelJudgePrompt::new("{dimension}"), |_| Err("offline".into())).unwrap();
        assert!((failing.score)(&Sample::new("x"), "y").is_nan());
    }

    #[test]
    fn invalid_dimensions_and_empty_judges_are_refused() {
        assert_eq!(ScoreDimension::new("  ", 1.0, 0.0, 1.0), Err(JudgeError::InvalidDimension("  ".into())));
        assert!(ScoreDimension::new("A", 0.0, 0.0, 1.0).is_err(), "weight must be positive");
        assert!(ScoreDimension::new("A", 1.0, 2.0, 2.0).is_err(), "the scale must not be empty");
        let none = model_judge("q", vec![], ModelJudgePrompt::new(""), |_| Ok(String::new()));
        assert!(matches!(none, Err(JudgeError::NoDimensions)));
        let forged = ScoreDimension { name: "A".into(), weight: -1.0, min: 0.0, max: 1.0 };
        let bad = model_judge("q", vec![forged], ModelJudgePrompt::new(""), |_| Ok(String::new()));
        assert!(matches!(bad, Err(JudgeError::InvalidDimension(_))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(name: &str, value: &str) -> ArgumentMatcher {
        ArgumentMatcher::Exact { name: name.into(), value: value.into() }
    }

    #[test]
    fn summary_statistics_of_an_even_and_an_odd_dataset() {
        let even = MetricsAggregator::summarize(&[4.0, 1.0, 3.0, 2.0]).unwrap();
        assert_eq!((even.count, even.min, even.max), (4, 1.0, 4.0));
        assert_eq!((even.mean, even.median), (2.5, 2.5));
        assert!((even.variance - 1.25).abs() < 1e-12);
        assert!((even.std_dev - 1.25f64.sqrt()).abs() < 1e-12);

        let odd = MetricsAggregator::summarize(&[9.0, 1.0, 5.0]).unwrap();
        assert_eq!(odd.median, 5.0, "the middle value of the sorted list");
        assert_eq!(MetricsAggregator::summarize(&[]), None, "no values, no statistics");
    }

    #[test]
    fn a_run_scores_each_response_and_the_summary_gives_the_pass_rate() {
        let eval = Evaluation::new(
            "upper",
            vec![
                Evaluator::check("matches", |s, r| s.expected.as_deref() == Some(r)),
                Evaluator::new("length", |_, r| r.len() as f64),
            ],
        );
        let samples = vec![Sample::expecting("a", "A"), Sample::expecting("b", "B"), Sample::expecting("c", "x")];
        let result = eval.run(&samples, |s| Ok(s.input.to_uppercase()));
        assert_eq!(result.name(), "upper");
        assert_eq!(result.rows().len(), 3);
        assert_eq!(result.metric_values("matches"), vec![1.0, 1.0, 0.0]);
        assert_eq!(result.column(&Column::Expected), vec!["A", "B", "x"]);
        assert_eq!(result.column(&Column::Metric("length".into())), vec!["1", "1", "1"]);

        let summary = result.summary();
        let (_, pass) = summary.iter().find(|(n, _)| n == "matches").unwrap();
        assert!((pass.mean - 2.0 / 3.0).abs() < 1e-9, "two of three responses matched");
        assert_eq!(pass.count, 3);
    }

    #[test]
    fn a_failed_response_is_recorded_and_does_not_stop_the_run() {
        let eval = Evaluation::new("echo", vec![Evaluator::check("nonempty", |_, r| !r.is_empty())]);
        let samples = vec![Sample::new("one"), Sample::new("two"), Sample::new("three")];
        let result = eval.run(&samples, |s| {
            if s.input == "two" {
                Err("timeout".into())
            } else {
                Ok(s.input.clone())
            }
        });
        assert_eq!(result.errors(), &[RunError { sample: 1, message: "timeout".into() }]);
        assert_eq!(result.rows().len(), 2, "the failed sample leaves no row");
        assert_eq!(result.rows()[1].input, "three", "later samples still run");
        assert_eq!(result.metric_values("nonempty"), vec![1.0, 1.0]);
    }

    #[test]
    fn argument_matchers_check_presence_value_range_and_text() {
        let call = ToolCall::new(
            "set",
            &[("mode", "dark"), ("level", "3"), ("path", "/home/x.txt"), ("flag", "")],
        );
        assert!(exact("mode", "dark").matches(&call));
        assert!(!exact("mode", "light").matches(&call));
        assert!(ArgumentMatcher::KeyOnly { name: "flag".into() }.matches(&call));
        assert!(!ArgumentMatcher::KeyOnly { name: "missing".into() }.matches(&call), "absent is never a match");
        assert!(ArgumentMatcher::OneOf { name: "mode".into(), allowed: vec!["light".into(), "dark".into()] }
            .matches(&call));
        assert!(ArgumentMatcher::Range { name: "level".into(), minimum: 1.0, maximum: 5.0 }.matches(&call));
        assert!(!ArgumentMatcher::Range { name: "level".into(), minimum: 4.0, maximum: 5.0 }.matches(&call));
        assert!(!ArgumentMatcher::Range { name: "mode".into(), minimum: 0.0, maximum: 9.0 }.matches(&call),
            "text that is not a number is outside any range");
        assert!(ArgumentMatcher::Contains { name: "path".into(), substring: "home".into() }.matches(&call));
        assert!(ArgumentMatcher::HasPrefix { name: "path".into(), prefix: "/home".into() }.matches(&call));
        assert!(ArgumentMatcher::HasSuffix { name: "path".into(), suffix: ".txt".into() }.matches(&call));
    }

    #[test]
    fn ordered_steps_pass_with_extra_calls_only_when_allowed() {
        let calls = [
            ToolCall::new("search", &[("query", "rust")]),
            ToolCall::new("open", &[("id", "7")]),
            ToolCall::new("answer", &[("text", "done")]),
        ];
        let steps = vec![
            ToolExpectation::new("search", vec![ArgumentMatcher::Contains { name: "query".into(), substring: "ru".into() }]),
            ToolExpectation::new("answer", vec![]),
        ];
        let lenient = TrajectoryExpectation::ordered(steps.clone()).allowing_additional_calls();
        assert!(lenient.check(&calls).is_empty(), "open is an allowed extra call");

        let strict = TrajectoryExpectation::ordered(steps);
        assert_eq!(strict.check(&calls), vec!["unexpected call open"]);
    }

    #[test]
    fn out_of_order_steps_fail_and_name_the_step() {
        let calls = [ToolCall::new("search", &[]), ToolCall::new("answer", &[])];
        let reversed = TrajectoryExpectation::ordered(vec![
            ToolExpectation::new("answer", vec![]),
            ToolExpectation::new("search", vec![]),
        ])
        .allowing_additional_calls();
        assert_eq!(reversed.check(&calls), vec!["ordered step 2 (search) not called in order"]);
    }

    #[test]
    fn unordered_calls_and_disallowed_calls() {
        let calls = [ToolCall::new("delete", &[("path", "/x")]), ToolCall::new("open", &[("id", "7")])];
        let expectation = TrajectoryExpectation {
            ordered: vec![],
            unordered: vec![ToolExpectation::new(
                "open",
                vec![ArgumentMatcher::Range { name: "id".into(), minimum: 1.0, maximum: 10.0 }],
            )],
            disallowed: vec!["delete".into()],
            allows_additional_calls: true,
        };
        assert_eq!(expectation.check(&calls), vec!["disallowed call delete"]);

        let missing = TrajectoryExpectation {
            unordered: vec![ToolExpectation::new("share", vec![])],
            allows_additional_calls: true,
            ..TrajectoryExpectation::default()
        };
        assert_eq!(missing.check(&calls), vec!["required call share not made"]);
    }
}
