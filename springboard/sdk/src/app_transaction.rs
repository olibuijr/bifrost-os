//! The app transaction (`AppTransaction`): the record of how the customer got this app. It holds the bundle identifier, the
//! version the customer obtained, the version first obtained, the day of that purchase, the store environment, the
//! SHA-256 digest of the package the store delivered, and the day the purchase was revoked, if it was. BifrOSt does not
//! sign these records: the installer keeps them in apps/installed.json and checks the digest against the package it
//! installed. Days count from 1970-01-01. Pure: this module converts the record to and from one tab-separated line and
//! does not read or write any file.

/// Where the store that delivered the app runs (`AppTransaction.environment`). `Production` is the PC App Store catalogue;
/// `Sandbox` marks a package delivered for testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Production,
    Sandbox,
}

impl Environment {
    pub fn as_str(self) -> &'static str {
        match self {
            Environment::Production => "production",
            Environment::Sandbox => "sandbox",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "production" => Some(Environment::Production),
            "sandbox" => Some(Environment::Sandbox),
            _ => None,
        }
    }
}

/// The app transaction of one installed app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppTransaction {
    /// The identifier the store gave this download (`appTransactionID`).
    pub id: String,
    /// The bundle identifier of the app, e.g. "is.olibuijr.notes".
    pub bundle_id: String,
    /// The version the app runs now (`appVersion`).
    pub app_version: String,
    /// The version the customer first obtained (`originalAppVersion`).
    pub original_version: String,
    /// The day the customer first obtained the app (`originalPurchaseDate`).
    pub original_purchase: i64,
    pub environment: Environment,
    /// The SHA-256 digest of the package the store delivered, in hexadecimal.
    pub package_sha256: String,
    /// The earliest day the purchase was revoked (`revocationDate`), if it was.
    pub revoked: Option<i64>,
}

impl AppTransaction {
    /// Whether the app runs a newer version than the one the customer first obtained.
    pub fn updated_since_purchase(&self) -> bool {
        self.app_version != self.original_version
    }

    /// Records a revocation on `day`. The earliest revocation day stands; a later one does not move it.
    pub fn revoke(&mut self, day: i64) {
        self.revoked = Some(self.revoked.map_or(day, |r| r.min(day)));
    }

    /// Whether the purchase is revoked on `day`.
    pub fn is_revoked_on(&self, day: i64) -> bool {
        self.revoked.is_some_and(|r| r <= day)
    }

    /// Whether `sha256_hex` is the digest of the package the store delivered. Hexadecimal case does not matter.
    pub fn verifies_package(&self, sha256_hex: &str) -> bool {
        self.package_sha256.eq_ignore_ascii_case(sha256_hex)
    }

    /// The record as one line of eight tab-separated fields; a revocation of "-" means none. None when a text field
    /// holds a tab or a line break, which would split the line.
    pub fn to_line(&self) -> Option<String> {
        let texts = [&self.id, &self.bundle_id, &self.app_version, &self.original_version, &self.package_sha256];
        if texts.iter().any(|t| t.contains(['\t', '\n', '\r'])) {
            return None;
        }
        let revoked = match self.revoked {
            Some(day) => day.to_string(),
            None => "-".to_string(),
        };
        let fields = [
            self.id.clone(),
            self.bundle_id.clone(),
            self.app_version.clone(),
            self.original_version.clone(),
            self.original_purchase.to_string(),
            self.environment.as_str().to_string(),
            self.package_sha256.clone(),
            revoked,
        ];
        Some(fields.join("\t"))
    }

    /// Reads a line written by [`AppTransaction::to_line`]. None when the line does not have eight fields or a field
    /// does not parse.
    pub fn parse_line(line: &str) -> Option<Self> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 8 {
            return None;
        }
        let revoked = if f[7] == "-" { None } else { Some(f[7].parse::<i64>().ok()?) };
        Some(AppTransaction {
            id: f[0].to_string(),
            bundle_id: f[1].to_string(),
            app_version: f[2].to_string(),
            original_version: f[3].to_string(),
            original_purchase: f[4].parse::<i64>().ok()?,
            environment: Environment::parse(f[5])?,
            package_sha256: f[6].to_string(),
            revoked,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AppTransaction {
        AppTransaction {
            id: "t1".to_string(),
            bundle_id: "is.olibuijr.notes".to_string(),
            app_version: "2.1.0".to_string(),
            original_version: "2.0.0".to_string(),
            original_purchase: 20000,
            environment: Environment::Production,
            package_sha256: "abc123".to_string(),
            revoked: None,
        }
    }

    #[test]
    fn a_record_is_written_as_eight_tab_separated_fields_and_read_back() {
        let t = sample();
        let line = t.to_line().expect("no tabs in the fields");
        assert_eq!(line, "t1\tis.olibuijr.notes\t2.1.0\t2.0.0\t20000\tproduction\tabc123\t-");
        assert_eq!(AppTransaction::parse_line(&line), Some(t));
    }

    #[test]
    fn a_revoked_record_keeps_its_revocation_day_through_a_round_trip() {
        let mut t = sample();
        t.revoke(30);
        let line = t.to_line().unwrap();
        assert!(line.ends_with("\t30"));
        assert_eq!(AppTransaction::parse_line(&line).and_then(|r| r.revoked), Some(30));
    }

    #[test]
    fn a_line_with_the_wrong_field_count_or_a_bad_field_is_refused() {
        assert!(AppTransaction::parse_line("a\tb").is_none());
        let bad_day = "t1\tis.olibuijr.notes\t2.1.0\t2.0.0\tsoon\tproduction\tabc123\t-";
        assert!(AppTransaction::parse_line(bad_day).is_none());
        let bad_env = "t1\tis.olibuijr.notes\t2.1.0\t2.0.0\t20000\tlive\tabc123\t-";
        assert!(AppTransaction::parse_line(bad_env).is_none());
    }

    #[test]
    fn a_text_field_with_a_tab_cannot_be_written() {
        let mut t = sample();
        t.bundle_id = "is.olibuijr\tnotes".to_string();
        assert!(t.to_line().is_none());
    }

    #[test]
    fn the_app_has_been_updated_only_when_its_version_differs_from_the_first_one() {
        let mut t = sample();
        assert!(t.updated_since_purchase(), "2.1.0 runs after the first 2.0.0");
        t.app_version = "2.0.0".to_string();
        assert!(!t.updated_since_purchase());
    }

    #[test]
    fn the_earliest_revocation_day_stands() {
        let mut t = sample();
        t.revoke(40);
        t.revoke(30);
        t.revoke(50);
        assert_eq!(t.revoked, Some(30));
        assert!(!t.is_revoked_on(29));
        assert!(t.is_revoked_on(30));
    }

    #[test]
    fn the_package_digest_is_compared_without_regard_to_case() {
        let t = sample();
        assert!(t.verifies_package("ABC123"));
        assert!(!t.verifies_package("abc124"));
    }
}
