//! Launch assessment: whether a program may run, and whether the person is asked first (Settings > Privacy & Security >
//! "Allow apps from", and the "downloaded from the internet" question on the first launch).
//!
//! The assessment is pure. The caller describes a [`Candidate`]: where the program came from ([`Source`]), the code
//! identity it read from the program ([`crate::codereq::Identity`]) and whether the signature still matches the
//! files. [`Policy::assess`] answers [`Verdict::Allow`], [`Verdict::Confirm`] (ask, then [`Policy::open_anyway`]) or
//! [`Verdict::Block`], with a [`Reason`] whose text names the program. Programs the store installed and programs of the
//! system are always allowed, since the store checks a package when it installs it. Only a program that arrived by a
//! download or an attachment (a [`crate::quarantine::Quarantine`] record) is assessed against the setting.
//!
//! "Open anyway" is remembered per program and per code: the key holds the path and the code directory hash, so a
//! program that is replaced by other code is asked about again. The remembered keys are saved as lines of text by
//! [`Policy::overrides_to_text`] and read back by [`Policy::with_overrides_text`]. The module reads no file and no
//! signature; BifrOSt does not sign code yet, so an identity comes from the package manifest (see `codereq`).

use crate::codereq::Identity;
use crate::quarantine::Quarantine;
use std::collections::BTreeSet;

/// Which programs the person lets run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// Only programs installed by the store.
    StoreOnly,
    /// Store programs, and downloaded programs signed by a developer with a team identifier.
    StoreAndIdentified,
    /// Any program, after the first-launch question for downloads.
    Anywhere,
}

impl Setting {
    /// The word saved in the settings file.
    pub fn label(self) -> &'static str {
        match self {
            Setting::StoreOnly => "store",
            Setting::StoreAndIdentified => "store-and-identified",
            Setting::Anywhere => "anywhere",
        }
    }

    pub fn parse(text: &str) -> Option<Setting> {
        match text.trim() {
            "store" => Some(Setting::StoreOnly),
            "store-and-identified" => Some(Setting::StoreAndIdentified),
            "anywhere" => Some(Setting::Anywhere),
            _ => None,
        }
    }
}

/// How the program got onto this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Installed from the store (the package was verified on install).
    Store,
    /// Part of the system.
    System,
    /// Made or copied here with no quarantine record.
    Local,
    /// Received from elsewhere, with the record the receiving agent wrote.
    Received(Quarantine),
}

/// What the assessment needs to know about one program.
#[derive(Clone, Debug)]
pub struct Candidate<'a> {
    /// The program's path, as the person sees it.
    pub path: &'a str,
    /// The name shown in the question.
    pub name: &'a str,
    pub source: Source,
    pub identity: &'a Identity,
    /// False when the files no longer match the signature that was recorded.
    pub signature_intact: bool,
}

/// Why the answer is not a plain yes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Downloaded or received; the first launch asks. `from` is the agent that received it.
    Received { from: String },
    /// The setting wants an identified developer and the program has none.
    NotIdentified,
    /// The setting allows the store only.
    NotFromStore,
    /// The files changed after they were signed.
    Modified,
}

impl Reason {
    /// The sentence shown to the person.
    pub fn explain(&self, name: &str) -> String {
        match self {
            Reason::Received { from } if from.is_empty() => {
                format!("\"{name}\" was received from outside this computer. Are you sure you want to open it?")
            }
            Reason::Received { from } => {
                format!("\"{name}\" was received by {from}. Are you sure you want to open it?")
            }
            Reason::NotIdentified => format!("\"{name}\" cannot be opened because its developer cannot be identified."),
            Reason::NotFromStore => format!("\"{name}\" cannot be opened because it did not come from the App Store."),
            Reason::Modified => format!("\"{name}\" cannot be opened because it has been changed since it was signed."),
        }
    }
}

/// The answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Ask the person; on yes call [`Policy::open_anyway`].
    Confirm(Reason),
    /// Refuse. Only the person's explicit "open anyway" in Settings lifts it.
    Block(Reason),
}

