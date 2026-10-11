//! Caller identification and blocking: a pure model of a call directory.
//!
//! A directory holds two lists of phone numbers. Blocking entries stop a call from that number. Identification
//! entries attach a label that the caller's name shows. A request to load entries starts with `begin_request`:
//! a full (non-incremental) request replaces both lists, and an incremental request keeps them. Within one
//! request each list takes numbers in strictly ascending order, so a number that is not larger than the last one
//! added is refused. A number must be between 1 and `PHONE_NUMBER_MAX`, which holds a country calling code
//! followed by the digits. Nothing here touches telephony; the caller supplies the numbers.

/// The largest phone number a directory accepts (a 15-digit number).
pub const PHONE_NUMBER_MAX: i64 = 9_999_999_999_999_999;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectoryError {
    /// The number is 0 or above `PHONE_NUMBER_MAX`.
    OutOfRange(i64),
    /// The number is not larger than the last number added to the same list in this request.
    NotAscending { number: i64, last: i64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallDirectory {
    blocking: Vec<i64>,
    identification: Vec<(i64, String)>,
    last_blocking: Option<i64>,
    last_identification: Option<i64>,
}

fn check(number: i64, last: Option<i64>) -> Result<(), DirectoryError> {
    if number < 1 || number > PHONE_NUMBER_MAX {
        return Err(DirectoryError::OutOfRange(number));
    }
    match last {
        Some(prev) if number <= prev => Err(DirectoryError::NotAscending { number, last: prev }),
        _ => Ok(()),
    }
}

impl CallDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a request. A full request clears both lists; an incremental one keeps them. The ascending check
    /// restarts for both lists.
    pub fn begin_request(&mut self, incremental: bool) {
        self.last_blocking = None;
        self.last_identification = None;
        if !incremental {
            self.blocking.clear();
            self.identification.clear();
        }
    }

    pub fn add_blocking(&mut self, number: i64) -> Result<(), DirectoryError> {
        check(number, self.last_blocking)?;
        self.blocking.push(number);
        self.last_blocking = Some(number);
        Ok(())
    }

    pub fn add_identification(&mut self, number: i64, label: &str) -> Result<(), DirectoryError> {
        check(number, self.last_identification)?;
        self.identification.push((number, label.to_string()));
        self.last_identification = Some(number);
        Ok(())
    }

    /// Removes the blocking entry for `number`. Returns whether one was removed.
    pub fn remove_blocking(&mut self, number: i64) -> bool {
        let before = self.blocking.len();
        self.blocking.retain(|&n| n != number);
        self.blocking.len() != before
    }

    /// Removes the identification entry for `number`. Returns whether one was removed.
    pub fn remove_identification(&mut self, number: i64) -> bool {
        let before = self.identification.len();
        self.identification.retain(|e| e.0 != number);
        self.identification.len() != before
    }

    pub fn remove_all_blocking(&mut self) {
        self.blocking.clear();
    }

    pub fn remove_all_identification(&mut self) {
        self.identification.clear();
    }

    /// Whether calls from `number` are blocked.
    pub fn blocks(&self, number: i64) -> bool {
        self.blocking.contains(&number)
    }

    /// The label for `number`, if an identification entry exists.
    pub fn label_for(&self, number: i64) -> Option<&str> {
        self.identification
            .iter()
            .find(|e| e.0 == number)
            .map(|e| e.1.as_str())
    }

    pub fn blocking_len(&self) -> usize {
        self.blocking.len()
    }

    pub fn identification_len(&self) -> usize {
        self.identification.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_numbers_must_ascend_within_a_request() {
        let mut d = CallDirectory::new();
        d.begin_request(false);
        assert_eq!(d.add_blocking(5), Ok(()));
        assert_eq!(
            d.add_blocking(3),
            Err(DirectoryError::NotAscending { number: 3, last: 5 })
        );
        assert_eq!(
            d.add_blocking(5),
            Err(DirectoryError::NotAscending { number: 5, last: 5 })
        );
        assert_eq!(d.blocking_len(), 1);
    }

    #[test]
    fn full_request_replaces_entries_and_restarts_the_order() {
        let mut d = CallDirectory::new();
        d.begin_request(false);
        d.add_blocking(10).unwrap();
        d.begin_request(false);
        assert!(!d.blocks(10));
        assert_eq!(d.add_blocking(2), Ok(()));
        assert!(d.blocks(2));
    }

    #[test]
    fn incremental_request_keeps_existing_entries() {
        let mut d = CallDirectory::new();
        d.begin_request(false);
        d.add_blocking(10).unwrap();
        d.begin_request(true);
        assert!(d.blocks(10));
        assert_eq!(d.add_blocking(4), Ok(()));
        assert!(d.blocks(4));
        assert_eq!(d.blocking_len(), 2);
    }

    #[test]
    fn numbers_outside_the_range_are_refused() {
        let mut d = CallDirectory::new();
        assert_eq!(d.add_blocking(0), Err(DirectoryError::OutOfRange(0)));
        assert_eq!(
            d.add_blocking(PHONE_NUMBER_MAX + 1),
            Err(DirectoryError::OutOfRange(PHONE_NUMBER_MAX + 1))
        );
        assert_eq!(d.add_blocking(PHONE_NUMBER_MAX), Ok(()));
    }

    #[test]
    fn identification_labels_are_looked_up_by_number() {
        let mut d = CallDirectory::new();
        d.begin_request(false);
        d.add_identification(4420000, "Pizza").unwrap();
        assert_eq!(d.label_for(4420000), Some("Pizza"));
        assert_eq!(d.label_for(1), None);
    }

    #[test]
    fn removing_one_list_leaves_the_other() {
        let mut d = CallDirectory::new();
        d.begin_request(false);
        d.add_blocking(7).unwrap();
        d.add_identification(7, "Work").unwrap();
        d.remove_all_identification();
        assert_eq!(d.label_for(7), None);
        assert!(d.blocks(7));
        assert!(d.remove_blocking(7));
        assert!(!d.remove_blocking(7));
        assert!(!d.blocks(7));
    }
}
