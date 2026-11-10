# Project status and remaining work

A contributor-oriented summary of where pz-rs stands and what is left. It is
derived from [ENGINE-WIRING.md](ENGINE-WIRING.md), [WORKSTREAMS.md](WORKSTREAMS.md)
and [../PROGRESS.md](../PROGRESS.md) as of 2026-11-10 (commit `94e0a27`). Those
files remain the detailed record; this one is the map. Where they disagree with
this file, `ENGINE-WIRING.md` is the more current source.

## What the project replaces

pz-rs replaces the **executable code** of Project Zomboid Build 42, not its
content:

| Replaced by Rust | Not replaced (read from your install at runtime) |
| --- | --- |
| The Java engine in `projectzomboid.jar` (`zombie.*`) | `media/`: Lua scripts, item/recipe scripts, textures, maps, sounds, translations |
| The embedded Lua runtime (`se.krka.kahlua`, `org.luaj.kahluafork`) | FMOD Studio bank files (decoded in Rust by `pz-audio`) |
| The native libraries the jar calls: `PZPopMan`, `PZClipper`, `PZPathFind`, Bullet physics | Built-in constants still read out of the installed `projectzomboid.jar` by `pz-classfile` (parsed as data; no JVM is run) |

So yes, the goal is to replace everything `projectzomboid.jar` *does*, plus
its native helpers, while the game's own Lua and assets run unmodified on top.
The stated target is 1:1 behaviour on macOS, Windows and Linux with no JVM,
Java bridge or C Lua at runtime ([README](../README.md)). You still need a
legally installed copy of the game, and the jar must remain on disk because
`pz-classfile` reads constants from it.

## Where it stands in one paragraph

Most subsystems exist as **ported libraries with their own tests**; the
remaining work is dominated by **wiring those libraries into the running game
loop** and by **filling in the Lua-facing API** that the game's shipped Lua
calls. The workspace has 49 crates and roughly 650,000 lines of Rust with about
2,900 unit tests. The client boots, runs the real main-menu Lua, and can create
or load a single-player world (`pz-client --new-game NAME` /
`--load-game MODE/NAME`, optionally `--headless-frames N`). It is not yet
playable: the in-game session still stops on unbound API methods (the most
recently recorded stop is `isFavouriteRecipeInput` at
`ISInventoryPane.renderdetails:2338`), and many world systems are not yet
connected to the frame loop.

The integration checklist in `ENGINE-WIRING.md` currently reads:

| Section | Done | Open |
| --- | ---: | ---: |
| Boot (`GameWindow.init`) | 9 | 16 |
| Per frame (`GameWindow.logic` / `render`) | 11 | 2 |
| World (`GameLoadingState` / `IngameState`) | 31 | 99 |
| World render pass | 4 | 27 |
| World map | 1 | 5 |
| Cleanups | 0 | 2 |
| **Total** | **56** | **151** |

A checked box means the connection is implemented, not that runtime parity
with the original has been verified.

## Status by workstream

Numbering follows [WORKSTREAMS.md](WORKSTREAMS.md).

