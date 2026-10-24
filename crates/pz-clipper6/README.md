# pz-clipper6

Pure-Rust replacement for the polygon code statically linked into the
game's native pathfinding library (`libPZPathFind`): the obstacle and
walkable polygons the visibility graph is built from. No native code at
runtime.

This is not the library behind `zombie.vehicles.Clipper` (that one is
Clipper2, see `pz-clipper`); the native pathfinder links the older
Clipper 1.

## What is inside

| Module | Origin | License |
| --- | --- | --- |
| `clipper` | Port of Angus Johnson's [Clipper](https://sourceforge.net/projects/polyclipping/) **6.4.2** (C++ edition, `ClipperLib`): `Clipper`, `ClipperOffset`, `PolyTree`, helpers | Boost Software License 1.0 (`LICENSE-BOOST`) |
| `earcut` | Port of [mapbox/earcut.hpp](https://github.com/mapbox/earcut.hpp) at commit `a57e4ce` (earcut 2.2.1 parity), as `Earcut<uint32_t>` over `IntPoint` rings | ISC (`LICENSE-ISC`) |
| `cxxsort` | Port of libc++ `std::sort` (introsort, unchanged up to libc++ 21.1.6): reproduces the tie order of the unstable sorts Clipper and earcut use | Apache-2.0 WITH LLVM-exception (`LICENSE-LLVM`) |
| `pz` | The game's `PZClipper` wrapper, reconstructed from the binary's behaviour | original code |

Ported code keeps its upstream copyright notice in each file header.

### Clipper configuration

The game's copy is stock 6.4.2 with the default `clipper.hpp` switches:
64-bit `cInt` (with the `Int128` slope tests above `loRange`), `use_lines`
on, `use_xyz` and `use_deprecated` off. No local modifications were found:
the differential run below compares every public entry point used by the
game, and `Execute`, `ClipperOffset` and earcut match on random input
outside the game's usage too.

### `PZClipper`

| Native method | Rust | Behaviour |
| --- | --- | --- |
| `PZClipper()` | `PZClipper::new` | `Clipper(0)` plus an empty result tree and polygon list |
| `clear()` | `clear` | `ClipperBase::Clear()`, path count = 0 |
| `addAABB(x1, y1, x2, y2)` | `add_aabb` | closed subject path `(x1,y1) (x2,y1) (x2,y2) (x1,y2)` in fixed point, count += 1 |
| `addAABBBevel(...)` | `add_aabb_bevel` | the shipped body is empty: does nothing |
| `addPolygon(EdgeRing2f)` | `add_polygon` | closed subject path from the ring's points, count += 1 |
| `generatePolygons(delta, jt)` | `generate_polygons` | `clearPolys`, union with `pftPositive`, then `getPolygons(delta, jt)` (a failed `Execute` is ignored) |
| `getPolygons(delta, jt)` | `get_polygons_offset` | if `delta != 0`: `ClipperOffset(2, 0.25)` over every tree node (open nodes butt-ended), tree replaced by the offset by `delta * 1000`; then `getPolygons()` |
| `generateObstaclePolygons()` | `generate_obstacle_polygons` | `clearPolys`; if any path was added, union with `pftPositive` (throws `"Clipper.Execute()"` on failure) and `getPolygons()` |
| `generateWalkablePolygons(cx, cy)` | `generate_walkable_polygons` | clears the clipper, adds the chunk square `(8cx, 8cy)-(8(cx+1), 8(cy+1))` as subject and every node of the current tree as clip, `clearPolys`, difference with even-odd fills, `getPolygons()` |
| `getPolygons()` | `get_polygons` | walks the tree depth-first; each outer node appends a `PZPoly`, each hole is added to its parent's `PZPoly` |
| `clearPolys()` | `clear_polys` | clears the tree and the polygon list |
| `triangulate(i, out)` | `triangulate` | earcut of polygon `i` (outer + holes); appends `Triangle2f`s in world units |

World coordinates become fixed point as `(int32_t)floorf(fmaf(v, 1000, 0.5))`
(`pz::to_fixed`) and come back as `(float)x / 1000.0f` (`pz::from_fixed`).
The pathfinder reads results through `PZClipper::polys()` (fixed point
`IntPoint` rings, like the native `std::vector<PZPoly*>`).

## Exactness

The arm64 slice was compiled by Apple clang with `-ffp-contract=on`: every
`a*b + c`, `c - a*b` and `a*b - c*d` inside one C++ expression is a fused
multiply-add, written here with `f64::mul_add` (`clipper::fma`). Exceptions
found in the disassembly are reproduced too: earcut's ring-area loop is
auto-vectorised for rings of 8+ points, which computes the products of each
full block of 8 unfused; `DoOffset`'s `sin`/`cos` pair is a single
`__sincos_stret` call. Unstable sorts use the libc++ algorithm so equal keys
keep the same order. Out-of-range or overflow-prone integer arithmetic wraps
like the machine code.

### Differential testing

Verified bit for bit against the shipped arm64 `libPZPathFind.dylib` with a
local, uncommitted harness that loads the library and calls its internal
functions directly (`PZClipper` methods, `Clipper::Execute` for paths and
trees, `ClipperOffset::Execute`, `mapbox::earcut`), replaying the same
command scripts through this crate:

* 1,149,948 randomized cases, 0 mismatches: 549,420 pathfinder-like
  `PZClipper` sessions (8x8 chunks and 10x10 areas of unit squares with
  radius 0.15-0.35, west/north walls, bevel calls, random float AABBs and
  polygons, coordinates beyond the 64-bit fast range, offsets with every
  join type, obstacle then walkable generation, triangulation of every
  result, reused and fresh instances), 300,539 direct `Clipper` executions
  (all clip and fill types, open paths, all init options, coordinates from
  tiny grids up to the full 62-bit range), 150,119 `ClipperOffset` runs
  (all join/end types, positive, negative and zero deltas, miter limits,
  arc tolerances) and 149,870 earcut calls (up to 120-point rings with
  holes, so both the plain and z-order-hashed paths run). 52 further cases
  were skipped because the original crashes on them (see below).
* The libc++ tie order of `cxxsort` matches libc++ 21.1.6 on 20,000 arrays.

## Known differences

* With `ioStrictlySimple`, the original dereferences a default-constructed
  (null) maxima iterator in `ProcessHorizontal` when a run of horizontal
  edges changes direction, and crashes (52 of the ~300,000 random direct
  `Clipper` cases; never reachable from `PZClipper`, which does not set
  that option). The port treats the iterator as exhausted.
* Matching was verified for the arm64 macOS build only. `sin`, `cos`,
  `acos`, `atan2` and `tan` come from the platform libm, as in the original;
  other platforms' libms (and x86-64 builds without FMA contraction) may
  round differently in rare cases.
