//! App ratings and written reviews: the model behind the store's rating sheet.
//!
//! A review is one star rating (1 to 5), an optional nickname, and an optional written body. Validation runs before a
//! review is sent, and the summary turns a list of star ratings into the average and the five-bar histogram the store
//! shows on a product page. The sheet is `shell::store::review_sheet`; [`ReviewBook`] keeps the reviews written on this
//! PC in a file, so one can be edited later and sent once the catalogue takes reviews (it has no review endpoint yet, a
//! server job outside this repository).

/// The lowest and highest star rating.
pub const MIN_STARS: u8 = 1;
pub const MAX_STARS: u8 = 5;
/// Length limits in characters, counted on the trimmed text.
pub const MAX_NICKNAME: usize = 32;
pub const MAX_BODY: usize = 4000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub stars: u8,
    pub nickname: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The rating is outside 1 to 5.
    Stars(u8),
    /// The nickname is longer than [`MAX_NICKNAME`] characters.
    NicknameTooLong(usize),
    /// The written review is longer than [`MAX_BODY`] characters.
    BodyTooLong(usize),
    /// The sheet's draft was already sent.
    AlreadySubmitted,
    /// The app id is empty, longer than [`MAX_APP_ID`] bytes, or holds a control character; or the book is full.
    AppId,
}

/// The longest app id a [`ReviewBook`] keeps, in bytes.
pub const MAX_APP_ID: usize = 128;
/// The most reviews a [`ReviewBook`] keeps; a review of a new app past it is refused.
pub const MAX_REVIEWS: usize = 500;

/// Checks a review before it is sent. The text fields are trimmed first, so spaces alone do not count as text.
pub fn validate(review: &Review) -> Result<(), Error> {
    if !(MIN_STARS..=MAX_STARS).contains(&review.stars) {
        return Err(Error::Stars(review.stars));
    }
    let nickname = review.nickname.trim().chars().count();
    if nickname > MAX_NICKNAME {
        return Err(Error::NicknameTooLong(nickname));
    }
    let body = review.body.trim().chars().count();
    if body > MAX_BODY {
        return Err(Error::BodyTooLong(body));
    }
    Ok(())
}

/// The request body for sending a review to the catalogue: `{"stars":N,"nickname":"...","body":"..."}`. The review is
/// validated first, and the text fields are trimmed and JSON-escaped, so no input can change the shape of the object.
pub fn to_json(review: &Review) -> Result<String, Error> {
    validate(review)?;
    Ok(format!(
        "{{\"stars\":{},\"nickname\":{},\"body\":{}}}",
        review.stars,
        json_string(review.nickname.trim()),
        json_string(review.body.trim())
    ))
}

/// A JSON string literal for `text`, with quotes, backslashes and control characters escaped.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The reviews written on this PC: the latest review of each app, by app id. The book is the sheet's memory (an app already
/// reviewed opens the sheet with its review to edit) and the queue of what the catalogue will receive. Its text form is one
/// JSON object, `{"reviews":[{"app":"...","stars":N,"nickname":"...","body":"..."}]}`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewBook {
    entries: std::collections::BTreeMap<String, Review>,
}

fn app_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_APP_ID && !id.chars().any(char::is_control)
}

