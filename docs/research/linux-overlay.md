# Linux implementation research

## Decision

Build a separate, clean-room Rust companion under `linux/`. Do not port, inspect, decompile, redistribute, or derive code, assets, data files, formulas, dialogue, or behavior from the Windows release.

For the first working overlay, use a native X11/ARGB Rust window inside gamescope's existing Xwayland sandbox and mark it with the gamescope-specific `GAMESCOPE_EXTERNAL_OVERLAY` X property. This is the shortest route that preserves Proton's current display model and does not require Wine transparency or changes to the Windows application.

Keep Wayland layer-shell as the second backend only if the X11 alpha smoke test fails on the target machine. Gamescope supports it, but a normal gamescope child receives X11 by default; standard Wayland client discovery requires `--expose-wayland` or explicitly connecting the overlay to `GAMESCOPE_WAYLAND_DISPLAY`.

## Source and legal boundary

The upstream checkout contains only `README.md`, with no source or repository license. GitHub says that, without a license, default copyright applies and others may not reproduce, distribute, or create derivative works, although GitHub's Terms permit viewing and forking a public repository through GitHub's functionality ([GitHub licensing guidance](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository), [GitHub Terms, D.5](https://docs.github.com/en/site-policy/github-terms/github-terms-of-service#5-license-grant-to-other-users)).

The portable `v.0.4.2-beta` archive includes `License.txt`. Its restrictions prohibit decompilation, reverse engineering, deriving source, modification, adaptation, translation, and derivative works based on the compiled software ([release](https://github.com/puffinflight/ForzaLife/releases/tag/v.0.4.2-beta), [portable archive containing the EULA](https://github.com/puffinflight/ForzaLife/releases/download/v.0.4.2-beta/ForzaLife_Portable_v.0.4.2-beta.zip)).

Consequences:

- The new Rust code must be independently authored from public interfaces only.
- Do not inspect IL, strings, resources, network behavior other than the documented Forza UDP feed, or the proprietary `WorldData`, audio, image, and data files in the release.
- Do not copy the Windows archive or its assets into the fork or Linux packages.
- Put the new code's license and clean-room notice inside `linux/`; do not add a root license that appears to relicense upstream's README.
- Preserve the upstream README and Windows download path. The Linux program is an additional companion, not a replacement or port of the proprietary Windows binary.
- Direct telemetry gauges are safe to implement from the official packet. Fuel/oil formulas and location data must be independently designed. Start with user-marked service locations; ship surveyed coordinates later only with provenance and permission.

This is an engineering boundary, not legal advice.

## Verified public interface

Forza Support documents FH6 Data Out as one-way UDP, sent at the game frame rate, including to `127.0.0.1`. It defines one fixed 324-byte packet and documents the ordered fields, including race state, timestamp, RPM, position, speed, boost, fuel, distance, car ordinal, and player input values. It warns against ports 5200 through 5300 ([official FH6 Data Out documentation](https://support.forza.net/hc/en-us/articles/51744149102611-Forza-Horizon-6-Data-Out-Documentation)).

On 2026-07-25, a local capture from the running FH6 instance bound to `127.0.0.1:8080` received 590 packets in five seconds; every datagram was exactly 324 bytes. This independently confirms the documented packet length on the target installation. Keep one sanitized live datagram as a parser test fixture, after confirming it contains no user identifier.

The official page does not state byte order. Do not silently assume it. The parser check should validate the candidate endian interpretation against documented ranges such as `IsRaceOn`, `CarClass`, `Gear`, `Accel`, and plausible RPM/speed values from the live fixture.

## Why the old overlay failed

Gamescope's own README says a nested game runs in a personal Xwayland sandbox desktop, isolated from the outer desktop ([gamescope README](https://github.com/ValveSoftware/gamescope#readme)). A Wine/WPF window launched on the outer display therefore cannot overlay the game. When launched inside gamescope, the current WPF transparent window renders as an opaque black rectangle on this machine. A native overlay must be a client of gamescope's own display and supply a real alpha surface.

Proton is a Wine-based Steam compatibility tool, not a compositor. Steam launch options can prepend commands before `%command%`; gamescope is what creates the nested display ([Proton README](https://github.com/ValveSoftware/Proton#runtime-config-options)). Gamescope sets its child `DISPLAY` to the nested Xwayland server, always publishes `GAMESCOPE_WAYLAND_DISPLAY`, and publishes standard `WAYLAND_DISPLAY` only with `--expose-wayland` ([gamescope source](https://github.com/ValveSoftware/gamescope/blob/17baf4abd1ab3353fb705e4d0d023f84e870f7e8/src/main.cpp#L1048-L1070)).

## Overlay backend comparison

| Route | Evidence | Cost and risk | Decision |
| --- | --- | --- | --- |
| X11 window inside gamescope | Gamescope defines `GAMESCOPE_EXTERNAL_OVERLAY`, reads it from X11 clients, and composites the selected external overlay separately ([definition and detection](https://github.com/ValveSoftware/gamescope/blob/17baf4abd1ab3353fb705e4d0d023f84e870f7e8/src/steamcompmgr.cpp#L1110-L1111), [window classification](https://github.com/ValveSoftware/gamescope/blob/17baf4abd1ab3353fb705e4d0d023f84e870f7e8/src/steamcompmgr.cpp#L4845-L4851)). X11 is exposed to children by default. | One transparent Rust window plus one X property. No change to Proton's backend. Must verify ARGB alpha and click-through in a real gamescope session. | Implement first. |
| Wayland layer-shell inside gamescope | Gamescope registers wlr-layer-shell v4 and classifies new layer surfaces as external overlays ([registration](https://github.com/ValveSoftware/gamescope/blob/17baf4abd1ab3353fb705e4d0d023f84e870f7e8/src/wlserver.cpp#L2108-L2115), [classification](https://github.com/ValveSoftware/gamescope/blob/17baf4abd1ab3353fb705e4d0d023f84e870f7e8/src/wlserver.cpp#L1974-L1985)). Smithay Client Toolkit exposes the protocol in Rust ([SCTK layer-shell API](https://smithay.github.io/client-toolkit/smithay_client_toolkit/shell/wlr_layer/index.html)). | Correct Wayland semantics, but more protocol/event-loop code. Needs `--expose-wayland` or an explicit connection to `GAMESCOPE_WAYLAND_DISPLAY`; if exposed globally, unset `WAYLAND_DISPLAY` for Proton so the game remains on its proven Xwayland path. | Fallback after the X11 smoke test, not parallel initial work. |
| Plain outer-desktop winit/egui window | winit can request transparency and cursor pass-through, but `AlwaysOnTop` is unsupported on Wayland ([winit transparency](https://docs.rs/winit/latest/winit/window/struct.WindowAttributes.html#method.with_transparent), [cursor hit testing](https://docs.rs/winit/latest/winit/window/struct.Window.html#method.set_cursor_hittest), [window levels](https://docs.rs/winit/latest/winit/window/enum.WindowLevel.html)). | Cannot guarantee placement over a fullscreen gamescope window and repeats the workspace/display problem already observed. | Reject. |

The wlr layer-shell protocol defines an overlay layer, defaults to no keyboard focus, and explicitly directs clients to use an empty Wayland input region for pointer pass-through ([protocol](https://wayland.app/protocols/wlr-layer-shell-unstable-v1)). The core Wayland protocol confirms that input outside a surface's input region falls through to the next surface ([`wl_surface.set_input_region`](https://wayland.app/protocols/wayland)). These are the click-through rules for the fallback backend.

## Minimal architecture

Use one Rust binary, not a speculative multi-crate framework:

```text
linux/
  Cargo.toml
  src/
    main.rs          session, UDP receive loop, state updates
    telemetry.rs     exact 324-byte parser
    model.rs         independently designed fuel/oil/odometer state
    overlay.rs       egui/eframe view plus gamescope X11 property
    input.rs         shortcuts; optional scoped brake output
  tests/fixtures/    one sanitized 324-byte packet
  LICENSE
  CLEAN_ROOM.md
```

Keep `telemetry` and `model` free of OS APIs so they are reusable if an authorized Windows Rust frontend is ever added. Do not add a Windows Rust backend now: the existing Windows distribution remains untouched, and there is no licensed Windows source to share or replace.

Use:

- `std::net::UdpSocket` and `from_*_bytes` for telemetry. No networking framework or binary parser dependency is needed.
- `eframe`/`egui` for a small transparent GPU-rendered HUD.
- `x11rb` only to set `GAMESCOPE_EXTERNAL_OVERLAY` and, if proven necessary, the X11 input shape.
- A small human-readable config/state file under the XDG config/state directories. Persist atomically with write-then-rename.

Render from the newest telemetry snapshot rather than queueing every frame. UDP loss is acceptable for a live gauge; state changes that affect persistence should be derived from monotonic timestamps and committed at a modest interval.

## Input without broad injection

For opening the menu, use the XDG Global Shortcuts portal. It provides shortcuts regardless of focused window and explicitly asks the user to approve/configure bindings ([portal documentation](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html)). When the menu is closed, the overlay remains click-through and never takes focus. If the menu needs navigation, temporarily enable its input region/focus and release it on close.

Do not read all of `/dev/input` by default. Add gamepad navigation only after the keyboard flow works and a real controller is available for testing; use a maintained gamepad mapping library rather than raw device heuristics.

Fuel-starvation braking cannot be expressed through the one-way Data Out protocol. Treat it as a separate, disabled-by-default capability:

1. First test XTEST against the focused game on gamescope's private Xwayland display. This limits synthetic input to the nested display instead of the whole desktop.
2. Only if XTEST is ineffective, offer an explicit `/dev/uinput` mode with a narrow udev rule and visible consent. The kernel documents uinput as creating a virtual device and delivering its events to kernel and userspace consumers ([kernel uinput documentation](https://www.kernel.org/doc/html/latest/input/uinput.html)).
3. Preserve the public behavior constraint from the README: never emit braking while `IsRaceOn` indicates a race. Make release of a held key unconditional on shutdown/error.

The telemetry HUD, persistence, menu, and service locations do not need input injection.

## Lifecycle and installation

Do not run a permanent polling service. Install one binary and a tiny session wrapper to `~/.local/bin`, then put the wrapper inside the existing Steam gamescope launch command:

```sh
gamescope <the user's existing flags> -- forzalife-session %command%
```

`forzalife-session` starts the native overlay on the inherited gamescope display, starts the exact Proton `%command%`, forwards termination signals, releases any held synthetic input, stops the overlay when the game command exits, and returns the game's exit status. This gives start/stop lifecycle without process-name polling or a user systemd service.

Ship a versioned `tar.gz` containing the release binary, wrapper, license, clean-room notice, and install/uninstall instructions. Build it in GitHub Actions with `cargo build --release --locked`. An AppImage, Flatpak, daemon, privileged installer, and distribution-specific packages are unnecessary until a portable tarball has real compatibility failures.

## Implementation and proof plan

1. Add `linux/CLEAN_ROOM.md`, a license for new Linux code only, the standalone Cargo package, and the exact packet layout transcribed from the official FH6 page.
2. Write one parser test using the sanitized 324-byte live fixture. Reject any non-324-byte datagram. Prove byte order through documented value ranges.
3. Create one transparent X11 overlay inside gamescope, set `GAMESCOPE_EXTERNAL_OVERLAY=1`, and make it mouse-pass-through. Render only packet status, RPM, speed, and boost.
4. Exercise it over a test client and then FH6. Verify with a screenshot that black pixels remain game pixels, use `xprop` to read back the gamescope property, and click through the overlay to the game.
5. Add the session wrapper and test normal exit plus SIGTERM cleanup with a dummy child before using it in Steam.
6. Add independently designed fuel, oil, and odometer state. Key identity may use `CarOrdinal`; document the same-model limitation already stated publicly in the README.
7. Add user-marked gas/workshop positions using documented `PositionX/Y/Z`. Do not import the proprietary `world_map.dat`.
8. Add the portal shortcut and interactive menu. Test optional scoped XTEST braking last, with race suppression and fail-safe key release.
9. Package the tarball and verify a clean install, FH6 launch, telemetry receipt, overlay visibility, click-through, and cleanup. Confirm the upstream Windows README/download path still works and no Windows release content was modified or bundled.

## Rejected alternatives

- Port or decompile the WPF application: prohibited by the release EULA and impossible from the README-only repository.
- Reuse release artwork, audio, world data, vehicle data, or dialogue: proprietary and unnecessary for the first useful Linux build.
- Keep running the WPF binary under Wine: its transparent surface is the confirmed black rectangle, and it remains on the wrong display when launched outside gamescope.
- Inject Vulkan/OpenGL code into FH6, build a DLL overlay, or hook Proton: violates the project's public non-invasive model and adds anti-cheat risk.
- Build both X11 and Wayland backends before one is exercised: duplicate work. The default gamescope X11 route is the first proof target.
- Add a background daemon or systemd watcher: the Steam child lifecycle already provides exact start and stop events.
