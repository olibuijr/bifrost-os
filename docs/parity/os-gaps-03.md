# OS policy and ambient models

This branch adds an intent parameter form, per-account app and feature restrictions, lux conversion/smoothing and
brightness policy, and recent-use suggestions for audio routes. These are pure models: no shortcut editor draws the
form, no account service persists or enforces the policy, no IIO reader changes display brightness, and the audio route
picker does not yet consume the suggestions.

Headless tests exercise the typed form, policy checks, sensor math and deterministic route ranking. The corresponding OS
rows remain partial until the shell integrations and Midget checks are complete.
