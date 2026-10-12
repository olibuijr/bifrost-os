# GSS (wave 4): name handling, token management and credential management

Three more pure-Rust models of the GSS-API surface that the official "GSS" collection documents. They are std-only,
live in the app SDK (`springboard/sdk/src/`) beside the wave 3 GSS modules, and do not implement a security
mechanism: no security context, Kerberos ticket, password check or message protection is added here.

## What was built

- `springboard/sdk/src/gss_name.rs`: `Name` (an imported `gss_name_t`) and `NameType` (user name and host-based
  service name). `Name::import` reads the octets for a known type, `Name::import_typed` takes a name type
  object identifier and refuses unknown ones. `display` is the `gss_display_name` / `GSSNameCreateDisplayString`
  text (`service@host`), `canonicalize` lowers the host to ASCII (`gss_canonicalize_name`), and `compare` matches
  canonical forms (`gss_compare_name`). `NameError` maps to `GSS_S_BAD_NAME` and `GSS_S_BAD_NAMETYPE`.
- `springboard/sdk/src/gss_token.rs`: `encapsulate` and `decapsulate` for the initial context token
  (`[APPLICATION 0]` tag 0x60, a DER length, the mechanism OBJECT IDENTIFIER, then the inner token). Lengths use the
  DER short and long forms; the decoder refuses indefinite and non-minimal lengths and any body that does not cover
  the input exactly. `TokenError` maps to `GSS_S_DEFECTIVE_TOKEN`.
- `springboard/sdk/src/gss_credential.rs`: `Credential` (a credential with name, usage and absolute expiry),
  `Usage` (`GSS_C_BOTH`, `GSS_C_INITIATE`, `GSS_C_ACCEPT`), `lifetime` (`GSSCredentialGetLifetime`, 0 once expired,
  `INDEFINITE` when unlimited), `is_expired`, and `usable_for`, which refuses an expired credential first and then a
  usage the operation does not cover. `CredentialError` maps to `GSS_S_CREDENTIALS_EXPIRED` and `GSS_S_NO_CRED`.

Each module has a `#[cfg(test)] mod tests` with hand-worked values. `springboard/sdk/src/lib.rs` gained one
`pub mod` line per module. Release and destroy calls are replaced by drop.

## Official API names mirrored

- Name Handling: `gss_name_t`, `gss_import_name`, `gss_display_name`, `GSSNameCreateDisplayString`, `GSSCreateName`
  (text and type only), `gss_compare_name`, `gss_canonicalize_name`.
- Token Management: `gss_encapsulate_token`, `gss_decapsulate_token`.
- Credential Management: `gss_acquire_cred` (model constructor only, no mechanism check), `gss_inquire_cred`
  lifetime, `GSSCredentialGetLifetime`, `GSS_C_INDEFINITE`, `GSS_C_INITIATE`, `GSS_C_ACCEPT`, `GSS_C_BOTH`,
  `gss_release_cred` and `gss_destroy_cred` (replaced by drop).

Source pages (DocC, via `tools/parity/docc.py`): `gss/name-handling`, `gss/token-management`,
`gss/credential-management`.

## Evidence

REA evidence of the rendered official pages (`tools/parity/rea-page.sh`):

- Name Handling: `ev_4675d881fc9df042991d7b2088474c9cce8ea1feeb9977e6edefd1d29ce44956`
- Token Management: `ev_afbfe27fd703c755db5b88b2decf644e6a2a4621d8c1a063dbff48412858b9c0`
- Credential Management: `ev_97cb568f22e6916399f0eb6abe2e5ec1407b72cf2e6154bdfc06a123b19589d3`

The rendered pages name the functions and types but do not print the object identifiers or the numeric macro
values. The name type identifiers (`1.2.840.113554.1.2.1.1` user name, `1.2.840.113554.1.2.1.4` host-based
service), the usage values (0, 1, 2) and the token tag (0x60) follow RFC 2744 and RFC 2743, the same convention
`gss_status` uses. The DER bytes of the mechanism identifier follow the ASN.1 rule; the token tests check them
against the worked value of `1.2.840.113554.1.2.2`.

## Tests

No builds were run in this round (the round is commit-only). The tests were worked out by hand from the rules the
code implements and are listed in each module's `mod tests`. Running them is left to the lead's build pass:
`cargo test -p bifrost-app gss_`.

## Matrix rows closed (status `partial`)

Updated centrally by the lead; not edited here.

| Framework slug | Feature (as in data/gss.json) | New status | BifrOSt path |
|---|---|---|---|
| gss | Name handling (gss_import_name, gss_display_name, gss_compare_name) | partial | springboard/sdk/src/gss_name.rs |
| gss | Token management (gss_encapsulate_token, gss_decapsulate_token) | partial | springboard/sdk/src/gss_token.rs |
| gss | Credential management (gss_acquire_cred, gss_inquire_cred, kGSSIC* keys) | partial | springboard/sdk/src/gss_credential.rs |

Partial, not parity, because: no mechanism maps an internal name to a mechanism name or exports it as a token;
no mechanism verifies a password or certificate, so `gss_acquire_cred` is only a model; the `kGSSIC*` initial
credential keys and the iteration calls are not modelled; the token model carries no mechanism-specific inner
token format; and nothing in the shell or apps uses these modules yet.

## Not built

Message protection, Kerberos and context services need a cryptographic mechanism and were left as gaps. Security
mechanisms (`gss_indicate_mechs`) would be an empty registry with no mechanism behind it and was not added.
