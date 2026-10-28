# pz-bullet

A pure-Rust replacement for the game's native physics library
(`libPZBullet`, the C++ side of `zombie.core.physics.Bullet`). It runs
without native code.

## What is inside

| Module | Origin | License |
| --- | --- | --- |
| `bt` | A Rust translation of [Bullet Physics](https://github.com/bulletphysics/bullet3) **2.82**, double precision, scalar code paths. It covers only the parts the game reaches (listed below). | zlib (`LICENSE-ZLIB`) |
| `pz` | The game's glue (`PZBullet.cpp`): `WorldSimulation`, the chunk maps and their static-body pools, server cells, `PZVehicle`, `PZVehicleScript`, physics objects, constraints, the collision/tick hooks, ragdolls (`Skeleton`, `SkeletonPose`, `RagdollBuilder`, `Ragdoll`, `PZRagdoll`, `PZRagdollScript`) and ballistics (`PZBallistics`, `PZBallisticsTarget`). It is reconstructed from the shipped library. | original code |
| `api`, `api_ragdoll`, `api_ballistics` | `PzBullet`, with one method per `Bullet` native. Each method is documented with the native's name, keeps its argument order, and uses the same float-array layouts and `ToBullet` byte buffer. | original code |

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

* **Shapes:** box, sphere, capsule (Y, X and Z; the game's
  `PZCapsuleShape` overrides nothing), convex hull, and compound (with its
  child tree).
* **Narrowphase:** the Voronoi simplex solver, the GJK pair detector,
  GJK-EPA, the subsimplex, GJK and continuous convex casts, and the box-box
  detector.
* **Broadphase:** `btDbvt`, `btDbvtBroadphase`, the hashed and hashed-simple
  pair caches, and union-find.
* **Collision dispatch:** the dispatcher with the 2.82 dispatch table,
  manifold result, the convex-convex (with its capsule-capsule collider),
  compound, compound-compound, box-box, sphere-sphere and empty algorithms,
  ray tests (closest and all hits) and convex sweeps, and simulation
  islands.
* **Dynamics:** rigid body, the sequential-impulse solver,
  point-to-point, hinge, cone-twist and generic 6-DOF constraints, the
  discrete dynamics
  world (predictive contacts, CCD and solver batching included), and the
  raycast vehicle.

Concave, triangle and plane shapes, continuous dispatch, the cone-twist
motor and obsolete solver, and serialization are not ported, because
nothing the game calls reaches them.

## Exactness

The arm64 library was compiled by clang with `-ffp-contract=on`.

* **Fused multiply-adds.** Inside one C++ expression, `a*b + c`, `a*b - c`
  and `c - a*b` become single fused multiply-adds, with the left operand
  checked first, so `a*b + c*d` becomes `fma(a, b, c*d)`. Separate
  statements and inline-function boundaries never fuse. Each fused
  operation is written out as `linear_math::fma` (`f64::mul_add`).
* **Folded negations.** In `btConeTwistConstraint::calcAngleInfo2` clang
  folded a few negations of fused results into the fused instruction
  (`fnmadd`), which turns an exact `-0` into `+0`. The port writes those
  expressions out the same way.
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
| Capsules (support, AABB, inertia, margins, scaling; GJK, EPA and casts against boxes, spheres, hulls and capsules) | 200,000 cases (117,000 capsules) | 0 |
| Cone-twist and frame-hinge `getInfo1`/`getInfo2` rows and limit state | 500,000 constraints | 0 |
| Ragdoll worlds (chains of compound capsule, box and sphere parts joined by cone-twist and hinge joints, damping, sleeping thresholds, rolling friction, impulses, `setMassProps` re-adds, closest and all-hits rays, per-child `rayTestSingle`, capsule sweeps) | 60 seeds, 90,000 steps, 154,717 compared state lines | 0 |
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

**Ragdolls** (`examples/ragdoll_replay.rs`). The inputs are real: the
`defineRagdoll*` arrays from `RagdollScript.toBullet`, the skeleton and
T-pose from `RagdollBuilder.Initialize()` on `Skinned/MaleBody.x` with
`Bob_TPose`, and per-frame bone transforms from an `AnimationPlayer`
playing seven clips (walks, idles, deaths, hit reactions), packed as
`RagdollController` packs them.

| Family | Logs | Calls | Steps | Mismatches |
| --- | --- | --- | --- | --- |
| falls on synthetic floors and Muldraugh | 4 | 85,598 | 12,000 | 0 |
| West Point stairs | 2 | 42,796 | 6,000 | 0 |
| Louisville walls | 2 | 42,746 | 6,000 | 0 |
| crowds of 8–12 ragdolls, synthetic and Riverside | 2 | 439,338 | 6,000 | 0 |
| lifecycle: remove/re-add, pool reuse, duplicate ids, active toggles, detach, re-pose, reset, network physics | 3 | 64,332 | 9,000 | 0 |
| dynamics: live redefinition, mass, set/reset body dynamics | 3 | 64,080 | 9,000 | 0 |
| updates applied to pooled ragdolls | 3 | 63,813 | 9,000 | 0 |
| moving vehicles and `checkWheelCollision` | 4 | 483,197 | 12,000 | 0 |
| argument and error paths | 1 | 15,434 | 3,000 | 0 |
| **total** | **24** | **1,301,334** | **72,000** | **0** |

**Ballistics** (`examples/ballistics_replay.rs`). Targets are animated with
real bone transforms (eleven clips of zombie and player animations).
Weapon ranges, spreads and projectile counts come from 22 real firearms.
The shooters and targets move among vehicles on the real chunk regions.

| Family | Logs | Calls | Steps | Mismatches |
| --- | --- | --- | --- | --- |
| crowds of 10–24 targets | 6 | 1,492,493 | 18,000 | 0 |
| shotguns (multi-projectile spread) | 4 | 621,229 | 12,000 | 0 |
| reticle updates and live body-part edits | 3 | 391,489 | 9,000 | 0 |
| no vehicles | 2 | 235,648 | 6,000 | 0 |
| small world coordinates, finer `f32` output resolution | 7 | 1,187,224 | 21,000 | 0 |
| **total** | **22** | **3,928,083** | **66,000** | **0** |

These logs cover tens of thousands of `getBallisticsTargets`,
`getBallisticsTargetsSpreadData` (47,500 spread rays),
`getBallisticsCameraTargets` and `getTargetedBodyPart` queries.

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
* **Ragdoll quirks kept:**
  * `addRagdoll` ignores its position; bodies are placed by
    `updateRagdoll` and the skeleton updates.
  * `setRagdollMass` gives every body the whole mass.
  * A live `defineRagdollBodyPartInfo`/`BodyDynamics` re-places every
    ragdoll's bodies in ragdoll-local space.
  * The first failing `simulateRagdoll*` call returns -1 because the
    error log clears the pending exception; later calls throw.
  * The ragdoll pool is LIFO, and a reused ragdoll keeps its state.
* **Ballistics quirks kept:**
  * Ballistics rays use group 1 / mask -1, so static geometry
    (mask 14) never blocks them.
  * Every camera target gets the ray's first hit point.
  * Body-part hit flags are never cleared, so `getTargetedBodyPart`
    reports the lowest part ever hit.
  * `getBallisticsTargets` returns at most one hit and ignores its count.
* **`rand()`.** Spread rays use C `rand()`, which the library never seeds.
  Parity is per platform, so `CRand` reproduces, in pure Rust and selected
  by `cfg`, the libc `rand()` each platform's game build links:
  * **macOS:** Apple libc (Park–Miller, seed 1, `RAND_MAX` 2³¹−1). Checked
    against the shipped arm64 library (the ballistics corpus) and libc
    (50M values).
  * **Windows:** the MSVC CRT linear congruential generator
    (`214013·s + 2531011`, bits 16–30, seed 1, `RAND_MAX` 32767; state per
    thread).
  * **Linux:** glibc `random()` `TYPE_3` (seed 1, `RAND_MAX` 2³¹−1).

  The Windows and Linux variants are checked only against their documented
  algorithms (first outputs from seed 1), because the game's Windows and
  Linux libraries can't be run here. The spread code divides by `RAND_MAX`
  and `RAND_MAX + 1`. In the macOS binary those are the constants 2147483647
  and 2147483648, and the other builds are assumed to compile in their own
  `RAND_MAX`, which is unverified.
* **NaN signs.** When `updateRagdollSkeletonPreviousTransforms` is given
  the NaN-terminated velocities Java actually sends, the port matches
  except for NaN sign bits (2,287 values in one test log). The main corpus
  sends finite velocities.
* **More crashes the port avoids:**
  * `destroyWorld` with ragdolls or ballistics targets still present;
  * ragdoll or ballistics natives called without a world;
  * `initializeRagdollPose` freeing shapes that live ragdolls still use.
* **Not ported** (unreachable through JNI):
  * `PZBallistics::raycastAllWithCompoundSupport`;
  * the mangled `getBallisticsTargetsSpread`;
  * the isometric target AABB (`useIsometricAabb` is never set).

  The debug-draw natives only set renderer flags, which are stored.
* **Platform.** Matching was verified only for the arm64 macOS build. The
  trigonometric and `pow` results come from the platform libm, as in the
  original. Other libms, or x86-64 builds of the original without FMA
  contraction, may round differently in rare cases.
