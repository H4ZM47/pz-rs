# pz-bullet

A pure-Rust replacement for the game's native physics library
(`libPZBullet`, the C++ side of `zombie.core.physics.Bullet`). It runs
without native code.

## What is inside

| Module | Origin | License |
| --- | --- | --- |
| `bt` | A Rust translation of [Bullet Physics](https://github.com/bulletphysics/bullet3) **2.82**, double precision, scalar code paths. It covers only the parts the game reaches (listed below). | zlib (`LICENSE-ZLIB`) |
| `pz` | The game's glue (`PZBullet.cpp`): `WorldSimulation`, the chunk maps and their static-body pools, server cells, `PZVehicle`, `PZVehicleScript`, physics objects, constraints and the collision/tick hooks. It is reconstructed from the shipped library. | original code |
| `api` | `PzBullet`, with one method per in-scope `Bullet` native. Each method is documented with the native's name, keeps its argument order, and uses the same float-array layouts and `ToBullet` byte buffer. | original code |

Each ported Bullet file keeps Bullet's copyright notice and is marked as
altered.

### Bullet version and changes

The game links stock Bullet 2.82, built with `BT_USE_DOUBLE_PRECISION`. The
version shows in its API: it has `setupRollingFrictionConstraint` and
`btRigidBody::computeGyroscopicForce`, and it lacks the 2.83 gyroscopic
API.

Every ported class was compared with the game's arm64 slice by a harness
linked directly to its exported functions, and none of them showed local
modifications. The game changes Bullet only from the outside:

* gravity is `(0, -10, 0)` and `gDeactivationTime` is `3`;
* the penetration-depth solver is GJK-EPA, and the dispatcher and
  algorithm pools hold 4096 entries;
* `m_allowedCcdPenetration` is set to 0;
* the vehicle tick runs as the world's pre-tick callback, and
  `saveKinematicState` is overridden;
* a `gContactAddedCallback` sets the combined friction to the clamped value
  `0.9f * friction` and the restitution to `0.1 * restitution`, and records
  vehicle collisions;
* `PZVehicleRaycaster` casts with filter group 2 and mask 13, and only
  rigid bodies with contact response count as hits.

### Ported Bullet subset

* **Shapes:** box, sphere, convex hull, and compound (with its child tree).
* **Narrowphase:** the Voronoi simplex solver, the GJK pair detector,
  GJK-EPA, the subsimplex, GJK and continuous convex casts, and the box-box
  detector.
* **Broadphase:** `btDbvt`, `btDbvtBroadphase`, the hashed and hashed-simple
  pair caches, and union-find.
* **Collision dispatch:** the dispatcher with the 2.82 dispatch table,
  manifold result, the convex-convex, compound, compound-compound, box-box,
  sphere-sphere and empty algorithms, ray tests and convex sweeps, and
  simulation islands.
* **Dynamics:** rigid body, the sequential-impulse solver,
  point-to-point, hinge and generic 6-DOF constraints, the discrete dynamics
  world (predictive contacts, CCD and solver batching included), and the
  raycast vehicle.

Concave, triangle and plane shapes, continuous dispatch, cone-twist and
serialization are not ported, because nothing the game calls reaches them.

## Exactness

The arm64 library was compiled by clang with `-ffp-contract=on`.

* **Fused multiply-adds.** Inside one C++ expression, `a*b + c`, `a*b - c`
  and `c - a*b` become single fused multiply-adds, with the left operand
  checked first, so `a*b + c*d` becomes `fma(a, b, c*d)`. Separate
  statements and inline-function boundaries never fuse. Each fused
  operation is written out as `linear_math::fma` (`f64::mul_add`).
* **Float and double.** Where the glue computes in `float` before
  converting (for example `x - (float)offsetX`), the port does the same.
* **libm.** `sin`, `cos`, `acos`, `asin`, `atan2`, `fmod` and `pow` are
  called from the platform libm through `extern "C"`, as in the original.
  Where clang merged a `sin` and `cos` of the same argument, the port calls
  `__sincos_stret`.
* **Sort order.** `btAlignedObjectArray::quickSort` is reproduced, so
  equal keys stay in the same order.

### Parity

Verified bit for bit against the shipped arm64 `libPZBullet.dylib`. All
harnesses and recorded data are local and uncommitted.

**Unit level.** C++ harnesses link the game's library and call its classes
directly. Each harness dumps every result, and a Rust replay compares the
dumps bit for bit.

| Part | Cases | Mismatches |
| --- | --- | --- |
| Shapes and narrowphase | 160,000 | 0 |
| Broadphase, `btDbvt` and pair caches | ~273,000 steps | 0 |
| Solver and constraints | 22,000 scenes (55,249 solver steps) | 0 |
| Raycast vehicle | 160,199 `updateVehicle` calls (635,582 wheel rays) | 0 |
| Whole worlds (statics, compound stairs, vehicles, loose bodies with CCD, all constraint types, rays and sweeps) | 244 seeds, ~545,000 compared state lines | 0 |

The whole-world runs include one stray difference, and it comes from
memory the C++ never initialises.

**End to end.** A JVM driver gives the real library its JNI entry points
through a stand-in `zombie.core.physics.Bullet` that declares only the
natives and the two callbacks. It runs scripted scenarios and writes every
call, result, output array and callback (including the `ToBullet` calls
made during `stepSimulation`) to an op log.

The inputs come from the game's own Java code, run headless:

* `VehicleScript.toBullet()` for all 241 vehicle scripts;
* `IsoChunk.updatePhysicsForLevel()` on real lotpack chunks of
  Muldraugh, West Point, Louisville and Riverside.

`examples/oracle_replay.rs` replays each log through `PzBullet`. Every
scenario was recorded twice from the native side, and both recordings were
byte-identical.

| Family | Logs | Calls | Steps | Mismatches |
| --- | --- | --- | --- | --- |
| flat ground, synthetic and Muldraugh | 6 | 91,622 | 18,000 | 0 |
| ground with holes, synthetic and West Point | 6 | 91,356 | 18,000 | 0 |
| crowds (6–9 vehicles), synthetic and West Point | 6 | 189,049 | 18,000 | 0 |
| head-on collisions, synthetic and Muldraugh | 6 | 108,709 | 18,000 | 0 |
| tire removal and inflation, synthetic and Riverside | 6 | 93,453 | 18,000 | 0 |
| towing (6-DOF, rope, hinge, point; ERP changes, breaking-impulse callbacks), synthetic and Louisville | 6 | 90,103 | 18,000 | 0 |
| West Point stairs | 3 | 45,127 | 9,000 | 0 |
| Louisville multi-level (levels up to 13) | 3 | 44,848 | 9,000 | 0 |
| chunk-map scrolling, synthetic and Muldraugh | 6 | 91,199 | 18,000 | 0 |
| chunk maps 0–3 activated and deactivated, synthetic and Riverside | 6 | 94,226 | 18,000 | 0 |
| server cells (`isServer` world), synthetic and Muldraugh | 6 | 92,934 | 18,000 | 0 |
| physics meshes in chunk data | 3 | 48,021 | 9,000 | 0 |
| vehicle physics meshes | 3 | 47,342 | 9,000 | 0 |
| `setOwnVehiclePhysics` with sweeps, synthetic and West Point | 6 | 91,774 | 18,000 | 0 |
| physics objects | 3 | 62,597 | 9,000 | 0 |
| **total** | **75** | **1,282,360** | **225,000** | **0** |

Every scenario also drives the vehicles: throttle, brake and steering;
`setVehicleParams` every 25 steps; teleports; central forces and torques;
mass and velocity-multiplier changes; static and active toggles;
`getOwnVehiclePhysics`; and `getVehiclePhysics` and `getObjectPhysics`
readback as `WorldSimulation.update` does it.

## Known differences and quirks

* **Crashes the port avoids.** In these cases the original crashes or
  corrupts memory, and the port does not:
  * `removeVehicle` on a vehicle that still has a constraint
    (`removeConstraintsOnVehicle` reads freed memory);
  * `activateChunkMap` adopting chunks from a map of a different width;
  * scrolling a chunk map of a server world.
* **Uninitialised stairs offset.** The original never initialises the x
  offset of the stairs compound's children; the port uses 0. No scenario
  drove onto stairs bodies, so this value is unverified.
* **Native bugs kept on purpose:**
  * east walls are released through the south-wall pool;
  * duplicate stairs objects leak;
  * re-activating a chunk map leaks the old one;
  * `ToBullet` ignores records for levels outside the chunk's range and
    skips any byte other than `8` (update chunk) or `0xff` (end).
* **Not ported.** The ragdoll and ballistics natives have no methods yet.
  `checkWheelCollision` only sees ragdolls, so it always returns `false`.
* **Platform.** Matching was verified only for the arm64 macOS build. The
  trigonometric and `pow` results come from the platform libm, as in the
  original. Other libms, or x86-64 builds of the original without FMA
  contraction, may round differently in rare cases.
