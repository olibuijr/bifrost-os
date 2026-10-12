# Wave 4, theme-25: spatial mixing and reverb (PHASE)

## What was built

Three pure modules in `springboard/ui/src` (bifrost-ui). None is wired into the shell or an app yet, and none has
run on the VM or Midget. There is no output path and no sample processing.

- `sound_space.rs`: a listener (position, unit forward, unit up and right, right-handed frame with x to the right).
  `Listener::hear` gives distance, azimuth, elevation and a pan value for a source. `pan_gains` is an equal-power
  pan law. `Occluder` is an axis-aligned box with a transmission gain, and `occlusion_gain` multiplies the
  transmissions of every box the direct path crosses (slab test).
- `sound_room.rs`: `Medium` (speed of sound, air at 343 m/s, travel delay), `Room` (volume, surface area and one
  average absorption coefficient) with the Sabine reverberation time and the critical distance, and `direct_share`
  (direct energy against a constant reverberant field, Dc^2 / (Dc^2 + d^2)).
- `sound_group.rs`: `Group` (gain 0 to 1 for a set of sounds) and `Ducker` (threshold, duck depth, attack and
  release in seconds, one-pole smoothing with the caller's block length).

Each module has its own `#[cfg(test)] mod tests` with hand-worked values. Lines added to `springboard/ui/src/lib.rs`:
`pub mod sound_space;`, `pub mod sound_room;`, `pub mod sound_group;`.

## Official API names mirrored

Official pages read with `python3 -I tools/parity/docc.py`:

- `phase`: the landing page groups Soundscape Creation (PHASESource, PHASEListener, PHASEOccluder, PHASEShape,
  PHASEMaterial), Setup (PHASEReverbPreset, PHASEMedium) and Sound Grouping and Management (PHASEGroup, PHASEDucker).
- `phase/phaseducker`: "An object that manages competing sounds."
- `phase/phaselistener`: "A central point of reference that defines the location within the scene that's most audible
  to the user."
- `phase/phasereverbpreset`: "The manner in which PHASE diffuses resonating sound."

Mirrored as pure models: PHASESource and PHASEListener (`Listener`, `Hearing`), PHASEOccluder and PHASEShape
(`Occluder`, box shapes only), PHASEMedium (`Medium`), PHASEReverbPreset (`Room` and its reverberation time, not a
preset list), PHASEGroup (`Group`) and PHASEDucker (`Ducker`).

Not mirrored: PHASEMaterial and PHASEMaterialPreset (one transmission value per box, no per-band material), spatial
output and HRTF, reverb rendering, and the engine, asset registry and update-mode objects.

## REA evidence

- `ev_596dd3292ef4f9730e06d9fbf80bd08cddfadf7fd0519c9af13227e5e28fcb55`: rendered page
  `https://developer.apple.com/documentation/phase` (`tools/parity/rea-page.sh`).
- Earlier wave-3 evidence for the same landing page, already in the matrix: `ev_ae3ef8f049211241c669b63265081488bc102f10cbbfb71dd8b9618b536127bc`.

## Matrix rows closed (status partial)

| framework slug | feature (as in data/phase.json) | new status | BifrOSt path |
|---|---|---|---|
| phase | Spatial sources and listener | partial | springboard/ui/src/sound_space.rs (Listener, Hearing, pan_gains) |
| phase | Occluders, shapes and materials | partial | springboard/ui/src/sound_space.rs (Occluder, occlusion_gain; boxes only, one transmission value, no materials) |
| phase | Spatial mixing and reverb | partial | springboard/ui/src/sound_room.rs (Medium, Room, direct_share; no reverb rendering or presets) |
| phase | Groups and ducking | partial | springboard/ui/src/sound_group.rs (Group, Ducker; not shared across apps) |

## Limits

- All four rows are partial, not parity. Nothing is connected to `springboard/audiod` or the SDK yet, so apps cannot
  use the models.
- `Occluder` takes one transmission value and an axis-aligned box. Shapes with elements and material presets are
  not modelled.
- `Room` uses one average absorption coefficient, as Sabine's rule does. It does not mirror PHASEReverbPreset's
  named presets.
- The build was not run (no cargo in this round). The modules are checked by reading only.
