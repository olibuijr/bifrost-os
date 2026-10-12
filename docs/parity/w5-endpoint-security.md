# Wave 5: Endpoint Security (pure Rust models)

Branch `parity/w5-endpoint-security`, built on `parity/integration` (waves 1 to 4). Two gap rows of the Endpoint
Security framework (`docs/parity-matrix/data/endpoint-security.json`) get pure-Rust models. Neither model is wired
into an app, the shell or a daemon yet, so both rows are `partial`. No build or test was run for this wave; the
tests are written and their expected values were worked out by hand.

## What was built

- `springboard/sdk/src/event_client.rs` (`bifrost_app::event_client`): the bookkeeping for a monitoring client.
  `ClientTable::open_client(Caller)` runs the checks in a fixed order (zero pid is an invalid argument, then
  missing entitlement, missing policy permission, not root, and table full) and returns a `ClientId` handle.
  `delete_client`, `delete_all`, `is_open` and `count` cover teardown. Handles are never reused. When handles run
  out, the result is `Internal`. The `OpenError` variants mirror the reference result set, minus success.
- `springboard/core/src/session_events.rs` (`session_events`): parses one authentication log line into a
  `SessionEvent`. It recognises `sudo` command lines (not failed password attempts), `sshd` accepted and failed
  logins (including `invalid user`), and `login` and `su` PAM sessions opened or closed. Lines from other processes
  return `None`. The module reads no journal or file itself.

## Official API names mirrored

- `es_new_client`, `es_delete_client`, `es_new_client_result_t` with `ES_NEW_CLIENT_RESULT_SUCCESS`,
  `_ERR_INTERNAL`, `_ERR_INVALID_ARGUMENT`, `_ERR_NOT_ENTITLED`, `_ERR_NOT_PERMITTED`, `_ERR_NOT_PRIVILEGED`,
  `_ERR_TOO_MANY_CLIENTS` (https://developer.apple.com/documentation/endpointsecurity/es_new_client_result_t).
- `es_event_sudo_t`, `es_event_openssh_login_t`, `es_event_login_login_t` and the matching notify event types
  (https://developer.apple.com/documentation/endpointsecurity, Event Types).

## REA evidence

- Framework landing page: `ev_6e7d1dd07f65b918f28cec15abebd9f8fb3647062573970d4f7c4bd62f41fbe4`
- `es_new_client_result_t` page: `ev_3d38e36d1a6e3f424d9b8ab361faab2ab110cab661ce7d78273c5f730df5f337`

## Matrix rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| Endpoint Security | Client creation and teardown | partial | springboard/sdk/src/event_client.rs |
| Endpoint Security | Login, logout, su, sudo, SSH and screen-sharing events | partial | springboard/core/src/session_events.rs |

## Still open

- Nothing consumes `ClientTable`; no client connects to a monitoring subsystem, so `Subscribing to events` still
  has nothing to subscribe to.
- `session_events::parse` has no feed. The journal would have to be read and each line passed to it.
- Screen-sharing, socket, pty, clock and kernel events, memory mapping, and authorization decisions remain gaps.
