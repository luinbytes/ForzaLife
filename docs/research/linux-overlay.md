# Linux overlay notes

The native Linux port lives in `linux/` and uses the authorized Windows implementation as the reference for its gameplay behavior, UI layout, colors, gauges, interactions, and supplied assets.

FH6 sends its Data Out packets over UDP. The Linux binary receives them on `127.0.0.1:8080` by default and renders its transparent click-through layer inside gamescope's nested Wayland display.

Enable the user watcher once, then keep the Steam launch option as gamescope only:

```sh
systemctl --user enable --now forzalife.service
gamescope <your flags> -- %command%
```

The watcher starts and stops one overlay for the game lifetime and selects the nested display automatically. Do not add `forzalife session` to the launch option. The default port can be changed with `FORZALIFE_PORT`. The main menu's `Reload overlay` action replaces only the overlay process in place, preserving Forza, gamescope, the telemetry port, and the inherited display environment; expect a brief redraw while the new process starts.

Validation uses the Rust test suite plus a live FH6 run: confirm the process is listening on the selected UDP port, verify the `forzalife-overlay` layer is on gamescope's nested Wayland display, and verify that input passes through the overlay to the game.

The Linux build embeds the Windows release's `cardata.csv` metadata and the 118 points of interest decoded from `WorldData/world_map.dat`. Its main menu, navigation submenu, vehicle card fields, selection behavior, four-second timeout, and default `L`, `;`, and `'` controls follow the decompiled v0.4.2-beta implementation.

The visual reference is the release's recovered `MainWindow`, `BoostGauge`, and `VehicleInfoCard` XAML. The Linux renderer uses their original 1920×1080 anchors, 290×220 menu frame, a compact 540×340 vehicle card anchored on the left, 400×291 bottom-right HUD, colors, bundled Roboto Condensed faces, and boost background asset.

The race HUD and navigation scale by `min(width / 1920, height / 1080)`, matching `MainWindow.RecalculateScale`. Fuel, battery, oil, and workshop indicators use masks rasterized from the release's `icons.xaml` geometries. The Linux-only Set odometer menu action accepts dashboard mileage in kilometers and updates the current car without resetting its trip odometer.

Known cars use the release database's per-model tank or battery capacity without replacing large legitimate tanks with a generic value. The cylinder fallback is only used when a database capacity is missing. FH6 Data Out does not expose a dependable fuel-flow sensor, so combustion usage follows the Windows release's telemetry model: measured engine power and torque-derived power at the current RPM and throttle are converted through an estimated BSFC derived from cylinder count and model year, with an engine-size idle allowance. RPM consumption is 1.25x through 50% of redline, then ramps smoothly through 2x at 75%, 3x at 90%, and 3.5x at redline. The persisted lifetime fuel and distance totals remain per-car, while the displayed imperial MPG and km/L use a 1.5 km recent-driving window so eco driving and hard driving respond promptly; refueling does not count as negative consumption.

The bundled map data contains 118 gas stations, workshops, and convenience stores as world-space points only. It supports a nearest-destination waypoint arrow and distance readout, but not turn-by-turn road routing: there is no road graph, intersection connectivity, lane direction, or route geometry. Road guidance would require a separate extracted or authored network of connected road nodes and edges plus a world-to-map projection.

## Implementation references

- [Upstream ForzaLife](https://github.com/puffinflight/ForzaLife) and its [v0.4.2-beta release](https://github.com/puffinflight/ForzaLife/releases/tag/v.0.4.2-beta) remain the authoritative Windows implementation and release reference.
- [Flowhooks window setup](https://github.com/luinbytes/flowhooks-cs2/blob/main/src/ui/window_context.rs) confirms the gamescope-compatible X11 approach: request a transparent GL surface and clear it with zero alpha.
- The local IR2 `app/src/overlay/wgpu_ctx.rs` implementation independently confirms premultiplied alpha with a transparent surface clear.
