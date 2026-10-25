# pz-vehicles

Project Zomboid's vehicle runtime (`zombie.vehicles`, the vehicle side of
`zombie.core.physics`), ported from the original's behaviour.

- `vehicle`, `part`, `engine`, `geometry`, `poly`: `BaseVehicle`,
  `VehiclePart`, `VehicleEngine`, polygons, areas, passenger positions.
- `controller`, `world_sim`, `physics`: `CarController`, `WorldSimulation`
  and the `VehiclePhysics` trait (one method per Bullet native);
  `physics::replay` answers calls from a recorded log.
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
new vehicle per script (`--scripts`), single-zone spawning (`--spawn`) and
`AddVehicles` end to end (`--dispatch`).

| Track | Result |
|---|---|
| Saved vehicles, 8 saves | 241/243 exact; 1 matches up to the original's crash, 1 trailer animal |
| Vehicle scripts | 241/241 exact |
| Spawn helpers | 3,000/3,000 |
| `AddVehicles` | 2,000/2,000 |
