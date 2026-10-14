//! The workflow model: the kinds of action, the fields each one edits, the values that flow from one step to the next,
//! the `.automator` file format, and the runner that executes a sequence and reports every step to a log. Nothing here
//! draws or touches the window; `actions.rs` does the work of each step.

use crate::actions;
use bifrost_core::json::{self, Json};
use std::path::PathBuf;

/// The file format's name and version, written into every workflow file.
pub const FORMAT: &str = "automator-workflow";
pub const VERSION: f64 = 1.0;
/// The name a new workflow starts with.
pub const UNTITLED: &str = "Untitled Workflow";
/// The most fields any action has (the layout reserves rows for them).
pub const MAX_FIELDS: usize = 3;

const YES_NO: &[&str] = &["no", "yes"];
const TEXT_OPS: &[&str] = &["upper", "lower", "trim", "sort lines", "unique lines", "replace"];
const IMAGE_FORMATS: &[&str] = &["png", "bmp"];

/// The actions in the library, in library order.
pub const KINDS: [Kind; 9] = [
    Kind::Find,
    Kind::Rename,
    Kind::Copy,
    Kind::Move,
    Kind::Transform,
    Kind::Combine,
    Kind::Resize,
    Kind::Convert,
    Kind::RunScript,
];

/// The kinds of action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Find,
    Rename,
    Copy,
    Move,
    Transform,
    Combine,
    Resize,
    Convert,
    RunScript,
}

/// One editable field of an action: its label and, for a choice, the values it cycles through (empty for free text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub choices: &'static [&'static str],
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::Find => "Find Files",
            Kind::Rename => "Rename",
            Kind::Copy => "Copy",
            Kind::Move => "Move",
            Kind::Transform => "Transform Text",
            Kind::Combine => "Combine Text",
            Kind::Resize => "Resize Images",
            Kind::Convert => "Convert Images",
            Kind::RunScript => "Run Shell Script",
        }
    }

    /// The library group the action sits in.
    pub fn group(self) -> &'static str {
        match self {
            Kind::Find | Kind::Rename | Kind::Copy | Kind::Move => "Files",
            Kind::Transform | Kind::Combine => "Text",
            Kind::Resize | Kind::Convert => "Images",
            Kind::RunScript => "Shell",
        }
    }

    /// The name used in workflow files.
    pub fn slug(self) -> &'static str {
        match self {
            Kind::Find => "find",
            Kind::Rename => "rename",
            Kind::Copy => "copy",
            Kind::Move => "move",
            Kind::Transform => "transform",
            Kind::Combine => "combine",
            Kind::Resize => "resize",
            Kind::Convert => "convert",
            Kind::RunScript => "run-script",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Kind> {
        KINDS.iter().copied().find(|k| k.slug() == slug)
    }

    /// The fields the action edits, in the order they are shown and stored.
    pub fn fields(self) -> &'static [Field] {
        match self {
            Kind::Find => &[
                Field { label: "Folder", choices: &[] },
                Field { label: "Name contains", choices: &[] },
                Field { label: "Subfolders", choices: YES_NO },
            ],
            Kind::Rename => &[Field { label: "Name", choices: &[] }],
            Kind::Copy | Kind::Move => &[Field { label: "Destination", choices: &[] }],
            Kind::Transform => &[
                Field { label: "Operation", choices: TEXT_OPS },
                Field { label: "Find", choices: &[] },
                Field { label: "Replace with", choices: &[] },
            ],
            Kind::Combine => &[Field { label: "Separator", choices: &[] }],
            Kind::Resize => &[
                Field { label: "Width", choices: &[] },
                Field { label: "Height", choices: &[] },
            ],
            Kind::Convert => &[Field { label: "Format", choices: IMAGE_FORMATS }],
            Kind::RunScript => &[Field { label: "Script", choices: &[] }],
        }
    }

    /// The starting value of every field. `home` is the folder Find starts in.
    pub fn defaults(self, home: &str) -> Vec<String> {
        match self {
            Kind::Find => vec![home.into(), String::new(), "no".into()],
            Kind::Rename => vec!["{name}-{n}".into()],
            Kind::Copy | Kind::Move => vec![String::new()],
            Kind::Transform => vec!["upper".into(), String::new(), String::new()],
            Kind::Combine => vec![", ".into()],
            Kind::Resize => vec!["1024".into(), "0".into()],
            Kind::Convert => vec!["png".into()],
            Kind::RunScript => vec!["cat".into()],
        }
    }

    /// One or two sentences on what the action does, shown under its fields.
    pub fn hint(self) -> &'static str {
        match self {
            Kind::Find => "Lists matching files. Name contains takes text, or *.ext for an extension. Subfolders searches every level below the folder.",
            Kind::Rename => "Names use {name} (without the extension), {ext} and {n} (the position in the list). Without {ext} the extension is kept.",
            Kind::Copy => "Copies the files into the destination folder, which is created if it is missing. Existing files are not replaced.",
            Kind::Move => "Moves the files into the destination folder. Existing files are not replaced.",
            Kind::Transform => "Works on each line of the input. Replace needs text to find; Replace with may be empty.",
            Kind::Combine => "Joins the input lines with the separator. Write \\n for a new line.",
            Kind::Resize => "Saves each picture beside the original as name-WxH.png. A width or height of 0 keeps the proportions.",
            Kind::Convert => "Saves each picture beside the original in the chosen format. Pictures already in it are left alone.",
            Kind::RunScript => "Runs with sh -c. The input arrives on standard input, and the standard output becomes the output.",
        }
    }
}

