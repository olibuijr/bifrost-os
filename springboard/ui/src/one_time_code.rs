//! One-time codes (task theme-28): finds a verification code in the text of a message, so a text field can offer it for
//! autofill. Pure text rules. There is no message store, no SMS reader and no field binding here; the caller chooses
//! when to look and what to fill.

/// Words that mark a code when they appear shortly before it. Matching is case-insensitive.
const KEYWORDS: [&str; 6] = ["code", "verification", "one-time", "otp", "kóði", "staðfest"];
/// How many bytes before a candidate are searched for a keyword.
const CONTEXT_BYTES: usize = 40;
const MIN_DIGITS: usize = 4;
const MAX_DIGITS: usize = 8;

/// A code found in a message. `start` and `end` are byte offsets, so `text[start..end] == code`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundCode {
    pub code: String,
    pub start: usize,
    pub end: usize,
}

/// Returns the first run of 4 to 8 digits that stands alone (no letter or digit touches it) and has a keyword within
/// 40 bytes before it. A run without a keyword is never taken, so a bare order or year number is ignored.
pub fn find_code(text: &str) -> Option<FoundCode> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let end = i;
        let len = end - start;
        let bounded_before = !text[..start].chars().next_back().is_some_and(char::is_alphanumeric);
        let bounded_after = !text[end..].chars().next().is_some_and(char::is_alphanumeric);
        if (MIN_DIGITS..=MAX_DIGITS).contains(&len) && bounded_before && bounded_after && has_keyword_before(text, start) {
            return Some(FoundCode { code: text[start..end].to_string(), start, end });
        }
    }
    None
}

/// Whether a keyword appears in the `CONTEXT_BYTES` bytes before `start`. The window is moved forward to a character
/// boundary so multi-byte text cannot split a character.
fn has_keyword_before(text: &str, start: usize) -> bool {
    let mut lo = start.saturating_sub(CONTEXT_BYTES);
    while !text.is_char_boundary(lo) {
        lo += 1;
    }
    let window = text[lo..start].to_lowercase();
    KEYWORDS.iter().any(|k| window.contains(*k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_code_after_a_verification_keyword_with_its_offsets() {
        let text = "Your verification code is 482913.";
        let found = find_code(text).unwrap();
        assert_eq!(found.code, "482913");
        assert_eq!(found.start, 26);
        assert_eq!(found.end, 32);
        assert_eq!(&text[found.start..found.end], "482913");
    }

    #[test]
    fn a_bare_number_without_a_keyword_is_ignored() {
        assert_eq!(find_code("Order 1234567 shipped"), None);
    }

    #[test]
    fn a_year_before_the_keyword_is_skipped() {
        assert_eq!(find_code("Sent 2026 with code 4411").unwrap().code, "4411");
    }

    #[test]
    fn runs_shorter_than_four_or_longer_than_eight_digits_are_rejected() {
        assert_eq!(find_code("code 1234567890"), None);
        assert_eq!(find_code("code 123"), None);
    }

    #[test]
    fn a_run_glued_to_a_letter_is_rejected() {
        assert_eq!(find_code("code ab1234"), None);
    }

    #[test]
    fn the_first_keyword_candidate_wins() {
        assert_eq!(find_code("code 111111 or 222222").unwrap().code, "111111");
    }

    #[test]
    fn multi_byte_text_before_the_code_does_not_panic_or_miss_the_keyword() {
        let icelandic = "Staðfestingarkóði: 483920";
        assert_eq!(find_code(icelandic).unwrap().code, "483920");
        let long_prefix = format!("{} code 5566", "á".repeat(30));
        assert_eq!(find_code(&long_prefix).unwrap().code, "5566");
    }
}
