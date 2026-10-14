# PHASE engine, sampler loops and loudness (sdk-complete-32)

Mirrors: the PHASE engine and asset registry (`PHASEEngine`, `PHASEAssetRegistry`, `PHASEPlaybackMode`) and the
normalization mode of a sound.

Code, all pure and device-free:

- `springboard/src/media/engine.rs`: `Engine` (start and stop, update mode locked while running, `play` by asset name)
  and `AssetRegistry` (named assets with a path and a loop mode, unique names, sorted listing).
- `springboard/src/media/loop_region.rs`: `LoopMode` (off, forward, ping-pong), `LoopRegion` and the read `Head`; exposed
  to apps as `bifrost_app::kit::loop_region`.
- `springboard/src/media/volume.rs`: `normalization_gain_db` (target and boost limit) and `normalized_linear`.
- `springboard/tests/parity_sdk-complete-32.rs`: headless tests of all four.

Wired: the sound server (`springboard/audiod`) still owns the speaker and the mixer; the player and the DAW sampler voice
keep their own loop points. Nothing yet sends an engine's asset to the server.

Needs the lab machine: an asset heard through the server, a sampler loop heard on the speakers, and a calibration of the
output device.