/// One action in a workflow with its field values, in the order of `Kind::fields`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub kind: Kind,
    pub values: Vec<String>,
}

impl Step {
    /// A step of `kind` with its default values.
    pub fn new(kind: Kind, home: &str) -> Step {
        Step { kind, values: kind.defaults(home) }
    }

    /// The value of field `i`, or "" when the step has no such field.
    pub fn value(&self, i: usize) -> &str {
        self.values.get(i).map_or("", String::as_str)
    }

    /// Moves a choice field to its next choice (wrapping). A free text field is left as it is.
    pub fn cycle(&mut self, i: usize) {
        let Some(field) = self.kind.fields().get(i).copied() else { return };
        if field.choices.is_empty() {
            return;
        }
        let current = self.value(i);
        let at = field.choices.iter().position(|c| *c == current).unwrap_or(field.choices.len() - 1);
        let next = field.choices[(at + 1) % field.choices.len()];
        if let Some(slot) = self.values.get_mut(i) {
            *slot = next.to_string();
        }
    }

    /// One line for the sequence list: the non-empty fields as `label: value`.
    pub fn summary(&self) -> String {
        let parts: Vec<String> = self
            .kind
            .fields()
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.value(*i).is_empty())
            .map(|(i, f)| format!("{}: {}", f.label, self.value(i)))
            .collect();
        parts.join("; ")
    }
}

/// What one step hands to the next: nothing, a list of files, or text.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Value {
    #[default]
    Empty,
    Paths(Vec<PathBuf>),
    Text(String),
}

impl Value {
    /// The value as lines: the file paths, or the text's lines.
    pub fn lines(&self) -> Vec<String> {
        match self {
            Value::Empty => Vec::new(),
            Value::Paths(paths) => paths.iter().map(|p| p.display().to_string()).collect(),
            Value::Text(text) => text.lines().map(str::to_string).collect(),
        }
    }

    /// The value as file paths: the paths themselves, or each non-blank line of the text as a path.
    pub fn paths(&self) -> Vec<PathBuf> {
        match self {
            Value::Empty => Vec::new(),
            Value::Paths(paths) => paths.clone(),
            Value::Text(text) => text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect(),
        }
    }

