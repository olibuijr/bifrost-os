//! Text framing for a script step (`NativeShellScriptAction`, input and output field separators, line-ending remapping).
//! Items sent to the script on standard input are joined with an input separator; the text the script prints is split
//! on an output separator. Only the framing lives here. Running the script is a separate step that no caller in the
//! tree does yet, so a script step cannot run in a workflow.

/// Separators and line-ending handling for one script step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptIo {
    /// Placed between the items sent on standard input.
    pub input_separator: String,
    /// Splits the text the script prints into items.
    pub output_separator: String,
    /// Maps carriage returns to newlines in the input string before it is sent.
    pub remap_line_endings: bool,
}

impl Default for ScriptIo {
    /// Newline separators and no remapping. The reference page does not state defaults, so these are BifrOSt choices.
    fn default() -> Self {
        ScriptIo { input_separator: "\n".into(), output_separator: "\n".into(), remap_line_endings: false }
    }
}

impl ScriptIo {
    /// The text sent on standard input for `items`.
    pub fn encode_input(&self, items: &[&str]) -> String {
        let joined = items.join(self.input_separator.as_str());
        if self.remap_line_endings {
            joined.replace('\r', "\n")
        } else {
            joined
        }
    }

    /// The items in text the script printed. Empty text has no items. An empty output separator keeps the text whole.
    pub fn decode_output(&self, text: &str) -> Vec<String> {
        if text.is_empty() {
            return Vec::new();
        }
        if self.output_separator.is_empty() {
            return vec![text.to_string()];
        }
        text.split(self.output_separator.as_str()).map(str::to_string).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_input_is_one_item_per_line() {
        assert_eq!(ScriptIo::default().encode_input(&["a", "b", "c"]), "a\nb\nc");
    }

    #[test]
    fn custom_input_separator_is_used_between_items() {
        let io = ScriptIo { input_separator: "\t".into(), ..ScriptIo::default() };
        assert_eq!(io.encode_input(&["name", "size"]), "name\tsize");
        assert_eq!(io.encode_input(&[]), "");
    }

    #[test]
    fn remapping_turns_carriage_returns_into_newlines_only_when_enabled() {
        let on = ScriptIo { remap_line_endings: true, ..ScriptIo::default() };
        let off = ScriptIo::default();
        assert_eq!(on.encode_input(&["x\ry"]), "x\ny");
        assert_eq!(off.encode_input(&["x\ry"]), "x\ry");
    }

    #[test]
    fn output_is_split_on_the_output_separator() {
        let io = ScriptIo { output_separator: ":".into(), ..ScriptIo::default() };
        assert_eq!(io.decode_output("a:b:c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn empty_output_has_no_items() {
        assert!(ScriptIo::default().decode_output("").is_empty());
    }

    #[test]
    fn empty_output_separator_keeps_the_text_whole() {
        let io = ScriptIo { output_separator: String::new(), ..ScriptIo::default() };
        assert_eq!(io.decode_output("a\nb"), vec!["a\nb"]);
    }

    #[test]
    fn items_survive_a_round_trip_through_the_same_separator() {
        let io = ScriptIo { input_separator: "|".into(), output_separator: "|".into(), remap_line_endings: false };
        let items = ["one", "two", "three"];
        assert_eq!(io.decode_output(&io.encode_input(&items)), items);
    }
}