/// The setting and the programs the person said to open anyway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    setting: Setting,
    overrides: BTreeSet<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { setting: Setting::StoreAndIdentified, overrides: BTreeSet::new() }
    }
}

/// The key of one program's override: its path and the hash of its code. A program with no recorded hash has the
/// empty hash, so the override covers whatever is at that path until a hash is recorded.
pub fn override_key(path: &str, identity: &Identity) -> String {
    format!("{}\t{}", path, identity.code_directory_hash.as_deref().unwrap_or(""))
}

impl Policy {
    pub fn new(setting: Setting) -> Policy {
        Policy { setting, overrides: BTreeSet::new() }
    }

    pub fn setting(&self) -> Setting {
        self.setting
    }

    pub fn set_setting(&mut self, setting: Setting) {
        self.setting = setting;
    }

    /// Remembers that the person said to open this program anyway.
    pub fn open_anyway(&mut self, c: &Candidate) {
        self.overrides.insert(override_key(c.path, c.identity));
    }

    /// Forgets the override for the program. Returns whether there was one.
    pub fn forget(&mut self, c: &Candidate) -> bool {
        self.overrides.remove(&override_key(c.path, c.identity))
    }

    pub fn has_override(&self, c: &Candidate) -> bool {
        self.overrides.contains(&override_key(c.path, c.identity))
    }

    /// The assessment.
    ///
    /// 1. A program whose signature no longer matches is blocked, whatever else is true; an override does not lift it,
    ///    because the override was given for the code that has since changed.
    /// 2. A program the person said to open anyway is allowed.
    /// 3. Store and system programs, and local programs with no record, are allowed.
    /// 4. A received program is assessed against the setting.
    pub fn assess(&self, c: &Candidate) -> Verdict {
        if !c.signature_intact {
            return Verdict::Block(Reason::Modified);
        }
        if self.has_override(c) {
            return Verdict::Allow;
        }
        let record = match &c.source {
            Source::Store | Source::System | Source::Local => return Verdict::Allow,
            Source::Received(record) => record,
        };
        let identified = c.identity.signed && c.identity.team_id.as_deref().map_or(false, |t| !t.is_empty());
        let received = Reason::Received { from: record.agent_name.clone() };
        match self.setting {
            Setting::StoreOnly => Verdict::Block(Reason::NotFromStore),
            Setting::StoreAndIdentified if !identified => Verdict::Block(Reason::NotIdentified),
            Setting::StoreAndIdentified | Setting::Anywhere => Verdict::Confirm(received),
        }
    }

    /// The overrides as text: one key per line. A key holds a tab between the path and the hash; a path with a line
    /// break cannot be saved and is left out.
    pub fn overrides_to_text(&self) -> String {
        let mut out = String::new();
        for key in &self.overrides {
            if !key.contains('\n') && !key.contains('\r') {
                out.push_str(key);
                out.push('\n');
            }
        }
        out
    }