    /// The value as one text: the lines joined with newlines.
    pub fn text(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Paths(_) => self.lines().join("\n") + "\n",
            Value::Text(text) => text.clone(),
        }
    }

    /// A short description for the log.
    pub fn summary(&self) -> String {
        match self {
            Value::Empty => "nothing".into(),
            Value::Paths(paths) => plural(paths.len(), "file"),
            Value::Text(text) => {
                let lines = text.lines().count();
                if lines == 0 {
                    "empty text".into()
                } else {
                    format!("text, {}", plural(lines, "line"))
                }
            }
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// A named sequence of steps: what Save writes and Open reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workflow {
    pub name: String,
    pub steps: Vec<Step>,
}

impl Default for Workflow {
    fn default() -> Self {
        Workflow { name: UNTITLED.into(), steps: Vec::new() }
    }
}

impl Workflow {
    /// The workflow as a `.automator` file's text.
    pub fn to_file(&self) -> String {
        let steps = self
            .steps
            .iter()
            .map(|s| {
                Json::Obj(vec![
                    ("action".into(), Json::Str(s.kind.slug().into())),
                    ("values".into(), Json::Arr(s.values.iter().map(|v| Json::Str(v.clone())).collect())),
                ])
            })
            .collect();
        Json::Obj(vec![
            ("format".into(), Json::Str(FORMAT.into())),
            ("version".into(), Json::Num(VERSION)),
            ("name".into(), Json::Str(self.name.clone())),
            ("steps".into(), Json::Arr(steps)),
        ])
        .encode()
    }

    /// Reads a `.automator` file. A step with fewer values than its action has is filled with the defaults; an
    /// unknown action or a file of another format is an error.
    pub fn from_file(text: &str) -> Result<Workflow, String> {
        let doc = json::parse(text).map_err(|e| format!("not a workflow file ({e})"))?;
        if doc.get("format").and_then(Json::as_str) != Some(FORMAT) {
            return Err("not an Automator workflow file".into());
        }
        if doc.get("version").and_then(Json::number) != Some(VERSION) {
            return Err("this workflow was saved by a newer version of Automator".into());
        }
        let name = doc.get("name").and_then(Json::as_str).unwrap_or(UNTITLED).to_string();
        let items = doc.get("steps").and_then(Json::as_array).ok_or("the workflow has no steps list")?;
        let mut steps = Vec::with_capacity(items.len());
        for item in items {
            let slug = item.get("action").and_then(Json::as_str).ok_or("a step has no action")?;
            let kind = Kind::from_slug(slug).ok_or_else(|| format!("unknown action {slug}"))?;
            let mut values = kind.defaults("");
            if let Some(stored) = item.get("values").and_then(Json::as_array) {
                for (i, v) in stored.iter().take(values.len()).enumerate() {
                    if let Some(text) = v.as_str() {
                        values[i] = text.to_string();
                    }
                }
            }
            steps.push(Step { kind, values });
        }
        Ok(Workflow { name, steps })
    }
}

/// One line of the run log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub error: bool,
    pub text: String,
}

impl LogLine {
    pub fn info(text: impl Into<String>) -> LogLine {
        LogLine { error: false, text: text.into() }
    }

    pub fn error(text: impl Into<String>) -> LogLine {
        LogLine { error: true, text: text.into() }
    }
}

