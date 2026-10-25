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
  `ToBullet` chunk shapes) and the `physicsCheck` flags.
- `manager`, `vehicle_type`, `spawn`: the vehicle id map and cell lists,
  `VehicleType`, `IsoChunk.AddVehicles` (`SpawnWorld`, `CellSpawnWorld`).
- `cell_link`: runtime vehicles in pz-world's `IsoChunk.vehicles`.
- `save`: conversion to and from `pz_world::save::vehicle::BaseVehicle`.
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
`AddVehicles` end to end (`--dispatch`) and `--e2e`: the original with the
real libPZBullet in the JVM against this crate on pz-bullet, on a
Muldraugh region's chunk shapes. `examples/chunk_physics_parity.rs`
compares the chunk shapes with the original's buffers.

| Track | Result |
|---|---|
| Saved vehicles, 8 saves | 241/243 exact; 1 matches up to the original's crash, 1 trailer animal |
| Vehicle scripts | 241/241 exact |
| Spawn helpers | 3,000/3,000 |
| `AddVehicles` | 2,000/2,000 |
| End to end on pz-bullet vs libPZBullet, 241 scripts, 400 frames | 240/241 exact; 1 matches up to the harness's missing world-sound native |
| Chunk shapes, 4 regions | 114,529/114,529 squares that load the same objects |
