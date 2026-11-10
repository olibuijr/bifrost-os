BifrOSt Browser bundles Servo 0.7.0 for Linux x86_64, built from the official
v0.7.0 source with the modifications listed below.
Servo's browser engine and servoshell frontend are primarily Rust. Servo uses
third-party native libraries including SpiderMonkey; this is not a pure-Rust
JavaScript engine. The first BifrOSt release uses Servo's upstream toolbar.

Corresponding source: https://github.com/servo/servo/tree/v0.7.0
Release and vendored source: https://github.com/servo/servo/releases/tag/v0.7.0
License: MPL-2.0, included as LICENSE-servo.txt; third-party notices/resources
remain inside the upstream archive. engine.json pins version and SHA-256.

Modifications to Servo's MPL-2.0 source files, as unified diffs in the BifrOSt
source tree under springboard/apps/browser/engine/patches/:
- 0001-page-color-scheme.patch: the page's <meta name="color-scheme"> applies
  to the root element as a presentational hint, and a top-level page whose
  root element uses a dark colour scheme gets a dark canvas.
