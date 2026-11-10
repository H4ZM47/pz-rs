# pz-rs

A from-scratch Rust reimplementation of the Project Zomboid (Build 42) game
engine, aiming for 1:1 behaviour with the original on macOS, Windows and Linux.

This repository ships no game files:
you need your own installed copy of Project Zomboid, whose `media/` folder
(Lua scripts, textures, maps, sounds, translations) the engine reads at
runtime. The Lua engine (Kahlua) and Lua-facing API are being ported to Rust
to retain game and mod behavior.

## Status

The project is not yet playable. Most subsystems exist as ported libraries
with their own tests; the remaining work is mainly wiring them into the
running game loop and filling in the Lua-facing API. The client boots, runs
the game's main-menu Lua and can create or load a single-player world, but an
in-game session still stops on unbound API methods and many world systems are
not yet connected. Multiplayer has not been started.

[docs/STATUS.md](docs/STATUS.md) has the per-workstream status and a checklist
of what remains.

## Running

Building needs a Rust toolchain and an installed copy of Project Zomboid
Build 42. The client looks for the install in the Steam libraries; otherwise
pass `--game-dir DIR` or set `PZ_GAME_DIR` to the directory containing
`media/`.

```sh
cargo run -p pz-client                                  # main menu
cargo run -p pz-client -- --new-game NAME               # skip the menu, new world
cargo run -p pz-client -- --load-game MODE/NAME         # skip the menu, load a save
cargo run -p pz-client -- --new-game NAME --headless-frames 120   # no window
```

`--game-mode`, `--map`, `--spawn X,Y,Z` and `--seed` shape a new game;
`--help` lists every option. Saves and settings go to `~/Zomboid-rs` (or
`--cache-dir DIR`); the original game's `~/Zomboid` is never written.

## Layout

