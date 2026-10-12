//! One-time codes that arrive in messages and mail. Two parts: the extractor that finds the code in a message's
//! text (what a text field needs to offer it for filling), and the inbox that holds delivered codes until one
//! process consumes each. The rules here are ours: a code is a run of 4 to 8 ASCII digits set off by
//! non-alphanumeric characters. When the text names a code word, the first such run after the first code word
//! wins; otherwise the text must hold exactly one run. Pure: nothing is read from a mailbox here.

/// Words that mark the code in a message, compared without case.
const CODE_WORDS: [&str; 4] = ["code", "passcode", "verification", "otp"];

/// Whether a text is a code this model accepts: 4 to 8 ASCII digits.
pub fn is_code(text: &str) -> bool {
    (4..=8).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

/// The one-time code in a message, if the text makes it clear which run of digits is the code.
pub fn extract_code(message: &str) -> Option<String> {
    let tokens: Vec<&str> = message.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect();
    let keyword = tokens.iter().position(|t| CODE_WORDS.iter().any(|w| t.eq_ignore_ascii_case(w)));
    let candidates: Vec<(usize, &str)> =
        tokens.iter().enumerate().filter(|(_, t)| is_code(t)).map(|(i, t)| (i, *t)).collect();
    if let Some(k) = keyword {
        if let Some((_, code)) = candidates.iter().find(|(i, _)| *i > k) {
            return Some(code.to_string());
        }
    }
    match candidates.as_slice() {
        [(_, only)] => Some(only.to_string()),
        _ => None,
    }
}

/// One code the system holds: when it arrived and which process consumed it, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveredCode {
    pub code: String,
    pub received: i64,
    pub consumed_by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerificationError {
    /// No such code was delivered.
    NotDelivered,
    /// Another process consumed the code already.
    AlreadyConsumed,
    /// The code is older than the inbox keeps codes.
    Expired,
}

/// The codes delivered to the system. A code lives for `lifetime` seconds after it arrives.
#[derive(Debug)]
pub struct DeliveredCodes {
    lifetime: i64,
    codes: Vec<DeliveredCode>,
}

impl DeliveredCodes {
    pub fn new(lifetime: i64) -> Self {
        DeliveredCodes { lifetime, codes: Vec::new() }
    }

    /// Records a code that arrived at `received` (seconds). Returns false for a text that is not a code, or for a
    /// code that is already held and not consumed.
    pub fn deliver(&mut self, code: &str, received: i64) -> bool {
        if !is_code(code) || self.codes.iter().any(|c| c.code == code && c.consumed_by.is_none()) {
            return false;
        }
        self.codes.push(DeliveredCode { code: code.to_string(), received, consumed_by: None });
        true
    }

    fn live(&self, c: &DeliveredCode, now: i64) -> bool {
        now >= c.received && now - c.received < self.lifetime
    }

    /// The codes not yet consumed that arrived within the last `within` seconds, oldest first.
    pub fn active(&self, now: i64, within: i64) -> Vec<&DeliveredCode> {
        let mut out: Vec<&DeliveredCode> = self
            .codes
            .iter()
            .filter(|c| c.consumed_by.is_none() && self.live(c, now) && now - c.received <= within)
            .collect();
        out.sort_by_key(|c| c.received);
        out
    }

    /// Marks the newest delivery of `code` as consumed by `process`.
    pub fn consume(&mut self, code: &str, now: i64, process: &str) -> Result<(), VerificationError> {
        let lifetime = self.lifetime;
        let entry = match self.codes.iter_mut().rev().find(|c| c.code == code) {
            None => return Err(VerificationError::NotDelivered),
            Some(entry) => entry,
        };
        if entry.consumed_by.is_some() {
            return Err(VerificationError::AlreadyConsumed);
        }
        if now < entry.received || now - entry.received >= lifetime {
            return Err(VerificationError::Expired);
        }
        entry.consumed_by = Some(process.to_string());
        Ok(())
    }

    /// Drops the codes that have expired.
    pub fn prune(&mut self, now: i64) {
        let lifetime = self.lifetime;
        self.codes.retain(|c| now >= c.received && now - c.received < lifetime);
    }

    pub fn len(&self) -> usize {
        self.codes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_four_to_eight_ascii_digits() {
        assert!(is_code("1234"));
        assert!(is_code("12345678"));
        assert!(!is_code("123"));
        assert!(!is_code("123456789"));
        assert!(!is_code("12a456"));
    }

    #[test]
    fn code_after_a_code_word_is_found() {
        assert_eq!(extract_code("Your code is 123456."), Some("123456".to_string()));
        assert_eq!(extract_code("Verification code: 4821 do not share it"), Some("4821".to_string()));
        assert_eq!(extract_code("Your code 98765432 expires soon"), Some("98765432".to_string()));
    }

    #[test]
    fn without_a_code_word_only_a_single_run_counts() {
        assert_eq!(extract_code("Call 555 or 12 for help"), None);
        assert_eq!(extract_code("Codes 1111 and 2222"), None);
        assert_eq!(extract_code("Your login is 7788"), Some("7788".to_string()));
    }

    #[test]
    fn digit_runs_with_letters_or_too_many_digits_are_not_codes() {
        assert_eq!(extract_code("Your code 123456789 expires"), None);
        assert_eq!(extract_code("Order A1234 is ready"), None);
    }

    #[test]
    fn delivered_code_is_active_then_consumed_once() {
        let mut inbox = DeliveredCodes::new(300);
        assert!(inbox.deliver("123456", 1000));
        assert!(!inbox.deliver("123456", 1010));
        assert!(!inbox.deliver("12", 1000));
        assert_eq!(inbox.active(1100, 300).len(), 1);
        assert_eq!(inbox.consume("123456", 1100, "Mail"), Ok(()));
        assert_eq!(inbox.consume("123456", 1100, "Other"), Err(VerificationError::AlreadyConsumed));
        assert_eq!(inbox.consume("999999", 1100, "Mail"), Err(VerificationError::NotDelivered));
        assert!(inbox.active(1100, 300).is_empty());
    }

    #[test]
    fn consume_after_the_lifetime_is_expired() {
        let mut inbox = DeliveredCodes::new(300);
        inbox.deliver("654321", 1000);
        assert_eq!(inbox.consume("654321", 1299, "Mail"), Ok(()));
        let mut late = DeliveredCodes::new(300);
        late.deliver("654321", 1000);
        assert_eq!(late.consume("654321", 1300, "Mail"), Err(VerificationError::Expired));
    }

    #[test]
    fn a_new_arrival_of_a_consumed_code_is_held_and_prune_drops_old_ones() {
        let mut inbox = DeliveredCodes::new(300);
        inbox.deliver("111222", 0);
        inbox.consume("111222", 10, "App").unwrap();
        assert!(inbox.deliver("111222", 500));
        assert_eq!(inbox.len(), 2);
        inbox.prune(700);
        assert_eq!(inbox.len(), 1);
        assert!(!inbox.is_empty());
        assert_eq!(inbox.active(600, 300)[0].received, 500);
    }
}
