# pz-vehicles

Project Zomboid's vehicle runtime (`zombie.vehicles`, the vehicle side of
`zombie.core.physics`), ported from the original's behaviour.

- `vehicle`, `part`, `engine`, `geometry`, `poly`: `BaseVehicle`,
  `VehiclePart`, `VehicleEngine`, polygons, areas, passenger positions.
- `controller`, `world_sim`, `physics`: `CarController`, `WorldSimulation`
  and the `VehiclePhysics` trait (one method per Bullet native, with the
  natives' Java exceptions and the `stepSimulation` callbacks);
  `physics::bullet` implements it on `pz_bullet::PzBullet`,
  `physics::replay` answers calls from a recorded log.
- `chunk_physics`: `IsoChunk.updatePhysicsForLevel` / `calcPhysics` (the
  `ToBullet` chunk shapes) and `updatePhysicsForLevelIfNeeded` on
  pz-world's `IsoChunk` physics-check flags.
- `manager`, `vehicle_type`, `spawn`: the vehicle id map and cell lists,
  `VehicleType`, `IsoChunk.AddVehicles` (`SpawnWorld`, `CellSpawnWorld`).
- `cell_link`: runtime vehicles in pz-world's `IsoChunk.vehicles`.
- `surround`: `SurroundVehicle`, the attack spots around a vehicle behind
  `chooseBestAttackPosition` (zombies as `SurroundZombie` snapshots; the
  blocked check through `VehicleWorld::line_clear_collide` /
  `can_stand_at`).
- `save`: conversion to and from `pz_world::save::vehicle::BaseVehicle`.
- `vehicles_db`, `virtual_vehicle`: `VehicleBuffer.set` rows for
  `vehicles.db`; `VirtualVehicle` / `VirtualVehicleManager` (vehicles that
  left with their chunk but keep their alarm, siren or engine running) and
  `VirtualVehicleMeta`, pz-world's `VehicleMetaHook`. `env::set_vehicle_ids`
  installs `VehiclesDB2`'s id sets for `addToWorld` / `removeFromWorld`.
- `pathfind_view`: what pz-pathfind reads; `pathfind_adapter!(pz_pathfind)`
  implements its traits in a crate that depends on both.
- `env`: game state outside vehicles (clock, sandbox, world, items,
  events such as sounds and the lightbar's world lights).
- `lua`: calling the vehicle Lua by name; bindings live in
  `pz-lua-api::bind_vehicles`.

## Parity

`pz-lua-api/examples/vehicle_runtime_parity.rs` replays the original's
runs (a local JVM harness with a recording Bullet stand-in, fake clock and
scripted driving; nothing from it is in the repository) and compares every
per-frame dump field and Bullet call. Modes: saved `vehicles.db` rows, one
new vehicle per script (`--scripts`), single-zone spawning (`--spawn`),
`AddVehicles` end to end (`--dispatch`), `SurroundVehicle` call sequences
(`--surround`), `VP_META=1` (vehicles leaving with their chunk into the
meta, `VehiclesDB2` id allocation, `VirtualVehicleManager` updates, saves
and re-adds) and `--e2e`: the original with the
real libPZBullet in the JVM against this crate on pz-bullet, on a
Muldraugh region's chunk shapes. `examples/chunk_physics_parity.rs`
compares the chunk shapes with the original's buffers.

| Track | Result |
|---|---|
| Saved vehicles, 8 saves, item load draws live on the vehicle stream (no post-load reseed) | 240/243 exact; 2 match up to the original's own crash (a broken window with no square), 1 trailer animal; load draws match for all 242 loadable |
| Vehicle scripts | 241/241 exact |
| Spawn helpers | 3,000/3,000 |
| `AddVehicles` | 2,000/2,000 |
| `AddVehicles` on real map chunks and zones (`examples/spawn_map_parity.rs`): 26 regions of Muldraugh, West Point, Louisville and traffic jams | 1,664/1,664 chunks (347 vehicles); pz-world's vehicle zones identical for all 563 cells |
| `SurroundVehicle`, 2,000 sequences (110,463 `getPositionForZombie` calls, `update` between) | 2,000/2,000 |
| End to end on pz-bullet vs libPZBullet, 241 scripts, 400 frames | 240/241 exact; 1 matches up to the harness's missing world-sound native |
| Meta (`VP_META`), 8 saves, 242 vehicles: id allocation, chunk unload into the meta, 120 meta frames, re-adds, game save | 10,270/10,270 lines exact (ids, 4,060 meta records with their vehicles, 182 queued rows' headers and blob hashes, 12 re-adds) |
| Chunk shapes, 4 regions | 114,529/114,529 squares that load the same objects |
