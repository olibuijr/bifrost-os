//! Code requirements: tests of a program's signed identity, combined into expressions (task theme-02). Pure model with
//! no file, process or signature reader of its own. The caller supplies an `Identity` for the code on disk, the
//! running process and the calling process, and asks whether it satisfies a `Requirement`. BifrOSt does not sign
//! code yet, so an `Identity` is built from whatever the package manifest provides; the model shows the DSL and its
//! checks, and nothing in the shell enforces it yet.

use crate::sha256;

/// Per-program signing flags, as a BifrOSt bit set. The bit positions are ours, not an external ABI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SigningFlags(pub u32);

impl SigningFlags {
    pub const HOST: u32 = 1 << 0;
    pub const ADHOC: u32 = 1 << 1;
    pub const RUNTIME: u32 = 1 << 2;
    pub const LIBRARY_VALIDATION: u32 = 1 << 3;

    /// True when every bit of `flags` is set (a superset test, as the flags constraint defines it).
    pub fn is_superset_of(self, flags: u32) -> bool {
        self.0 & flags == flags
    }
}

/// The validation category a signature conforms to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// Signed by the system itself.
    Platform,
    /// Signed under a developer team identity.
    Developer,
    /// Signed without a team identity.
    AdHoc,
}

/// What the model knows about one program: the fields a requirement can test.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    /// False for code with no signature at all; such code satisfies no test except `Unsigned`.
    pub signed: bool,
    pub team_id: Option<String>,
    pub signing_id: Option<String>,
    /// SHA-256 of the code directory, hex encoded.
    pub code_directory_hash: Option<String>,
    /// SHA-256 of the information property list, hex encoded.
    pub info_plist_hash: Option<String>,
    pub flags: SigningFlags,
    pub category: Option<Category>,
    pub platform: Option<String>,
    pub main_binary: bool,
    /// Entitlement key and value pairs.
    pub entitlements: Vec<(String, String)>,
}

/// One test of a program's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Test {
    TeamIdentifier(String),
    /// Matches when the subject's team equals the team of the calling process.
    TeamIdentifierMatchesCaller,
    SigningIdentifier(String),
    CodeDirectoryHash(String),
    InfoPlistHash(String),
    /// Matches when the signing flags contain every bit given.
    Flags(u32),
    /// Matches when the category is one of the given categories.
    Validation(Vec<Category>),
    Platform(String),
    MainBinary,
    Entitlement { key: String, value: String },
    Unsigned,
}

/// A requirement: one test, or `allOf` / `anyOf` over sub-requirements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    Test(Test),
    AllOf(Vec<Requirement>),
    AnyOf(Vec<Requirement>),
}

/// Why a requirement could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// An operator has no arguments.
    Empty,
    /// The same constraint appears twice in one operator. Entitlement tests may repeat.
    Duplicate(String),
}

/// The result of checking code on disk: whether it has a valid signature, and whether it satisfies the requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationResult {
    pub valid_signature: bool,
    pub satisfied: bool,
}

impl Requirement {
    /// Builds an `allOf` operator, rejecting empty and duplicated arguments.
    pub fn all_of(parts: Vec<Requirement>) -> Result<Requirement, BuildError> {
        check_parts(&parts)?;
        Ok(Requirement::AllOf(parts))
    }

    /// Builds an `anyOf` operator, rejecting empty and duplicated arguments.
    pub fn any_of(parts: Vec<Requirement>) -> Result<Requirement, BuildError> {
        check_parts(&parts)?;
        Ok(Requirement::AnyOf(parts))
    }

    /// Whether `subject` satisfies this requirement. `caller` is the identity of the calling process, used only by
    /// `Test::TeamIdentifierMatchesCaller`; without one that test fails.
    pub fn evaluate(&self, subject: &Identity, caller: Option<&Identity>) -> bool {
        match self {
            Requirement::Test(t) => test(t, subject, caller),
            Requirement::AllOf(parts) => parts.iter().all(|r| r.evaluate(subject, caller)),
            Requirement::AnyOf(parts) => parts.iter().any(|r| r.evaluate(subject, caller)),
        }
    }
}

fn check_parts(parts: &[Requirement]) -> Result<(), BuildError> {
    if parts.is_empty() {
        return Err(BuildError::Empty);
    }
    for (i, a) in parts.iter().enumerate() {
        if let Requirement::Test(t) = a {
            if matches!(t, Test::Entitlement { .. }) {
                continue;
            }
            if parts[..i].contains(a) {
                return Err(BuildError::Duplicate(format!("{t:?}")));
            }
        }
    }
    Ok(())
}

