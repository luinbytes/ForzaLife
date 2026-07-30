# Continuity

## [PLANS]

- 2026-07-29T20:21+01:00 [USER] Review and re-enable empty-fuel behavior through the Linux virtual controller path.
- 2026-07-30T18:28+01:00 [USER] Implement the driving HUD roadmap beginning with derived driving state, session statistics, and fixed HUD modes.
- 2026-07-30T18:22+01:00 [USER] Add an in-game overlay reload action that preserves the running Forza process.

## [DECISIONS]

- 2026-07-29T20:21+01:00 [CODE] Keep starvation restricted to free-roam driving: empty fuel, usage enabled, `race_on`, position zero, and a driving gear.
- 2026-07-30T18:22+01:00 [CODE] Reload replaces only the overlay process with the current executable after saving simulation state; ordinary overlay closure does not reload.

## [PROGRESS]

- 2026-07-29T20:21+01:00 [TOOL] Current dirty worktree already contains `FuelStarvation` transition handling and virtual keyboard/controller throttle limiting; no duplicate implementation added.
- 2026-07-29T20:45+01:00 [TOOL] Live cache log showed proxy startup failures (`could not find gamepad with right trigger`); both reported symptoms share the resulting `InputProxy == None` state.
- 2026-07-29T20:45+01:00 [CODE] Added 2-second input-proxy startup retry and reapplied the starvation state after successful recovery.
- 2026-07-30T18:28+01:00 [CODE] Added `driving::DriveSnapshot` and resettable per-car `DriveSession`; integrated Drive, Minimal, and Life HUD modes plus menu actions for mode cycling and session reset.
- 2026-07-30T18:22+01:00 [TOOL] Added the final main-menu reload row, Wayland reload signal, process replacement, layout update, docs, and regression coverage.

## [DISCOVERIES]

- 2026-07-29T20:21+01:00 [CODE] The proxy starts neutral and receives the starvation state from the telemetry receiver; `FORZALIFE_DISABLE_INPUT_PROXY` remains the explicit opt-out.
- 2026-07-30T18:28+01:00 [TOOL] `cargo fmt --check`, strict clippy, release build, and the full host-permission `cargo test --locked` passed; live FH6/gamescope HUD validation remains unconfirmed.
- 2026-07-30T18:22+01:00 [TOOL] Full workspace tests passed with UDP binding allowed; fmt, clippy, release build, and diff checks passed. Live FH6 PID-preservation acceptance remains UNCONFIRMED.
