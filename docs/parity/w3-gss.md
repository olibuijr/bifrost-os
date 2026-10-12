# GSS (wave 3): status, buffers and object identifiers

Three pure-Rust models of the GSS-API surface that the official "GSS" collection documents. They are std-only
and live in the app SDK (`springboard/sdk/src/`), next to the keychain they sit beside. They do not implement a
security mechanism: no security context, Kerberos ticket, credential or message protection is added here.

## What was built

- `springboard/sdk/src/gss_status.rs`: the major status code (calling error, routine error and supplementary
  fields, with the mask and offset constants), the calling and routine error enums with their official
  descriptions, `gss_display_status`-style text (`MajorStatus::describe`, `Display`), and a `StatusError` that
  pairs a major code with a minor code and its origin (`GSS_C_GSS_CODE` or `GSS_C_MECH_CODE`).
- `springboard/sdk/src/gss_buffer.rs`: `Buffer` (an owned `gss_buffer_desc`) and `BufferSet` (a
  `gss_buffer_set_desc`) with copy-on-add (`gss_add_buffer_set_member`), `get`, `total_len`. Release calls are
  replaced by drop.
- `springboard/sdk/src/gss_oid.rs`: `Oid` (a `gss_OID`) with dotted parse and write (`gss_oid_to_str`), DER content
  octet encode and decode, the ASN.1 first-arc rule, and `OidSet` (`gss_create_empty_oid_set`,
  `gss_add_oid_set_member`, `gss_test_oid_set_member`).

Each module has a `#[cfg(test)] mod tests` with hand-checked values. `springboard/sdk/src/lib.rs` gained one
`pub mod` line per module.

## Official API names mirrored

- Function Status: `GSS_S_COMPLETE`, `GSS_S_CALL_*`, `GSS_S_*` routine errors, `GSS_C_CALLING_ERROR_MASK`,
  `GSS_C_ROUTINE_ERROR_MASK`, `GSS_C_SUPPLEMENTARY_MASK` and their offsets, `GSS_C_MECH_CODE`, `GSS_C_GSS_CODE`,
  `gss_display_status`, `GSSCreateError`.
- Buffer Management: `gss_buffer_desc`, `gss_buffer_set_desc`, `gss_create_empty_buffer_set`,
  `gss_add_buffer_set_member`, `gss_release_buffer`, `gss_release_buffer_set`.
- Object Identifiers: `gss_OID_desc`, `gss_OID_set`, `gss_create_empty_oid_set`, `gss_add_oid_set_member`,
  `gss_test_oid_set_member`, `gss_oid_equal`, `gss_oid_to_str`.

Source pages (DocC, via `tools/parity/docc.py`): `gss/function-status`, `gss/buffer-management`,
`gss/object-identifiers`.

## Evidence

REA evidence of the rendered official pages (`tools/parity/rea-page.sh`):

- Function Status: `ev_8221b99bae795a2e4210b83245f9209b26f89d4c4a65a57781dd6a7a10110ee6`
- Buffer Management: `ev_69d84793424e9fdbd630b4f617dad8f9fcc70f36dc34ba5f6e3a37e839a238fe`
- Object Identifiers: `ev_65b472ff83cdd7dab38eb098e6da9ade63cbf24a33739489ad3311632781a863`

The rendered pages name the fields, masks and functions but do not print numeric values. The bit widths (8, 8, 16)
and the routine error numbers follow RFC 2744 section 1.2.1 and the RFC routine error table. The DER element
bytes follow the standard ASN.1 rule; the tests check them against the worked value of `1.2.840.113549`.

## Matrix rows closed (status `partial`)

Updated centrally by the lead; not edited here.

| Framework slug | Feature (as in data/gss.json) | New status | BifrOSt path |
|---|---|---|---|
| gss | Function status codes (GSS_S_*, GSSCreateError, gss_display_status) | partial | springboard/sdk/src/gss_status.rs |
| gss | Buffer management (gss_buffer_desc, buffer sets, IOV buffers) | partial | springboard/sdk/src/gss_buffer.rs |
| gss | Object identifiers (gss_OID, gss_OID_set) | partial | springboard/sdk/src/gss_oid.rs |

Partial, not parity, because: the IOV buffer forms are not modelled; the status model has no mechanism that
produces minor codes; the OID model has no mechanism registry and no name-type mapping.

## Not built (still gap)

Context services, credential management, security mechanisms, name handling, token management, message
protection and the Kerberos implementation. They need a mechanism, a KDC, or a per-message cipher, which are not
pure-Rust std work.
