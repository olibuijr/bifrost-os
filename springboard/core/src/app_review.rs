//! App ratings and written reviews: the model behind the store's rating sheet.
//!
//! A review is one star rating (1 to 5), an optional nickname, and an optional written body. Validation runs before a
//! review is sent, and the summary turns a list of star ratings into the average and the five-bar histogram the store
//! shows on a product page. The sheet and the catalogue endpoints are not built yet; this module is the rules they
//! will call.

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
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn review(stars: u8, nickname: &str, body: &str) -> Review {
        Review { stars, nickname: nickname.to_string(), body: body.to_string() }
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
}
