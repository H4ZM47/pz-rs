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
- World/maps (`pz-world`): lot headers, lotpacks, `MapFiles`, meta grid
  rooms/buildings, `.tiles` reader, `Fix2x`, and chunk loading through
  `IsoCell.PlaceLot`/`CellLoader.LoadCellBinaryChunk` with generic tile
  objects. Header and lotpack decoding match the original loader on 163
  cells (Muldraugh sample plus six challenge maps), checked locally.
  Meta-grid zones (`Zone` and the vehicle/mannequin/world-gen/animal
  subclasses, chunk zone arrays, Lua-facing registration, lot-priority
  removal) match the original `IsoMetaGrid` on randomized registrations;
  the native clipper is an injected trait and the zone tables are runtime
  input.
- Climate and clock (`pz-core::weather`, `GameTime`): `ClimateManager`
  with its float/colour/bool layers, `ClimateValues`, `WeatherPeriod`,
  `ThunderStorm`, `ClimateForecaster`, `ClimateMoon`, `ErosionSeason`,
  `SimplexNoise` and `GameTime.update`, bound to Lua (`bind_climate`) and
  feeding the character simulation's air temperature, humidity, rain and
  wind chill. Hourly and daily dumps match the original bit for bit over
  more than 400 simulated days across seasons, sandbox weather cycles and
  daylight-saving changes (checked locally).
- Weather effects (`pz-iso-render::weather_fx`, `weather_fx_mask`):
  `IsoWeatherFX` cloud, fog, snow and rain particle layers driven by the
  climate's render values, the fog circle, `WeatherParticleDrawer`
  batching, and `WeatherFxMask` keeping them out of the player's building
  (mask and particle render textures in `pz-gpu`, `fogCircle` shader).
  Particle state and drawer quads over 600-900 frames match the original
  bit for bit but for last-ulp rain rotations; `RasterizeBounds` and
  `Rasterize.scanTriangle` match exactly (checked locally). Mask walls are
  drawn without cutaways and characters are not in the mask yet.
- World save files: `map_t.bin` (`GameTime` + climate, in `pz-core`),
  `map_meta.bin` (room/building state, safehouses, factions, designation
  zones, stashes), `map_zone.bin` and `gos_*.bin` (global-object systems)
  read and write byte-identically to the original on 7 local saves
  (checked with the original's load/save on scratch copies).
- StringLib and OsLib. Local differential run against the original Kahlua
  (`examples/differential.rs`): 64 of 65 corpus scripts produce identical
  output; the remaining one differs only in an identity hash.

- Character simulation core (`pz-characters::character`): `Stats`,
  `CharacterStat`, `Moodle`/`MoodleStat`/`Moodles`, `IsoGameCharacter.XP`
  and perk levels, `applyTraits`, `calculateStats` and the stat part of
  `updateInternal`, with Lua bindings. A scripted headless run (XP with
  boosts/traits/multipliers, `applyTraits`, stat clamps, 300 moodle ticks,
  200 `calculateStats` ticks awake and asleep) matches the original bit for
  bit (checked locally). BodyDamage is a stub.
- `Nutrition`, `Thermoregulator` (thermal nodes, clothing insulation,
  metabolic rate from the jar's `Metabolics`, cold damage) and `Fitness`
  (regularity, stiffness, exercise XP; exercises read from Lua's
  `FitnessExercises`), with Lua bindings. Scripted headless runs (7000
  nutrition updates with meals, starvation and weight traits; 3000 combined
  thermoregulator/moodle/nutrition/fitness ticks with clothing, wetness,
  climate and exercise; 6000 extreme cold/heat ticks; profession
  regularities and 400 repetitions) match the original bit for bit
  (checked locally). Climate readings, worn clothing and body-part
  wetness/stiffness are inputs until those systems are ported.

- BodyDamage (`pz-characters::character::body_damage`, `body_part`):
  `BodyDamage`, `BodyPart`, `BodyPartLast`, `BodyPartContacts` and
  `BodyPartType`'s statics over the jar table: injuries, bandages,
  stitches, splints, infection/zombification, bleeding, healing, pain,
  cold, food/pill effects, save/load and `BodyDamage.Update`, with Lua
  bindings (`getBodyDamage`, `BodyDamage`, `BodyPart`). A headless run of
  two players over 2400 ticks of scripted injuries plus a save/load matches
  the original bit for bit (checked locally). Item, zombie and weapon inputs
  are plain data until pz-inventory lands.

## In progress
- UIElement/UIManager, Core options, client window loop.

## Next (main-menu milestone)
1. Kahlua stdlib; load `stdlib.lbc`/`stdlib.lua` from the install.
2. `LuaManager.init` + `LoadDirBase("shared")`, `("client")` with API stubs.
3. Fonts, translations, textures, UIElement/UIManager.
4. `MainScreenState` running `MainScreen.lua` in the client.
