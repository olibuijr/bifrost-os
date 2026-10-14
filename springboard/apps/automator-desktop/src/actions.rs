//! What each action does. `run` takes the step's field values and the previous step's output, does the work, and returns
//! this step's output; the lines it writes to the log report what changed. Files are changed only by the Rename, Copy,
//! Move, Resize and Convert steps, and only where the step says so. A target that already exists is never replaced.

use crate::imageio::{self, Format};
use crate::model::{LogLine, Kind, Step, Value};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The largest side an image may be resized to (pixels).
const MAX_SIDE: u32 = 10_000;

/// Runs one step on `input` and returns its output.
pub fn run(step: &Step, input: Value, log: &mut dyn FnMut(LogLine)) -> Result<Value, String> {
    match step.kind {
        Kind::Find => find(step),
        Kind::Rename => rename(step, &input, log),
        Kind::Copy => copy_or_move(step, &input, false, log),
        Kind::Move => copy_or_move(step, &input, true, log),
        Kind::Transform => transform(step, &input),
        Kind::Combine => Ok(Value::Text(combine(step, &input))),
        Kind::Resize => resize(step, &input, log),
        Kind::Convert => convert(step, &input, log),
        Kind::RunScript => run_script(step, &input),
    }
}

/// The files a step works on, or the reason it cannot run without them.
fn need_files(input: &Value, action: &str) -> Result<Vec<PathBuf>, String> {
    let files = input.paths();
    if files.is_empty() {
        return Err(format!("{action} needs files from the step before it"));
    }
    Ok(files)
}

/// A folder named in a field: a leading `~/` is the home folder.
fn folder(text: &str) -> PathBuf {
    let text = text.trim();
    match text.strip_prefix("~/") {
        Some(rest) => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(rest),
            Err(_) => PathBuf::from(text),
        },
        None => PathBuf::from(text),
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn stem_of(path: &Path) -> String {
    path.file_stem().map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned())
}

/// Whether a file name matches a Find pattern: empty matches everything, `*.ext` matches an extension, and any other
/// text must appear in the name. The comparison ignores case.
pub fn name_matches(name: &str, pattern: &str) -> bool {
    let name = name.to_lowercase();
    let pattern = pattern.trim().to_lowercase();
    if pattern.is_empty() {
        return true;
    }
    match pattern.strip_prefix('*') {
        Some(suffix) if suffix.starts_with('.') => name.ends_with(suffix),
        _ => name.contains(&pattern),
    }
}

fn find(step: &Step) -> Result<Value, String> {
    let root = folder(step.value(0));
    if root.as_os_str().is_empty() {
        return Err("choose a folder to search in".into());
    }
    let recursive = step.value(2) == "yes";
    let mut found = Vec::new();
    collect(&root, step.value(1), recursive, &mut found)?;
    found.sort();
    Ok(Value::Paths(found))
}

fn collect(dir: &Path, pattern: &str, recursive: bool, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        // the file type does not follow links, so a link back up the tree cannot loop
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if recursive {
                collect(&path, pattern, true, out)?;
            }
        } else if name_matches(&file_name_of(&path), pattern) {
            out.push(path);
        }
    }
    Ok(())
}

/// The name a file gets from a Rename template. `{name}` is the file's name without its extension, `{ext}` its
/// extension and `{n}` its position in the list (from 1). A template without `{ext}` keeps the extension.
pub fn new_name(template: &str, path: &Path, n: usize) -> String {
    let stem = stem_of(path);
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    let mut name = template.replace("{name}", &stem).replace("{n}", &n.to_string());
    match &ext {
        Some(ext) if template.contains("{ext}") => name = name.replace("{ext}", ext),
        Some(ext) => name = format!("{name}.{ext}"),
        None => name = name.replace("{ext}", ""),
    }
    name
}

fn valid_file_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(format!("'{name}' is not a valid file name"));
    }
    Ok(())
}