/// Runs the steps in order. Each step gets the previous step's output (the first gets nothing). `log` hears every
/// step as it starts, what it did, and its output. The first failing step stops the run and its error is returned.
pub fn execute(steps: &[Step], log: &mut dyn FnMut(LogLine)) -> Result<Value, String> {
    if steps.is_empty() {
        return Err("the workflow has no actions".into());
    }
    let total = steps.len();
    let mut value = Value::Empty;
    for (i, step) in steps.iter().enumerate() {
        log(LogLine::info(format!("Step {} of {}: {}", i + 1, total, step.kind.title())));
        match actions::run(step, value, &mut *log) {
            Ok(next) => {
                log(LogLine::info(format!("Output: {}", next.summary())));
                value = next;
            }
            Err(error) => {
                log(LogLine::error(error.clone()));
                return Err(format!("stopped at step {}", i + 1));
            }
        }
    }
    log(LogLine::info("Done"));
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_default_for_each_field() {
        for kind in KINDS {
            assert_eq!(kind.defaults("/home/u").len(), kind.fields().len(), "{:?}", kind);
            assert!(kind.fields().len() <= MAX_FIELDS);
        }
    }

    #[test]
    fn slugs_round_trip() {
        for kind in KINDS {
            assert_eq!(Kind::from_slug(kind.slug()), Some(kind));
        }
        assert_eq!(Kind::from_slug("nope"), None);
    }

    #[test]
    fn choices_cycle_and_wrap() {
        let mut step = Step::new(Kind::Find, "/home/u");
        assert_eq!(step.value(2), "no");
        step.cycle(2);
        assert_eq!(step.value(2), "yes");
        step.cycle(2);
        assert_eq!(step.value(2), "no");
        // free text fields do not cycle
        step.cycle(0);
        assert_eq!(step.value(0), "/home/u");
    }

    #[test]
    fn summary_lists_non_empty_fields() {
        let step = Step { kind: Kind::Find, values: vec!["/x".into(), String::new(), "yes".into()] };
        assert_eq!(step.summary(), "Folder: /x; Subfolders: yes");
    }

    #[test]
    fn values_convert_between_lines_paths_and_text() {
        let paths = Value::Paths(vec![PathBuf::from("/a/b.txt"), PathBuf::from("/a/c.txt")]);
        assert_eq!(paths.lines(), vec!["/a/b.txt", "/a/c.txt"]);
        assert_eq!(paths.text(), "/a/b.txt\n/a/c.txt\n");
        let text = Value::Text("  /x/one.png \n\n/x/two.png\n".into());
        assert_eq!(text.paths(), vec![PathBuf::from("/x/one.png"), PathBuf::from("/x/two.png")]);
        assert_eq!(Value::Empty.paths(), Vec::<PathBuf>::new());
        assert_eq!(Value::Empty.summary(), "nothing");
        assert_eq!(paths.summary(), "2 files");
        assert_eq!(Value::Paths(vec![PathBuf::from("/a")]).summary(), "1 file");
        assert_eq!(Value::Text("a\nb".into()).summary(), "text, 2 lines");
    }

    #[test]
    fn workflow_file_round_trips() {
        let mut find = Step::new(Kind::Find, "/home/u/Pictures");
        find.values[1] = "*.png".into();
        find.values[2] = "yes".into();
        let workflow = Workflow {
            name: "Resize shots".into(),
            steps: vec![find, Step::new(Kind::Resize, "/"), Step::new(Kind::RunScript, "/")],
        };
        let text = workflow.to_file();
        assert_eq!(Workflow::from_file(&text), Ok(workflow));
    }

    #[test]
    fn workflow_file_rejects_other_documents() {
        assert!(Workflow::from_file("not json").is_err());
        assert_eq!(
            Workflow::from_file(r#"{"format":"other","version":1,"steps":[]}"#),
            Err("not an Automator workflow file".into())
        );
        assert!(Workflow::from_file(r#"{"format":"automator-workflow","version":2,"steps":[]}"#).is_err());
        let unknown = r#"{"format":"automator-workflow","version":1,"steps":[{"action":"fly","values":[]}]}"#;
        assert_eq!(Workflow::from_file(unknown), Err("unknown action fly".into()));
    }

    #[test]
    fn short_value_lists_are_filled_with_defaults() {
        let text = r#"{"format":"automator-workflow","version":1,"name":"x","steps":[{"action":"find","values":["/only"]}]}"#;
        let workflow = Workflow::from_file(text).unwrap();
        assert_eq!(workflow.steps[0].values, vec!["/only".to_string(), String::new(), "no".to_string()]);
    }

    #[test]
    fn an_empty_workflow_does_not_run() {
        let mut lines = Vec::new();
        let result = execute(&[], &mut |l| lines.push(l));
        assert_eq!(result, Err("the workflow has no actions".into()));
        assert!(lines.is_empty());
    }

    #[test]
    fn a_failing_step_stops_the_run_and_is_logged() {
        // Rename needs files from the step before it; the first step has none.
        let steps = vec![Step::new(Kind::Rename, "/"), Step::new(Kind::Transform, "/")];
        let mut lines = Vec::new();
        let result = execute(&steps, &mut |l| lines.push(l));
        assert_eq!(result, Err("stopped at step 1".into()));
        assert!(lines.iter().any(|l| l.error && l.text.contains("Rename")));
        assert!(!lines.iter().any(|l| l.text.contains("Step 2")));
    }

    #[test]
    fn the_output_of_one_step_feeds_the_next() {
        let mut combine = Step::new(Kind::Combine, "/");
        combine.values[0] = "\\n".into();
        let steps = vec![Step::new(Kind::Transform, "/"), combine];
        let mut lines = Vec::new();
        // the first step has no input, so its output is empty text and combine joins nothing
        let result = execute(&steps, &mut |l| lines.push(l));
        assert_eq!(result, Ok(Value::Text(String::new())));
        assert_eq!(lines.last().map(|l| l.text.as_str()), Some("Done"));
    }
}
