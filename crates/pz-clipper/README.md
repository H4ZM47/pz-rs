# pz-clipper

Pure-Rust replacement for the game's `PZClipper` native library, the JNI
backend of `zombie.vehicles.Clipper` and `zombie.vehicles.ClipperOffset`
(zones, pathfinding obstacles, world-map geometry, streets, tile depth).
No native code at runtime.

## What is inside

| Module | Origin | License |
| --- | --- | --- |
| `core`, `engine`, `offset` | Port of Angus Johnson's [Clipper2](https://github.com/AngusJohnson/Clipper2) **1.5.4** (tag `Clipper2_1.5.4`, commit `ef88ee9`): `Clipper64` boolean ops, `PolyTree64`, `ClipperOffset` | Boost Software License 1.0 (`LICENSE-BOOST`) |
| `earcut` | Port of [mapbox/earcut.hpp](https://github.com/mapbox/earcut.hpp) at commit `a57e4ce` (2019-09-19, "parity with earcut v2.2.1") | ISC (`LICENSE-ISC`) |
| `cxxsort` | Port of libc++ 19 `std::sort` (introsort): reproduces the tie order of the unstable sorts Clipper2 and earcut use | Apache-2.0 WITH LLVM-exception (`LICENSE-LLVM`) |
| `pz` | The game's JNI glue, reconstructed from the binary's behaviour | original code |

Ported code keeps its upstream copyright notice in each file header.

## Exactness

The shipped library is a universal binary; the arm64 slice (the one the
bundled arm64 JRE loads) was compiled by clang with `-ffp-contract=on`, so
`a*b ± c*d` inside one C++ expression is a fused multiply-add. Every such
site is reproduced with `f64::mul_add`, `nearbyint` is ties-to-even,
coordinate conversion is `(int32)floorf(fmaf(v, 1000, 0.5))` in, and
`(float)i64 / 1000` out. Unstable sorts use a port of libc++'s algorithm so
equal keys keep the same order.

Results were compared bit-for-bit against the real library through a local,
uncommitted JVM harness that drives `zombie.vehicles.Clipper` by reflection
with the same scripts that `examples/replay.rs` executes.

## API

`pz::Clipper` and `pz::ClipperOffset` mirror the Java classes
(`add_path`, `add_line`, `add_aabb`, `add_aabb_bevel`, `add_polygon`,
`clip_aabb`, `generate_polygons*`, `get_polygon`, `triangulate`,
`triangulate2`, `generate_triangulate_polygons`, `execute`, ...). Each
buffer-writing method has a `*_into(&mut [u8])` twin producing the exact
big-endian layout the game reads. `pz::ZoneClipper` bundles the two call
sequences `Zone`/`IsoMetaGrid` use.

## Known differences

* Out-of-enum clip types (anything but 0..=4, never passed by the game) make
  the native library dereference null or read freed memory. The port returns
  -1 (failed sweep) or a deterministic result instead of crashing.
* Matching was verified for the arm64 macOS build only. x86-64 builds of the
  library (no FMA contraction, possibly a different C++ standard library on
  Windows/Linux) may round differently in rare cases.