fn rename(step: &Step, input: &Value, log: &mut dyn FnMut(LogLine)) -> Result<Value, String> {
    let files = need_files(input, "Rename")?;
    let template = step.value(0).trim();
    if template.is_empty() {
        return Err("the name template is empty".into());
    }
    let mut out = Vec::with_capacity(files.len());
    for (i, path) in files.iter().enumerate() {
        let name = new_name(template, path, i + 1);
        valid_file_name(&name)?;
        let target = path.with_file_name(&name);
        if target == *path {
            out.push(target);
            continue;
        }
        if target.exists() {
            return Err(format!("{} already exists", target.display()));
        }
        fs::rename(path, &target).map_err(|e| format!("{}: {e}", path.display()))?;
        log(LogLine::info(format!("{} to {}", file_name_of(path), name)));
        out.push(target);
    }
    Ok(Value::Paths(out))
}

fn copy_or_move(step: &Step, input: &Value, mv: bool, log: &mut dyn FnMut(LogLine)) -> Result<Value, String> {
    let action = if mv { "Move" } else { "Copy" };
    let files = need_files(input, action)?;
    let dest = folder(step.value(0));
    if dest.as_os_str().is_empty() {
        return Err(format!("choose a destination folder for {action}"));
    }
    fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut out = Vec::with_capacity(files.len());
    for path in &files {
        if !path.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        let name = path.file_name().ok_or_else(|| format!("{} has no file name", path.display()))?;
        let target = dest.join(name);
        if target == *path {
            out.push(target);
            continue;
        }
        if target.exists() {
            return Err(format!("{} already exists", target.display()));
        }
        if mv {
            move_file(path, &target)?;
        } else {
            fs::copy(path, &target).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let verb = if mv { "Moved" } else { "Copied" };
        log(LogLine::info(format!("{verb} {} to {}", file_name_of(path), dest.display())));
        out.push(target);
    }
    Ok(Value::Paths(out))
}

/// Renames a file, or copies and then removes it when the rename cannot cross to another disk.
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).map_err(|e| format!("{}: {e}", from.display()))?;
    fs::remove_file(from).map_err(|e| format!("{}: {e}", from.display()))
}

/// Text lines as the Transform step sees them: the input's lines, with an operation applied.
fn transform(step: &Step, input: &Value) -> Result<Value, String> {
    let lines = input.lines();
    let text: Vec<String> = match step.value(0) {
        "upper" => lines.iter().map(|l| l.to_uppercase()).collect(),
        "lower" => lines.iter().map(|l| l.to_lowercase()).collect(),
        "trim" => lines.iter().map(|l| l.trim().to_string()).collect(),
        "sort lines" => {
            let mut sorted = lines;
            sorted.sort();
            sorted
        }
        "unique lines" => {
            let mut seen = std::collections::HashSet::new();
            lines.into_iter().filter(|l| seen.insert(l.clone())).collect()
        }
        "replace" => {
            let find = step.value(1);
            if find.is_empty() {
                return Err("the Replace operation needs text to find".into());
            }
            lines.iter().map(|l| l.replace(find, step.value(2))).collect()
        }
        other => return Err(format!("unknown operation {other}")),
    };
    Ok(Value::Text(text.join("\n")))
}

/// Reads the separator field: `\n` and `\t` stand for a newline and a tab.
fn separator(field: &str) -> String {
    field.replace("\\n", "\n").replace("\\t", "\t")
}

fn combine(step: &Step, input: &Value) -> String {
    input.lines().join(&separator(step.value(0)))
}

/// The size a picture gets when a width or a height of 0 means "keep the proportions".
pub fn fit(src_w: usize, src_h: usize, w: u32, h: u32) -> (usize, usize) {
    let (sw, sh) = (src_w.max(1) as f64, src_h.max(1) as f64);
    match (w, h) {
        (0, 0) => (src_w, src_h),
        (0, h) => (((sw * h as f64 / sh).round() as usize).max(1), h as usize),
        (w, 0) => (w as usize, ((sh * w as f64 / sw).round() as usize).max(1)),
        (w, h) => (w as usize, h as usize),
    }
}