| # | Workstream | Crates | Status |
| --- | --- | --- | --- |
| 1 | Kahlua compiler, VM, stdlib | `pz-kahlua` | **Largely done.** All 1,395 shipped Lua files compile to bytecode byte-identical to the original compiler; VM, coroutines and the standard libraries are ported. Ongoing work is GC/ownership behaviour at host boundaries. |
| 2 | Lua API bindings | `pz-lua-api` | **In progress; the largest open area.** The biggest crate (~120k lines) and still the usual place a session stops. Example of scale: 1 of 58 `ItemCodeOnCreate` callbacks is implemented. |
| 3 | File system and mods | `pz-fs` | **Library done.** Mod loading at boot is not wired. |
| 4 | Textures, packs, draw list | `pz-render`, `pz-gpu` | **Done for UI and chunk rendering.** GPU readbacks run in CI on four platforms. |
| 5 | Fonts and translations | `pz-text` | **Done.** |
| 6 | UI layer | `pz-ui` | **Working** (main menu and HUD run); HUD inputs partly wired. |
| 7 | Game loop, Core, game states | `pz-engine`, `pz-client` | **In progress; the integration bottleneck.** State machine, loading and in-game states exist; most open checklist items land here. |
| 8 | Script parser | `pz-scripts`, `pz-script-parser` | **Done as a library** and loaded at boot. |
| 9 | Maps, chunks, world | `pz-world`, `pz-worldgen`, `pz-erosion`, `pz-lighting`, `pz-worldmap` | **Libraries ported**, chunk load/save works. Square mutations, erosion, fire, regions and the world map are only partly wired. |
| 10 | Characters, animation, models | `pz-characters`, `pz-anim`, `pz-model`, `pz-character-model`, `pz-ragdoll`, `pz-animals`, `pz-vehicles`, `pz-pathfind`, `pz-popman` | **Libraries ported**, several checked bit-for-bit against the original in isolation (e.g. zombie chase, body damage). Players and zombies update in the loop; animals, ragdolls, vehicles, ballistics and pathfinding are not yet wired. |
| 11 | Audio | `pz-audio`, `pz-sound`, `pz-world-sound`, `pz-radio` | **Decoding done** (FSB5/Vorbis from FMOD banks). Sound manager ticks per frame; positioned world sounds, fire sounds and radio are not wired. |
| 12 | Saves | in `pz-world::save`, `pz-core`, `pz-sqlite`, `pz-inventory` (no separate `pz-save` crate) | **Partial.** Chunk files, `map_t.bin`, player load and quit-save exist. Vehicle DB, sandbox load, basements, reanimated players and the full `GameWindow.save` path are open. |
| 13 | Multiplayer | `pz-network` | **Not started.** The crate only holds the offline server list and account database; there is no RakNet protocol, `GameClient` or `GameServer`. |

## What remains

Grouped from the 151 open items in `ENGINE-WIRING.md`. Each bullet names the
section to search there for exact hook names and call order.

### 1. Lua API coverage (workstream 2)
- [ ] Bind the methods the shipped client Lua hits next, starting from the
      recorded stop in the inventory pane (`isFavouriteRecipeInput`).
- [ ] Remaining 57 `ItemCodeOnCreate` callbacks.
- [ ] Wire the new actual-reference `ArrayList` foundation to script,
      literature and character recipe-list fields.
- [ ] Event and hook names read from the jar via `pz-classfile` instead of a
      hand-kept list.
- [ ] Fold `bind_vehicles`' item subset into the full item bindings (#86).

