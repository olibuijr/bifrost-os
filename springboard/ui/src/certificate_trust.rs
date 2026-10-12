//! Certificate trust editor: the trust choice for each certificate in a chain, and the result they give.
//!
//! A chain is a list of entries, leaf first and root last. Each entry carries the trust the system's own evaluation
//! gives it (`default_trusted`, supplied by the caller) and the user's choice ([`TrustSetting`]). The effective trust
//! of an entry is the user's choice when it is set, else the default. A chain is trusted when every entry is
//! effectively trusted. The model is pure: it evaluates no signature and reads no store; the caller supplies the
//! evaluation and the panel shows and edits the result.

/// Most certificates kept in one chain.
pub const MAX_CHAIN: usize = 16;

/// The user's trust choice for one certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustSetting {
    /// Follow the system evaluation.
    UseDefault,
    /// Trust this certificate whatever the evaluation says.
    AlwaysTrust,
    /// Do not trust this certificate whatever the evaluation says.
    NeverTrust,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainEntry {
    pub label: String,
    /// Trust the system evaluation gives this certificate.
    pub default_trusted: bool,
    pub setting: TrustSetting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustError {
    EmptyChain,
    /// More than [`MAX_CHAIN`] certificates. Holds the count.
    ChainTooLong(usize),
    /// No entry at this index. Holds the index.
    BadIndex(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateTrustPanel {
    chain: Vec<ChainEntry>,
    message: String,
    informative: Option<String>,
}

impl CertificateTrustPanel {
    /// A panel over the chain, with the message shown at its top. The chain must hold 1 to [`MAX_CHAIN`] entries.
    pub fn new(chain: Vec<ChainEntry>, message: &str) -> Result<Self, TrustError> {
        if chain.is_empty() {
            return Err(TrustError::EmptyChain);
        }
        if chain.len() > MAX_CHAIN {
            return Err(TrustError::ChainTooLong(chain.len()));
        }
        Ok(CertificateTrustPanel { chain, message: message.to_string(), informative: None })
    }

    pub fn chain(&self) -> &[ChainEntry] {
        &self.chain
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// The optional informative text under the message.
    pub fn informative_text(&self) -> Option<&str> {
        self.informative.as_deref()
    }

    /// Sets or clears (`None`) the informative text.
    pub fn set_informative_text(&mut self, text: Option<&str>) {
        self.informative = text.map(|t| t.to_string());
    }

    /// Sets the user's trust choice for one certificate.
    pub fn set_setting(&mut self, index: usize, setting: TrustSetting) -> Result<(), TrustError> {
        let entry = self.chain.get_mut(index).ok_or(TrustError::BadIndex(index))?;
        entry.setting = setting;
        Ok(())
    }

    /// The trust of one certificate after the user's choice is applied.
    pub fn effective_trust(&self, index: usize) -> Result<bool, TrustError> {
        self.chain.get(index).map(effective).ok_or(TrustError::BadIndex(index))
    }

    /// `true` when every certificate in the chain is effectively trusted.
    pub fn chain_trusted(&self) -> bool {
        self.chain.iter().all(effective)
    }

    /// Puts every certificate back to [`TrustSetting::UseDefault`].
    pub fn reset_all(&mut self) {
        for entry in &mut self.chain {
            entry.setting = TrustSetting::UseDefault;
        }
    }
}

fn effective(entry: &ChainEntry) -> bool {
    match entry.setting {
        TrustSetting::UseDefault => entry.default_trusted,
        TrustSetting::AlwaysTrust => true,
        TrustSetting::NeverTrust => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, default_trusted: bool) -> ChainEntry {
        ChainEntry { label: label.to_string(), default_trusted, setting: TrustSetting::UseDefault }
    }

    fn chain(n: usize, default_trusted: bool) -> Vec<ChainEntry> {
        (0..n).map(|i| entry(&format!("cert {i}"), default_trusted)).collect()
    }

    #[test]
    fn chain_size_is_one_to_sixteen() {
        assert_eq!(CertificateTrustPanel::new(vec![], "m").unwrap_err(), TrustError::EmptyChain);
        assert_eq!(
            CertificateTrustPanel::new(chain(17, true), "m").unwrap_err(),
            TrustError::ChainTooLong(17)
        );
        assert!(CertificateTrustPanel::new(chain(16, true), "m").is_ok());
    }

    #[test]
    fn default_follows_the_evaluation_until_the_user_chooses() {
        let mut p = CertificateTrustPanel::new(vec![entry("leaf", false)], "m").unwrap();
        assert_eq!(p.effective_trust(0), Ok(false));
        p.set_setting(0, TrustSetting::AlwaysTrust).unwrap();
        assert_eq!(p.effective_trust(0), Ok(true));
    }

    #[test]
    fn never_trust_overrides_a_trusted_evaluation() {
        let mut p = CertificateTrustPanel::new(vec![entry("leaf", true)], "m").unwrap();
        p.set_setting(0, TrustSetting::NeverTrust).unwrap();
        assert_eq!(p.effective_trust(0), Ok(false));
        assert!(!p.chain_trusted());
    }

    #[test]
    fn chain_is_trusted_only_when_every_entry_is() {
        let mut p = CertificateTrustPanel::new(vec![entry("leaf", true), entry("root", true)], "m").unwrap();
        assert!(p.chain_trusted());
        p.set_setting(1, TrustSetting::NeverTrust).unwrap();
        assert!(!p.chain_trusted());
    }

    #[test]
    fn always_trust_can_rescue_an_untrusted_chain() {
        let mut p = CertificateTrustPanel::new(vec![entry("leaf", false), entry("root", false)], "m").unwrap();
        assert!(!p.chain_trusted());
        p.set_setting(0, TrustSetting::AlwaysTrust).unwrap();
        p.set_setting(1, TrustSetting::AlwaysTrust).unwrap();
        assert!(p.chain_trusted());
    }

    #[test]
    fn bad_index_is_reported_and_leaves_the_chain_alone() {
        let mut p = CertificateTrustPanel::new(chain(2, true), "m").unwrap();
        assert_eq!(p.set_setting(2, TrustSetting::NeverTrust), Err(TrustError::BadIndex(2)));
        assert_eq!(p.effective_trust(5), Err(TrustError::BadIndex(5)));
        assert!(p.chain_trusted());
    }

    #[test]
    fn reset_all_returns_the_chain_to_its_evaluation() {
        let mut p = CertificateTrustPanel::new(vec![entry("leaf", true), entry("root", false)], "m").unwrap();
        p.set_setting(0, TrustSetting::NeverTrust).unwrap();
        p.set_setting(1, TrustSetting::AlwaysTrust).unwrap();
        assert!(!p.chain_trusted());
        p.reset_all();
        assert!(!p.chain_trusted());
        assert_eq!(p.effective_trust(0), Ok(true));
        assert_eq!(p.effective_trust(1), Ok(false));
    }

    #[test]
    fn informative_text_can_be_set_and_cleared() {
        let mut p = CertificateTrustPanel::new(chain(1, true), "Check this chain").unwrap();
        assert_eq!(p.message(), "Check this chain");
        assert_eq!(p.informative_text(), None);
        p.set_informative_text(Some("The root is not in the store."));
        assert_eq!(p.informative_text(), Some("The root is not in the store."));
        p.set_informative_text(None);
        assert_eq!(p.informative_text(), None);
    }
}
