# Linux overlay notes

The native Linux port lives in `linux/` and uses the authorized Windows implementation as the reference for its gameplay behavior, UI layout, colors, gauges, interactions, and supplied assets.

FH6 sends its Data Out packets over UDP. The Linux binary receives them on `127.0.0.1:8080` by default and renders its transparent overlay inside gamescope's nested Xwayland display. It marks its window with `GAMESCOPE_EXTERNAL_OVERLAY=1` so gamescope composites it above the Proton game while keeping the window click-through.

Run it through the Steam gamescope command:

```sh
gamescope <your flags> -- ~/.local/bin/forzalife session %command%
```

The session command starts the overlay for the game lifetime, forwards termination, and returns the game's exit code. The default port can be changed with `FORZALIFE_PORT`.

Validation uses the Rust test suite plus a live FH6 run: confirm the process is listening on the selected UDP port, read back `GAMESCOPE_EXTERNAL_OVERLAY` with `xprop` on the nested Xwayland display, and verify that input passes through the overlay to the game.

The Linux build embeds the Windows release's `cardata.csv` metadata and the 118 points of interest decoded from `WorldData/world_map.dat`. Its main menu, navigation submenu, vehicle card fields, selection behavior, four-second timeout, and default `L`, `;`, and `'` controls follow the decompiled v0.4.2-beta implementation.

## Implementation references

- [Upstream ForzaLife](https://github.com/puffinflight/ForzaLife) and its [v0.4.2-beta release](https://github.com/puffinflight/ForzaLife/releases/tag/v.0.4.2-beta) remain the authoritative Windows implementation and release reference.
- [Flowhooks window setup](https://github.com/luinbytes/flowhooks-cs2/blob/main/src/ui/window_context.rs) confirms the gamescope-compatible X11 approach: request a transparent GL surface and clear it with zero alpha.
- The local IR2 `app/src/overlay/wgpu_ctx.rs` implementation independently confirms premultiplied alpha with a transparent surface clear.
