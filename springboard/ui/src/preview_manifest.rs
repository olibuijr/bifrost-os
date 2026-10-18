//! App-declared previews from the app manifest (`[[preview]]` blocks), and the wait for an app's generator.
//!
//! A block declares that the app can draw a preview of some file types:
//!
//! ```toml
//! [[preview]]
//! id = "org.example.model"
//! types = ["org.example.model-file"]
//! generator = "preview.render"
//! timeout_ms = 3000
//! ```
//!
//! `id` names the provider and must be unique. `types` are type identifiers (see [`crate::uttype`]) that go to
//! `Registry::register`. `generator` is the control method the shell calls on the app's control socket with
//! `{"file": "<absolute path>", "width": w, "height": h}`; the app answers `{"picture": "<absolute path to a PNG>"}`.
//! `timeout_ms` is optional (default [`DEFAULT_TIMEOUT_MS`], clamped to [`MAX_TIMEOUT_MS`]).
//!
//! The shell does not wait forever. [`Wait`] records the deadline when the request goes out: an answer in time is
//! used, and a late, missing or unusable answer falls back to the generic preview ([`Outcome::Generic`]). The first
//! decision is final, so a second answer cannot replace it.

use crate::uttype_declarations::quoted;

/// The generator wait when a block gives no `timeout_ms`.
pub const DEFAULT_TIMEOUT_MS: u32 = 3_000;
/// Longest wait a block may ask for; larger values are clamped.
pub const MAX_TIMEOUT_MS: u32 = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewDecl {
    pub id: String,
    pub types: Vec<String>,
    pub generator: String,
    pub timeout_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// A line that is not a table header, a comment or `key = value`.
    Syntax(usize),
    UnknownKey { line: usize, key: String },
    /// A `[[preview]]` block without `id`, `types` or `generator` (the line of the block header).
    MissingKey { line: usize, key: &'static str },
    /// `timeout_ms` is not a whole number of milliseconds.
    BadTimeout(usize),
    /// Two blocks share an `id`.
    DuplicateId { line: usize, id: String },
}

/// The effective wait for a block's `timeout_ms`: the default when absent, never below 1 ms or above the maximum.
pub fn timeout_ms(requested: Option<u32>) -> u32 {
    requested.unwrap_or(DEFAULT_TIMEOUT_MS).clamp(1, MAX_TIMEOUT_MS)
}

/// One `[[preview]]` block while its keys are read; checked when the block ends.
struct Block {
    line: usize,
    id: Option<String>,
    types: Option<Vec<String>>,
    generator: Option<String>,
    timeout_ms: Option<u32>,
}

/// Parses the `[[preview]]` blocks of an app manifest. Other tables are skipped, and their keys too.
pub fn parse_manifest(text: &str) -> Result<Vec<PreviewDecl>, ManifestError> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut in_block = false;
    for (n, raw) in text.lines().enumerate() {
        let line = n + 1;
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with('[') {
            in_block = t.starts_with("[[") && t.trim_start_matches('[').split(']').next().map(str::trim) == Some("preview");
            if in_block {
                blocks.push(Block { line, id: None, types: None, generator: None, timeout_ms: None });
            }
            continue;
        }
        if !in_block {
            continue;
        }
        let (key, value) = t.split_once('=').ok_or(ManifestError::Syntax(line))?;
        let b = blocks.last_mut().ok_or(ManifestError::Syntax(line))?;
        let first = || -> Result<String, ManifestError> {
            quoted(value).and_then(|v| v.into_iter().next()).ok_or(ManifestError::Syntax(line))
        };
        match key.trim() {
            "id" => b.id = Some(first()?),
            "generator" => b.generator = Some(first()?),
            "types" => b.types = Some(quoted(value).ok_or(ManifestError::Syntax(line))?),
            "timeout_ms" => {
                let v = value.split('#').next().unwrap_or("").trim();
                b.timeout_ms = Some(v.parse::<u32>().map_err(|_| ManifestError::BadTimeout(line))?);
            }
            other => return Err(ManifestError::UnknownKey { line, key: other.into() }),
        }
    }

    let mut out: Vec<PreviewDecl> = Vec::new();
    for b in blocks {
        let line = b.line;
        let id = b.id.ok_or(ManifestError::MissingKey { line, key: "id" })?;
        let types = b.types.filter(|t| !t.is_empty()).ok_or(ManifestError::MissingKey { line, key: "types" })?;
        let generator = b.generator.ok_or(ManifestError::MissingKey { line, key: "generator" })?;
        if out.iter().any(|d| d.id == id) {
            return Err(ManifestError::DuplicateId { line, id });
        }
        out.push(PreviewDecl { id, types, generator, timeout_ms: timeout_ms(b.timeout_ms) });
    }
    Ok(out)
}