### 2. Boot
- [ ] Mods (when Core's mods option is on) and the real `MapGroupsHost`.
- [ ] `PerkFactory::init` and `BodyLocations.lua` before Lua loads.
- [ ] Character simulation context, world sounds and states world installed
      at boot.
- [ ] `ScriptManager` loading in the windowed boot path (currently the
      client's own path).
- [ ] HUD hosts for local/current player.

### 3. World simulation
- [ ] Pathfinding service: world bounds, thread/main updates, square-edit and
      vehicle hooks.
- [ ] Vehicles: chunk load/save, `VehiclesDB2`, virtual vehicle manager,
      physics world ownership (Bullet), models and lights.
- [ ] Fire: per-frame `IsoFireManager`, mutations, heat sources, sounds,
      fire fighting.
- [ ] Erosion and `ClimateManager` snow after cell load; climate values into
      the character simulation.
- [ ] Animals: definitions, live animals, behaviour, animal paths.
- [ ] Ragdolls and ballistics: boot scripts, per-character controllers,
      shots, corpses.
- [ ] Square mutations and floor items routed through one path (objects
      added/removed, `AddWorldInventoryItem`, item drops), with the
      pathfinding, render and Lua hooks that depend on it.
- [ ] Tile overlays, broken/bent fences, key naming, story clutter,
      `MapObjects` callbacks.
- [ ] Climbs, stairs, fences, windows, sheet ropes and player death.
- [ ] Radio: one `ZomboidRadio` per world, devices, Lua binding.
- [ ] Corpse rot, the live corpse registry, flies and reanimation.
- [ ] Building regions updates.

### 4. Inventory, HUD and interaction
- [ ] Character inventories, item owners, worn items and world-container
      parents for characters created outside the load path.
- [ ] Object picking (world and render side), HUD camera, shared tooltip.
- [ ] Health panel, moodles per character, alarm clocks, debug console.
- [ ] Chat (single player), world markers, gizmos.
- [ ] Timed-action and action-context phase timing; character visibility
      fading.

### 5. Saves
- [ ] Full game save (`GameWindow.save(true)`: sleep, menu save, quit) and
      hot save.
- [ ] Session save, characters side; reanimated players.
- [ ] `SandboxOptions` from `map_sand.bin` on load.
- [ ] `Basements` save file.
- [ ] `WorldDictionary`: string/script register population and version
      hashing.
- [ ] Route the older value-container floor save through the
      actual-reference serializer.

### 6. Rendering
- [ ] Single `world_frame_full` call per frame, with sky, water and weather
      effects.
- [ ] Lighting from `ClimateManager::render_values`; screen (weather)
      shader inputs.
- [ ] Vehicles, corpses, world items, mannequins and animal corpses in the
      frame.
- [ ] Character lights, outlines, held/attached item textures.
- [ ] Frame extras: highlights, markers, area highlights, vision cone, fog.
- [ ] Split screen.
- [ ] GPU fault handling; vsync and frame size from Core options.

### 7. World map
- [ ] Map host inputs, live `MapFilesInfo`, image frame hooks, visited
      areas, symbols, streets and overlays.

### 8. Multiplayer
- [ ] Everything: RakNet transport, `GameClient`, `GameServer`, network
      character states. Global-object command delivery exists for single
      player only.

### 9. Verification
- [ ] Parity runs for subsystems only unit-tested so far (ragdolls, vehicles,
      stairs and slopes are called out in `PROGRESS.md`).
- [ ] CI builds and tests on Linux, Windows and both macOS architectures but
      has no game assets, so a green run does not establish gameplay parity.
      Installed-asset session checks are run locally and are opt-in.

## How to pick something up

1. Read the contribution rules in the [README](../README.md): no decompiled
   source, game scripts, assets or data derived from them may be committed.
   Keep that material under the git-ignored `local/` folder.
2. Set up the reference material listed in
   [WORKSTREAMS.md](WORKSTREAMS.md#reference-material): the installed game and
   a local decompile of `projectzomboid.jar` to port from. Point the client at
   the install with `--game-dir` or `PZ_GAME_DIR`.
3. Choose a scope:
   - **Smallest:** run a headless session
     (`pz-client --new-game test --headless-frames 120`), take the first Lua
     error naming a missing method, and bind it in `pz-lua-api` with tests.
   - **Medium:** take one open checkbox in `ENGINE-WIRING.md` whose library
     side already exists (most of them) and connect it in `pz-engine`.
   - **Large:** an unstarted area such as multiplayer.
4. Search `ENGINE-WIRING.md` for the item; each entry gives the functions to
   call and where in the original's order they belong.

## Known problems with the existing status files

- `PROGRESS.md` contains many duplicated entries from merges and its "In
  progress" / "Next" sections describe the long-passed main-menu milestone.
  Treat it as history.
- `ENGINE-WIRING.md` opens with roughly 600 lines of per-commit narrative
  before the checklist begins at "Boot".
- `WORKSTREAMS.md` names crates that were never created under those names
  (`pz-save`, `pz-net`) and a `tools/` directory that does not exist yet.
