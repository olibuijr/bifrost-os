/// The marker is durable on the ESP. Never retry an unaccepted generation.
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    Current,
    Attempt,
    Previous,
}
pub fn choose(marker: Option<&str>) -> Choice {
    match marker {
        None => Choice::Current,
        Some("0") => Choice::Attempt,
        Some(_) => Choice::Previous,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_attempt_then_fallback() {
        assert_eq!(choose(None), Choice::Current);
        assert_eq!(choose(Some("0")), Choice::Attempt);
        assert_eq!(choose(Some("1")), Choice::Previous);
        assert_eq!(choose(Some("corrupt")), Choice::Previous);
    }
}