fn parse_size(text: &str, label: &str) -> Result<u32, String> {
    let value: u32 = text.trim().parse().map_err(|_| format!("{label} must be a whole number of pixels"))?;
    if value > MAX_SIDE {
        return Err(format!("{label} is larger than {MAX_SIDE} pixels"));
    }
    Ok(value)
}

/// The file beside `path` named `stem.ext`. Fails when that file already exists.
fn beside(path: &Path, stem: &str, ext: &str) -> Result<PathBuf, String> {
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let target = dir.join(format!("{stem}.{ext}"));
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    Ok(target)
}

fn resize(step: &Step, input: &Value, log: &mut dyn FnMut(LogLine)) -> Result<Value, String> {
    let files = need_files(input, "Resize")?;
    let w = parse_size(step.value(0), "Width")?;
    let h = parse_size(step.value(1), "Height")?;
    if w == 0 && h == 0 {
        return Err("set a width or a height to resize to".into());
    }
    let mut out = Vec::with_capacity(files.len());
    for path in &files {
        let image = imageio::decode_file(path)?;
        let (tw, th) = fit(image.w, image.h, w, h);
        if tw as u32 > MAX_SIDE || th as u32 > MAX_SIDE {
            return Err(format!("{} would be larger than {MAX_SIDE} pixels", path.display()));
        }
        let scaled = image.resized(tw, th);
        let target = beside(path, &format!("{}-{tw}x{th}", stem_of(path)), Format::Png.extension())?;
        fs::write(&target, imageio::encode(&scaled, Format::Png)?).map_err(|e| format!("{}: {e}", target.display()))?;
        log(LogLine::info(format!("{} at {tw} x {th}", file_name_of(&target))));
        out.push(target);
    }
    Ok(Value::Paths(out))
}

fn convert(step: &Step, input: &Value, log: &mut dyn FnMut(LogLine)) -> Result<Value, String> {
    let files = need_files(input, "Convert")?;
    let format = Format::from_name(step.value(0)).ok_or_else(|| format!("unknown image format {}", step.value(0)))?;
    let mut out = Vec::with_capacity(files.len());
    for path in &files {
        let already = path.extension().is_some_and(|e| e.eq_ignore_ascii_case(format.extension()));
        if already {
            log(LogLine::info(format!("{} is already {}", file_name_of(path), format.extension())));
            out.push(path.clone());
            continue;
        }
        let image = imageio::decode_file(path)?;
        let target = beside(path, &stem_of(path), format.extension())?;
        fs::write(&target, imageio::encode(&image, format)?).map_err(|e| format!("{}: {e}", target.display()))?;
        log(LogLine::info(format!("{} as {}", file_name_of(&target), format.extension())));
        out.push(target);
    }
    Ok(Value::Paths(out))
}

