# pz-rs

A from-scratch Rust reimplementation of the Project Zomboid (Build 42) game
engine. The goal is 1:1 behaviour with the original on macOS, Windows and
Linux, with the game's own Lua scripts and assets running unmodified on a
native engine: no JVM, no Java bridge and no C Lua at runtime.

> [!IMPORTANT]
> **Not yet playable.** The client boots, runs the game's main menu and can
> create or load a single-player world, but an in-game session still stops on
> Lua API methods that are not bound yet.
>
> **No game files are in this repository.** pz-rs reads `media/` (Lua,
> scripts, textures, maps, sounds, translations) and some built-in constants
> from your own installed copy of the game at runtime. Without an install it
> has nothing to run.
>
> Unofficial project, not affiliated with or endorsed by The Indie Stone.

[Status](#status) · [Limits](#limits) · [How results are checked](#how-results-are-checked) ·
[Running](#running) · [Next steps](#next-steps) · [Layout](#layout) · [Contributing](#contributing) · [License](#license)

## What it replaces

pz-rs replaces the game's executable code, not its content:

* the Java engine in `projectzomboid.jar` (`zombie.*`);
* the embedded Kahlua Lua runtime;
* the native libraries the jar calls (`PZPopMan`, `PZClipper`,
  `PZPathFind`, Bullet physics) and the FMOD runtime (banks are decoded in
  Rust).

The jar stays on disk because `pz-classfile` parses constants out of it as
data; it is never executed. Details are in
[docs/STATUS.md](docs/STATUS.md#what-the-project-replaces).

## Status

Most subsystems exist as ported libraries with their own tests. What remains
is mostly wiring those libraries into the running game loop and binding the
Lua-facing API that the shipped Lua calls. The workspace has 49 crates and
about 2,900 unit tests.

Selected results, each with what it does and does not show:

| Area | Result | Conditions |
| --- | --- | --- |
| Lua compiler | All 1,395 Lua files shipped with Build 42.21 compile to bytecode byte-identical to the original compiler's | Checked locally against the original's `Prototype.dump` |
| Chunk saves | 14,850 of 14,894 real chunk re-saves byte-identical to the original's own re-save | Checked locally; the remainder differ in random key ids and one model name ([PROGRESS.md](PROGRESS.md)) |
| World generation | Object lists match the game on all 167,232 squares of 24 regions | Checked locally; blending and foraging zones not ported |
| Erosion, climate, zombie chase, body damage | Match the original bit for bit | Each library compared in isolation, not inside a running session |
| Engine integration | 56 of 207 items in the integration checklist done | A checked item means the connection exists, not that runtime parity is verified ([ENGINE-WIRING.md](docs/ENGINE-WIRING.md)) |
| Lua API | Largest open area; for example 1 of 58 `ItemCodeOnCreate` callbacks is implemented | The most recent recorded stop is in the inventory pane |

[docs/STATUS.md](docs/STATUS.md) has the per-workstream status and the full
checklist of what remains.

## Limits

* **Gameplay.** No in-game session runs to completion yet. Animals,
  ragdolls, vehicles, ballistics, pathfinding, fire and radio exist as
  libraries but are not connected to the frame loop.
* **Mods.** The mod loader is ported as a library, but mods are not loaded
  at boot yet.
* **Saves.** Chunk files, `map_t.bin`, player load and quit-save exist. A full
  game save, sandbox options on load and the vehicle database do not.
* **Multiplayer.** Not started. There is no network protocol, client or
  server.
* **Platforms.** Sessions against an installed game have been run on macOS
  only. Windows and Linux are covered by CI builds and unit tests, which run
  without game assets.
* **Game version.** Comparisons were made against Build 42.21. Other builds
  may differ.

## How results are checked

* **Against the original, locally.** Parity claims come from comparing the
  port's output with the original game's on the same input (bytecode, save
  files, generated squares, simulation dumps). These checks need the
  installed game and a local decompile, so they run on a developer machine
  and are opt-in; nothing derived from the game is committed.
* **In CI, without the game.** Every push builds `pz-client` and runs the
  workspace tests on Linux x64, Windows x64 and macOS on Apple Silicon and
  Intel ([workflow](.github/workflows/platform-checks.yml)). A green run shows
  the code builds and its unit tests pass on those platforms. It does not
  show gameplay parity.
* **Integration is tracked separately from parity.** Being wired into the
  game loop and behaving like the original are recorded as different things,
  and the status files say which one a claim means.

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

## Next steps

Grouped from [docs/STATUS.md](docs/STATUS.md#what-remains). The order is a
suggested dependency order, not a schedule:

1. Bind the Lua API methods the shipped client Lua reaches next, until an
   in-game single-player session runs without stopping.
2. Finish boot: mods, perks and body locations before Lua loads, scripts in
   the windowed boot.
3. Connect the ported world systems to the frame loop: pathfinding,
   vehicles, fire, animals, ragdolls, square mutations.
4. Full game save and load.
5. The rest of the world render pass and the world map.
6. Parity runs for subsystems that are only unit-tested so far.
7. Multiplayer.

## Layout

<details>
<summary>49 crates, grouped by area, with the original code each one replaces</summary>

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

</details>

## Documentation

* [docs/STATUS.md](docs/STATUS.md): what the project replaces, what is done
  and a checklist of what remains. Start here.
* [docs/ENGINE-WIRING.md](docs/ENGINE-WIRING.md): the detailed integration
  checklist, with the hooks each subsystem exposes and where the game loop
  must call them.
* [docs/WORKSTREAMS.md](docs/WORKSTREAMS.md): how the work is split and the
  reference material a contributor needs locally.
* [PROGRESS.md](PROGRESS.md): the earlier workstream history.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, how to find work and what a
pull request needs. The two hard rules:

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

Project Zomboid is a trademark of The Indie Stone. This project is not
affiliated with, endorsed by or sponsored by The Indie Stone.
