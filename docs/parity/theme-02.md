# Theme 02: session properties and code identity (parity wave 2)

Theme: "Session properties" (`theme-02`, 50 gap rows). This wave builds two pure models in `springboard/core/src`
that cover the identity and authentication rows the theme shares. Both are models only: nothing in the shell or SDK
calls them yet, and BifrOSt does not sign code, so the code-identity model has no real signatures behind it.

## What was built

- `springboard/core/src/codereq.rs` (`bifrost_core::codereq`): a lightweight code requirement model.
  - `Identity` holds the testable fields of a program (signed flag, team, signing identifier, code-directory and
    info-plist hashes, signing flags, validation category, platform, main-binary flag, entitlements).
  - `Test` covers the constraints: team identifier, team matches the caller, signing identifier, code-directory hash,
    info-plist hash, flags superset, validation category set, platform, main binary, entitlement, unsigned.
  - `Requirement` is a test or an `all_of` / `any_of` operator. Operators reject empty arguments and duplicated
    non-entitlement tests (`BuildError`), following the simplification rule in the official page.
  - `check_on_disk` returns a `ValidationResult` (signature valid, requirement satisfied), `check_process` takes the
    calling process as the reference for the caller-team test, and `check_launch` gates a launch.
  - `SigningFlags` is a BifrOSt bit set; the bit positions are ours, not an external ABI.
- `springboard/core/src/biometry.rs` (`bifrost_core::biometry`): a local authentication model.
  - `BiometryType` (none, touch, face, optic), `Device` (sensor, enrolled identities, passcode, failure count and
    limit), `Policy` (biometrics only, device owner), `Requirement` (default, or biometry with `current_set` and an
    optional `Fallback`).
  - `resolve` returns the `Method` (biometry or passcode) or a `Failure` (unavailable, not enrolled, lockout,
    passcode not set, state changed).
  - `DomainState::of` is a SHA-256 digest of the sensor type and sorted enrolled identities. A `current_set`
    requirement only passes when the stored digest matches, so a secret stored before an enrolment change is refused.
  - `Device::record_failure` and `record_success` drive the lockout.

## Official API names mirrored

Sources: `python3 -I tools/parity/docc.py` on `lightweightcoderequirements`, `lightweightcoderequirements/teamidentifier`,
`.../validationcategory`, `.../processcodesigningflags`, `localauthentication`, `localauthentication/labiometrytype`,
`.../laauthenticationrequirement`, `.../labiometryfallbackrequirement`, `.../ladomainstate`, `.../lapolicy`,
`.../lacontext`.

- LightweightCodeRequirements: `TeamIdentifier`, `TeamIdentifierMatchesCurrentProcess`, `SigningIdentifier`,
  `CodeDirectoryHash`, `InfoPlistHash`, `ProcessCodeSigningFlags`, `ValidationCategory`, `anyOf(requirement:)`,
  `allOf(requirement:)`, `ProcessCodeRequirement`, `OnDiskCodeRequirement`, `LaunchCodeRequirement`,
  `SecTaskValidateForRequirement`, `SecStaticCodeCheckValidityWithOnDiskRequirement`,
  `SecCodeCheckValidityWithOnDiskRequirement`, `SecCodeCheckValidityWithProcessRequirement`, `ValidationResult`.
- Local Authentication: `LABiometryType` (`none`, `touchID`, `faceID`, `opticID`), `LAPolicy`
  (`deviceOwnerAuthenticationWithBiometrics`, `deviceOwnerAuthentication`), `LAContext.canEvaluatePolicy` and
  `evaluatePolicy` (modelled as `resolve`), `LADomainState.stateHash`, `LAAuthenticationRequirement`
  (`default`, `biometry`, `biometryCurrentSet`, `biometry(fallback:)`), `LABiometryFallbackRequirement`
  (`default`, `devicePasscode`), `LAContext.maxBiometryFailures` (modelled as `Device::max_failures`).

Apple's `ValidationCategory.Value` cases and the `ProcessCodeSigningFlags.ValueSet` members are not listed on the
fetched pages, so `Category` (platform, developer, ad hoc) and the flag bits are BifrOSt's own. The LAError codes are
not mapped; `Failure` names are ours.

## REA evidence

- Local Authentication framework page: `ev_6b050c6245ba202648358d679371b7f9fa79bf4dc8d906760ecc4414b07ff960`
  (`https://developer.apple.com/documentation/localauthentication`)
- LightweightCodeRequirements framework page: `ev_c60fd000bcd602b89d63fe3b50c8977f796d221eab05096effbcbea846837349`
  (`https://developer.apple.com/documentation/lightweightcoderequirements`)

## Matrix rows closed

Status moves to `partial`: the model exists, but no app or the shell uses it yet.

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| lightweightcoderequirements | Signing flags and validation category | partial | springboard/core/src/codereq.rs (`SigningFlags`, `Category`, `Test::Flags`, `Test::Validation`) |
| lightweightcoderequirements | Signing identity constraints | partial | springboard/core/src/codereq.rs (`Test::TeamIdentifier`, `TeamIdentifierMatchesCaller`, `SigningIdentifier`, hashes) |
| lightweightcoderequirements | Checking code requirements for running processes | partial | springboard/core/src/codereq.rs (`check_process`, `Requirement::all_of` / `any_of`) |
| lightweightcoderequirements | Checking code requirements for code files on disk | partial | springboard/core/src/codereq.rs (`check_on_disk`, `ValidationResult`) |
| lightweightcoderequirements | Checking code requirements for launching processes | partial | springboard/core/src/codereq.rs (`check_launch`); not wired into springboard/src/bare/app_policy.rs |
| local-authentication | Biometry type and device state (LADomainState, LAEnvironment, kLABiometryType*) | partial | springboard/core/src/biometry.rs (`BiometryType`, `Device`, `DomainState`) |
| local-authentication | Biometric policy evaluation (LAContext, evaluatePolicy) | partial | springboard/core/src/biometry.rs (`Policy`, `resolve`, `Device::record_failure`) |
| local-authentication | Authentication requirements and biometry fallback (LAAuthenticationRequirement, LABiometryFallbackRequirement) | partial | springboard/core/src/biometry.rs (`Requirement`, `Fallback`, `Failure`) |

Not closed: the Session properties rows of foundation-models, and the rest of the theme-02 list, need the
language-model session and other subsystems this wave does not touch.

## Tests

Each module has a `#[cfg(test)] mod tests`: 7 behaviour tests in `codereq.rs`, 7 in `biometry.rs`. No build or test
was run in this wave, per the task rules.
