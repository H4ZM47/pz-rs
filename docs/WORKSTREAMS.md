# Workstreams

Each workstream owns one crate (or module) and codes against the interfaces
listed here. Interfaces marked *stub first* exist in the scaffold so
dependants can compile before the implementation lands.

| # | Workstream | Crate | Depends on | Interfaces it provides |
| --- | --- | --- | --- | --- |
| 1 | Kahlua VM, compiler, stdlib | `pz-kahlua` | – | `Value`, `TableRef`, `Thread`, `NativeFunction`/`NativeCall` (*stub first*), `LuaObject` |
| 2 | Lua API bindings | `pz-lua-api` | 1, all engine crates | `register_all`; per-class binding tables matching the original exposure |
| 3 | File system and mods | `pz-fs` | – | `GameFs::resolve`, `lua_files` (*stub first*), mod load order |
| 4 | Textures, texture packs, draw list | `pz-render` | 3 | `Texture`, `DrawList`, `TextureLoader` (*stub first*) |
| 5 | Fonts and translations | `pz-text` | 3, 4 | `Translator`, `AngelCodeFont`, `TextManager` |
| 6 | UI layer | `pz-ui` | 1, 4, 5 | `UIElement`, `UIManager` input/render dispatch |
| 7 | Game loop, Core, game states | `pz-engine` | all above | `GameWindow`, state machine, `Core` options |
| 8 | Script parser (items, recipes, …) | new `pz-scripts` | 3 | `ScriptManager` |
| 9 | Maps, chunks, world | new `pz-world` | 3, 4 | `IsoWorld`, `IsoCell`, chunk loading |
| 10 | Characters, animation, models | new crates | 4, 9 | – |
| 11 | Audio (FMOD bank decoding in Rust) | new `pz-audio` | 3 | `SoundManager` |
| 12 | Saves | new `pz-save` | 9 | save/load formats |
| 13 | Multiplayer | new `pz-net` | 9, 12 | RakNet protocol, `GameClient`/`GameServer` |

## Reference material

Porting needs the original game, which cannot be committed here. Each
contributor needs, locally and git-ignored:

* the installed game (`media/` for scripts and assets);
* a decompile of `projectzomboid.jar` (`zombie.*`, `se.krka.kahlua.*`,
  `org.luaj.*`) to port from;
* for workstream 2, the Lua exposure manifest dumped from the original
  exposer (`tools/` will hold the dumping tool; its output stays local).

Parity checks compare against the original at runtime on a machine with the
game installed; results are recorded as pass/fail notes, never as committed
game data.
