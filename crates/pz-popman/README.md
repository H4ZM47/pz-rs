# pz-popman

The zombie population: `zombie.popman` (`ZombiePopulationManager`,
`LoadedAreas`, `PlayerSpawns`, `RealZombieMap`, `ZombieStateFlags`,
`ObjectPool`), the Java side of `zombie.MapCollisionData`, and a pure-Rust
port of the game's native `PZPopMan` library (`src/native`).

## Provenance of `src/native`

The game ships `PZPopMan` only as a binary (`libPZPopMan.dylib`,
`PZPopMan64.dll`, `libPZPopMan64.so`). `src/native` was reconstructed from
the arm64 macOS binary's behaviour: its exported `Java_zombie_popman_*` and
`Java_zombie_MapCollisionData_*` entry points, a disassembly of what they
call, and differential runs. No part of the binary or of a decompilation is
in this repository.

The differential check is local and uncommitted: a small JVM program loads
the shipped library, seeds its `std::mt19937` and replaces its clock, then
replays a script of JNI calls; `examples/popman_probe` replays the same
script against this port, and the two outputs (zombies handed back,
radar counts, path results, the generator state after every tick, and the
SHA-256 of every save file) are compared line by line. `popman_probe gen`
writes such scripts by playing a scripted player against the port (chunk
loading and unloading with virtualization, registration, real-zombie
updates, sounds, hordes, aggro targets, debug commands, collision changes,
periodic saves).

## Save files

All written and read by the library, under
`GameModeCacheDir + GameSaveWorld`:

- `zpop/zpop_X_Y.bin`: one population cell (version 7, outfit names,
  last repopulation/redistribution hours, then per chunk the desired
  count, the stored zombies and the last-seen/last-repopulation hours).
- `zpop/zpop_virtual.bin`: walking virtual zombie groups.
- `zpop/zpop_ids.bin`: the persistent id counter (written through a
  `.tmp` file and renamed).
- `chunkdata/chunkdata_X_Y.bin`: the collision cache (version 1, per chunk
  a uniform type or 64 square bytes).

## Known differences

- A `WorldSound` that the worker frees can still be referenced by virtual
  zombies walking toward it, and the next sound may reuse its memory.
  Whether `malloc` returns the same block varies between runs of the
  original. The port reuses the most recently freed sound unless
  `Native::reuse_freed_sound` is cleared for a call. The difference shows
  up as virtual zombies that do or do not retarget to the new sound.
- A fresh `WorldSound`'s `lastApplied` field is uninitialized in the
  original. The port starts it at 0.
- The worker thread runs inline in the caller. The batches exchanged
  between the two sides are the same, so the results are the same as when
  the original's threads alternate one step at a time.
- `n_getRadarZombieData` and the path task's current position are written
  into Java arrays with `JNI_ABORT` in the original, so Java never sees
  them. The port returns them anyway.
- The ids of released zombies are written to the save in sorted order. The
  original writes them in `unordered_set` iteration order, which is not
  observable afterwards.

## Not ported

- Multiplayer only: `NetworkZombie*`, `ZombieCountOptimiser`,
  `MPDebugInfo` (the native side keeps the data it sends), server cell
  areas.
- `VirtualZombieManager` and `ZombiesZoneDefinition`. They create real
  `IsoZombie`s, which are reached through `manager::PopulationHost`.
- The `ZombiePopulationRenderer` drawing.
