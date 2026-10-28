# pz-rs

A from-scratch Rust reimplementation of the Project Zomboid (Build 42) game
engine, aiming for 1:1 behaviour with the original on macOS, Windows and Linux.

This repository contains **only original code**. It ships no game files:
you need your own installed copy of Project Zomboid, whose `media/` folder
(Lua scripts, textures, maps, sounds, translations) the engine reads at
runtime. The Lua engine (Kahlua) and Lua-facing API are being ported to Rust
to retain game and mod behavior. Full compatibility is still in progress.

## Layout

| Crate | Original counterpart |
| --- | --- |
| `pz-kahlua` | `se.krka.kahlua` + `org.luaj.kahluafork` (Lua 5.1 compiler, VM, stdlib) |
| `pz-lua-api` | `zombie.Lua.LuaManager` globals and exposed classes |
| `pz-fs` | `zombie.ZomboidFileSystem` |
| `pz-render` | `zombie.core.textures`, `SpriteRenderer` |
| `pz-text` | `zombie.core.Translator`, `zombie.ui.TextManager`, `AngelCodeFont` |
| `pz-ui` | `zombie.ui.UIElement`, `UIManager` |
| `pz-scripts` | `zombie.scripting` (`ScriptParser`, `ScriptManager`, script objects) |
| `pz-script-parser` | `zombie.scripting.ScriptParser` (shared by `pz-scripts`, `pz-fs`, `pz-text`, `pz-render`) |
| `pz-world` | `zombie.iso` map files, `IsoMetaGrid`, `IsoCell`/`IsoChunk`/`IsoGridSquare` |
| `pz-classfile` | reads built-in constants from the installed `projectzomboid.jar` (no JVM) |
| `pz-inventory` | `zombie.inventory` items, `ItemContainer`, `zombie.world.WorldDictionary` |
| `pz-erosion` | `zombie.erosion` (seasons, erosion categories, `ErosionMain`) |
| `pz-worldgen` | `zombie.iso.worldgen` (B42 procedural world generation) |
| `pz-popman` | `zombie.popman`, `zombie.MapCollisionData` and the native `PZPopMan` library |
| `pz-vehicle-render` | `BaseVehicle.render`/`renderShadow`, `VehicleModelInstance`, `Model.DrawVehicle` |
| `pz-engine` | `zombie.GameWindow`, `zombie.core.Core`, `zombie.gameStates` |
| `pz-clipper` | `zombie.vehicles.Clipper`/`ClipperOffset` native library (Clipper2 1.5.4, earcut.hpp; see its README for licenses) |
| `pz-clipper6` | `PZClipper` + Clipper 6.4.2 + earcut inside the native pathfinder (`libPZPathFind`) |
| `pz-client` | desktop binary |

See [docs/WORKSTREAMS.md](docs/WORKSTREAMS.md) for how the work is split and
[docs/ENGINE-WIRING.md](docs/ENGINE-WIRING.md) for current integration
progress and known gaps. [PROGRESS.md](PROGRESS.md) preserves the earlier
workstream status.

## Rules for contributions

* Never commit original game material: no decompiled source, Lua scripts,
  assets, extracted data, fixtures or golden files generated from the game.
  Keep such material under the git-ignored `local/` folder.
* 100% Rust at runtime: no JVM, no Java bridge, no C Lua.
