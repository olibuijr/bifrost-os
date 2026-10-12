//! Certificate panel: the details of each certificate in a chain, and whether each one is valid on a given day.
//!
//! The panel shows the chain's certificates (leaf first, root last), one of them selected. It shows details only:
//! trust is edited in [`crate::certificate_trust`]. A certificate's validity is decided from its not-before and
//! not-after times, in Unix seconds, against a time the caller supplies, so the model reads no clock. A certificate
//! whose subject, issuer or serial number is empty, or whose not-before time is after its not-after time, is
//! malformed and refused. The panel does not parse certificate encodings; the caller supplies the fields.
use crate::certificate_trust::MAX_CHAIN;

/// The fields of one certificate that the panel shows. Times are Unix seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateInfo {
    pub subject: String,
    pub issuer: String,
    pub serial: String,
    pub not_before: i64,
    pub not_after: i64,
}

/// Whether a certificate is valid at a given time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    NotYetValid,
    Valid,
    Expired,
}

impl Validity {
    /// The text the panel shows for this state.
    pub fn text(self) -> &'static str {
        match self {
            Validity::NotYetValid => "Not yet valid",
            Validity::Valid => "Valid",
            Validity::Expired => "Expired",
        }
    }
}

impl CertificateInfo {
    /// `true` when the subject, issuer and serial are present and the validity period is not reversed.
    pub fn is_well_formed(&self) -> bool {
        !self.subject.is_empty()
            && !self.issuer.is_empty()
            && !self.serial.is_empty()
            && self.not_before <= self.not_after
    }

    /// The validity at `now`. Both ends of the period count as valid.
    pub fn validity(&self, now: i64) -> Validity {
        if now < self.not_before {
            Validity::NotYetValid
        } else if now > self.not_after {
            Validity::Expired
        } else {
            Validity::Valid
        }
    }
}

/// Errors of the certificate panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelError {
    EmptyChain,
    /// More than [`MAX_CHAIN`] certificates. Holds the count.
    ChainTooLong(usize),
    /// No certificate at this index. Holds the index.
    BadIndex(usize),
    /// The certificate at this index is malformed. Holds the index.
    Malformed(usize),
}

/// A chain of certificates with one selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificatePanel {
    chain: Vec<CertificateInfo>,
    selected: usize,
}

impl CertificatePanel {
    /// A panel over the chain. It holds 1 to [`MAX_CHAIN`] well-formed certificates and selects the first (the leaf).
    pub fn new(chain: Vec<CertificateInfo>) -> Result<Self, PanelError> {
        if chain.is_empty() {
            return Err(PanelError::EmptyChain);
        }
        if chain.len() > MAX_CHAIN {
            return Err(PanelError::ChainTooLong(chain.len()));
        }
        if let Some(i) = chain.iter().position(|c| !c.is_well_formed()) {
            return Err(PanelError::Malformed(i));
        }
        Ok(CertificatePanel { chain, selected: 0 })
    }

    pub fn chain(&self) -> &[CertificateInfo] {
        &self.chain
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects a certificate. An index past the end is `BadIndex` and leaves the selection unchanged.
    pub fn select(&mut self, index: usize) -> Result<(), PanelError> {
        if index >= self.chain.len() {
            return Err(PanelError::BadIndex(index));
        }
        self.selected = index;
        Ok(())
    }

    /// The validity of one certificate at `now`.
    pub fn status(&self, index: usize, now: i64) -> Result<Validity, PanelError> {
        self.chain.get(index).map(|c| c.validity(now)).ok_or(PanelError::BadIndex(index))
    }

    /// `true` when every certificate in the chain is valid at `now`.
    pub fn chain_valid(&self, now: i64) -> bool {
        self.chain.iter().all(|c| c.validity(now) == Validity::Valid)
    }

    /// The detail rows for one certificate: label and value, in the order the panel shows them.
    pub fn rows(&self, index: usize, now: i64) -> Result<Vec<(&'static str, String)>, PanelError> {
        let cert = self.chain.get(index).ok_or(PanelError::BadIndex(index))?;
        Ok(vec![
            ("Subject", cert.subject.clone()),
            ("Issuer", cert.issuer.clone()),
            ("Serial number", cert.serial.clone()),
            ("Status", cert.validity(now).text().to_string()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert(subject: &str, not_before: i64, not_after: i64) -> CertificateInfo {
        CertificateInfo {
            subject: subject.to_string(),
            issuer: "CN=Root".to_string(),
            serial: "01AF".to_string(),
            not_before,
            not_after,
        }
    }

    #[test]
    fn validity_includes_both_ends_of_the_period() {
        let c = cert("CN=Ana", 100, 200);
        assert_eq!(c.validity(99), Validity::NotYetValid);
        assert_eq!(c.validity(100), Validity::Valid);
        assert_eq!(c.validity(200), Validity::Valid);
        assert_eq!(c.validity(201), Validity::Expired);
    }

    #[test]
    fn panel_refuses_an_empty_chain_a_long_chain_and_a_malformed_certificate() {
        assert_eq!(CertificatePanel::new(vec![]), Err(PanelError::EmptyChain));
        let long: Vec<CertificateInfo> = (0..17).map(|_| cert("CN=A", 0, 10)).collect();
        assert_eq!(CertificatePanel::new(long), Err(PanelError::ChainTooLong(17)));
        let mut no_serial = cert("CN=A", 0, 10);
        no_serial.serial = String::new();
        assert_eq!(CertificatePanel::new(vec![cert("CN=A", 0, 10), no_serial]), Err(PanelError::Malformed(1)));
        assert_eq!(CertificatePanel::new(vec![cert("CN=A", 20, 10)]), Err(PanelError::Malformed(0)));
    }

    #[test]
    fn chain_is_valid_only_when_every_certificate_is() {
        let panel = CertificatePanel::new(vec![cert("CN=Leaf", 0, 500), cert("CN=Root", 0, 100)]).unwrap();
        assert_eq!(panel.status(0, 300), Ok(Validity::Valid));
        assert_eq!(panel.status(1, 300), Ok(Validity::Expired));
        assert!(!panel.chain_valid(300));
        assert!(panel.chain_valid(50));
    }

    #[test]
    fn selection_moves_and_a_bad_index_keeps_it() {
        let mut panel = CertificatePanel::new(vec![cert("CN=Leaf", 0, 9), cert("CN=Root", 0, 9)]).unwrap();
        assert_eq!(panel.selected(), 0);
        assert_eq!(panel.select(1), Ok(()));
        assert_eq!(panel.selected(), 1);
        assert_eq!(panel.select(2), Err(PanelError::BadIndex(2)));
        assert_eq!(panel.selected(), 1);
    }

    #[test]
    fn rows_show_the_details_and_the_status_at_the_given_time() {
        let panel = CertificatePanel::new(vec![cert("CN=Ana", 100, 200)]).unwrap();
        let rows = panel.rows(0, 300).unwrap();
        assert_eq!(
            rows,
            vec![
                ("Subject", "CN=Ana".to_string()),
                ("Issuer", "CN=Root".to_string()),
                ("Serial number", "01AF".to_string()),
                ("Status", "Expired".to_string()),
            ]
        );
        assert_eq!(panel.rows(0, 150).unwrap()[3].1, "Valid");
        assert_eq!(panel.rows(0, 50).unwrap()[3].1, "Not yet valid");
        assert_eq!(panel.rows(4, 50), Err(PanelError::BadIndex(4)));
    }
}
