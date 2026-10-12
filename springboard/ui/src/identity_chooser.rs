//! Choose identity panel: the list of identities an app offers, the one the user picks, and the button text.
//!
//! An identity is a certificate together with its private key (a key on a token counts). The panel keeps the
//! identities that pass its [`IdentityPolicy`] at a given time, sorted by subject. It selects the first one by
//! default, the way the list opens with its first row chosen. [`IdentityChooser::confirm`] returns the selected
//! identity for the app to use. The model does not sign anything and does not show a sheet.
use crate::certificate_panel::{CertificateInfo, Validity};

/// A certificate and whether its private key is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub certificate: CertificateInfo,
    pub has_private_key: bool,
}

/// What the panel lists. An identity must have a key when `require_private_key` is set, and must be valid at the
/// panel's time unless `allow_expired` is set (a not-yet-valid identity is never listed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityPolicy {
    pub require_private_key: bool,
    pub allow_expired: bool,
}

/// Errors of the chooser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChooserError {
    /// Nothing is listed, so nothing can be confirmed.
    NothingSelected,
    /// No listed identity at this index. Holds the index.
    BadIndex(usize),
}

/// The choose identity panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityChooser {
    listed: Vec<Identity>,
    selected: Option<usize>,
    default_title: String,
    alternate_title: String,
    informative: Option<String>,
}

impl IdentityChooser {
    /// Lists the identities that pass `policy` at `now`, sorted by subject, and selects the first of them.
    pub fn new(identities: Vec<Identity>, policy: IdentityPolicy, now: i64) -> Self {
        let mut listed: Vec<Identity> = identities
            .into_iter()
            .filter(|id| !policy.require_private_key || id.has_private_key)
            .filter(|id| match id.certificate.validity(now) {
                Validity::Valid => true,
                Validity::Expired => policy.allow_expired,
                Validity::NotYetValid => false,
            })
            .collect();
        listed.sort_by(|a, b| a.certificate.subject.cmp(&b.certificate.subject));
        let selected = if listed.is_empty() { None } else { Some(0) };
        IdentityChooser {
            listed,
            selected,
            default_title: "Choose".to_string(),
            alternate_title: "Cancel".to_string(),
            informative: None,
        }
    }

    /// The identities listed, in the order the panel shows them.
    pub fn listed(&self) -> &[Identity] {
        &self.listed
    }

    /// The selected row, if any identity is listed.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Selects a listed identity. An index past the end is `BadIndex` and leaves the selection unchanged.
    pub fn select(&mut self, index: usize) -> Result<(), ChooserError> {
        if index >= self.listed.len() {
            return Err(ChooserError::BadIndex(index));
        }
        self.selected = Some(index);
        Ok(())
    }

    /// The identity the user would confirm, or `NothingSelected` when the list is empty.
    pub fn confirm(&self) -> Result<&Identity, ChooserError> {
        let i = self.selected.ok_or(ChooserError::NothingSelected)?;
        self.listed.get(i).ok_or(ChooserError::NothingSelected)
    }

    pub fn default_button_title(&self) -> &str {
        &self.default_title
    }

    pub fn set_default_button_title(&mut self, title: &str) {
        self.default_title = title.to_string();
    }

    pub fn alternate_button_title(&self) -> &str {
        &self.alternate_title
    }

    pub fn set_alternate_button_title(&mut self, title: &str) {
        self.alternate_title = title.to_string();
    }

    pub fn informative_text(&self) -> Option<&str> {
        self.informative.as_deref()
    }

    /// Sets or clears (`None`) the informative text under the message.
    pub fn set_informative_text(&mut self, text: Option<&str>) {
        self.informative = text.map(|t| t.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(subject: &str, has_key: bool, not_before: i64, not_after: i64) -> Identity {
        Identity {
            certificate: CertificateInfo {
                subject: subject.to_string(),
                issuer: "CN=Root".to_string(),
                serial: "01".to_string(),
                not_before,
                not_after,
            },
            has_private_key: has_key,
        }
    }

    fn policy(require_private_key: bool, allow_expired: bool) -> IdentityPolicy {
        IdentityPolicy { require_private_key, allow_expired }
    }

    fn subjects(c: &IdentityChooser) -> Vec<&str> {
        c.listed().iter().map(|i| i.certificate.subject.as_str()).collect()
    }

    #[test]
    fn identities_without_a_key_are_hidden_when_a_key_is_required() {
        let all = vec![id("CN=Cy", true, 0, 9), id("CN=Bo", false, 0, 9), id("CN=Ana", true, 0, 9)];
        let c = IdentityChooser::new(all.clone(), policy(true, true), 5);
        assert_eq!(subjects(&c), vec!["CN=Ana", "CN=Cy"]);
        let c = IdentityChooser::new(all, policy(false, true), 5);
        assert_eq!(subjects(&c), vec!["CN=Ana", "CN=Bo", "CN=Cy"]);
    }

    #[test]
    fn expired_identities_are_listed_only_when_allowed() {
        let all = vec![id("CN=Old", true, 0, 100), id("CN=New", true, 0, 1000)];
        let hidden = IdentityChooser::new(all.clone(), policy(false, false), 200);
        assert_eq!(subjects(&hidden), vec!["CN=New"]);
        let shown = IdentityChooser::new(all, policy(false, true), 200);
        assert_eq!(subjects(&shown), vec!["CN=New", "CN=Old"]);
    }

    #[test]
    fn a_not_yet_valid_identity_is_never_listed() {
        let all = vec![id("CN=Later", true, 500, 900)];
        let c = IdentityChooser::new(all, policy(false, true), 100);
        assert!(c.listed().is_empty());
        assert_eq!(c.selected(), None);
    }

    #[test]
    fn the_first_identity_is_selected_and_a_bad_index_keeps_the_selection() {
        let all = vec![id("CN=Bo", true, 0, 9), id("CN=Ana", true, 0, 9)];
        let mut c = IdentityChooser::new(all, policy(false, false), 5);
        assert_eq!(c.selected(), Some(0));
        assert_eq!(c.confirm().unwrap().certificate.subject, "CN=Ana");
        assert_eq!(c.select(1), Ok(()));
        assert_eq!(c.confirm().unwrap().certificate.subject, "CN=Bo");
        assert_eq!(c.select(2), Err(ChooserError::BadIndex(2)));
        assert_eq!(c.selected(), Some(1));
    }

    #[test]
    fn confirm_with_nothing_listed_is_refused() {
        let c = IdentityChooser::new(vec![], policy(false, false), 5);
        assert_eq!(c.confirm(), Err(ChooserError::NothingSelected));
    }

    #[test]
    fn button_titles_default_to_choose_and_cancel_and_can_be_changed() {
        let mut c = IdentityChooser::new(vec![], policy(false, false), 5);
        assert_eq!(c.default_button_title(), "Choose");
        assert_eq!(c.alternate_button_title(), "Cancel");
        c.set_default_button_title("Use Certificate");
        c.set_alternate_button_title("Not Now");
        assert_eq!(c.default_button_title(), "Use Certificate");
        assert_eq!(c.alternate_button_title(), "Not Now");
    }

    #[test]
    fn informative_text_can_be_set_and_cleared() {
        let mut c = IdentityChooser::new(vec![], policy(false, false), 5);
        assert_eq!(c.informative_text(), None);
        c.set_informative_text(Some("Pick the identity for this site."));
        assert_eq!(c.informative_text(), Some("Pick the identity for this site."));
        c.set_informative_text(None);
        assert_eq!(c.informative_text(), None);
    }
}