    /// This policy with the overrides read from text written by [`Policy::overrides_to_text`]. Lines without the tab
    /// are skipped.
    pub fn with_overrides_text(mut self, text: &str) -> Policy {
        for line in text.lines() {
            if line.contains('\t') {
                self.overrides.insert(line.to_string());
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quarantine::QuarantineKind;

    fn identity(signed: bool, team: Option<&str>, hash: Option<&str>) -> Identity {
        Identity {
            signed,
            team_id: team.map(str::to_string),
            code_directory_hash: hash.map(str::to_string),
            ..Identity::default()
        }
    }

    fn received() -> Source {
        Source::Received(Quarantine::new(QuarantineKind::WebDownload, "org.example.browser", "Browser", 100))
    }

    fn cand<'a>(path: &'a str, source: Source, id: &'a Identity) -> Candidate<'a> {
        Candidate { path, name: "Tool", source, identity: id, signature_intact: true }
    }

    #[test]
    fn store_system_and_local_programs_are_allowed_under_every_setting() {
        let id = identity(false, None, None);
        for setting in [Setting::StoreOnly, Setting::StoreAndIdentified, Setting::Anywhere] {
            let p = Policy::new(setting);
            for source in [Source::Store, Source::System, Source::Local] {
                assert_eq!(p.assess(&cand("/apps/tool", source, &id)), Verdict::Allow);
            }
        }
    }

    #[test]
    fn a_received_program_asks_under_anywhere_and_names_the_agent() {
        let id = identity(false, None, None);
        let v = Policy::new(Setting::Anywhere).assess(&cand("/dl/tool", received(), &id));
        assert_eq!(v, Verdict::Confirm(Reason::Received { from: "Browser".into() }));
        let Verdict::Confirm(r) = v else { panic!() };
        assert!(r.explain("Tool").contains("received by Browser"));
    }

    #[test]
    fn identified_only_blocks_an_unsigned_download_and_asks_for_a_signed_one() {
        let p = Policy::new(Setting::StoreAndIdentified);
        let unsigned = identity(false, None, None);
        assert_eq!(p.assess(&cand("/dl/tool", received(), &unsigned)), Verdict::Block(Reason::NotIdentified));
        let no_team = identity(true, Some(""), None);
        assert_eq!(p.assess(&cand("/dl/tool", received(), &no_team)), Verdict::Block(Reason::NotIdentified));
        let signed = identity(true, Some("ABCDE12345"), Some("aa"));
        assert!(matches!(p.assess(&cand("/dl/tool", received(), &signed)), Verdict::Confirm(Reason::Received { .. })));
    }

    #[test]
    fn store_only_blocks_every_download() {
        let p = Policy::new(Setting::StoreOnly);
        let signed = identity(true, Some("ABCDE12345"), Some("aa"));
        assert_eq!(p.assess(&cand("/dl/tool", received(), &signed)), Verdict::Block(Reason::NotFromStore));
    }

    #[test]
    fn open_anyway_lifts_a_block_for_that_code_only() {
        let mut p = Policy::new(Setting::StoreOnly);
        let first = identity(false, None, Some("h1"));
        let c = cand("/dl/tool", received(), &first);
        assert!(matches!(p.assess(&c), Verdict::Block(_)));
        p.open_anyway(&c);
        assert_eq!(p.assess(&c), Verdict::Allow);
        // the same path with other code is asked about again
        let replaced = identity(false, None, Some("h2"));
        assert!(matches!(p.assess(&cand("/dl/tool", received(), &replaced)), Verdict::Block(_)));
        assert!(p.forget(&c));
        assert!(!p.forget(&c));
        assert!(matches!(p.assess(&c), Verdict::Block(_)));
    }

    #[test]
    fn a_changed_program_is_blocked_even_when_it_was_opened_anyway() {
        let mut p = Policy::new(Setting::Anywhere);
        let id = identity(true, Some("T"), Some("h1"));
        let mut c = cand("/apps/tool", Source::Store, &id);
        p.open_anyway(&c);
        c.signature_intact = false;
        assert_eq!(p.assess(&c), Verdict::Block(Reason::Modified));
        assert!(Reason::Modified.explain("Tool").contains("changed since it was signed"));
    }

    #[test]
    fn the_setting_and_the_overrides_survive_a_round_trip_through_text() {
        assert_eq!(Setting::parse(Setting::StoreAndIdentified.label()), Some(Setting::StoreAndIdentified));
        assert_eq!(Setting::parse("nowhere"), None);
        let mut p = Policy::new(Setting::Anywhere);
        let id = identity(false, None, Some("h1"));
        p.open_anyway(&cand("/dl/tool", received(), &id));
        p.open_anyway(&cand("/dl/bad\nname", received(), &id));
        let text = p.overrides_to_text();
        assert_eq!(text, "/dl/tool\th1\n");
        let back = Policy::new(Setting::Anywhere).with_overrides_text(&format!("junk\n{text}"));
        assert_eq!(back, Policy { setting: Setting::Anywhere, overrides: [String::from("/dl/tool\th1")].into_iter().collect() });
    }
}
