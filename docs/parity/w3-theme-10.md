# Wave 3, theme-10: certificate, key and trust evaluation

## What was built

- `springboard/ui/src/trust.rs` (new, pure model, std only). A certificate chain, leaf first, is checked against
  trust anchors under a policy at a given time. `Certificate` holds the fields the checks need. `Policy::Basic`
  checks validity, the chain links, CA and path-length rules and the anchor. `Policy::Ssl { host }` also checks
  that the leaf names the host (one-label wildcards) and that the leaf may create digital signatures. `evaluate`
  returns a `Verdict` (`Invalid`, `Unspecified`, `RecoverableTrustFailure`, `FatalTrustFailure`) and every
  `Failure` with its chain index. `CertificateView` is the display model: a summary that is always shown, and
  details that the user discloses. `set_disclosed` reports whether the state changed.
- `springboard/ui/src/access_control.rs` (extended). `KeyOperation` names the private-half operations (sign,
  decrypt, key exchange) and the public-half operations (encrypt, verify, export).
  `KeyPairPolicy` gates the private half with the existing `AccessControl` expression and lets the public half
  through without presence. Reuses `Expr`, `Presence` and `Error` from the same module.
- `springboard/ui/src/lib.rs`: one line, `pub mod trust;`.

## What the code does not do

- No signature mathematics. `evaluate` takes the signature check as a closure, `verify(child, issuer)`. The
  caller supplies it. No crate is added.
- No X.509 parsing. The caller builds `Certificate` values from its own parser.
- No certificate storage, no key generation, no key operations on key material.
- No trust settings UI and no certificate panel. `CertificateView` is the model a view would draw.
- `Verdict` never returns a "proceed" outcome. That outcome means the user chose to trust a certificate, and
  the caller records that choice.
- The `KeyPairPolicy` model treats public-half operations as needing no presence. This is a model choice: the
  public half is not secret. It is not checked against a device.

## Official API names mirrored

| Official name | Where it lives in BifrOSt |
|---|---|
| `SecTrust` (an object used to evaluate trust) | `trust::evaluate` and `trust::Evaluation` |
| `SecTrustEvaluateWithError`, the trust result categories | `trust::Verdict` |
| `SecPolicyCreate*` (basic and SSL policies) | `trust::Policy::Basic`, `trust::Policy::Ssl` |
| `SFCertificateView` (summary, disclosure state, change notification) | `trust::CertificateView`, `set_disclosed` |
| `LAPublicKey`, `LAPrivateKey` (key pairs gated by authentication) | `access_control::KeyPairPolicy`, `access_control::KeyOperation` |

Note: the DocC page for the evaluate call (`security/sectrustevaluatewitherror`) returned 404 from the
documentation tool. The evaluate names above come from the `SecTrust` page and the matrix row, not from that
page.

## REA evidence

- `ev_945eb71ff5d13bc5a2bbabed2115f81b0060cd19e8d00d10e6a15d14e3a40708`: `SecTrust` rendered page
  (https://developer.apple.com/documentation/security/sectrust)
- `ev_06202aa22a98478f257f5f69a4b16fbe26af8e694aefce927a5c73abe7ea1a22`: `LAPrivateKey` rendered page
  (https://developer.apple.com/documentation/localauthentication/laprivatekey)
- `ev_d2da2815f56101667277e7ebcd9c849675d01e2a8346fc37fcfbdeeba043f6ca`: `SFCertificateView` rendered page
  (https://developer.apple.com/documentation/securityinterface/sfcertificateview)

## Matrix rows closed (status `partial`)

| Framework slug | Feature (as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| security | Certificate, key and trust evaluation | partial | springboard/ui/src/trust.rs |
| security-interface | Certificate view | partial | springboard/ui/src/trust.rs (CertificateView) |
| local-authentication | Key pairs gated by authentication (LAPublicKey, LAPrivateKey) | partial | springboard/ui/src/access_control.rs (KeyPairPolicy) |

## Verification

- Not built and not run. This wave's rules forbid cargo, rustc and tests. The code and tests were checked by hand
  against the rules they implement. The next person to build should run the `bifrost-ui` tests first.
- Tests added: 10 in `trust.rs` (valid chain, expired and not-yet-valid, untrusted root, broken link and signature
  failures, host and wildcard names, path length, CA rule, key usage, empty chain, view disclosure), and 5 in
  `access_control.rs` (private versus public operations, current-set lock, conjunction, empty expression, the
  private-half set).
- `python3 tools/check-names.py` exits 0.