| Crate | Original counterpart |
| --- | --- |
| **Lua** | |
| `pz-kahlua` | `se.krka.kahlua` + `org.luaj.kahluafork` (Lua 5.1 compiler, VM, stdlib) |
| `pz-lua-api` | `zombie.Lua.LuaManager` globals and exposed classes |
| **Engine and platform** | |
| `pz-client` | desktop binary |
| `pz-engine` | `zombie.GameWindow`, `zombie.gameStates`, boot and the in-game loop |
| `pz-core` | `zombie.core.Core` options, `zombie.config`, `GameTime`, `ClimateManager` |
| `pz-fs` | `zombie.ZomboidFileSystem` |
| `pz-classfile` | reads built-in constants from the installed `projectzomboid.jar` (no JVM) |
| `pz-input` | `org.lwjglx.input` keyboard and mouse state |
| `pz-player-input` | what `IsoPlayer.update` reads from keyboard, mouse and joypads |
| `pz-sandbox` | `zombie.SandboxOptions` |
| `pz-server-settings` | `zombie.network.ServerOptions`, `ServerSettings`, `SpawnRegions` |
| `pz-network` | offline server list and accounts (`zombie.network.Server`, `Account`); no networking |
| `pz-sqlite` | SQLite file reader/writer for `players.db` and `vehicles.db` |
| `pz-joml` | the `org.joml.Matrix4f` operations the engine uses |
| **Scripts, UI and text** | |
| `pz-scripts` | `zombie.scripting` (`ScriptManager`, script objects) |
| `pz-script-parser` | `zombie.scripting.ScriptParser` (shared by `pz-scripts`, `pz-fs`, `pz-text`, `pz-render`) |
| `pz-text` | `zombie.core.Translator`, `zombie.ui.TextManager`, `AngelCodeFont` |
| `pz-ui` | `zombie.ui.UIElement`, `UIManager` |
| **World** | |
| `pz-world` | `zombie.iso` map files, `IsoMetaGrid`, `IsoCell`/`IsoChunk`/`IsoGridSquare`, chunk saves |
| `pz-worldgen` | `zombie.iso.worldgen` (B42 procedural world generation) |
| `pz-erosion` | `zombie.erosion` (seasons, erosion categories, `ErosionMain`) |
| `pz-entity` | `zombie.entity` component storage and save format |
| `pz-inventory` | `zombie.inventory` items, `ItemContainer`, `zombie.world.WorldDictionary` |
| `pz-lighting` | native lighting library (`libLighting`, `zombie.iso.LightingJNI`) |
| `pz-worldmap` | `zombie.worldMap` (in-game map, mini-map, spawn map) |
| `pz-radio` | `zombie.radio` (broadcasts, channels, `ZomboidRadio`) |
| **Characters, vehicles and physics** | |
| `pz-characters` | `zombie.characters`: traits, survivor descriptors, stats, body damage, AI states, zombies |
| `pz-animals` | `zombie.characters.animals` |
| `pz-popman` | `zombie.popman`, `zombie.MapCollisionData` and the native `PZPopMan` library |
| `pz-pathfind` | native pathfinder (`libPZPathFind`, `zombie.pathfind`) |
| `pz-clipper6` | `PZClipper` + Clipper 6.4.2 + earcut inside the native pathfinder |
| `pz-clipper` | `zombie.vehicles.Clipper`/`ClipperOffset` native library (Clipper2 1.5.4, earcut.hpp; see its README for licenses) |
| `pz-vehicles` | `zombie.vehicles` runtime (`BaseVehicle`, parts) |
| `pz-bullet` | native physics library (`libPZBullet`, Bullet Physics 2.82) |
| `pz-ragdoll` | the Java side of ragdolls, over `pz-bullet` and `pz-anim` |
| `pz-world-states` | glue: character AI states ↔ loaded world |
| `pz-world-popman` | glue: zombie population ↔ loaded world |
| **Rendering** | |
| `pz-render` | `zombie.core.textures`, `SpriteRenderer` draw list |
| `pz-gpu` | GPU backend on wgpu (the original's render thread) |
| `pz-iso-render` | isometric world renderer (`FBORenderCell`, sprite render paths) |
| `pz-world-render` | glue: world data → renderer and lighting inputs |
| `pz-model` | models, skeletons and animation clips (`.X`, `.fbx`, `.txt` loaders) |
| `pz-anim` | `zombie.core.skinnedmodel.animation` and `.advancedanimation` |
| `pz-character-model` | character model tree, composited textures, per-frame lights |
| `pz-vehicle-render` | `BaseVehicle.render`/`renderShadow`, `VehicleModelInstance`, `Model.DrawVehicle` |
| `pz-video` | `VideoTexture` playback (the `libbink64` layer) |
| **Audio** | |
| `pz-audio` | FMOD Studio banks decoded and played in Rust (replaces the FMOD runtime) |
| `pz-sound` | `zombie.GameSounds`, `GameSound`/`GameSoundClip` logic |
| `pz-world-sound` | glue: world sound manager ↔ loaded world |

## Documentation

* [docs/STATUS.md](docs/STATUS.md): what the project replaces, what is done
  and a checklist of what remains. Start here.
* [docs/ENGINE-WIRING.md](docs/ENGINE-WIRING.md): the detailed integration
  checklist, with the hooks each subsystem exposes and where the game loop
  must call them.
* [docs/WORKSTREAMS.md](docs/WORKSTREAMS.md): how the work is split and the
  reference material a contributor needs locally.
* [PROGRESS.md](PROGRESS.md): the earlier workstream history.

## Rules for contributions

* Never commit original game material: no decompiled source, Lua scripts,
  assets, extracted data, fixtures or golden files generated from the game.
  Keep such material under the git-ignored `local/` folder.
* 100% Rust at runtime: no JVM, no Java bridge, no C Lua.

## License

Original pz-rs code and documentation are licensed under the [MIT License](LICENSE),
except where a file or component states otherwise.

Bundled third-party code retains its existing licenses and copyright notices:

* [`pz-bullet`](crates/pz-bullet/README.md) includes Bullet Physics code under
  the [zlib license](crates/pz-bullet/LICENSE-ZLIB), with additional credits
  preserved in the source files.
* [`pz-clipper`](crates/pz-clipper/README.md) and
  [`pz-clipper6`](crates/pz-clipper6/README.md) include Clipper code under the
  Boost Software License 1.0, earcut under ISC, and libc++ sorting code under
  Apache-2.0 with the LLVM exception. Their license files and source notices
  remain applicable.
* External dependencies remain under their respective licenses.

The MIT license applies only to rights held by pz-rs contributors. It does not
grant rights to Project Zomboid's code, scripts, assets, data, or trademarks.
You must provide your own installed copy of the game.
