# pz-rs

A from-scratch Rust reimplementation of the Project Zomboid (Build 42) game
engine, aiming for 1:1 behaviour with the original on macOS, Windows and Linux.

This repository contains **only original code**. It ships no game files:
you need your own installed copy of Project Zomboid, whose `media/` folder
(Lua scripts, textures, maps, sounds, translations) the engine reads at
runtime. Mods keep working because the game's Lua engine (Kahlua) is ported
to Rust with the original semantics and the same Lua-facing API.

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
| `pz-world` | `zombie.iso` map files, `IsoMetaGrid`, `IsoCell`/`IsoChunk`/`IsoGridSquare` |
| `pz-classfile` | reads built-in constants from the installed `projectzomboid.jar` (no JVM) |
| `pz-world` | `zombie.iso` map files, `IsoMetaGrid`, `IsoCell`/`IsoChunk`/`IsoGridSquare` |
| `pz-classfile` | reads built-in constants from the installed `projectzomboid.jar` (no JVM) |
| `pz-world` | `zombie.iso` map files, `IsoMetaGrid`, `IsoCell`/`IsoChunk`/`IsoGridSquare` |
| `pz-classfile` | reads built-in constants from the installed `projectzomboid.jar` (no JVM) |
| `pz-world` | `zombie.iso` map files, `IsoMetaGrid`, `IsoCell`/`IsoChunk`/`IsoGridSquare` |
| `pz-world` | `zombie.iso` map files, `IsoMetaGrid`, `IsoCell`/`IsoChunk`/`IsoGridSquare` |
| `pz-classfile` | reads built-in constants from the installed `projectzomboid.jar` (no JVM) |
| `pz-engine` | `zombie.GameWindow`, `zombie.core.Core`, `zombie.gameStates` |
| `pz-clipper` | `zombie.vehicles.Clipper`/`ClipperOffset` native library (Clipper2 1.5.4, earcut.hpp; see its README for licenses) |
| `pz-client` | desktop binary |

See [docs/WORKSTREAMS.md](docs/WORKSTREAMS.md) for how the work is split and
[PROGRESS.md](PROGRESS.md) for current status.

## Rules for contributions

* Never commit original game material: no decompiled source, Lua scripts,
  assets, extracted data, fixtures or golden files generated from the game.
  Keep such material under the git-ignored `local/` folder.
* 100% Rust at runtime: no JVM, no Java bridge, no C Lua.
