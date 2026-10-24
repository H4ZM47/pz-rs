# Progress

Status of the Rust port. Update this file with every merged change.

## Done
- Workspace scaffold and shared interfaces (`pz-kahlua` values/tables/native
  call contract, `pz-fs`, `pz-render` draw list, `pz-text`, `pz-ui`).
- Kahlua tables: insertion order, metatable-aware `rawget`, `#` length.
- Kahlua number formatting/parsing (Java `Double.toString`/`parseDouble`).

- Kahlua compiler: all 1,395 Lua files shipped with Build 42.21 compile to
  bytecode byte-identical to the original compiler (checked locally with
  `examples/compare_bytecode.rs` against the original's `Prototype.dump`).
- Kahlua VM (KahluaThread loop, metatables, errors, coroutines) and
  BaseLib, TableLib, MathLib, CoroutineLib, RandomLib.

- `pz-scripts`: ScriptParser, ScriptManager/ScriptModule/ScriptBucket loading
  of `media/scripts` (game + mods, template-first file order), with Java
  `HashMap` iteration order and script version hashes. On Build 42 all
  37 script types load with the same per-type order and script versions as
  the original (checked locally against the original `ScriptManager`); only
  `StringList` has its typed class so far, others load as raw blocks.
- `pz-scripts` script classes: items, recipes, evolved recipes, fixing,
  models, sounds, animations, ragdolls, vehicles and templates, fluids,
  energy, timed actions, XUI scripts, entities with all components, craft
  recipes, item configs, traits/professions, item filters, hit reactions.
  Field-level dumps of every Build 42 script of these types match the
  original (checked locally). Built-in registry keys come from the installed
  `projectzomboid.jar` via `pz-classfile`.
- StringLib and OsLib. Local differential run against the original Kahlua
  (`examples/differential.rs`): 64 of 65 corpus scripts produce identical
  output; the remaining one differs only in an identity hash.

- Character simulation core (`pz-characters::character`): `Stats`,
  `CharacterStat`, `Moodle`/`MoodleStat`/`Moodles`, `IsoGameCharacter.XP`
  and perk levels, `applyTraits`, `calculateStats` and the stat part of
  `updateInternal`, with Lua bindings. A scripted headless run (XP with
  boosts/traits/multipliers, `applyTraits`, stat clamps, 300 moodle ticks,
  200 `calculateStats` ticks awake and asleep) matches the original bit for
  bit (checked locally). BodyDamage, Nutrition, Thermoregulator and Fitness
  are stubs.

## In progress
- UIElement/UIManager, Core options, client window loop.

## Next (main-menu milestone)
1. Kahlua stdlib; load `stdlib.lbc`/`stdlib.lua` from the install.
2. `LuaManager.init` + `LoadDirBase("shared")`, `("client")` with API stubs.
3. Fonts, translations, textures, UIElement/UIManager.
4. `MainScreenState` running `MainScreen.lua` in the client.
