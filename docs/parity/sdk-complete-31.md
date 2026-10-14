# Parity sdk-complete-31

Pure models and tests for finishing six partial rows: media library types, groups and objects, passkey authenticator
data, scene sessions for several windows, Auto Layout priorities and guides, and lazily evaluated images.

## What mirrors what

| Row | Module | Mirrors |
|---|---|---|
| Media types | `springboard/ui/src/media_kind.rs` (`kind_of_name`) | the picture, audio and movie file kinds, decided from the name |
| Media groups | `springboard/ui/src/media_group.rs` (`MediaGroup`, `GroupType`) | a tree of named groups with a group type, unique ids and subtree item listing |
| Media objects | `springboard/ui/src/media_attributes.rs` (`MediaAttributes`, `MediaObject`) | a typed attribute dictionary per media object |
| Passkeys | `springboard/sdk/src/passkey_authenticator_data.rs` (`AuthenticatorData`, `assertion_signed_bytes`) | authenticator data bytes: rpIdHash, flags, counter, attested credential, extensions |
| Windows | `springboard/sdk/src/scene_session.rs` (`SceneSessions`) | several scenes of one app, focus, limit and lifecycle callbacks |
| Layout | `springboard/sdk/src/view/autolayout.rs` (`Spec::priority`, `Constraints::guide`) | Auto Layout priorities and layout guides, already in place and now tested through the public path |
| Core image | `springboard/sdk/src/lazy_image.rs` (re-exports and `filter`) | lazily evaluated images with a render context |

## Tests

- `springboard/ui/tests/parity_sdk-complete-31.rs`: media kinds, groups and attributes.
- `springboard/sdk/tests/parity_sdk-complete-31.rs`: scene sessions, authenticator data, layout priorities and guides,
  and lazy images.
- Unit tests in each new module.

## What still needs the lab machine

- The Music and Photos apps do not yet read the shared kind table or the attribute dictionary.
- Several windows, the lazy image path and the authenticator data are not yet checked on Midget.
- No key generation, signing or credential registration exists; a platform authenticator needs hardware.
