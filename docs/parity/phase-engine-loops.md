# PHASE engine, sampler loops and loudness (sdk-complete-32)

Mirrors: the PHASE engine and asset registry (`PHASEEngine`, `PHASEAssetRegistry`, `PHASEPlaybackMode`) and the
normalization mode of a sound.

Code, all pure and device-free:

- `springboard/src/media/engine.rs`: `Engine` (start and stop, update mode locked while running, `play` by asset name)
  and `AssetRegistry` (named assets with a path and a loop mode, unique names, sorted listing).
- `springboard/src/media/loop_region.rs`: `LoopMode` (off, forward, ping-pong), `LoopRegion` and the read `Head`; exposed
  to apps as `bifrost_app::kit::loop_region`.
- `springboard/src/media/volume.rs`: `normalization_gain_db` (target and boost limit) and `normalized_linear`.
- `springboard/src/media/push_queue.rs`: `PushQueue` (bounded sample blocks with priorities, exact-length pulls and
  underrun counts); exposed as `bifrost_app::kit::push_queue`.
- `springboard/src/media/mixer.rs`: `mix` (gain and mute strips summed with clipping counted) and `db_to_linear`;
  exposed as `bifrost_app::kit::mixer`.
- `springboard/tests/parity_sdk-complete-32.rs`: headless tests of all of the above.

Wired: the sound server (`springboard/audiod`) still owns the speaker and the mixer; the player and the DAW sampler voice
keep their own loop points. Nothing yet sends an engine's asset to the server.

Needs the lab machine: an asset heard through the server, a sampler loop heard on the speakers, and a calibration of the
output device.
