# ForzaLife context

## Purpose

ForzaLife adds persistent vehicle-life simulation around Forza telemetry. This fork ports the Windows application to a native Linux companion.

## Vocabulary

- **Data Out packet**: one 324-byte FH6 UDP telemetry datagram.
- **Telemetry snapshot**: the newest validated packet available to the UI.
- **Overlay**: a transparent, non-interactive gamescope surface displaying live telemetry and companion state.
- **Session wrapper**: the Steam launch-command entry point that owns the overlay and game child lifecycles.

## Boundaries

- Linux code consumes FH6 Data Out UDP telemetry.
- Linux code does not inspect or modify the game process.
- The authorized Windows implementation is the reference for Linux behavior, presentation, and assets.
- The upstream Windows usage and download documentation remains available.
