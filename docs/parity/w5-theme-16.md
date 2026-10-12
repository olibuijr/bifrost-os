# Wave 5, theme 16: context services, mechanisms, Kerberos settings, legacy login items, memory events

Theme 16 covers six matrix rows across three frameworks: GSS (context services, security mechanisms, the Kerberos
implementation), Service Management (the legacy login item check and the deprecated enable call) and Endpoint
Security (memory mapping and protection events). All of it is pure Rust in `springboard/sdk/src/`, std only, with no
hardware, no network and no unsafe code. None of it is wired into apps or the shell, and none of it has run on the VM
or Midget. The rows move from `gap` to `partial`.

## What was built

- **`springboard/sdk/src/gss_context.rs`** (new): the state of one security context. `SecContext::initiate` and
  `SecContext::accept` create the two sides. `ContextFlags` is the flag set (13 named flags with their RFC 2744 bit
  values, with union, intersection, difference and names). `SecContext::step` drives a `Handshake` trait that a
  mechanism implements. It enforces the exchange order and the required flags, and it keeps requested apart from
  engaged, which is the reference's rule that a requested service is not guaranteed. A missing required flag moves the
  context to `Failed`.
- **`springboard/sdk/src/gss_mechanism.rs`** (new): the mechanism registry. `MechanismRegistry` holds mechanisms by
  identifier (`Oid`, from `gss_oid.rs`) with a display name, an optional SASL name and attribute names. It answers
  `indicate` (every identifier), `indicate_by_attrs` (critical, desired and excluded attributes), `attributes_for`
  (one mechanism or the sorted union), `for_sasl_name` and `sasl_name_for`. Registration refuses duplicate identifiers
  and duplicate SASL names.
- **`springboard/sdk/src/gss_krb5.rs`** (new): the Kerberos settings. `CacheName` parses `TYPE:residual` credential
  cache names (FILE, DIR, KEYRING, KCM, MEMORY, and a bare name is a file). `KeytabName` accepts files only.
  `Krb5Settings` holds the cache name (`gss_krb5_ccache_name`), the allowable encryption types
  (`gss_krb5_set_allowable_enctypes`, unrestricted until set, and exact once set) and the acceptor keytab
  (`gsskrb5_register_acceptor_identity`).
- **`springboard/sdk/src/memory_events.rs`** (new): the memory mapping and protection events. `MapEvent` (mmap) and
  `ProtectEvent` (mprotect) are checked on construction: the protection must fit inside the maximum, the source must
  be named, the length must be non-zero and the range must not wrap. `MemoryMonitor` records events and reports a
  finding when a page is left writable and executable at once. It is notify-only.
- **`springboard/sdk/src/service_management.rs`** (extended, not a new module): `status_for_legacy_plist` answers the
  legacy plist check from a list of known legacy items (absolute `.plist` path, no `..`, otherwise `NotFound`).
  `set_login_item_enabled` is the deprecated enable-by-identifier call. It fails with `JobNotFound` as the existing
  `SMAppService::login_item` does, because no helper bundles ship, and it sends nothing to the shell.
- **`springboard/sdk/src/lib.rs`**: four `pub mod` lines (`gss_context`, `gss_mechanism`, `gss_krb5`, `memory_events`).

Every new module has a `#[cfg(test)] mod tests` with hand-worked behaviour tests (flag bit values and set operations,
two-step handshake and required-flag failure, exchange order, mechanism attribute rules and SASL mapping, cache name
parsing, enctype filtering, protection range rules, the writable and executable finding, legacy path validation).

## Official API names mirrored

- GSS (`https://developer.apple.com/documentation/gss`): `gss_init_sec_context`, `gss_accept_sec_context`,
  `gss_delete_sec_context`, `GSS_C_DELEG_FLAG`, `GSS_C_MUTUAL_FLAG`, `GSS_C_REPLAY_FLAG`, `GSS_C_SEQUENCE_FLAG`,
  `GSS_C_CONF_FLAG`, `GSS_C_INTEG_FLAG`, `GSS_C_ANON_FLAG`, `GSS_C_PROT_READY_FLAG`, `GSS_C_TRANS_FLAG`,
  `GSS_C_DCE_STYLE`, `GSS_C_IDENTIFY_FLAG`, `GSS_C_EXTENDED_ERROR_FLAG`, `GSS_C_DELEG_POLICY_FLAG` (Context Services);
  `gss_indicate_mechs`, `gss_indicate_mechs_by_attrs`, `gss_inquire_attrs_for_mech`, `gss_inquire_mech_for_saslname`,
  `gss_inquire_saslname_for_mech` (Security Mechanisms); `gss_krb5_ccache_name`, `gss_krb5_set_allowable_enctypes`,
  `gsskrb5_register_acceptor_identity` (Kerberos Implementation).
