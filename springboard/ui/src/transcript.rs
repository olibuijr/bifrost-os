//! A session transcript: the linear, typed history of one conversation with a language model.
//! The leading instructions entry is kept apart from the history, and the tool calls, tool outputs and responses can
//! be read as typed lists.

/// One piece of an entry's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    /// Plain text.
    Text(String),
    /// Structured content, held in its serialized form.
    Structured(String),
    /// A file or image sent with the entry. Only its name and size are kept.
    Attachment { name: String, bytes: usize },
}

/// A tool the model asked to run, with the arguments it chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub arguments: String,
}

/// What a tool returned, handed back to the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolOutput {
    pub name: String,
    pub text: String,
}

/// One entry of a transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Instructions that define the model's behaviour. Only a leading entry counts as the instructions.
    Instructions(Vec<Segment>),
    Prompt(Vec<Segment>),
    Response(Vec<Segment>),
    Reasoning(String),
    ToolCalls(Vec<ToolCall>),
    ToolOutput(ToolOutput),
}

/// A linear history of entries, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transcript {
    entries: Vec<Entry>,
}

/// The typed view of a transcript: tool calls, tool outputs and responses, each in transcript order.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Structured<'a> {
    pub tool_calls: Vec<&'a ToolCall>,
    pub tool_outputs: Vec<&'a ToolOutput>,
    pub responses: Vec<&'a [Segment]>,
}

impl Transcript {
    /// Creates a transcript from its entries, oldest first.
    pub fn new(entries: Vec<Entry>) -> Self {
        Transcript { entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// The segments of the instructions entry, when the transcript starts with one.
    pub fn instructions(&self) -> Option<&[Segment]> {
        match self.entries.first() {
            Some(Entry::Instructions(segments)) => Some(segments.as_slice()),
            _ => None,
        }
    }

    /// The conversational entries: everything after the leading instructions entry.
    pub fn history(&self) -> &[Entry] {
        let skip = usize::from(self.instructions().is_some());
        &self.entries[skip..]
    }

    /// The text of the prompts and responses, one paragraph per entry. Attachments, structured content and
    /// reasoning are left out.
    pub fn text(&self) -> String {
        let mut paragraphs = Vec::new();
        for entry in &self.entries {
            if let Entry::Prompt(segments) | Entry::Response(segments) = entry {
                let text: String = segments
                    .iter()
                    .filter_map(|s| match s {
                        Segment::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect();
                if !text.is_empty() {
                    paragraphs.push(text);
                }
            }
        }
        paragraphs.join("\n\n")
    }

    /// Collects the tool calls, tool outputs and responses into typed lists, keeping their order.
    pub fn structured(&self) -> Structured<'_> {
        let mut out = Structured::default();
        for entry in &self.entries {
            match entry {
                Entry::ToolCalls(calls) => out.tool_calls.extend(calls.iter()),
                Entry::ToolOutput(output) => out.tool_outputs.push(output),
                Entry::Response(segments) => out.responses.push(segments.as_slice()),
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Segment {
        Segment::Text(s.into())
    }

    fn sample() -> Transcript {
        Transcript::new(vec![
            Entry::Instructions(vec![text("Answer briefly.")]),
            Entry::Prompt(vec![text("What is the weather?")]),
            Entry::ToolCalls(vec![ToolCall { name: "weather".into(), arguments: "{\"city\":\"Reykjavik\"}".into() }]),
            Entry::ToolOutput(ToolOutput { name: "weather".into(), text: "4 C, rain".into() }),
            Entry::Reasoning("the tool answered".into()),
            Entry::Response(vec![text("It is 4 C and raining.")]),
        ])
    }

    #[test]
    fn the_leading_instructions_are_kept_apart_from_the_history() {
        let t = sample();
        assert_eq!(t.instructions().map(|s| s.len()), Some(1));
        assert_eq!(t.history().len(), 5);
        assert!(matches!(t.history()[0], Entry::Prompt(_)));
    }

    #[test]
    fn instructions_that_are_not_first_stay_in_the_history() {
        let mut t = Transcript::default();
        assert_eq!(t.instructions(), None);
        t.push(Entry::Prompt(vec![text("hi")]));
        t.push(Entry::Instructions(vec![text("late")]));
        assert_eq!(t.instructions(), None);
        assert_eq!(t.history().len(), 2);
    }

    #[test]
    fn text_joins_prompts_and_responses_and_skips_attachments_and_reasoning() {
        let mut t = sample();
        t.push(Entry::Response(vec![
            text("Ask again "),
            Segment::Attachment { name: "map.png".into(), bytes: 900 },
            text("if needed."),
        ]));
        assert_eq!(t.text(), "What is the weather?\n\nIt is 4 C and raining.\n\nAsk again if needed.");
    }

    #[test]
    fn structured_collects_tool_calls_outputs_and_responses_in_order() {
        let t = sample();
        let s = t.structured();
        assert_eq!(s.tool_calls.len(), 1);
        assert_eq!(s.tool_calls[0].name, "weather");
        assert_eq!(s.tool_outputs.len(), 1);
        assert_eq!(s.tool_outputs[0].text, "4 C, rain");
        assert_eq!(s.responses.len(), 1);
        assert_eq!(s.responses[0][0], text("It is 4 C and raining."));
    }

    #[test]
    fn an_empty_transcript_has_no_instructions_history_or_text() {
        let t = Transcript::default();
        assert!(t.instructions().is_none());
        assert!(t.history().is_empty());
        assert_eq!(t.text(), "");
        assert!(t.structured().responses.is_empty());
    }
}
