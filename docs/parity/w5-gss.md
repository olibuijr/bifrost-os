# GSS (wave 5): security mechanisms, context services and message protection

Three more pure-Rust models of the GSS-API surface that the official "GSS" collection documents. They are std-only,
live in the app SDK (`springboard/sdk/src/`) beside the wave 3 and wave 4 GSS modules, and reuse what those waves and
the SDK already provide: `gss_oid` (identifiers and sets), `gss_token` (initial context token framing), `gss_status`
(routine errors), `gss_credential` (`INDEFINITE`) and `digest` (HMAC-SHA256 and constant-time comparison). No security
mechanism, Kerberos ticket, password check or key exchange is added. Confidentiality is not built.

## What was built

- `springboard/sdk/src/gss_mechanism.rs`: `Attr` (the `GSS_C_MA_*` attribute names, `name` and `from_name`),
  `Mechanism` (identifier, optional SASL name, attributes) and `Registry`, which starts empty and holds the installed
  mechanisms. `Registry::register` refuses a duplicate identifier or SASL name, `indicate_mechs` lists identifiers in
  registration order, `indicate_by_attrs` keeps mechanisms with every desired attribute and none of the excluded ones,
  `attrs_for_mech`, `mech_for_saslname` and `saslname_for_mech` map identifiers and SASL names both ways. An unknown
  mechanism is `GSS_S_BAD_MECH`, a mechanism without a SASL name is `GSS_S_UNAVAILABLE`.
- `springboard/sdk/src/gss_context.rs`: `Flags` (the core `GSS_C_*_FLAG` set, `from_wire` refuses unknown bits) and
  `Context`, one side of a security context. The initiator calls `initiate`, then `init_token`, then `complete` with the
  acceptor's answer. The acceptor calls `Context::accept`, which checks the mechanism against its supported set, grants
  the requested flags it offers and the shorter of the requested lifetime and its cap, and returns the answer token.
  `flags`, `is_established`, `mech` and `context_time` (`GSS_C_INDEFINITE` for no limit, `GSS_S_CONTEXT_EXPIRED` at
  the expiry instant) inquire the open context. The request and answer are nine-byte inner tokens (tag `I` or `A`, flags,
  lifetime, 32-bit big-endian) inside the `gss_token` framing. This inner token is the model's own; it talks only to
  another instance of this module.
- `springboard/sdk/src/gss_message.rs`: `MessageKey` (a session key; `Debug` shows its length, never its bytes; an
  empty key is refused), `get_mic` and `verify_mic` (tag `M` and the HMAC-SHA256 of the message, 33 bytes),
  `wrap` and `unwrap` (tag `W`, the MAC, then the message in the clear, 33 bytes of overhead), and `wrap_size_limit`.
  The MAC is compared with `constant_time_eq`. A request for confidentiality is `GSS_S_UNAVAILABLE`; unwrapped messages
  always report `conf_state` false.

Each module has a `#[cfg(test)] mod tests` with hand-worked values. `springboard/sdk/src/lib.rs` gained one `pub mod`
line per module (`gss_mechanism`, `gss_context`, `gss_message`). Release and destroy calls are replaced by drop.

## Official API names mirrored

- Security Mechanisms: `gss_indicate_mechs`, `gss_indicate_mechs_by_attrs`, `gss_display_mech_attr` (name only),
  `gss_inquire_attrs_for_mech`, `gss_inquire_mech_for_saslname`, `gss_inquire_saslname_for_mech`.
- Context Services: `gss_init_sec_context` (initiator steps), `gss_accept_sec_context` (acceptor step),
  `gss_context_time`, `gss_inquire_context` (flags and mechanism only), `gss_delete_sec_context` (replaced by drop),
  `GSS_C_DELEG_FLAG`, `GSS_C_MUTUAL_FLAG`, `GSS_C_REPLAY_FLAG`, `GSS_C_SEQUENCE_FLAG`, `GSS_C_CONF_FLAG`,
  `GSS_C_INTEG_FLAG`, `GSS_C_ANON_FLAG`, `GSS_C_PROT_READY_FLAG`, `GSS_C_TRANS_FLAG`.
