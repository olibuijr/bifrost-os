//! Writing assistance for a text view: the behaviour levels a view can ask for, the context the assistant may read,
//! which kinds of result content it may return, and a session that holds proofreading or rewriting suggestions until
//! the person accepts or rejects each one.
//! A suggestion is a byte-range replacement in the view's text. Applying one checks the range against the text, so a
//! stale suggestion cannot cut a character in half or write past the end. The assistant itself is not part of this
//! module: the caller asks for suggestions and offers them to the session.

use std::ops::Range;

/// How much assistance a view gets. `None` keeps the assistant away from the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behavior {
    None,
    /// Let the system pick the best level it offers.
    Default,
    /// The full experience, with suggestions inline in the text.
    Complete,
    /// A limited experience: results shown in an overlay panel.
    Limited,
}

impl Behavior {
    /// The level a view actually gets. `available` is whether the assistant is enabled on this system.
    /// `None` always stays off. `Default` resolves to `Complete` when the assistant is available. The other levels
    /// are granted as asked.
    pub fn resolve(self, available: bool) -> Behavior {
        match (self, available) {
            (Behavior::None, _) | (_, false) => Behavior::None,
            (Behavior::Default, true) => Behavior::Complete,
            (other, true) => other,
        }
    }
}

/// The kinds of result content the assistant may return. Plain text is always allowed as the floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultOptions(u8);

impl ResultOptions {
    pub const NONE: ResultOptions = ResultOptions(0);
    pub const PLAIN_TEXT: ResultOptions = ResultOptions(1);
    /// Style attributes that match the rich text format.
    pub const RICH_TEXT: ResultOptions = ResultOptions(2);
    pub const LIST: ResultOptions = ResultOptions(4);
    pub const TABLE: ResultOptions = ResultOptions(8);

    pub fn union(self, other: ResultOptions) -> ResultOptions {
        ResultOptions(self.0 | other.0)
    }

    pub fn intersect(self, other: ResultOptions) -> ResultOptions {
        ResultOptions(self.0 & other.0)
    }

    /// True when every option in `other` is set here. An empty `other` is never contained.
    pub fn contains(self, other: ResultOptions) -> bool {
        other.0 != 0 && self.0 & other.0 == other.0
    }

    /// What a suggestion may carry: the asked-for options that the view supports. Plain text when nothing is left.
    pub fn negotiate(preferred: ResultOptions, supported: ResultOptions) -> ResultOptions {
        let granted = preferred.intersect(supported);
        if granted == ResultOptions::NONE {
            ResultOptions::PLAIN_TEXT
        } else {
            granted
        }
    }
}

/// The part of the text the assistant reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextScope {
    UserSelection,
    VisibleArea,
    FullDocument,
}

/// The byte range of `text` the assistant reads for `scope`. `selection` and `visible` are byte ranges in `text`.
/// Both ends are clamped to the text and snapped down to char boundaries, so the result is always valid to slice.
pub fn context_range(text: &str, scope: ContextScope, selection: Range<usize>, visible: Range<usize>) -> Range<usize> {
    let wanted = match scope {
        ContextScope::UserSelection => selection,
        ContextScope::VisibleArea => visible,
        ContextScope::FullDocument => 0..text.len(),
    };
    let start = floor_boundary(text, wanted.start);
    let end = floor_boundary(text, wanted.end).max(start);
    start..end
}

fn floor_boundary(text: &str, index: usize) -> usize {
    let mut i = index.min(text.len());
    // Index 0 is always a boundary, so this stops.
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// What the assistant was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Proofread,
    Rewrite,
}

/// One proposed change: put `replacement` in place of the bytes at `range`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub range: Range<usize>,
    pub replacement: String,
}

/// Why a suggestion could not be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The range is reversed or runs past the end of the text.
    OutOfRange(Range<usize>),
    /// A range end falls inside a character. The value is the offending byte offset.
    NotCharBoundary(usize),
    /// Two suggestions cover the same bytes.
    Overlap(Range<usize>, Range<usize>),
}