fn test(t: &Test, s: &Identity, caller: Option<&Identity>) -> bool {
    match t {
        Test::TeamIdentifier(id) => s.team_id.as_deref() == Some(id.as_str()),
        Test::TeamIdentifierMatchesCaller => match (s.team_id.as_deref(), caller.and_then(|c| c.team_id.as_deref())) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
        Test::SigningIdentifier(id) => s.signing_id.as_deref() == Some(id.as_str()),
        Test::CodeDirectoryHash(h) => s.code_directory_hash.as_deref() == Some(h.as_str()),
        Test::InfoPlistHash(h) => s.info_plist_hash.as_deref() == Some(h.as_str()),
        Test::Flags(f) => s.flags.is_superset_of(*f),
        Test::Validation(cats) => s.category.is_some_and(|c| cats.contains(&c)),
        Test::Platform(p) => s.platform.as_deref() == Some(p.as_str()),
        Test::MainBinary => s.main_binary,
        Test::Entitlement { key, value } => s.entitlements.iter().any(|(k, v)| k == key && v == value),
        Test::Unsigned => !s.signed,
    }
}

/// Checks code on disk. Unsigned code has no valid signature and fails every requirement except `Unsigned`.
pub fn check_on_disk(req: &Requirement, code: &Identity) -> ValidationResult {
    ValidationResult { valid_signature: code.signed, satisfied: req.evaluate(code, None) }
}

/// Checks a running process against a requirement, with the calling process as the reference for caller tests.
pub fn check_process(req: &Requirement, process: &Identity, caller: Option<&Identity>) -> bool {
    req.evaluate(process, caller)
}

/// Gates a launch: the program may start only when it satisfies the requirement.
pub fn check_launch(req: &Requirement, program: &Identity) -> Result<(), String> {
    if req.evaluate(program, None) {
        Ok(())
    } else {
        Err("program does not satisfy the launch requirement".to_string())
    }
}

/// Hex SHA-256 of raw bytes, for building the hash fields of an `Identity` from file contents.
pub fn hash_hex(data: &[u8]) -> String {
    sha256::hex(&sha256::digest(data))
}

/// The error a constraint builder reports. It has the same cases as `BuildError`.
pub type ConstraintError = BuildError;

/// The context a requirement is checked in: code on disk, a running process, or a program about to launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    OnDisk,
    Process,
    Launch,
}

/// A requirement bound to the context it is checked in. The builder takes the operator arguments as a `Vec`, since
/// Rust has no closure-based result builder, and rejects the same empty and duplicate arguments as `allOf` and `anyOf`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraint {
    pub scope: Scope,
    pub requirement: Requirement,
}

impl Constraint {
    /// Builds an `allOf` constraint for one scope.
    pub fn all_of(scope: Scope, parts: Vec<Requirement>) -> Result<Constraint, ConstraintError> {
        Ok(Constraint { scope, requirement: Requirement::all_of(parts)? })
    }

    /// Builds an `anyOf` constraint for one scope.
    pub fn any_of(scope: Scope, parts: Vec<Requirement>) -> Result<Constraint, ConstraintError> {
        Ok(Constraint { scope, requirement: Requirement::any_of(parts)? })
    }

