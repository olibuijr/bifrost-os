//! Safety and quality of the chat's answers. Three parts, all wired into the window:
//!
//! - A guardrail screens every answer before it is shown or saved: it cuts an answer over [`MAX_ANSWER_CHARS`] and
//!   masks the phrases listed in `guardrail.txt` in the app's data folder (one per line, `#` for a comment).
//! - A text classifier routes each request to a skill. A skill is a system prompt, so a request about code is
//!   answered with the code instructions. A request with no skill word goes out with no system prompt.
//! - Diagnostics scores the answers of the open chat: rule checks always, and a model judge when there is an API key.
//!   The judge asks the same model to rate each answer on two dimensions, and each reply is read by the evaluation.
//!
//! Everything here is pure except [`judge_call`] and [`run_evaluation`], which make requests through `api::stream`
//! and are meant to run on a background thread.

use crate::api::{self, Event};
use crate::model::Message;
use bifrost_app::controls::evaluation::{model_judge, Evaluation, EvaluationResult, Evaluator, MetricsAggregator, ModelJudgePrompt, Sample, ScoreDimension};
use bifrost_app::controls::guardrail::Guardrail;
use bifrost_app::controls::text_classifier::TextClassifier;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

/// The longest answer shown or saved, in characters. A longer answer is cut and the cut is said.
pub const MAX_ANSWER_CHARS: usize = 20_000;
/// What a masked phrase becomes.
pub const MASK: &str = "[removed]";
/// The file in the app's data folder that lists blocked phrases.
pub const BLOCKED_FILE: &str = "guardrail.txt";
/// Diagnostics scores at most this many of the newest exchanges of the open chat.
pub const MAX_EXCHANGES: usize = 10;

// Guardrail -------------------------------------------------------------------------------------------------------

/// The guardrail from the phrases in `text`, one per line. A line starting with `#` is a comment, and blank lines
/// are skipped.
pub fn guardrail_from_text(text: &str) -> Guardrail {
    let mut g = Guardrail::new(MAX_ANSWER_CHARS);
    for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
        // A blank line is refused by `block`, which is what skips it.
        let _ = g.block(line);
    }
    g
}

/// The guardrail for the data folder: `guardrail.txt` if it exists, else no blocked phrases (the length limit still
/// applies).
pub fn load_guardrail(dir: &Path) -> Guardrail {
    guardrail_from_text(&std::fs::read_to_string(dir.join(BLOCKED_FILE)).unwrap_or_default())
}

/// The first `MAX_ANSWER_CHARS` characters of `raw`, and whether the text was longer.
fn capped(raw: &str) -> (&str, bool) {
    match raw.char_indices().nth(MAX_ANSWER_CHARS) {
        Some((i, _)) => (&raw[..i], true),
        None => (raw, false),
    }
}

/// The text of an answer that is still streaming. A blocked phrase is masked once it is whole. The tail that could
/// still become a blocked phrase is held back, so a phrase is never shown in part: a tail is held back when it is
/// the start of a blocked phrase, and it stays hidden until the answer ends.
pub fn screen_partial(g: &Guardrail, raw: &str) -> String {
    let (head, _) = capped(raw);
    let len = head.len();
    let hold = g.phrases().iter().map(String::len).max().unwrap_or(0);
    let lower = head.to_ascii_lowercase();
    let cut = (len.saturating_sub(hold)..len)
        .find(|&s| head.is_char_boundary(s) && g.phrases().iter().any(|p| p.starts_with(&lower[s..])))
        .unwrap_or(len);
    g.redact(&head[..cut], MASK)
}

/// The text of a finished answer: every blocked phrase is masked, and an answer over the limit is cut with a note.
pub fn screen_final(g: &Guardrail, raw: &str) -> String {
    let (head, cut) = capped(raw);
    let mut out = g.redact(head, MASK);
    if cut {
        out.push_str(&format!("\n\n[The answer was cut at {MAX_ANSWER_CHARS} of {} characters.]", raw.chars().count()));
    }
    out
}