/// Runs the script with `sh -c`. The input goes to its standard input as lines; its standard output is the step's
/// output. A script that exits with an error stops the run, with its error output in the message.
fn run_script(step: &Step, input: &Value) -> Result<Value, String> {
    let script = step.value(0).trim();
    if script.is_empty() {
        return Err("the script is empty".into());
    }
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the shell: {e}"))?;
    // the input is written on its own thread, so a script that writes a lot before it reads cannot block the run
    let input_text = input.text();
    let feeder = child.stdin.take().map(|mut stdin| {
        DispatchQueue::global(Qos::Utility).async_(move || {
            let _ = stdin.write_all(input_text.as_bytes());
        })
    });
    let output = child.wait_with_output().map_err(|e| format!("the script did not finish: {e}"))?;
    if let Some(feeder) = feeder {
        let _ = feeder.join();
    }
    if !output.status.success() {
        let code = output.status.code().map_or_else(|| "a signal".to_string(), |c| c.to_string());
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if error.is_empty() {
            format!("the script exited with {code}")
        } else {
            format!("the script exited with {code}: {error}")
        });
    }
    Ok(Value::Text(String::from_utf8_lossy(&output.stdout).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clean folder for one test, under the crate's target directory (never the system temp folder).
    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/automator-tests")).join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn step(kind: Kind, values: &[&str]) -> Step {
        let mut s = Step::new(kind, "/");
        for (i, v) in values.iter().enumerate() {
            s.values[i] = v.to_string();
        }
        s
    }

    fn quiet(_: LogLine) {}

    #[test]
    fn name_patterns_match() {
        assert!(name_matches("Photo.PNG", "*.png"));
        assert!(!name_matches("photo.jpg", "*.png"));
        assert!(name_matches("holiday-2026.jpg", "2026"));
        assert!(name_matches("anything", ""));
        assert!(!name_matches("notes.txt", "photo"));
    }

    #[test]
    fn rename_template_fills_name_counter_and_keeps_extension() {
        let path = Path::new("/x/photo.jpg");
        assert_eq!(new_name("{name}-{n}", path, 3), "photo-3.jpg");
        assert_eq!(new_name("shot-{n}.{ext}", path, 1), "shot-1.jpg");
        assert_eq!(new_name("{name}", Path::new("/x/README"), 1), "README");
    }

    #[test]
    fn find_filters_by_pattern_and_depth() {
        let root = scratch("find");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.png"), b"x").unwrap();
        fs::write(root.join("b.txt"), b"x").unwrap();
        fs::write(root.join("sub/c.png"), b"x").unwrap();
        let folder_text = root.display().to_string();
        let flat = run(&step(Kind::Find, &[&folder_text, "*.png", "no"]), Value::Empty, &mut quiet).unwrap();
        assert_eq!(flat, Value::Paths(vec![root.join("a.png")]));
        let deep = run(&step(Kind::Find, &[&folder_text, "*.png", "yes"]), Value::Empty, &mut quiet).unwrap();
        assert_eq!(deep, Value::Paths(vec![root.join("a.png"), root.join("sub/c.png")]));
    }

    #[test]
    fn find_reports_a_missing_folder() {
        let result = run(&step(Kind::Find, &["/no/such/automator/folder", "", "no"]), Value::Empty, &mut quiet);
        assert!(result.unwrap_err().contains("/no/such/automator/folder"));
    }

    #[test]
    fn rename_moves_files_and_refuses_to_overwrite() {
        let root = scratch("rename");
        fs::write(root.join("first.jpg"), b"1").unwrap();
        fs::write(root.join("second.jpg"), b"2").unwrap();
        let input = Value::Paths(vec![root.join("first.jpg"), root.join("second.jpg")]);
        let renamed = run(&step(Kind::Rename, &["holiday-{n}"]), input.clone(), &mut quiet).unwrap();
        assert_eq!(renamed, Value::Paths(vec![root.join("holiday-1.jpg"), root.join("holiday-2.jpg")]));
        assert!(root.join("holiday-2.jpg").exists());
        assert!(!root.join("first.jpg").exists());
        // a second run against a target that exists is refused and nothing is replaced
        fs::write(root.join("again.jpg"), b"3").unwrap();
        let clash = run(&step(Kind::Rename, &["again"]), Value::Paths(vec![root.join("holiday-1.jpg")]), &mut quiet);
        assert!(clash.unwrap_err().contains("already exists"));
        assert_eq!(fs::read(root.join("again.jpg")).unwrap(), b"3");
    }

    #[test]
    fn rename_needs_files() {
        let result = run(&step(Kind::Rename, &["x-{n}"]), Value::Empty, &mut quiet);
        assert_eq!(result.unwrap_err(), "Rename needs files from the step before it");
    }

    #[test]
    fn copy_then_move_into_a_new_folder() {
        let root = scratch("copy-move");
        let dest = root.join("out");
        fs::write(root.join("note.txt"), b"hi").unwrap();
        let input = Value::Paths(vec![root.join("note.txt")]);
        let dest_text = dest.display().to_string();
        let copied = run(&step(Kind::Copy, &[&dest_text]), input, &mut quiet).unwrap();
        assert_eq!(copied, Value::Paths(vec![dest.join("note.txt")]));
        assert!(root.join("note.txt").exists());
        assert_eq!(fs::read(dest.join("note.txt")).unwrap(), b"hi");

        let moved_dest = root.join("moved");
        let moved_text = moved_dest.display().to_string();
        let moved = run(&step(Kind::Move, &[&moved_text]), copied, &mut quiet).unwrap();
        assert_eq!(moved, Value::Paths(vec![moved_dest.join("note.txt")]));
        assert!(!dest.join("note.txt").exists());
        assert_eq!(fs::read(moved_dest.join("note.txt")).unwrap(), b"hi");
    }

    #[test]
    fn copy_needs_a_destination() {
        let result = run(&step(Kind::Copy, &[""]), Value::Paths(vec![PathBuf::from("/x/a")]), &mut quiet);
        assert!(result.unwrap_err().contains("destination"));
    }

    #[test]
    fn transform_operations() {
        let input = Value::Text("b\na\nb\n  c  ".into());
        let t = |op: &str| run(&step(Kind::Transform, &[op]), input.clone(), &mut quiet).unwrap().text();
        assert_eq!(t("upper"), "B\nA\nB\n  C  ");
        assert_eq!(t("trim"), "b\na\nb\nc");
        assert_eq!(t("sort lines"), "  c  \na\nb\nb");
        assert_eq!(t("unique lines"), "b\na\n  c  ");
        let replaced = run(&step(Kind::Transform, &["replace", "b", "beta"]), input, &mut quiet).unwrap();
        assert_eq!(replaced.text(), "beta\na\nbeta\n  c  ");
    }

    #[test]
    fn transform_replace_needs_text_to_find() {
        let result = run(&step(Kind::Transform, &["replace", "", "x"]), Value::Text("a".into()), &mut quiet);
        assert!(result.is_err());
    }

    #[test]
    fn combine_joins_lines_with_the_separator() {
        let input = Value::Text("one\ntwo\nthree".into());
        let joined = run(&step(Kind::Combine, &[", "]), input.clone(), &mut quiet).unwrap();
        assert_eq!(joined, Value::Text("one, two, three".into()));
        let newlines = run(&step(Kind::Combine, &["\\n"]), input, &mut quiet).unwrap();
        assert_eq!(newlines, Value::Text("one\ntwo\nthree".into()));
    }

    #[test]
    fn fit_keeps_proportions_when_one_side_is_zero() {
        assert_eq!(fit(400, 200, 200, 0), (200, 100));
        assert_eq!(fit(400, 200, 0, 50), (100, 50));
        assert_eq!(fit(400, 200, 100, 100), (100, 100));
        assert_eq!(fit(400, 200, 0, 0), (400, 200));
        assert_eq!(fit(3, 1000, 0, 1), (1, 1));
    }

    #[test]
    fn parse_size_checks_range() {
        assert_eq!(parse_size(" 640 ", "Width"), Ok(640));
        assert!(parse_size("wide", "Width").unwrap_err().contains("Width"));
        assert!(parse_size("20000", "Height").is_err());
    }

    #[test]
    fn resize_reports_a_file_that_is_not_a_picture() {
        let root = scratch("resize-bad");
        let bad = root.join("note.png");
        fs::write(&bad, b"definitely not a picture").unwrap();
        let result = run(&step(Kind::Resize, &["10", "0"]), Value::Paths(vec![bad]), &mut quiet);
        assert!(result.is_err());
    }

    #[test]
    fn script_reads_input_and_its_output_is_the_result() {
        let out = run(&step(Kind::RunScript, &["tr a-z A-Z"]), Value::Text("abc".into()), &mut quiet).unwrap();
        assert_eq!(out, Value::Text("ABC".into()));
    }

    #[test]
    fn script_failure_is_an_error_with_its_message() {
        let result = run(&step(Kind::RunScript, &["echo broken >&2; exit 3"]), Value::Empty, &mut quiet);
        let error = result.unwrap_err();
        assert!(error.contains("exited with 3"), "{error}");
        assert!(error.contains("broken"), "{error}");
    }

    #[test]
    fn empty_script_is_refused() {
        assert!(run(&step(Kind::RunScript, &["  "]), Value::Empty, &mut quiet).is_err());
    }
}