impl ReviewBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// The review of `app_id`, if one was written.
    pub fn get(&self, app_id: &str) -> Option<&Review> {
        self.entries.get(app_id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The app ids with a review, in order.
    pub fn app_ids(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    /// Keeps `review` as the review of `app_id`, replacing an earlier one. The review is validated and its text trimmed
    /// first; a bad app id, an invalid review, or a new app past [`MAX_REVIEWS`] is refused and the book stays as it was.
    pub fn set(&mut self, app_id: &str, review: &Review) -> Result<(), Error> {
        if !app_id_ok(app_id) {
            return Err(Error::AppId);
        }
        validate(review)?;
        if !self.entries.contains_key(app_id) && self.entries.len() >= MAX_REVIEWS {
            return Err(Error::AppId);
        }
        let tidy = Review { stars: review.stars, nickname: review.nickname.trim().to_string(), body: review.body.trim().to_string() };
        self.entries.insert(app_id.to_string(), tidy);
        Ok(())
    }

    /// Drops the review of `app_id`; true when there was one.
    pub fn remove(&mut self, app_id: &str) -> bool {
        self.entries.remove(app_id).is_some()
    }

    /// The book as text, apps in order.
    pub fn to_text(&self) -> String {
        let items: Vec<String> = self
            .entries
            .iter()
            .map(|(app, r)| format!("{{\"app\":{},\"stars\":{},\"nickname\":{},\"body\":{}}}", json_string(app), r.stars, json_string(&r.nickname), json_string(&r.body)))
            .collect();
        format!("{{\"reviews\":[{}]}}\n", items.join(","))
    }

    /// Reads [`ReviewBook::to_text`] output. Text that is not a book gives an empty one, and an entry that does not
    /// validate (bad app id, stars outside 1 to 5, text over its limit) is skipped, so a damaged file loses only that entry.
    pub fn from_text(text: &str) -> Self {
        let mut book = Self::new();
        let Ok(root) = crate::json::parse(text) else { return book };
        for item in root.get("reviews").and_then(crate::json::Json::as_array).unwrap_or(&[]) {
            let (Some(app), Some(stars)) = (item.str_field("app"), item.get("stars").and_then(crate::json::Json::as_u64)) else { continue };
            let review = Review {
                stars: u8::try_from(stars).unwrap_or(0),
                nickname: item.str_field("nickname").unwrap_or_default(),
                body: item.str_field("body").unwrap_or_default(),
            };
            let _ = book.set(&app, &review);
        }
        book
    }
}

/// The rating summary for one product: the number of ratings at each star count (index 0 is one star) and the average.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub counts: [u32; 5],
    pub total: u32,
    /// The average in tenths of a star, rounded half up, or None when there are no ratings.
    pub average_tenths: Option<u32>,
}

/// Summarises a list of star ratings. Ratings outside 1 to 5 are ignored rather than counted.
pub fn summarise(stars: &[u8]) -> Summary {
    let mut counts = [0u32; 5];
    let mut sum = 0u64;
    for &s in stars {
        if (MIN_STARS..=MAX_STARS).contains(&s) {
            counts[(s - 1) as usize] += 1;
            sum += s as u64;
        }
    }
    let total: u32 = counts.iter().sum();
    let average_tenths = if total == 0 { None } else { Some(((sum * 20 + total as u64) / (2 * total as u64)) as u32) };
    Summary { counts, total, average_tenths }
}

/// The average as text with one decimal place, for example "4.3", or an empty string when nothing is rated.
pub fn average_label(summary: &Summary) -> String {
    match summary.average_tenths {
        Some(t) => format!("{}.{}", t / 10, t % 10),
        None => String::new(),
    }
}

/// The rating sheet's editing state: what the person has chosen and typed so far, before the review is sent. The sheet
/// draws it and feeds it touches and keys; this type holds the rules, so it needs no screen. Typing past a limit is
/// refused rather than cut later, tapping the star already chosen clears the rating, and a review can only be submitted
/// once it passes [`validate`] with at least one star.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewDraft {
    stars: u8,
    nickname: String,
    body: String,
    submitted: bool,
}

impl ReviewDraft {
    /// An empty draft: no stars, no text.
    pub fn new() -> Self {
        Self::default()
    }

    /// A draft that starts from an earlier review, so the person can edit it.
    pub fn from_review(review: &Review) -> Self {
        ReviewDraft { stars: review.stars, nickname: review.nickname.clone(), body: review.body.clone(), submitted: false }
    }

    /// The chosen rating, 0 while none is chosen.
    pub fn stars(&self) -> u8 {
        self.stars
    }

