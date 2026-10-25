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

- World/maps (`pz-world`): lot headers, lotpacks, `MapFiles`, meta grid
  rooms/buildings, `.tiles` reader, `Fix2x`, and chunk loading through
  `IsoCell.PlaceLot`/`CellLoader.LoadCellBinaryChunk`. Object creation
  (`CellLoader.DoTileObjectCreation`: doors, windows, curtains, light
  switches, trees, stoves, radios/TVs, ...), `FixStackableObjects`,
  associated buildings and adjacent-cell rooms match the original per
  square, field and `Rand` draw on 10,240 Muldraugh chunks, checked locally.
  Header and lotpack decoding match the original loader on 163
  cells (Muldraugh sample plus six challenge maps), checked locally.
  Tile definitions (`pz_world::sprite`): `IsoSpriteManager`,
  `PropertyContainer`, `TilePropertyAliasMap` and the full
  `IsoWorld.LoadTileDefinitions` property pass; all 61,229 sprites match the
  original loader locally. Enum and lookup tables are runtime data
  (`SpriteTables`, `ObjectTables`); the jar loader awaits `pz-classfile`.
- Chunk save files (`pz_world::save`): `map/<wx>/<wy>.bin` load and save
  (`IsoChunk`, `IsoGridSquare`, `IsoObject` and the object classes chunks
  hold, corpses, containers, mod data, erosion data). Items and ECS
  components are kept as their length-framed bytes. On 14,894 real chunk
  files the decoded squares match the original's load except for one
  constructor-added component class, and 13,348 re-saves are byte-identical
  to the original's own re-save (the rest differ inside item or component
  records, which the original re-saves through classes not ported here);
  4,085 map-loaded chunks saved here load and re-save as expected in the
  original. Checked locally.
- Items (`pz-inventory`): `InventoryItem` and the subclasses saves hold
  (Food, Clothing, AlarmClock(Clothing), HandWeapon with weapon parts,
  DrainableComboItem, ComboItem, WeaponPart, Literature, Key, Moveable,
  Radio, InventoryContainer, MapItem with opaque symbols), creation from
  item scripts, `save`/`load` with their load-time changes,
  `ItemContainer` and `CompressIdenticalItems`, mod data tables and
  `WorldDictionary.bin`. Chunk files now hold decoded items: 14,850 of
  14,894 chunk re-saves are byte-identical to the original's (13 still
  differ in items: keys saved with id -1 draw a random id, and a
  respirator's alternate model name), 62,900 of 62,914 items load with the
  original's field values (the 14 others are those random key ids), and
  the original loads the chunks written here with the same item fields.
  `AnimalInventoryItem` stays raw bytes. Checked locally.
- Vehicles and moving objects in saves (`pz_world::save::vehicle`,
  `save::movers`): `BaseVehicle.save`/`load` with `VehiclePart`,
  `VehicleEngine`, lights, doors, windows, device data, part items and
  containers, shared by `vehicles.db` rows and chunk-file vehicle lists
  (read; saving writes the empty list as the original), with the load-time
  changes re-saves show (the JOML transform round trip of the rotation,
  physics-height and position clamps, engine/lightbar/condition clamps,
  dropped vehicle id, alarm sound without a script). `IsoPushableObject`,
  `IsoWheelieBin` and `IsoAnimalTrack` in square object lists. All 243
  `vehicles.db` rows of 8 local saves load with the original's field values
  and 242 re-save byte-identically to the original's re-save (the other
  holds a trailer animal, kept as bytes, whose record the original
  re-derives); on 1,400 synthetic chunk files built with the original's
  classes, 1,248 chunk-list vehicles, 1,869 pushables, 1,862 wheelie bins
  and 1,869 animal tracks decode and re-save identically. Checked locally.
- Feeding troughs and hutch animals in chunk files: the `IsoFeedingTrough`
  constructor's fluid container (components the constructor makes load in
  place), `initWithDef`, and `IsoAnimal.save`/`load` as hutches and nest
  boxes store animals (stats, genome, `AnimalData` fields) with their
  load-time changes and `addAnimalInside`. 14,870 of 14,894 re-saves are
  byte-identical to the original's; all 60 troughs load with the original's
  fields. Test hutches filled with 34 animals by the original's own classes
  re-save byte-identically except one wild animal (`setWild(true)` draws a
  random stress), and the original loads the chunks written here with the
  same animal fields. Checked locally.
- World hooks (`pz_world::WorldState`, `SquareHooks` and the renderer's
  `WorldHooks`): `IsoBarricade` queries (barricades on both sides of door,
  window and thumpable edges, planks, metal, vision, light transmission),
  `IsoThumpable` blocked doors and windows, `IsoDoor` curtains,
  `haveElectricity` from the chunks' generator lists, `hasGridPower`,
  `IsoLightSwitch.canSwitchLight`, and light items (`isEmittingLight`, the
  light getters, `HandWeapon`'s weapon light) as player torches; sandbox
  and world power values are runtime input. On 7 local saves and on 3
  copies barricaded, curtained and powered by the original's own classes
  (with the grid on and shut off), every compared line matches the
  original's queries: 14,723 + 751,347 square lines (blocked doors and
  windows, electricity), 5,570 barricades, 7,358 doors, 8,895 windows,
  157 thumpables, 17,055 light switches and 378 light items per sandbox
  setting. Checked locally.
- World regions (`pz_world::isoregion`): `IsoRegions` square flags, chunk
  and world regions, enclosure, roofs, neighbours, square updates, the
  worker's job batching and `isoregiondata` files; `IsoWindow.load`'s
  sprite changes on chunk load; the weather mask reads them through
  `pz_world_render::WorldFxMask`. On 7 local saves (14,894 chunks) the
  region dumps match the original's after loading the saved regions,
  reading every chunk, 300 square changes and a reload (232,555 lines),
  and all 14,901 written region files are byte-identical. Checked locally.
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
- `pz-entity`: `GameEntity` component storage and `saveEntity`/`loadEntity`
  (`ByteBlock` framing, load-in-place), the saved state of every component
  type found in Build 42 saves (FluidContainer with fluids/filters,
  Attributes, Durability, SpriteConfig, SpriteOverlayConfig, UiConfig,
  Script, CraftBench, CraftLogic/DryingCraftLogic, Resources, MetaTag and
  the data-less ones), meta entities (`entity_data.bin`), and
  `GameEntityFactory` components from scripts. Checked locally on 7 saves:
  all 148,039 component blocks (objects, items, meta entities) decode and
  re-encode byte-identically with fields matching the original's loaded
  objects.
- StringLib and OsLib. Local differential run against the original Kahlua
  (`examples/differential.rs`): 64 of 65 corpus scripts produce identical
  output; the remaining one differs only in an identity hash.
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