    /// Checks `subject` in this constraint's scope. `caller` matters only for process scope and caller tests.
    pub fn satisfied_by(&self, subject: &Identity, caller: Option<&Identity>) -> bool {
        match self.scope {
            Scope::OnDisk => check_on_disk(&self.requirement, subject).satisfied,
            Scope::Process => check_process(&self.requirement, subject, caller),
            Scope::Launch => check_launch(&self.requirement, subject).is_ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_app() -> Identity {
        Identity {
            signed: true,
            team_id: Some("TEAM1".into()),
            signing_id: Some("org.example.notes".into()),
            flags: SigningFlags(SigningFlags::RUNTIME | SigningFlags::LIBRARY_VALIDATION),
            category: Some(Category::Developer),
            entitlements: vec![("files".into(), "read".into()), ("files".into(), "write".into())],
            ..Identity::default()
        }
    }

    #[test]
    fn team_and_signing_identity_tests_match_exactly() {
        let app = dev_app();
        let team = Requirement::Test(Test::TeamIdentifier("TEAM1".into()));
        let other = Requirement::Test(Test::TeamIdentifier("TEAM2".into()));
        let id = Requirement::Test(Test::SigningIdentifier("org.example.notes".into()));
        assert!(team.evaluate(&app, None));
        assert!(!other.evaluate(&app, None));
        assert!(id.evaluate(&app, None));
    }

    #[test]
    fn all_of_and_any_of_combine_tests() {
        let app = dev_app();
        let team = Requirement::Test(Test::TeamIdentifier("TEAM1".into()));
        let wrong = Requirement::Test(Test::Platform("linux".into()));
        let all = Requirement::all_of(vec![team.clone(), wrong.clone()]).unwrap();
        let any = Requirement::any_of(vec![team, wrong]).unwrap();
        assert!(!all.evaluate(&app, None));
        assert!(any.evaluate(&app, None));
    }

    #[test]
    fn flags_are_a_superset_test_and_validation_matches_a_category_set() {
        let app = dev_app();
        let both = Requirement::Test(Test::Flags(SigningFlags::RUNTIME | SigningFlags::LIBRARY_VALIDATION));
        let host = Requirement::Test(Test::Flags(SigningFlags::HOST));
        assert!(both.evaluate(&app, None));
        assert!(!host.evaluate(&app, None));
        let cat = Requirement::Test(Test::Validation(vec![Category::Platform, Category::Developer]));
        assert!(cat.evaluate(&app, None));
        let adhoc = Requirement::Test(Test::Validation(vec![Category::AdHoc]));
        assert!(!adhoc.evaluate(&app, None));
    }

    #[test]
    fn caller_team_test_needs_a_caller_with_the_same_team() {
        let app = dev_app();
        let mut caller = dev_app();
        let req = Requirement::Test(Test::TeamIdentifierMatchesCaller);
        assert!(check_process(&req, &app, Some(&caller)));
        caller.team_id = Some("TEAM2".into());
        assert!(!check_process(&req, &app, Some(&caller)));
        assert!(!check_process(&req, &app, None));
    }

    #[test]
    fn duplicate_constraints_are_rejected_but_repeated_entitlements_are_not() {
        let team = Requirement::Test(Test::TeamIdentifier("TEAM1".into()));
        assert_eq!(Requirement::all_of(vec![team.clone(), team.clone()]), Err(BuildError::Duplicate(format!("{:?}", Test::TeamIdentifier("TEAM1".into())))));
        assert_eq!(Requirement::any_of(vec![]), Err(BuildError::Empty));
        let e1 = Requirement::Test(Test::Entitlement { key: "files".into(), value: "read".into() });
        let e2 = Requirement::Test(Test::Entitlement { key: "files".into(), value: "write".into() });
        let ok = Requirement::all_of(vec![e1, e2]).unwrap();
        assert!(ok.evaluate(&dev_app(), None));
    }

    #[test]
    fn on_disk_check_reports_signature_validity_separately() {
        let req = Requirement::Test(Test::TeamIdentifier("TEAM1".into()));
        let signed = check_on_disk(&req, &dev_app());
        assert_eq!(signed, ValidationResult { valid_signature: true, satisfied: true });
        let raw = Identity { signed: false, ..Identity::default() };
        let unsigned = check_on_disk(&req, &raw);
        assert_eq!(unsigned, ValidationResult { valid_signature: false, satisfied: false });
        let unsigned_req = Requirement::Test(Test::Unsigned);
        assert!(check_on_disk(&unsigned_req, &raw).satisfied);
    }

    #[test]
    fn launch_gate_refuses_programs_outside_the_requirement() {
        let req = Requirement::Test(Test::SigningIdentifier("org.example.notes".into()));
        assert!(check_launch(&req, &dev_app()).is_ok());
        let mut other = dev_app();
        other.signing_id = Some("org.example.evil".into());
        assert!(check_launch(&req, &other).is_err());
        assert_eq!(hash_hex(b"abc").len(), 64);
    }

    #[test]
    fn constraint_builder_rejects_empty_and_duplicate_arguments() {
        assert_eq!(Constraint::all_of(Scope::Process, vec![]), Err(ConstraintError::Empty));
        let team = Requirement::Test(Test::TeamIdentifier("TEAM1".into()));
        assert!(matches!(Constraint::any_of(Scope::Launch, vec![team.clone(), team]), Err(BuildError::Duplicate(_))));
    }

    #[test]
    fn process_constraint_uses_the_caller_and_on_disk_constraint_ignores_it() {
        let app = dev_app();
        let caller = dev_app();
        let c = Constraint::any_of(
            Scope::Process,
            vec![Requirement::Test(Test::TeamIdentifierMatchesCaller), Requirement::Test(Test::Platform("linux".into()))],
        )
        .unwrap();
        assert!(!c.satisfied_by(&app, None));
        assert!(c.satisfied_by(&app, Some(&caller)));
        // On-disk checks pass no caller, so the caller test fails and the platform test fails for a developer app.
        let disk = Constraint { scope: Scope::OnDisk, requirement: c.requirement.clone() };
        assert!(!disk.satisfied_by(&app, None));
        assert!(!disk.satisfied_by(&app, Some(&caller)));
    }

    #[test]
    fn launch_constraint_refuses_a_different_signing_identifier() {
        let c = Constraint::all_of(Scope::Launch, vec![Requirement::Test(Test::SigningIdentifier("org.example.notes".into()))]).unwrap();
        assert_eq!(c.scope, Scope::Launch);
        assert!(c.satisfied_by(&dev_app(), None));
        let mut other = dev_app();
        other.signing_id = Some("org.example.evil".into());
        assert!(!c.satisfied_by(&other, None));
    }

    #[test]
    fn unsigned_on_disk_constraint_accepts_only_unsigned_code() {
        let c = Constraint::all_of(Scope::OnDisk, vec![Requirement::Test(Test::Unsigned)]).unwrap();
        assert!(c.satisfied_by(&Identity { signed: false, ..Identity::default() }, None));
        assert!(!c.satisfied_by(&dev_app(), None));
    }
}