// Skill routing ---------------------------------------------------------------------------------------------------

/// The skills a request can be routed to. Each label has the words that route to it.
pub const LABELS: [&str; 4] = ["Code", "Writing", "Summary", "Translation"];
const KEYWORDS: [&[&str]; 4] = [
    &["code", "function", "bug", "compile", "rust", "python", "program", "implement", "debug", "error", "script"],
    &["write", "email", "letter", "essay", "story", "poem", "draft", "rewrite", "paragraph"],
    &["summarize", "summary", "tldr", "shorten", "bullet", "brief"],
    &["translate", "translation", "icelandic", "english", "danish", "german"],
];

/// The classifier that routes requests. Each keyword scores one for its own label and nothing for the others, so the
/// label with the most keywords in the request wins. `None` only if the table itself were invalid.
pub fn router() -> Option<TextClassifier> {
    let vocabulary: Vec<&str> = KEYWORDS.iter().flat_map(|words| words.iter().copied()).collect();
    let n = LABELS.len();
    let mut weights = vec![0.0_f32; vocabulary.len() * n];
    for (label, words) in KEYWORDS.iter().enumerate() {
        for (w, word) in vocabulary.iter().enumerate() {
            if words.contains(word) {
                weights[w * n + label] = 1.0;
            }
        }
    }
    TextClassifier::new(&vocabulary, &LABELS, weights, vec![0.0; n]).ok()
}

/// The skill label for a request, or `None` when no skill word is in it.
pub fn route(router: Option<&TextClassifier>, text: &str) -> Option<&'static str> {
    let label = router?.predicted_label(text)?;
    LABELS.iter().copied().find(|l| *l == label)
}

/// The system prompt of a skill label.
pub fn skill_prompt(label: &str) -> Option<&'static str> {
    match label {
        "Code" => Some("You are a careful programming assistant. Give working code in fenced blocks and explain the fix in a few sentences."),
        "Writing" => Some("You are a writing assistant. Keep the user's voice and tone, and give the finished text first."),
        "Summary" => Some("Summarize the material in a few short bullet points, the most important first."),
        "Translation" => Some("Translate the user's text faithfully. Return only the translation unless the user asks for an explanation."),
        _ => None,
    }
}

// Diagnostics -----------------------------------------------------------------------------------------------------

/// The question and answer of each of the newest exchanges (at most [`MAX_EXCHANGES`]). A question with no answer yet
/// is left out.
pub fn exchanges(messages: &[Message]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut pending: Option<&str> = None;
    for m in messages {
        if m.user {
            pending = Some(m.text.as_str());
        } else if let Some(question) = pending.take() {
            if !m.text.trim().is_empty() {
                pairs.push((question.to_string(), m.text.clone()));
            }
        }
    }
    let skip = pairs.len().saturating_sub(MAX_EXCHANGES);
    pairs.split_off(skip)
}

/// The rubric the model judge scores on, each from 1 to 5.
pub const JUDGE_DIMENSIONS: [(&str, f64, f64, f64); 2] = [("Helpfulness", 1.0, 1.0, 5.0), ("Clarity", 1.0, 1.0, 5.0)];
pub const JUDGE_TEMPLATE: &str = "Rate the response to the message below on {dimension}, from {min} (poor) to {max} (excellent).\n\nMessage:\n{input}\n\nResponse:\n{response}\n\nReply with one line only, in the form: {dimension}: <number>";