- Message Protection: `gss_get_mic`, `gss_verify_mic`, `gss_wrap`, `gss_unwrap`, `gss_wrap_size_limit`. Not mirrored:
  `gss_sign`, `gss_verify`, `gss_seal`, `gss_unseal` (the confidentiality and signature forms).

Source pages (DocC, via `tools/parity/docc.py`): `gss/security-mechanisms`, `gss/context-services`,
`gss/message-protection`, `gss` (framework page, with the Function Status and Token Management topics it links).

## Evidence

REA evidence of the rendered official pages (`tools/parity/rea-page.sh`):

- GSS framework page: `ev_fa356567f93049871d5aad3e889a1dcf0510bf8b9cd6550ab2160fd4d5796598`
- Security Mechanisms: `ev_12b4764e1fb3809a2bc9a90d738690282cbe2ad809ff253514b5f6e14d18f4ca`
- Context Services: `ev_62d4b1759bcf0c5ecb441da649a21efc46091786bab242bd6e1202337360834f`
- Message Protection: `ev_55c47c1c30c1d8a324584044a9bf594e5988985c076ee4e528b17e93965e1c2c`

The rendered Context Services page lists the flag variables (`GSS_C_DELEG_FLAG` through `GSS_C_TRANS_FLAG`, and the
four further flags this model omits) as `Int32` and does not print their values. The flag bit values (1, 2, 4, 8, 16,
32, 64, 128, 256), the `GSS_C_INDEFINITE` value (all ones) and the nine-byte inner token layout are this model's
conventions, following RFC 2744 as `gss_status` does. The MAC is HMAC-SHA256, and the known-answer test uses the
published HMAC-SHA256 value for key `key` and the fox sentence (`f7bc83f4...2d1a3cd8`).

## Tests

No builds were run in this round (the round is commit-only). The tests were worked out by hand from the rules the
code implements and are listed in each module's `mod tests`:

- `gss_mechanism`: 7 tests (empty registry, identifier order, duplicate refusal, two attribute filters, SASL name
  mapping, unknown mechanism with attribute name round trip).
- `gss_context`: 7 tests. The initial token has a worked 22-byte value. A handshake grants `MUTUAL` only and the
  600 s cap (answer `41 00 00 00 02 00 00 02 58`), with expiry at 1600 s. An unlimited context reports
  `GSS_C_INDEFINITE`. An unsupported mechanism, out-of-order steps, an unrequested granted flag and an unknown flag bit
  are each refused with the stated routine error.
- `gss_message`: 10 tests. The MIC equals the tag plus the published HMAC-SHA256 value. A MIC verifies, a changed
  message and a different key fail with `GSS_S_BAD_MIC`, a wrapped message is 33 + 43 = 76 bytes and round-trips, a
  tampered wrap fails, confidentiality is refused, malformed tokens are `GSS_S_DEFECTIVE_TOKEN`, the size limit is
  `100 - 33 = 67`, and an empty key is refused and never printed.

Running them is left to the lead's build pass: `cargo test -p bifrost-app gss_`.

## Matrix rows closed (status `partial`)

| Framework | Feature (as written in `docs/parity-matrix/data/gss.json`) | New status | BifrOSt path |
|---|---|---|---|
| gss | Context services (gss_init_sec_context, gss_accept_sec_context, context flags) | partial | springboard/sdk/src/gss_context.rs |
| gss | Security mechanisms (gss_indicate_mechs, mechanism attributes) | partial | springboard/sdk/src/gss_mechanism.rs |
| gss | Message protection (gss_wrap, gss_unwrap, gss_get_mic, gss_verify_mic) | partial | springboard/sdk/src/gss_message.rs |

Still open: Kerberos implementation (no KDC or ticket support), Memory and context (Rust drop replaces the release
calls, no change), and the Context Services flags DCE style, identify, extended error and delegation policy.
The matrix is updated centrally by the lead; this round does not edit `docs/parity-matrix/`.
