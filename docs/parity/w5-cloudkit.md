# Wave 5: CloudKit gap rows (record sharing, web service tokens)

Branch `parity/w5-cloudkit`, on top of waves 1 to 4 (`parity/integration`). Pure Rust, std only, host unit tests only:
nothing was built or run on the VM or Midget, so the status is `partial`.

## What was built

- `springboard/ui/src/record_share.rs` (`bifrost_ui::record_share`): a share of one root record in a record database
  (`RecordID` from `record_store`). It has an owner (always read and write), invited participants with read-only or
  read-write permission, acceptance (pending, accepted, removed), a public permission for anyone holding the URL,
  a participant cap, access requests with deny and block, and the access rules written in the module comment
  (a pending or removed participant gets no access; the public permission does not apply to a removed one).
  No network, no disk, no URL is opened. Invitations and identity checks are left to a driver.
- `springboard/ui/src/web_service_token.rs` (`bifrost_ui::web_service_token`): a one-shot fetch of a web token for an
  API token. The API token is checked (not empty, no whitespace or control characters) before a caller-supplied
  transport runs. An empty issued token is a failure. `WebToken` redacts its value in `Debug`.
- `pub mod` lines for both in `springboard/ui/src/lib.rs`, after `record_asset`.

Module tests: 8 in `record_share` and 5 in `web_service_token`, with expected values worked out by hand from the rules.

## Official API names mirrored

- Shared records: `CKShare` (init(rootRecord:), owner, currentUserParticipant, participants, url, publicPermission,
  addParticipant(_:), removeParticipant(_:), allowsAccessRequests, requesters, blockRequesters(_:),
  denyRequesters(_:), unblockIdentities(_:), blockedIdentities), `CKShare.Participant`,
  `CKShare.ParticipantPermission` (none, readOnly, readWrite, unknown), `CKShare.ParticipantAcceptanceStatus`
  (unknown, pending, accepted, removed). Source: https://developer.apple.com/documentation/cloudkit/ckshare
- Web auth tokens: `CKFetchWebAuthTokenOperation` (init(apiToken:), apiToken, fetchWebAuthTokenResultBlock).
  Source: https://developer.apple.com/documentation/cloudkit/ckfetchwebauthtokenoperation

Rules our code decides and the reference page does not state: the default public permission is none; a removed
participant does not fall back to the public permission; the participant cap counts invited users, not the owner;
invited users with a blocked identity are refused; the error codes used for each refusal come from the existing
`record_sync::ErrorCode` set.

## REA evidence

- Landing page `https://developer.apple.com/documentation/cloudkit`: `ev_9c70bbda9eb8fd8ae8c4881f117fa195b7ebf245b2f551eca5606060b54bd29d`
  (lists the Shared Records topic and the share-request operation under Classes).
- `https://developer.apple.com/documentation/cloudkit/ckshare`: `ev_131c6e57a9bb93bc99a0f997188b117a64b5185e6f2fabc438f4e0a31fc43ec9`
- `https://developer.apple.com/documentation/cloudkit/ckfetchwebauthtokenoperation`: `ev_086e5c6bd4c58f042381a99264eb5a31556990315b0d97c85915eca004f46a60`

## Matrix rows closed (framework `cloudkit`)

| Framework slug | Feature (as in data/cloudkit.json) | New status | BifrOSt path |
|---|---|---|---|
| cloudkit | Shared records | partial | springboard/ui/src/record_share.rs |
| cloudkit | Web auth tokens | partial | springboard/ui/src/web_service_token.rs |

Not closed: User discovery and identity (`os`), Per-app containers, Subscriptions and push, and the sync and
record rows already listed as partial by wave 2. The share is not yet stored in `record_store::Database`, and no
app or the shell uses either module.

## Gaps

- Shares are held in memory by the owner; there is no store for them and no sync to participants.
- No invitation sending, no identity lookup for participants, no expiry, no one-time request URLs.
- The web token has no expiry, cache or retry; the transport is supplied by the caller and is not written here.
