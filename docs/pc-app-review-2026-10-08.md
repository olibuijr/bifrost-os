# Integrated PC app and update review — 2026-10-08

This batch integrates the completed HTTPS hardening and the independently packaged PC app ports. The platform maturity program remains partially implemented: SDK decoupling, stable app identity, document grants, privilege brokers, lifecycle restoration, accessibility and measured rendering work are still backlog items.

## Improvements

- Desktop entry points own their package identities rather than sharing entry points. Icons are checked into source so a clean checkout can package the apps.
- Physical keyboards retain editing focus without an on-screen keyboard. Calculator accepts arithmetic keys; scientific exponent labels use ordinary SF UI letters composed at smaller sizes.
- Books scans user document folders, resolves file URLs and saves reading state on suspend. Freeform exports to Pictures; Music scans the user Music folder.
- Notifications require the manifest permission and are rate limited per running app.
- Update, App Store and diagnostics requests reject HTTP, including redirects. Existing publisher signature output remains for compatibility with older clients.
- Atomic persistence creates exclusive temporary files and cleans up only files it created. Directory synchronization remains best effort.
- Package stripping preserves local profiling symbols. Files and Remote versions are advanced for rebuilt SDK dependencies; Notes is 1.0.2.

## Known limitations

PC media playback/recording depends on an audio daemon which is not installed on the inspected Midget system. No audio was played or recorded. Shortcuts execution and Contacts sharing remain unsupported on PC. Clock background alarms and closed-app reminder/calendar scheduling are not implemented. Store summaries now describe these limitations where relevant.

SDK app smoke checks exercise real binaries over an isolated socketpair, checking manifest identity, descriptor buffers, nonuniform rendering, resize, suspend/resume and clean exit. They do not prove compositor input, installed sandbox policy or hardware functionality. Browser has its own native Wayland surface and is outside this SDK smoke test.

## Evidence

Final build, workspace acceptance and Python tool results are recorded in `out/pc-release-build.log`, `out/pc-release-acceptance.log` and `out/pc-release-python-tests.log`. App frame captures and protocol results are in `out/pc-app-smoke/`. See the release handoff for published versions and device verification.

Review checkpoint: the combined optimized fast build finished in 3m 55s. All 17 SDK desktop app binaries passed identity/render/resize/lifecycle smoke checks. The 150 Python tooling tests passed. DAW is another agent’s independent release and is excluded. Browser source/version is unchanged and does not need republication.

Final acceptance passed: 3,084 tests, zero failures, five ignored. The package-resume fixture uses a controlled transport with HTTPS enforcement, retaining range and checksum coverage. The final system binary was built from clean commit bd6c967e4a51; build revision tracking now follows branch refs and linked worktrees.

Published OS: `springboard-shell 0.4.31.1791470427-1`. Published 17 app packages: the fourteen new ports at 1.0.0, Notes 1.0.2, Files 1.0.6 and Remote 0.1.6. Public catalogue versions, sizes and hashes match; a Calculator HTTPS package download was verified. Midget's guarded transaction installed the package and is `system-prepared`, retaining 0.4.30.1791467826-1 as previous. A reboot is still required for generation activation and probation. No reboot or app installation was performed in this batch.