/// One request to the model with the judge prompt; the reply text, or the error that stopped it.
pub fn judge_call(key: &str, model: &str, prompt: &str) -> Result<String, String> {
    let reply = Mutex::new(String::new());
    let failure: Mutex<Option<String>> = Mutex::new(None);
    let cancel = AtomicBool::new(false);
    let send = |ev: Event| -> bool {
        match ev {
            Event::Delta(d) => {
                if let Ok(mut r) = reply.lock() {
                    r.push_str(&d);
                }
                true
            }
            // The judge only reads the reply text; a tool call it made is not part of the rating.
            Event::Tool { .. } | Event::Done => true,
            Event::Error(e) => {
                if let Ok(mut f) = failure.lock() {
                    *f = Some(e);
                }
                false
            }
        }
    };
    api::stream(key, model, None, &[(true, prompt.to_string())], &cancel, &send);
    if let Some(e) = failure.into_inner().ok().flatten() {
        return Err(e);
    }
    Ok(reply.into_inner().unwrap_or_default())
}

/// Scores each answer of `pairs` (question, answer). The rule checks always run. The model judge runs too when `key`
/// is given, one request per answer and dimension. A judge that fails scores NaN, which the report leaves out.
/// Blocking: call it from a background thread.
pub fn run_evaluation(g: &Guardrail, key: Option<&str>, model: &str, pairs: &[(String, String)]) -> EvaluationResult {
    let mut evaluators = vec![
        Evaluator::check("Not Empty", |_, r| !r.trim().is_empty()),
        {
            let g = g.clone();
            Evaluator::check("Passes Guardrail", move |_, r| g.check(r).is_ok())
        },
    ];
    if let Some(key) = key {
        let dims: Vec<ScoreDimension> = JUDGE_DIMENSIONS.iter().filter_map(|(name, w, lo, hi)| ScoreDimension::new(*name, *w, *lo, *hi).ok()).collect();
        let (key, model) = (key.to_string(), model.to_string());
        if let Ok(judge) = model_judge("Model Judge", dims, ModelJudgePrompt::new(JUDGE_TEMPLATE), move |prompt: &str| judge_call(&key, &model, prompt)) {
            evaluators.push(judge);
        }
    }
    let evaluation = Evaluation::new("Answers", evaluators);
    let samples: Vec<Sample> = pairs.iter().map(|(question, _)| Sample::new(question.clone())).collect();
    let answers: Vec<String> = pairs.iter().map(|(_, answer)| answer.clone()).collect();
    // The responder hands out the stored answers in order: `run` asks for one response per sample, in sample order.
    let mut next = 0;
    evaluation.run(&samples, |_| {
        let answer = answers.get(next).cloned().ok_or_else(|| "no stored answer".to_string());
        next += 1;
        answer
    })
}

