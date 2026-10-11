//! Context window budget: estimates how much of a transcript fits in a prompt and keeps the newest turns that do.
//! The estimate counts characters (about four per token), because no model tokenizer runs here. It is a planning
//! figure. Callers keep a margin for the response through the `reserve` argument of `fit`.

use crate::transcript::{Entry, Segment, ToolCall, Transcript};

/// Characters per token in the estimate.
pub const CHARS_PER_TOKEN: usize = 4;

/// Tokens counted for each attachment, whatever its size: an image takes a fixed share of the context.
pub const ATTACHMENT_TOKENS: usize = 256;

/// The estimated tokens of `text`, rounded up.
pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    (chars + CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN
}

fn segments_tokens(segments: &[Segment]) -> usize {
    segments
        .iter()
        .map(|s| match s {
            Segment::Text(t) | Segment::Structured(t) => estimate_tokens(t),
            Segment::Attachment { name, .. } => estimate_tokens(name) + ATTACHMENT_TOKENS,
        })
        .sum()
}

fn call_tokens(call: &ToolCall) -> usize {
    estimate_tokens(&call.name) + estimate_tokens(&call.arguments)
}

/// The estimated tokens of one entry.
pub fn entry_tokens(entry: &Entry) -> usize {
    match entry {
        Entry::Instructions(s) | Entry::Prompt(s) | Entry::Response(s) => segments_tokens(s),
        Entry::Reasoning(text) => estimate_tokens(text),
        Entry::ToolCalls(calls) => calls.iter().map(call_tokens).sum(),
        Entry::ToolOutput(output) => estimate_tokens(&output.name) + estimate_tokens(&output.text),
    }
}

/// The outcome of fitting a transcript to a budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fitted {
    /// The instructions, kept whatever their size, followed by the newest history that fits.
    pub transcript: Transcript,
    /// How many history entries were removed to make room.
    pub dropped: usize,
    /// The estimated tokens of the kept transcript, reserve not included.
    pub tokens: usize,
    /// True when the instructions alone do not fit. Then no history is kept.
    pub over_budget: bool,
}

/// Keeps the instructions and the newest history that fits in `budget` tokens, leaving `reserve` tokens free for the
/// response. Older entries go first. The kept history starts at a prompt, so a tool output or a response never keeps
/// without the prompt it answers.
pub fn fit(transcript: &Transcript, budget: usize, reserve: usize) -> Fitted {
    let room = budget.saturating_sub(reserve);
    let mut leading = Vec::new();
    if let Some(first) = transcript.entries().first() {
        if matches!(first, Entry::Instructions(_)) {
            leading.push(first.clone());
        }
    }
    let base: usize = leading.iter().map(entry_tokens).sum();
    let history = transcript.history();
    if base > room {
        return Fitted {
            transcript: Transcript::new(leading),
            dropped: history.len(),
            tokens: base,
            over_budget: true,
        };
    }

    let mut used = base;
    let mut start = history.len();
    for (i, entry) in history.iter().enumerate().rev() {
        let cost = entry_tokens(entry);
        if used + cost > room {
            break;
        }
        used += cost;
        start = i;
    }
    while start < history.len() && !matches!(history[start], Entry::Prompt(_)) {
        start += 1;
    }

    let kept = &history[start..];
    let mut entries = leading;
    entries.extend(kept.iter().cloned());
    let tokens: usize = entries.iter().map(entry_tokens).sum();
    Fitted {
        transcript: Transcript::new(entries),
        dropped: history.len() - kept.len(),
        tokens,
        over_budget: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::ToolOutput;

    /// Forty characters: ten tokens.
    fn long(prefix: char) -> String {
        std::iter::repeat(prefix).take(40).collect()
    }

    fn prompt(c: char) -> Entry {
        Entry::Prompt(vec![Segment::Text(long(c))])
    }

    fn response(c: char) -> Entry {
        Entry::Response(vec![Segment::Text(long(c))])
    }

    /// Instructions of eight characters (two tokens), then three prompt and response pairs of ten tokens each.
    fn chat() -> Transcript {
        Transcript::new(vec![
            Entry::Instructions(vec![Segment::Text("be brief".into())]),
            prompt('a'),
            response('b'),
            prompt('c'),
            response('d'),
            prompt('e'),
            response('f'),
        ])
    }

    #[test]
    fn estimates_round_up_per_four_characters_and_count_characters_not_bytes() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("åäö"), 1, "three characters are one token");
    }

    #[test]
    fn a_transcript_that_fits_is_returned_whole() {
        let t = chat();
        let fitted = fit(&t, 1000, 100);
        assert_eq!(fitted.transcript, t);
        assert_eq!(fitted.dropped, 0);
        assert!(!fitted.over_budget);
        assert_eq!(fitted.tokens, 2 + 60);
    }

    #[test]
    fn older_turns_go_first_and_the_instructions_stay() {
        let t = chat();
        // Room for 22 tokens: the instructions take 2, and only the last prompt and response (20) fit.
        let fitted = fit(&t, 22, 0);
        assert_eq!(fitted.dropped, 4);
        assert_eq!(fitted.tokens, 22);
        assert!(fitted.transcript.instructions().is_some());
        assert_eq!(fitted.transcript.history().len(), 2);
        assert!(matches!(fitted.transcript.history()[0], Entry::Prompt(_)));
    }

    #[test]
    fn the_kept_history_starts_at_a_prompt_never_at_an_answer() {
        let t = Transcript::new(vec![
            prompt('a'),
            response('b'),
            prompt('c'),
            Entry::ToolOutput(ToolOutput { name: String::new(), text: long('x') }),
            response('d'),
        ]);
        // Room for 35 tokens: the newest entries fit, and the kept history begins at the prompt that they answer.
        let fitted = fit(&t, 35, 0);
        assert_eq!(fitted.dropped, 2);
        assert!(matches!(fitted.transcript.entries()[0], Entry::Prompt(_)));
        // Room for 25 tokens: the tool output would start the history, so it and the answer are dropped too.
        let tight = fit(&t, 25, 0);
        assert!(tight.transcript.history().is_empty());
        assert_eq!(tight.dropped, 5);
    }

    #[test]
    fn instructions_larger_than_the_room_mean_no_history_is_kept() {
        let t = chat();
        let fitted = fit(&t, 1, 0);
        assert!(fitted.over_budget);
        assert_eq!(fitted.dropped, 6);
        assert!(fitted.transcript.history().is_empty());
        assert_eq!(fitted.transcript.entries().len(), 1);
    }

    #[test]
    fn the_reserve_for_the_response_leaves_less_room_for_history() {
        let t = chat();
        // Room of 42 keeps the last four entries. Reserving 10 of it leaves 32, which keeps only the last two.
        let without = fit(&t, 42, 0);
        let with = fit(&t, 42, 10);
        assert_eq!((without.dropped, with.dropped), (2, 4));
        assert!(with.tokens <= 32);
    }

    #[test]
    fn tool_calls_count_their_name_and_arguments() {
        let call = Entry::ToolCalls(vec![ToolCall { name: "tide".into(), arguments: long('q') }]);
        assert_eq!(entry_tokens(&call), 1 + 10);
        let image = Entry::Prompt(vec![Segment::Attachment { name: "map".into(), bytes: 1 }]);
        assert_eq!(entry_tokens(&image), 1 + ATTACHMENT_TOKENS);
    }
}
