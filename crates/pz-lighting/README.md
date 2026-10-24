# pz-lighting

Pure-Rust replacement for the game's native lighting library
(`libLighting`), the backend of `zombie.iso.LightingJNI`. No native code at
runtime, no threads, no unsafe.

## What it computes

Per local player, from the squares the game streams in chunk by chunk:

* visibility (`seen`, `canSee`, `couldSee`) with a cached line-of-sight
  walk from the player's square, the vision cone and the perception range;
* sunlight: open-air columns are seeded with the sky level and flooded
  indoors (-16 per square, -32 across levels, scaled through closed edges
  by their curtain/door/barricade transmission);
* point lights, temporary lights and room lights, flooded into per-square
  light lists of at most five entries;
* torch light (flashlights, headlights) on the player's level;
* the per-square light colour, the darkness multiplier, and eight vertex
  colours (four floor corners, four wall tops).

## API

`LightingSession` has one method per native of `LightingJNI`
(`state_begin_update`, `player_set`, `state_end_frame`, `chunk_begin_update`,
`square_set`, `square_set_light_transmission`, `add_light`,
`update_torch`, `state_end_update`, `do_lighting_update`,
`get_square_lighting`, ...), with the same arguments and the same
`IllegalArgument`/`IllegalState` errors. Levels are native levels (`z + 32`).

The native library computes on a lighting thread and double-buffers its
results. The port is synchronous: `state_end_update` hands the queued
changes over (or, when an update has finished, swaps its results in), and
`do_lighting_update` runs the update. Results lag the inputs by one
hand-off, exactly as in the game.

`get_square_lighting` returns the 44-int block the Java side decodes;
`SquareLighting` gives it names.

For a frame without a player (a map render at noon), set
`Character::see_all`: every square is visible with a darkness multiplier of
1, so the colours come straight from sun, sky and lights.

## Exactness

The port follows the arm64 build of the library: `f32` throughout, fused
multiply-adds exactly where the binary has them, truncating conversions,
the one `f64` multiply in `getSquareLighting`, and the native iteration
orders where they matter (the visibility pass fills the line-of-sight cache
in visit order, the floods are FIFO, the vertex blending is in place).

The sunlight flood is seeded from a `std::unordered_set` keyed by heap
address in the native code, and the flood replaces all three channels of a
square when any one improves. When candidates from different seeds are not
ordered channel by channel (coloured transmissions, or a non-grey sky
reaching a square through both open and closed edges), the result depends
on that hash order, which cannot be reproduced. The port floods the seeds
in chunk/column order; wherever the native result does not depend on the
order, the two agree. (A hidden `debug_set_sky_seed_order` hook lets a
parity harness supply the native order.)

Results were compared bit-for-bit against the real library through a
local, uncommitted harness that loads it with a minimal fake `JNIEnv` and
drives both with the same synthetic call sequences.

## Known differences

* See-everything mode: the native code leaves the target darkness
  unwritten (uninitialised memory); the port writes 1.
* Arrays the native code leaves uninitialised read as zero here (for
  example the targets of every square when the player stands outside every
  square).
* Matching was verified for the arm64 macOS build only.