/// How a preview request ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// the app's picture, an absolute path
    Generated(String),
    /// no usable answer in time: the generic preview (icon and info) is shown instead
    Generic,
}

/// A generator request in flight. Created when the request is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wait {
    deadline_ms: u64,
    outcome: Option<Outcome>,
}

impl Wait {
    /// Starts waiting at `now_ms` for `timeout_ms` (already clamped by [`timeout_ms`]).
    pub fn start(now_ms: u64, timeout_ms: u32) -> Self {
        Self { deadline_ms: now_ms.saturating_add(u64::from(timeout_ms)), outcome: None }
    }

    /// The app's answer at `now_ms`: `picture` is the path it returned, or None for an error. Returns true when the
    /// answer was used. An answer after the deadline, a second answer, and a path that is not absolute all fall back.
    pub fn answer(&mut self, now_ms: u64, picture: Option<&str>) -> bool {
        if self.outcome.is_some() {
            return false;
        }
        if now_ms >= self.deadline_ms {
            self.outcome = Some(Outcome::Generic);
            return false;
        }
        match picture.filter(|p| p.starts_with('/') && !p.contains('\0')) {
            Some(p) => {
                self.outcome = Some(Outcome::Generated(p.to_string()));
                true
            }
            None => {
                self.outcome = Some(Outcome::Generic);
                false
            }
        }
    }