- Service Management: `SMAppService.statusForLegacyPlist(at:)`, `SMLoginItemSetEnabled(_:_:)` (deprecated).
- Endpoint Security: `es_event_mmap_t` (fields protection, max_protection, flags, file_pos, source) and
  `es_event_mprotect_t` (fields address, size, protection). The models use the official field names in Rust form.

Apple's pages are abstracts and symbol lists. The reference does not give the mechanism attribute names, the lucid
context layout, the numeric values of the Kerberos encryption types, or the rule for `indicate_by_attrs`. The
attribute rule in `gss_mechanism.rs` is the one of the GSS-API attribute specification (RFC 5587), not Apple's text, and
the flag bit values are the GSS-API ones (RFC 2744). Both are marked in the module comments.

## REA evidence

Rendered official pages, from `tools/parity/rea-page.sh`:

- Context Services, `https://developer.apple.com/documentation/gss/context-services`:
  `ev_3bb7d7ed2bfb7896a3194a46d08875844abdb1abfb7b07909f966d6c682a0b2b`. The rendered text says that a requested
  service is not guaranteed and that the acceptor and initiator get back the flags that are engaged. That is the rule
  the context model follows.
- Kerberos Implementation, `https://developer.apple.com/documentation/gss/kerberos-implementation`:
  `ev_d3d6e24ec3e0c417080632194d5f96c5ca4dcfdb07063c960e4cd113a0072f2e`.
- es_event_mmap_t, `https://developer.apple.com/documentation/endpointsecurity/es_event_mmap_t`:
  `ev_f4728a3380a6d5df9056f51f864d3c4df01a27d1864c720890424288e2f390e4`.
- statusForLegacyPlist(at:), `https://developer.apple.com/documentation/servicemanagement/smappservice/statusforlegacyplist(at:)`:
  `ev_7ffbee6c8f45a7a29988c7105b9f0b2e03b39bb2131ea859b41a4b14419d5440`.

The DocC JSON (`tools/parity/docc.py`) was read for `gss`, `gss/context-services`, `gss/security-mechanisms`,
`gss/kerberos-implementation`, the GSS symbol pages, `servicemanagement/smappservice/statusforlegacyplist(at:)`,
`servicemanagement/smloginitemsetenabled(_:_:)` (its abstract says it enables a helper executable in the app bundle
directory) and the Endpoint Security mmap and mprotect pages. No REA id was taken for `SMLoginItemSetEnabled`.

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| gss | Context services (gss_init_sec_context, gss_accept_sec_context, context flags) | partial | springboard/sdk/src/gss_context.rs |
| gss | Security mechanisms (gss_indicate_mechs, mechanism attributes) | partial | springboard/sdk/src/gss_mechanism.rs |
| gss | Kerberos implementation (gss_krb5_*, lucid context, acceptor identity) | partial | springboard/sdk/src/gss_krb5.rs |
| service-management | Legacy login item authorization check | partial | springboard/sdk/src/service_management.rs (status_for_legacy_plist) |
| service-management | Login item enablement (deprecated) | partial | springboard/sdk/src/service_management.rs (set_login_item_enabled) |
| endpoint-security | Memory mapping and protection events | partial | springboard/sdk/src/memory_events.rs |

## Not done in this theme

- GSS: no real mechanism implements `Handshake`, so no context completes; no wire framing beyond the existing
  `gss_token.rs`; no lucid context export (`gss_krb5_export_lucid_sec_context`), no ticket cache copy and no Kerberos
  ticket exchange. The Kerberos settings are recorded only.
- Service Management: `statusForLegacyPlist` reads a list the caller supplies, and the shell supplies none, so it
  always answers `NotFound` in a running app. The deprecated call cannot enable anything until helper login items exist.
- Endpoint Security: no shell path produces memory events, and nothing blocks a mapping.
- Nothing was built or run, as the task required. The modules were checked by reading only.