/// The Diagnostics lines for a finished run: each metric's mean, median and range over the answers that were scored,
/// then how many answers could not be run. A NaN score (a judge that failed) is counted as not scored.
pub fn report_lines(result: &EvaluationResult) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for row in result.rows() {
        for m in &row.metrics {
            if !names.contains(&m.name) {
                names.push(m.name.clone());
            }
        }
    }
    let mut lines = Vec::new();
    for name in names {
        let all = result.metric_values(&name);
        let scored: Vec<f64> = all.iter().copied().filter(|v| v.is_finite()).collect();
        let unscored = all.len() - scored.len();
        let note = if unscored > 0 { format!(" ({unscored} not scored)") } else { String::new() };
        match MetricsAggregator::summarize(&scored) {
            Some(s) => lines.push(format!(
                "{name}: mean {:.2}, median {:.2}, range {:.2} to {:.2} over {} answers{note}",
                s.mean, s.median, s.min, s.max, s.count
            )),
            None => lines.push(format!("{name}: not scored{note}")),
        }
    }
    let failed = result.errors().len();
    if failed > 0 {
        lines.push(format!("{failed} answers could not be run."));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(phrases: &[&str]) -> Guardrail {
        let mut g = Guardrail::new(MAX_ANSWER_CHARS);
        for p in phrases {
            g.block(p).unwrap();
        }
        g
    }

    #[test]
    fn the_blocked_file_skips_comments_and_blank_lines() {
        let g = guardrail_from_text("# comment\nSecret Code\n\n   \nlaunch codes\n");
        assert_eq!(g.phrases(), &["secret code".to_string(), "launch codes".to_string()]);
    }

    #[test]
    fn a_finished_answer_has_its_blocked_phrases_masked() {
        let g = guard(&["secret"]);
        assert_eq!(screen_final(&g, "The SECRET key."), "The [removed] key.");
        assert_eq!(screen_final(&g, "Nothing here."), "Nothing here.");
    }

    #[test]
    fn a_streaming_answer_never_shows_part_of_a_blocked_phrase() {
        let g = guard(&["secret"]);
        // "sec" could start "secret", so it is held back; "The " is safe.
        assert_eq!(screen_partial(&g, "The sec"), "The ");
        // Whole and masked only when the answer ends; while streaming the phrase is still held back.
        assert_eq!(screen_partial(&g, "The secret"), "The ");
        assert_eq!(screen_partial(&g, "The sex"), "The sex");
    }

    #[test]
    fn a_streaming_answer_without_phrases_is_shown_whole() {
        assert_eq!(screen_partial(&guard(&[]), "Hello there"), "Hello there");
    }

    #[test]
    fn an_answer_over_the_limit_is_cut_and_says_so() {
        let raw = "a".repeat(MAX_ANSWER_CHARS + 1);
        let out = screen_final(&guard(&[]), &raw);
        assert!(out.starts_with(&"a".repeat(MAX_ANSWER_CHARS)));
        assert!(out.ends_with("cut at 20000 of 20001 characters.]"), "got {out:?}");
    }

    #[test]
    fn requests_route_to_the_skill_with_the_most_keywords() {
        let r = router();
        assert_eq!(route(r.as_ref(), "fix this rust bug in my function"), Some("Code"));
        assert_eq!(route(r.as_ref(), "Please translate this to Icelandic"), Some("Translation"));
        assert_eq!(route(r.as_ref(), "write an email to my landlord"), Some("Writing"));
        assert_eq!(route(r.as_ref(), "summarize the key points"), Some("Summary"));
        assert_eq!(route(r.as_ref(), "hello there"), None);
        assert_eq!(route(None, "fix this bug"), None);
    }

    #[test]
    fn every_routed_label_has_a_prompt_and_general_has_none() {
        for label in LABELS {
            assert!(skill_prompt(label).is_some(), "{label}");
        }
        assert_eq!(skill_prompt("General"), None);
    }

    fn message(user: bool, text: &str) -> Message {
        Message { user, text: text.to_string(), tools: Vec::new() }
    }

    #[test]
    fn exchanges_pair_each_question_with_its_answer() {
        let messages = vec![message(true, "a"), message(false, "A"), message(true, "b"), message(false, ""), message(true, "c"), message(false, "C")];
        assert_eq!(exchanges(&messages), vec![("a".to_string(), "A".to_string()), ("c".to_string(), "C".to_string())]);
    }

    #[test]
    fn only_the_newest_exchanges_are_scored() {
        let mut messages = Vec::new();
        for i in 0..12 {
            messages.push(message(true, &format!("q{i}")));
            messages.push(message(false, &format!("a{i}")));
        }
        let pairs = exchanges(&messages);
        assert_eq!(pairs.len(), MAX_EXCHANGES);
        assert_eq!(pairs[0], ("q2".to_string(), "a2".to_string()));
    }

    #[test]
    fn rule_checks_score_each_answer_and_summarize_the_run() {
        let pairs = vec![("q1".to_string(), "hello".to_string()), ("q2".to_string(), String::new()), ("q3".to_string(), "x".to_string())];
        let result = run_evaluation(&guard(&[]), None, "model", &pairs);
        let lines = report_lines(&result);
        // "Not Empty" scores 1, 0, 1: mean 2/3, median 1, range 0 to 1.
        assert_eq!(lines[0], "Not Empty: mean 0.67, median 1.00, range 0.00 to 1.00 over 3 answers");
        assert_eq!(lines[1], "Passes Guardrail: mean 1.00, median 1.00, range 1.00 to 1.00 over 3 answers");
        assert_eq!(lines.len(), 2);
    }
}