    /// The outcome at `now_ms`, deciding the fallback once the deadline has passed with no answer.
    pub fn poll(&mut self, now_ms: u64) -> Option<&Outcome> {
        if self.outcome.is_none() && now_ms >= self.deadline_ms {
            self.outcome = Some(Outcome::Generic);
        }
        self.outcome.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "[[preview]]\nid = \"org.example.model\"\ntypes = [\"org.example.model-file\", \"org.example.mesh\"]\ngenerator = \"preview.render\"\ntimeout_ms = 1500\n";

    #[test]
    fn a_preview_block_parses_its_keys() {
        let d = parse_manifest(ONE).unwrap();
        assert_eq!(
            d,
            vec![PreviewDecl {
                id: "org.example.model".into(),
                types: vec!["org.example.model-file".into(), "org.example.mesh".into()],
                generator: "preview.render".into(),
                timeout_ms: 1500,
            }]
        );
    }

    #[test]
    fn other_tables_and_their_keys_are_skipped() {
        let text = format!("[[exported_types]]\nid = \"a.b\"\nconforms_to = [\"public.data\"]\n\n[package]\nname = \"x\"\n\n{ONE}\n[[imported_types]]\nid = \"c.d\"\n");
        let d = parse_manifest(&text).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].id, "org.example.model");
    }

    #[test]
    fn the_timeout_defaults_and_is_clamped() {
        let text = "[[preview]]\nid = \"a\"\ntypes = [\"x.y\"]\ngenerator = \"g\"\n";
        assert_eq!(parse_manifest(text).unwrap()[0].timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(timeout_ms(None), DEFAULT_TIMEOUT_MS);
        assert_eq!(timeout_ms(Some(0)), 1, "never below one millisecond");
        assert_eq!(timeout_ms(Some(u32::MAX)), MAX_TIMEOUT_MS);
        assert_eq!(timeout_ms(Some(250)), 250);
    }

    #[test]
    fn a_comment_after_the_timeout_is_ignored() {
        let text = "[[preview]]\nid = \"a\"\ntypes = [\"x.y\"]\ngenerator = \"g\"\ntimeout_ms = 900 # ms\n";
        assert_eq!(parse_manifest(text).unwrap()[0].timeout_ms, 900);
    }

    #[test]
    fn a_block_without_its_required_keys_is_refused() {
        let no_types = "[[preview]]\nid = \"a\"\ngenerator = \"g\"\n";
        assert_eq!(parse_manifest(no_types), Err(ManifestError::MissingKey { line: 1, key: "types" }));
        let no_gen = "[[preview]]\nid = \"a\"\ntypes = [\"x.y\"]\n";
        assert_eq!(parse_manifest(no_gen), Err(ManifestError::MissingKey { line: 1, key: "generator" }));
        let no_id = "[[preview]]\ntypes = [\"x.y\"]\ngenerator = \"g\"\n";
        assert_eq!(parse_manifest(no_id), Err(ManifestError::MissingKey { line: 1, key: "id" }));
    }

    #[test]
    fn unknown_keys_bad_timeouts_and_duplicate_ids_are_refused() {
        let bogus = "[[preview]]\nid = \"a\"\nbogus = \"x\"\n";
        assert_eq!(parse_manifest(bogus), Err(ManifestError::UnknownKey { line: 3, key: "bogus".into() }));
        let slow = "[[preview]]\nid = \"a\"\ntypes = [\"x.y\"]\ngenerator = \"g\"\ntimeout_ms = soon\n";
        assert_eq!(parse_manifest(slow), Err(ManifestError::BadTimeout(5)));
        let twice = format!("{ONE}\n{ONE}");
        assert_eq!(parse_manifest(&twice), Err(ManifestError::DuplicateId { line: 7, id: "org.example.model".into() }));
    }

    #[test]
    fn a_key_outside_a_block_or_an_open_quote_is_a_syntax_error() {
        assert_eq!(parse_manifest("id = \"a\"\n").unwrap(), vec![], "keys outside a block belong to other tables");
        let open = "[[preview]]\nid = \"a\n";
        assert_eq!(parse_manifest(open), Err(ManifestError::Syntax(2)));
    }

    #[test]
    fn an_answer_in_time_is_used() {
        let mut w = Wait::start(1000, 500);
        assert_eq!(w.poll(1200), None, "still waiting inside the timeout");
        assert!(w.answer(1400, Some("/tmp/out.png")));
        assert_eq!(w.poll(9999), Some(&Outcome::Generated("/tmp/out.png".into())));
    }

    #[test]
    fn a_timeout_without_an_answer_falls_back_to_the_generic_preview() {
        let mut w = Wait::start(1000, 500);
        assert_eq!(w.poll(1500), Some(&Outcome::Generic), "the deadline decides the fallback");
        assert!(!w.answer(1200, Some("/tmp/late.png")), "a late answer cannot replace the fallback");
        assert_eq!(w.poll(1600), Some(&Outcome::Generic));
    }

    #[test]
    fn a_late_answer_is_refused_even_before_anyone_polls() {
        let mut w = Wait::start(0, 100);
        assert!(!w.answer(100, Some("/tmp/late.png")));
        assert_eq!(w.poll(101), Some(&Outcome::Generic));
    }

    #[test]
    fn the_first_decision_is_final() {
        let mut w = Wait::start(0, 100);
        assert!(w.answer(10, Some("/tmp/first.png")));
        assert!(!w.answer(20, Some("/tmp/second.png")));
        assert_eq!(w.poll(30), Some(&Outcome::Generated("/tmp/first.png".into())));
    }

    #[test]
    fn an_error_or_a_relative_path_falls_back_at_once() {
        let mut w = Wait::start(0, 100);
        assert!(!w.answer(5, None), "the app reported an error");
        assert_eq!(w.poll(6), Some(&Outcome::Generic));
        let mut r = Wait::start(0, 100);
        assert!(!r.answer(5, Some("relative.png")));
        assert_eq!(r.poll(6), Some(&Outcome::Generic));
    }
}
