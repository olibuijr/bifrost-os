# Wave 3: CryptoTokenKit (token layer, smart card slots, token errors)

Official reference: https://developer.apple.com/documentation/cryptotokenkit (Smart Cards, Smart Card App
Extensions, Tokens, Errors sections).

REA evidence (rendered official landing page): `ev_c99120978850cdbd0e7b2226b83f2c5a7f84292fe81f317a2c146a5396ba6808`.

## What was built

Pure Rust models in `springboard/ui/src` (std only, no hardware, no transport, no unsafe). No SDK or shell wiring
yet; a driver or reader service would call these models.

- `springboard/ui/src/smartcard_slot.rs`: `SlotManager` (the reader slots, unique names, in add order), `Slot`
  (name, state, answer to reset), `SlotState` (`Missing`, `Empty`, `Probing`, `MuteCard`, `ValidCard`), `SlotEvent`
  (`CardInserted`, `CardRemoved`). A valid card must carry a non-empty answer to reset, and the answer is kept only
  for a valid card. Mute cards have a card but no answer to reset.
- `springboard/ui/src/token_watch.rs`: `TokenWatcher` (tokens by ID, sorted, with insert and remove events that
  carry the token's `TokenInfo`), `TokenInfo` (token ID, driver name, slot name), `TokenSession` (authentication
  state `Unauthenticated`, `Authenticated`, `Locked`; failure counting with a default limit of 3; `require`,
  `logout`), and `TokenError` (the ten error codes, each with a fixed `Display` message).

Both modules have a `#[cfg(test)] mod tests` block with behaviour tests. Expected values were worked out by hand
from the rules above. Tests were not run (no builds in this wave).

## Official API names mirrored

| Official | BifrOSt model |
|---|---|
| TKSmartCardSlotManager | `SlotManager` |
| TKSmartCardSlot, TKSmartCardSlot.State | `Slot`, `SlotState` |
| TKSmartCard (card present) | `SlotState::has_card`, `Slot::atr` |
| TKTokenWatcher, TKTokenWatcher.TokenInfo | `TokenWatcher`, `TokenInfo` |
| TKTokenSession (authentication state) | `TokenSession`, `AuthState` |
| TKTokenDriver, TKToken | `TokenInfo.driver_name`, `TokenInfo.token_id` (data only) |
| TKError, TKError.Code | `TokenError` |

Differences from the official API:
- Insertion and removal are returned as events from `insert`, `remove` and `report`. There are no handler closures.
- `TokenSession::authenticate` takes the outcome of the check; the model checks no PIN or signature.
- The error domain string is not mirrored, and raw error code values are not mirrored.
- Not built: the ATR parser (TKSmartCardATR), slot max APDU lengths, the token driver and app extension mechanism,
  smart card authentication configuration, the NFC slot session, persistent token and HSM support, and the
  using-keys-on-a-card path. These rows stay gap or n/a.

## Matrix rows closed (for the lead to update centrally)

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| CryptoTokenKit | Smart card slots and cards | partial | springboard/ui/src/smartcard_slot.rs |
| CryptoTokenKit | Token watcher, driver and session base classes | partial | springboard/ui/src/token_watch.rs |
| CryptoTokenKit | Errors | partial | springboard/ui/src/token_watch.rs (TokenError) |

Rows left as gap: smart card token driver and token; using cryptographic assets stored on a smart card;
authenticating users with a cryptographic token; smart card authentication configuration; persistent tokens (HSM).
The NFC smart card slot session stays n/a.

## Notes

- The shared index search (`code_search`, repo bifrost-os) timed out, so the repo search for existing token code
  used local grep. The file list of `springboard/ui/src` and `springboard/sdk/src` showed no smart card, slot or
  token module, and `springboard/sdk/src/keychain.rs` stays generic-password only.
- The owner name guard `python3 tools/check-names.py` exits 0.