/// Applies non-overlapping suggestions to `text` in one pass and returns the new text. The input order does not
/// matter. Every range is checked first, so on error nothing is applied.
pub fn apply(text: &str, suggestions: &[Suggestion]) -> Result<String, EditError> {
    let mut sorted: Vec<&Suggestion> = suggestions.iter().collect();
    sorted.sort_by_key(|s| (s.range.start, s.range.end));
    for s in &sorted {
        let (start, end) = (s.range.start, s.range.end);
        if start > end || end > text.len() {
            return Err(EditError::OutOfRange(s.range.clone()));
        }
        if !text.is_char_boundary(start) {
            return Err(EditError::NotCharBoundary(start));
        }
        if !text.is_char_boundary(end) {
            return Err(EditError::NotCharBoundary(end));
        }
    }
    for pair in sorted.windows(2) {
        if pair[0].range.end > pair[1].range.start {
            return Err(EditError::Overlap(pair[0].range.clone(), pair[1].range.clone()));
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for s in sorted {
        out.push_str(&text[at..s.range.start]);
        out.push_str(&s.replacement);
        at = s.range.end;
    }
    out.push_str(&text[at..]);
    Ok(out)
}

/// Where `range` sits after the bytes `edit` were replaced by `new_len` bytes. Used to keep the assistant's context
/// and the pending suggestions on the same words after an edit. A position before the edit does not move. A
/// position after it shifts by the change in length. A position inside the edit moves to its start, or to the end of
/// the new text when it is the range end.
pub fn map_range(range: &Range<usize>, edit: &Range<usize>, new_len: usize) -> Range<usize> {
    let old_len = edit.end - edit.start;
    let map = |p: usize, is_end: bool| -> usize {
        if p <= edit.start {
            p
        } else if p >= edit.end {
            p - old_len + new_len
        } else if is_end {
            edit.start + new_len
        } else {
            edit.start
        }
    };
    map(range.start, false)..map(range.end, true)
}

/// Why a session call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The view's resolved behaviour is `None`.
    Disabled,
    /// A task is already running.
    Busy,
    /// No task is running.
    Idle,
    /// There is no pending suggestion with this index.
    NoSuggestion(usize),
    Edit(EditError),
}

/// One assistant session on one text view: at most one task at a time, with its pending suggestions.
#[derive(Debug)]
pub struct Session {
    behavior: Behavior,
    task: Option<Task>,
    context: Range<usize>,
    pending: Vec<Suggestion>,
}

impl Session {
    /// A session for a view whose behaviour has already been resolved with `Behavior::resolve`.
    pub fn new(behavior: Behavior) -> Session {
        Session { behavior, task: None, context: 0..0, pending: Vec::new() }
    }

    pub fn behavior(&self) -> Behavior {
        self.behavior
    }

    pub fn task(&self) -> Option<Task> {
        self.task
    }

    pub fn context(&self) -> Range<usize> {
        self.context.clone()
    }

    pub fn pending(&self) -> &[Suggestion] {
        &self.pending
    }

    /// Starts `task` over `context`, which the caller got from `context_range`.
    pub fn begin(&mut self, task: Task, context: Range<usize>) -> Result<(), SessionError> {
        if self.behavior == Behavior::None {
            return Err(SessionError::Disabled);
        }
        if self.task.is_some() {
            return Err(SessionError::Busy);
        }
        self.task = Some(task);
        self.context = context;
        self.pending.clear();
        Ok(())
    }

    /// Stores the suggestions the assistant returned for the running task. This replaces earlier ones.
    pub fn offer(&mut self, suggestions: Vec<Suggestion>) -> Result<(), SessionError> {
        if self.task.is_none() {
            return Err(SessionError::Idle);
        }
        self.pending = suggestions;
        Ok(())
    }

    /// Accepts suggestion `index` and returns the text with it applied. Pending suggestions that overlap it are
    /// dropped as stale. The others and the context move with the change in length.
    pub fn accept(&mut self, text: &str, index: usize) -> Result<String, SessionError> {
        let chosen = self.pending.get(index).cloned().ok_or(SessionError::NoSuggestion(index))?;
        let new_text = apply(text, std::slice::from_ref(&chosen)).map_err(SessionError::Edit)?;
        self.pending.remove(index);
        let (edit, new_len) = (chosen.range.clone(), chosen.replacement.len());
        self.pending
            .retain(|p| !(p.range.start < edit.end && edit.start < p.range.end));
        for p in &mut self.pending {
            p.range = map_range(&p.range, &edit, new_len);
        }
        self.context = map_range(&self.context, &edit, new_len);
        Ok(new_text)
    }

    /// Rejects suggestion `index`. The text does not change.
    pub fn reject(&mut self, index: usize) -> Result<(), SessionError> {
        if index >= self.pending.len() {
            return Err(SessionError::NoSuggestion(index));
        }
        self.pending.remove(index);
        Ok(())
    }

    /// Ends the running task and drops its suggestions.
    pub fn stop(&mut self) -> Result<(), SessionError> {
        if self.task.take().is_none() {
            return Err(SessionError::Idle);
        }
        self.pending.clear();
        self.context = 0..0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sug(range: Range<usize>, replacement: &str) -> Suggestion {
        Suggestion { range, replacement: replacement.to_string() }
    }

    #[test]
    fn behaviour_resolves_against_availability() {
        assert_eq!(Behavior::Default.resolve(true), Behavior::Complete);
        assert_eq!(Behavior::Default.resolve(false), Behavior::None);
        assert_eq!(Behavior::None.resolve(true), Behavior::None);
        assert_eq!(Behavior::Limited.resolve(true), Behavior::Limited);
        assert_eq!(Behavior::Complete.resolve(false), Behavior::None);
    }

    #[test]
    fn result_options_are_negotiated_down_to_plain_text() {
        let rich_list = ResultOptions::RICH_TEXT.union(ResultOptions::LIST);
        let granted = ResultOptions::negotiate(rich_list.union(ResultOptions::TABLE), rich_list);
        assert!(granted.contains(ResultOptions::RICH_TEXT));
        assert!(granted.contains(ResultOptions::LIST));
        assert!(!granted.contains(ResultOptions::TABLE));
        // Nothing in common: the floor is plain text.
        let floor = ResultOptions::negotiate(ResultOptions::TABLE, ResultOptions::LIST);
        assert_eq!(floor, ResultOptions::PLAIN_TEXT);
        assert!(!ResultOptions::NONE.contains(ResultOptions::NONE));
    }

    #[test]
    fn context_is_clamped_and_snapped_to_char_boundaries() {
        let text = "é ok"; // 'é' is two bytes
        assert_eq!(context_range(text, ContextScope::UserSelection, 1..2, 0..0), 0..2);
        assert_eq!(context_range(text, ContextScope::VisibleArea, 0..0, 2..4), 2..4);
        assert_eq!(context_range(text, ContextScope::FullDocument, 0..0, 0..0), 0..text.len());
        assert_eq!(context_range(text, ContextScope::UserSelection, 3..99, 0..0), 3..text.len());
    }

    #[test]
    fn apply_takes_suggestions_in_any_order() {
        let text = "teh cat and teh dog";
        let got = apply(text, &[sug(12..15, "the"), sug(0..3, "the")]).unwrap();
        assert_eq!(got, "the cat and the dog");
        // Removal and insertion work too.
        assert_eq!(apply("abc", &[sug(1..1, "X")]).unwrap(), "aXbc");
        assert_eq!(apply("abc", &[sug(0..3, "")]).unwrap(), "");
    }

    #[test]
    fn apply_refuses_bad_ranges_and_applies_nothing() {
        assert_eq!(apply("abc", &[sug(2..9, "x")]), Err(EditError::OutOfRange(2..9)));
        assert_eq!(apply("abc", &[sug(2..1, "x")]), Err(EditError::OutOfRange(2..1)));
        assert_eq!(apply("é", &[sug(1..2, "x")]), Err(EditError::NotCharBoundary(1)));
        assert_eq!(
            apply("abcd", &[sug(0..3, "x"), sug(2..4, "y")]),
            Err(EditError::Overlap(0..3, 2..4))
        );
    }

    #[test]
    fn map_range_moves_positions_around_an_edit() {
        // Replace bytes 5..10 (five bytes) with three bytes.
        let edit = 5..10;
        assert_eq!(map_range(&(0..4), &edit, 3), 0..4, "before the edit stays");
        assert_eq!(map_range(&(12..15), &edit, 3), 10..13, "after the edit shifts by -2");
        assert_eq!(map_range(&(3..12), &edit, 3), 3..10, "a range over the edit end moves its end by -2");
        assert_eq!(map_range(&(6..8), &edit, 3), 5..8, "a range starting inside starts at the edit");
    }

    #[test]
    fn session_accepts_rejects_and_keeps_the_rest_in_step() {
        let text = "teh cat and teh dog";
        let mut s = Session::new(Behavior::Complete);
        assert_eq!(s.offer(vec![]), Err(SessionError::Idle));
        s.begin(Task::Proofread, 0..text.len()).unwrap();
        assert_eq!(s.begin(Task::Rewrite, 0..1), Err(SessionError::Busy));
        s.offer(vec![sug(0..3, "the"), sug(12..15, "the"), sug(1..2, "X")]).unwrap();

        // Accepting "teh" -> "the" keeps the second suggestion on its word and drops the overlapping one.
        let text = s.accept(text, 0).unwrap();
        assert_eq!(text, "the cat and teh dog");
        assert_eq!(s.pending().len(), 1);
        assert_eq!(s.pending()[0].range, 12..15);
        assert_eq!(s.context(), 0..text.len());

        // Rejecting leaves the text alone.
        s.reject(0).unwrap();
        assert!(s.pending().is_empty());
        assert_eq!(s.accept(&text, 0), Err(SessionError::NoSuggestion(0)));

        s.stop().unwrap();
        assert_eq!(s.task(), None);
        assert_eq!(s.stop(), Err(SessionError::Idle));
    }

    #[test]
    fn disabled_behaviour_refuses_to_start() {
        let mut s = Session::new(Behavior::None);
        assert_eq!(s.begin(Task::Proofread, 0..0), Err(SessionError::Disabled));
        assert_eq!(s.task(), None);
    }
}