    pub fn nickname(&self) -> &str {
        &self.nickname
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    /// Whether [`ReviewDraft::submit`] has produced the request body. A submitted draft takes no more edits.
    pub fn is_submitted(&self) -> bool {
        self.submitted
    }

    /// A tap on star `n` (1 to 5). Tapping the star already chosen clears the rating; a number outside 1 to 5 changes
    /// nothing. Returns whether the rating changed.
    pub fn tap_star(&mut self, n: u8) -> bool {
        if self.submitted || !(MIN_STARS..=MAX_STARS).contains(&n) {
            return false;
        }
        self.stars = if self.stars == n { 0 } else { n };
        true
    }

    /// Types `text` into the nickname. Characters past [`MAX_NICKNAME`] and line breaks are dropped. Returns how many
    /// characters were taken.
    pub fn type_nickname(&mut self, text: &str) -> usize {
        if self.submitted {
            return 0;
        }
        let mut taken = 0;
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.nickname.chars().count() >= MAX_NICKNAME {
                break;
            }
            self.nickname.push(c);
            taken += 1;
        }
        taken
    }

    /// Types `text` into the written review. Characters past [`MAX_BODY`] are dropped; new lines and tabs are kept,
    /// other control characters are not. Returns how many characters were taken.
    pub fn type_body(&mut self, text: &str) -> usize {
        if self.submitted {
            return 0;
        }
        let mut taken = 0;
        for c in text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')) {
            if self.body.chars().count() >= MAX_BODY {
                break;
            }
            self.body.push(c);
            taken += 1;
        }
        taken
    }

    /// Removes the last character of the nickname. Returns whether one was removed.
    pub fn backspace_nickname(&mut self) -> bool {
        !self.submitted && self.nickname.pop().is_some()
    }

    /// Removes the last character of the written review. Returns whether one was removed.
    pub fn backspace_body(&mut self) -> bool {
        !self.submitted && self.body.pop().is_some()
    }

    /// The review as it stands, whatever its state.
    pub fn review(&self) -> Review {
        Review { stars: self.stars, nickname: self.nickname.clone(), body: self.body.clone() }
    }

    /// Whether Send may be enabled: a star rating is chosen and the review is valid.
    pub fn can_submit(&self) -> bool {
        !self.submitted && validate(&self.review()).is_ok()
    }

    /// Produces the request body for the catalogue and locks the draft, so a double tap sends once. The error is the
    /// reason a review that is not ready cannot be sent; the draft stays open then.
    pub fn submit(&mut self) -> Result<String, Error> {
        if self.submitted {
            return Err(Error::AlreadySubmitted);
        }
        let body = to_json(&self.review())?;
        self.submitted = true;
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(stars: u8, nickname: &str, body: &str) -> Review {
        Review { stars, nickname: nickname.to_string(), body: body.to_string() }
    }

    #[test]
    fn the_book_keeps_the_latest_trimmed_review_of_each_app() {
        let mut book = ReviewBook::new();
        assert!(book.is_empty());
        book.set("is.o.chess", &review(4, "  Ana ", " fun  ")).unwrap();
        assert_eq!(book.get("is.o.chess"), Some(&review(4, "Ana", "fun")));
        book.set("is.o.chess", &review(5, "", "")).unwrap();
        book.set("is.o.arc", &review(2, "", "slow")).unwrap();
        assert_eq!((book.len(), book.get("is.o.chess").unwrap().stars), (2, 5), "a second review replaces the first");
        assert_eq!(book.app_ids(), ["is.o.arc", "is.o.chess"]);
        assert!(book.remove("is.o.arc") && !book.remove("is.o.arc"));
    }

    #[test]
    fn a_bad_review_or_app_id_changes_nothing() {
        let mut book = ReviewBook::new();
        book.set("is.o.chess", &review(3, "", "")).unwrap();
        assert_eq!(book.set("", &review(3, "", "")), Err(Error::AppId));
        assert_eq!(book.set("bad\nid", &review(3, "", "")), Err(Error::AppId));
        assert_eq!(book.set(&"a".repeat(MAX_APP_ID + 1), &review(3, "", "")), Err(Error::AppId));
        assert_eq!(book.set("is.o.chess", &review(0, "", "")), Err(Error::Stars(0)));
        assert_eq!(book.get("is.o.chess").unwrap().stars, 3);
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn the_book_is_capped() {
        let mut book = ReviewBook::new();
        for n in 0..MAX_REVIEWS {
            book.set(&format!("app{n}"), &review(1, "", "")).unwrap();
        }
        assert_eq!(book.set("one.more", &review(1, "", "")), Err(Error::AppId));
        book.set("app0", &review(5, "", "")).unwrap();
        assert_eq!(book.len(), MAX_REVIEWS, "an app already in the book can still be edited");
    }

    #[test]
    fn the_book_round_trips_through_its_text() {
        let mut book = ReviewBook::new();
        book.set("is.o.chess", &review(4, "Ana \"A\"", "line one\nline \\two\tx")).unwrap();
        book.set("is.o.arc", &review(1, "", "")).unwrap();
        let text = book.to_text();
        assert!(text.starts_with("{\"reviews\":[{\"app\":\"is.o.arc\""), "{text}");
        assert_eq!(ReviewBook::from_text(&text), book);
        assert_eq!(ReviewBook::new().to_text(), "{\"reviews\":[]}\n");
    }

    #[test]
    fn a_damaged_book_loses_only_the_damaged_entries() {
        assert!(ReviewBook::from_text("not json").is_empty());
        assert!(ReviewBook::from_text("{}").is_empty());
        let text = r#"{"reviews":[{"app":"a.ok","stars":3},{"app":"a.zero","stars":0},{"app":"a.six","stars":6},{"stars":4},{"app":"","stars":4},{"app":"a.big","stars":300}]}"#;
        let book = ReviewBook::from_text(text);
        assert_eq!(book.app_ids(), ["a.ok"]);
        assert_eq!(book.get("a.ok"), Some(&review(3, "", "")));
    }

    #[test]
    fn a_star_rating_outside_one_to_five_is_refused() {
        assert_eq!(validate(&review(0, "", "")), Err(Error::Stars(0)));
        assert_eq!(validate(&review(6, "", "")), Err(Error::Stars(6)));
        assert_eq!(validate(&review(1, "", "")), Ok(()));
        assert_eq!(validate(&review(5, "", "")), Ok(()));
    }

    #[test]
    fn nickname_and_body_limits_count_characters_after_trimming() {
        let long_nick = "n".repeat(MAX_NICKNAME + 1);
        assert_eq!(validate(&review(3, &long_nick, "")), Err(Error::NicknameTooLong(MAX_NICKNAME + 1)));
        let padded = format!("   {}   ", "n".repeat(MAX_NICKNAME));
        assert_eq!(validate(&review(3, &padded, "")), Ok(()), "spaces around the nickname do not count");
        let long_body = "b".repeat(MAX_BODY + 1);
        assert_eq!(validate(&review(3, "", &long_body)), Err(Error::BodyTooLong(MAX_BODY + 1)));
    }

    #[test]
    fn a_review_becomes_a_json_body_with_escaped_text() {
        let body = to_json(&review(4, "  Ana \"A\"  ", "line one\nline \\two\tx  ")).unwrap();
        assert_eq!(body, r#"{"stars":4,"nickname":"Ana \"A\"","body":"line one\nline \\two\tx"}"#);
    }

    #[test]
    fn an_invalid_review_has_no_json_body() {
        assert_eq!(to_json(&review(0, "", "")), Err(Error::Stars(0)));
        assert!(to_json(&review(5, &"n".repeat(MAX_NICKNAME + 1), "")).is_err());
    }

    #[test]
    fn control_characters_are_escaped_as_unicode() {
        assert_eq!(json_string("a\u{1}b"), r#""a\u0001b""#);
    }

    #[test]
    fn an_empty_list_has_no_average_and_an_empty_label() {
        let s = summarise(&[]);
        assert_eq!(s.total, 0);
        assert_eq!(s.average_tenths, None);
        assert_eq!(average_label(&s), "");
    }

    #[test]
    fn the_histogram_and_average_come_from_the_ratings() {
        let s = summarise(&[5, 5, 4, 1]);
        assert_eq!(s.counts, [1, 0, 0, 1, 2]);
        assert_eq!(s.total, 4);
        assert_eq!(s.average_tenths, Some(38), "(5+5+4+1)/4 = 3.75 rounds half up to 3.8");
        assert_eq!(average_label(&s), "3.8");
    }

    #[test]
    fn out_of_range_ratings_are_ignored() {
        let s = summarise(&[0, 9, 4]);
        assert_eq!(s.total, 1);
        assert_eq!(average_label(&s), "4.0");
    }

    #[test]
    fn a_draft_needs_a_star_before_it_can_be_sent() {
        let mut d = ReviewDraft::new();
        assert!(!d.can_submit());
        assert_eq!(d.submit(), Err(Error::Stars(0)));
        assert!(!d.is_submitted(), "a refused draft stays open");
        assert!(d.tap_star(4));
        assert!(d.can_submit());
    }

    #[test]
    fn tapping_the_chosen_star_clears_it_and_a_bad_star_changes_nothing() {
        let mut d = ReviewDraft::new();
        assert!(d.tap_star(3));
        assert_eq!(d.stars(), 3);
        assert!(d.tap_star(5));
        assert_eq!(d.stars(), 5);
        assert!(d.tap_star(5));
        assert_eq!(d.stars(), 0);
        assert!(!d.tap_star(0));
        assert!(!d.tap_star(6));
        assert_eq!(d.stars(), 0);
    }

    #[test]
    fn typing_stops_at_the_limits_and_drops_control_characters() {
        let mut d = ReviewDraft::new();
        assert_eq!(d.type_nickname(&"n".repeat(MAX_NICKNAME + 5)), MAX_NICKNAME);
        assert_eq!(d.nickname().chars().count(), MAX_NICKNAME);
        assert_eq!(d.type_nickname("x"), 0, "a full nickname takes nothing more");
        let mut d = ReviewDraft::new();
        assert_eq!(d.type_nickname("a\nb\u{7}c"), 3, "line breaks and control characters are dropped from the nickname");
        assert_eq!(d.nickname(), "abc");
        assert_eq!(d.type_body("one\ntwo\tx\u{7}"), 9);
        assert_eq!(d.body(), "one\ntwo\tx");
        assert_eq!(d.type_body(&"b".repeat(MAX_BODY)), MAX_BODY - 9);
        assert_eq!(d.body().chars().count(), MAX_BODY);
    }

    #[test]
    fn backspace_removes_one_character_at_a_time() {
        let mut d = ReviewDraft::new();
        assert!(!d.backspace_nickname());
        d.type_nickname("ab");
        d.type_body("é!");
        assert!(d.backspace_nickname());
        assert_eq!(d.nickname(), "a");
        assert!(d.backspace_body() && d.backspace_body());
        assert!(!d.backspace_body());
        assert_eq!(d.body(), "");
    }

    #[test]
    fn submitting_locks_the_draft_so_a_double_tap_sends_once() {
        let mut d = ReviewDraft::new();
        d.tap_star(5);
        d.type_nickname(" Ana ");
        d.type_body("Great");
        let body = d.submit().unwrap();
        assert_eq!(body, r#"{"stars":5,"nickname":"Ana","body":"Great"}"#);
        assert!(d.is_submitted());
        assert_eq!(d.submit(), Err(Error::AlreadySubmitted));
        assert!(!d.tap_star(1));
        assert_eq!(d.type_body("more"), 0);
        assert!(!d.backspace_body());
        assert_eq!(d.stars(), 5);
        assert!(!d.can_submit());
    }

    #[test]
    fn a_draft_can_start_from_an_earlier_review() {
        let d = ReviewDraft::from_review(&review(2, "Bo", "meh"));
        assert_eq!((d.stars(), d.nickname(), d.body()), (2, "Bo", "meh"));
        assert!(d.can_submit());
        assert_eq!(d.review(), review(2, "Bo", "meh"));
    }
}
