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

- World generation (`pz-worldgen`): B42 `WorldGenChunk.generateChunks`
  for brand-new chunks (map biomes from the biome-map images, procedural
  biomes, roads, ore veins, static modules, map `WorldGen` zones), plugged
  into `pz_world::chunk::load_brand_new` as a `BrandNewHook`. Object lists
  match the game on all 167,232 squares of 24 regions, checked locally,
  including procedural basements (placements, `.pzby` stamping) on 7 regions.
  Basement rooms and buildings join the meta grid; object lists, room ids
  and buildings match on every square. Blending/attachments (chunk update)
  and foraging zones are not ported.

- World/maps (`pz-world`): lot headers, lotpacks, `MapFiles`, meta grid
  rooms/buildings, `.tiles` reader, `Fix2x`, and chunk loading through
  `IsoCell.PlaceLot`/`CellLoader.LoadCellBinaryChunk`. Object creation
  (`CellLoader.DoTileObjectCreation`: doors, windows, curtains, light
  switches, trees, stoves, radios/TVs, ...), `FixStackableObjects`,
  associated buildings and adjacent-cell rooms match the original per
  square, field and `Rand` draw on 10,240 Muldraugh chunks, checked locally.
  Header and lotpack decoding match the original loader on 163
  cells (Muldraugh sample plus six challenge maps), checked locally.
  Meta-grid zones (`Zone` and the vehicle/mannequin/world-gen/animal
  subclasses, chunk zone arrays, Lua-facing registration, lot-priority
  removal) match the original `IsoMetaGrid` on randomized registrations;
  the native clipper is an injected trait and the zone tables are runtime
  input.
  Tile definitions (`pz_world::sprite`): `IsoSpriteManager`,
  `PropertyContainer`, `TilePropertyAliasMap` and the full
  `IsoWorld.LoadTileDefinitions` property pass; all 61,229 sprites match the
  original loader locally. Enum and lookup tables are runtime data
  (`SpriteTables`, `ObjectTables`); the jar loader awaits `pz-classfile`.
  Meta-grid zones (`Zone` and the vehicle/mannequin/world-gen/animal
  subclasses, chunk zone arrays, Lua-facing registration, lot-priority
  removal) match the original `IsoMetaGrid` on randomized registrations;
  the native clipper is an injected trait and the zone tables are runtime
  input.
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
- Erosion (`pz-erosion`): `ErosionMain`, `ErosionConfig`, `ErosionSeason`,
  `ErosionIceQueen`, `Noise2D`, `RandLocation`, the eight categories and
  the erosion objects, driven through `ChunkLoadHook`; tables are read from
  the jar at runtime. Against the original classes run in a JVM, noise,
  location randoms, chunk/square initialisation, a year of seasons, config
  defaults and category spawn decisions match bit for bit (checked
  locally). Square flags and wall/window edge queries come from
  `pz_world::square`; season dates use `pz_core::calendar` (the host time
  zone with daylight saving). Lua: `bind_erosion`.
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
  New items carry the vehicle-part values, `ItemCapacity`/`MaxCapacity`,
  radio `DeviceData` with presets and channel, and random map ids, with
  the original's `Rand` draws (channel lists and map definitions are
  runtime input); weights count fluid contents. Of 5,092 created script
  items, all match the original's draws, ids and vehicle values and 5,081
  every compared field (11 moveables differ in sprite-derived weight), and
  the part items of 242 `vehicles.db` vehicles match the original's
  weights and totals. Checked locally.
  New items carry the vehicle-part values, `ItemCapacity`/`MaxCapacity`,
  radio `DeviceData` with presets and channel, and random map ids, with
  the original's `Rand` draws (channel lists and map definitions are
  runtime input); weights count fluid contents. All 5,092 created script
  items match the original on every compared field, including
  moveables' sprite-derived weights, and the part items of 242
  `vehicles.db` vehicles match the original's weights and totals and
  make the same load-time `Rand` draws in the same order (every copy of
  an identical-item run is loaded again, as `CompressIdenticalItems`
  does). `IsoChunk` carries the per-level `physicsCheck` flags. Checked
  locally.
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
  `IsoChunk.vehicles` is a real list (chunk-file and `vehicles.db` loads,
  chunk moves, removal) and `IsoMetaCell.vehicleZones` is readable as
  `VehicleZone` views: the 9,693 Muldraugh vehicle zones per cell and
  `getVehicleZoneAt` match the original. Checked locally.
- `pz-vehicles`: the vehicle runtime. `BaseVehicle` with parts (script
  binding, the vehicle Lua `Create`/`Init`/`Update` called by name),
  engine and transmission, fuel, battery, keys, alarm, lightbar, hotwire,
  polygons, areas and passenger positions, damage; `CarController` and
  `WorldSimulation` over a `VehiclePhysics` trait mirroring the Bullet
  natives; the offline vehicle id map; `VehicleType`; `IsoChunk.AddVehicles`
  spawning; chunk vehicle lists on pz-world's cell; a pathfinding view
  with a `pathfind_adapter!` for pz-pathfind; Lua bindings in
  `pz-lua-api::bind_vehicles`. Against the original with a recording
  Bullet stand-in, a fake clock and scripted driving (400 frames, every
  dump field and Bullet call, the item load draws made on the vehicle's
  own `Rand` stream as the original does): 240 of 243 saved `vehicles.db`
  vehicles are bit-exact, two match up to the original's own crash (a
  broken window with no square) and one holds a trailer animal the
  original cannot run headless; the load draws match for all 242 loadable
  vehicles; all 241 vanilla vehicle scripts as new vehicles are
  bit-exact; 3,000 single-zone spawns and 2,000 `AddVehicles` runs on
  synthetic zone lists give the original's vehicles and `Rand` state;
  on real map chunks with pz-world's vehicle zones (identical to the
  original's in all 563 cells), `AddVehicles` gives the original's vehicles
  and next draw in 1,664 of 1,664 chunks of 26 street, parking-lot and
  traffic-jam regions (347 vehicles).
  End to end on pz-bullet (`VehiclePhysics` for `PzBullet`, with the
  natives' exceptions and step callbacks), against the original running
  the real libPZBullet on a Muldraugh region's chunk shapes: 240 of 241
  vehicle scripts are bit-exact for 400 driven frames (the other matches
  until the harness lacks a world-sound native). `IsoChunk
  .updatePhysicsForLevel` gives the original's `ToBullet` shapes for every
  one of 114,529 squares of four regions that load the same objects.
  `SurroundVehicle` (zombie attack spots, `chooseBestAttackPosition`)
  matches the original's slot assignments and spot states over 2,000
  sequences of zombie, target and vehicle moves, clock steps and
  `update` calls; `permanentlyRemove` and the removal paths are ported.
  Checked locally.
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
  `Rasterize.scanTriangle` match exactly (checked locally). Mask walls go
  through `CutawayNoDepth` with the player's cutaway flags, particles
  carry float colours, and characters reach the mask through
  `FxMaskCharacters`, which replays the frame objects' draws (no shadows)
  on their squares (ws/render-join).
- World save files: `map_t.bin` (`GameTime` + climate, in `pz-core`),
  `map_meta.bin` (room/building state, safehouses, factions, designation
  zones, stashes), `map_zone.bin` and `gos_*.bin` (global-object systems)
  read and write byte-identically to the original on 7 local saves
  (checked with the original's load/save on scratch copies).
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
- `pz-scripts`: ScriptParser, ScriptManager/ScriptModule/ScriptBucket loading
  of `media/scripts` (game + mods, template-first file order), with Java
  `HashMap` iteration order and script version hashes. On Build 42 all
  37 script types load with the same per-type order and script versions as
  the original (checked locally against the original `ScriptManager`); only
  `StringList` has its typed class so far, others load as raw blocks.
- World/maps (`pz-world`): lot headers, lotpacks, `MapFiles`, meta grid
  rooms/buildings, `.tiles` reader, `Fix2x`, and chunk loading through
  `IsoCell.PlaceLot`/`CellLoader.LoadCellBinaryChunk` with generic tile
  objects. Header and lotpack decoding match the original loader on 163
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
  original loader locally. Enum tables are runtime data (`SpriteTables`); the
  jar loader awaits `pz-classfile`.

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.
  cells (Muldraugh sample plus six challenge maps), checked locally.
  Meta-grid zones (`Zone` and the vehicle/mannequin/world-gen/animal
  subclasses, chunk zone arrays, Lua-facing registration, lot-priority
  removal) match the original `IsoMetaGrid` on randomized registrations;
  the native clipper is an injected trait and the zone tables are runtime
  input.

- `MapGroups` (`pz_world::map_groups`) with its Lua binding; ActiveMods,
  mod folders and Translator come through an engine-installed host.

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup. The
  Studio object model (events, parameters, timelines, instruments, buses)
  parses from every game bank; event lengths, distances and parameter
  descriptions match the FMOD runtime for all 3,490 events but one unused
  legacy event (see `crates/pz-audio/FORMAT.md`).
  original loader locally. Enum and lookup tables are runtime data
  (`SpriteTables`, `ObjectTables`); the jar loader awaits `pz-classfile`.

- `pz-clipper`: the `PZClipper` native library (Clipper2 1.5.4 + earcut.hpp
  + the game's JNI glue) in pure Rust. Local differential run against the
  shipped arm64 library: every in-contract case bit-identical.
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

  original loader locally. Enum and lookup tables are runtime data
  (`SpriteTables`, `ObjectTables`); the jar loader awaits `pz-classfile`.
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
  original loader locally. Enum and lookup tables are runtime data
  (`SpriteTables`, `ObjectTables`); the jar loader awaits `pz-classfile`.

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

- `pz-clipper6`: the polygon code of the native pathfinder (Clipper 6.4.2,
  earcut.hpp and the game's `PZClipper` wrapper) in pure Rust. Local
  differential run against the shipped arm64 library: 1.15 million random
  cases bit-identical.

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- `pz-clipper6`: the polygon code of the native pathfinder (Clipper 6.4.2,
  earcut.hpp and the game's `PZClipper` wrapper) in pure Rust. Local
  differential run against the shipped arm64 library: 1.15 million random
  cases bit-identical.

- `pz-clipper6`: the polygon code of the native pathfinder (Clipper 6.4.2,
  earcut.hpp and the game's `PZClipper` wrapper) in pure Rust. Local
  differential run against the shipped arm64 library: 1.15 million random
  cases bit-identical.

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Appearance tables and outfit draws (`pz-characters::population`):
  `ClothingDecals`, `HairStyles`, `BeardStyles`, `VoiceStyles` (JAXB
  binding rules, mod overrides), `PopTemplateManager` skin lists,
  `HairOutfitDefinitions`, `OutfitRNG`/`LocationRNG` (`pz-core`),
  `OutfitManager`/`Outfit`/`ClothingItemReference` with `Randomize`, the
  random outfit picks and `dressInOutfit`'s clothing choices,
  `DefaultClothing`, and a `HumanStyles` for `HumanVisual`. Lua bindings
  (`bind_appearance`). Loaded lists and seeded draws match the original on
  the installed media (every outfit randomized and dressed on three seeds)
  and on invented edge-case XML (checked locally).

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Appearance tables and outfit draws (`pz-characters::population`):
  `ClothingDecals`, `HairStyles`, `BeardStyles`, `VoiceStyles` (JAXB
  binding rules, mod overrides), `PopTemplateManager` skin lists,
  `HairOutfitDefinitions`, `OutfitRNG`/`LocationRNG` (`pz-core`),
  `OutfitManager`/`Outfit`/`ClothingItemReference` with `Randomize`, the
  random outfit picks and `dressInOutfit`'s clothing choices,
  `DefaultClothing`, and a `HumanStyles` for `HumanVisual`. Lua bindings
  (`bind_appearance`). Loaded lists and seeded draws match the original on
  the installed media (every outfit randomized and dressed on three seeds)
  and on invented edge-case XML (checked locally).

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Zombie population (`pz-popman`): a pure-Rust port of the native
  `PZPopMan` library (population cells, repopulation and redistribution,
  virtual zombies and their wall follower, world sounds, hordes, aggro
  targets, the collision grid and its path tasks, and the `zpop_*.bin` /
  `chunkdata_*.bin` save files), the single-player
  `ZombiePopulationManager` with `LoadedAreas`, `PlayerSpawns`,
  `RealZombieMap`, `ZombieStateFlags` and `ObjectPool`, and the population
  Lua globals (`bind_popman`). Four scripted runs of 600 to 12000 ticks
  match the shipped library line for line and save file for save file
  (checked locally). The comparison follows the original's allocator
  wherever a freed world sound was reused differently. Creating real
  zombies goes through `PopulationHost`, which waits on `IsoZombie` and
  `VirtualZombieManager`.

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).
- `pz-clipper6`: the polygon code of the native pathfinder (Clipper 6.4.2,
  earcut.hpp and the game's `PZClipper` wrapper) in pure Rust. Local
  differential run against the shipped arm64 library: 1.15 million random
  cases bit-identical.

- Character state machine (`pz-characters::character::state`, `states`,
  `combat`): `State`/`StateMachine`/`State.Param`/`StateMachineComponent`,
  the action system (`ActionGroup` from `media/actiongroups`,
  `ActionState`, `ActionTransition`, the variable/event/Lua conditions,
  `ActionContext`), the character's animation variables (game variables
  plus the `IsoGameCharacter`/`IsoLivingCharacter`/`IsoPlayer` callback
  slots, as `pz_anim::AnimationVariableSource`), anim-event routing
  (`OnAnimEvent`), `pz_pathfind`'s `PathMover`/`PlayerMover` for
  characters, the player states (`IdleState`, `SwipeStatePlayer`,
  `StaggerBackState`, knocked down, fall down, hit reactions, get up,
  collide, aim, strafe) and the melee combat helpers (`Hit`,
  `processHitDamage`, `hitConsequences`, crit chance, endurance, damage
  split), with Lua bindings (`bind_states`, `bind_combat`). A seeded run of
  the real `player`, `zombie` and `zombie-crawler` action groups (6000
  scripted variable/event steps plus every state's sorted transition list)
  and 48 weapon/perk/trait setups of the combat helpers (weapon type and
  mods, crit chance, `processHitDamage` on zombie and player targets,
  endurance loss, a full `Hit` on a player) match the original bit for bit
  (checked locally). Climbing, window, sitting and timed-action states are
  stand-ins; ranged combat and target bones are TODO.

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.
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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).
- `pz-clipper6`: the polygon code of the native pathfinder (Clipper 6.4.2,
  earcut.hpp and the game's `PZClipper` wrapper) in pure Rust. Local
  differential run against the shipped arm64 library: 1.15 million random
  cases bit-identical.

- Character state machine (`pz-characters::character::state`, `states`,
  `combat`): `State`/`StateMachine`/`State.Param`/`StateMachineComponent`,
  the action system (`ActionGroup` from `media/actiongroups`,
  `ActionState`, `ActionTransition`, the variable/event/Lua conditions,
  `ActionContext`), the character's animation variables (game variables
  plus the `IsoGameCharacter`/`IsoLivingCharacter`/`IsoPlayer` callback
  slots, as `pz_anim::AnimationVariableSource`), anim-event routing
  (`OnAnimEvent`), `pz_pathfind`'s `PathMover`/`PlayerMover` for
  characters, the player states (`IdleState`, `SwipeStatePlayer`,
  `StaggerBackState`, knocked down, fall down, hit reactions, get up,
  collide, aim, strafe) and the melee combat helpers (`Hit`,
  `processHitDamage`, `hitConsequences`, crit chance, endurance, damage
  split), with Lua bindings (`bind_states`, `bind_combat`). A seeded run of
  the real `player`, `zombie` and `zombie-crawler` action groups (6000
  scripted variable/event steps plus every state's sorted transition list)
  and 48 weapon/perk/trait setups of the combat helpers (weapon type and
  mods, crit chance, `processHitDamage` on zombie and player targets,
  endurance loss, a full `Hit` on a player) match the original bit for bit
  (checked locally). Climbing, window, sitting and timed-action states are
  stand-ins; ranged combat and target bones are TODO.
- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.

- Zombie population (`pz-popman`): a pure-Rust port of the native
  `PZPopMan` library (population cells, repopulation and redistribution,
  virtual zombies and their wall follower, world sounds, hordes, aggro
  targets, the collision grid and its path tasks, and the `zpop_*.bin` /
  `chunkdata_*.bin` save files), the single-player
  `ZombiePopulationManager` with `LoadedAreas`, `PlayerSpawns`,
  `RealZombieMap`, `ZombieStateFlags` and `ObjectPool`, and the population
  Lua globals (`bind_popman`). Four scripted runs of 600 to 12000 ticks
  match the shipped library line for line and save file for save file
  (checked locally). The comparison follows the original's allocator
  wherever a freed world sound was reused differently. Creating real
  zombies goes through `PopulationHost`, which waits on `IsoZombie` and
  `VirtualZombieManager`.

- Zombies (`pz-characters::character::zombie`, `states::zombie`):
  `IsoZombie`'s simulation (construction from a descriptor, the lore-driven
  `DoZombieStats`/speeds/`makeInactive`/`toggleCrawling`, sight
  (`spottedNew`/`spottedOld`, vision radius, cone, light, sneaking, traits,
  close-sneak-bonus sprites read from the jar), hearing through
  `pz_audio::world_sound` (`RespondToSound`, `setTurnAlertedValues`),
  memory and target selection, thumping, eating bodies, the corpse
  search, `updateInternal`/`postupdate`), the zombie animation variable
  slots, the zombie states (idle, walk toward, path find, lunge, attack
  with `AddRandomDamageFromZombie`, thump, fall down, get up, on ground,
  reanimate, eat body, face target, get down, sitting, crawler turn, fake
  dead, turn alerted, get up from crawl, falling), `initializeStates`,
  `pz_pathfind`'s `ZombieMover`, the `pz-popman` host's zombie side
  (`create_real_zombie_always` and the setters) and Lua bindings
  (`bind_zombie`: `IsoZombie` members, `getWorldSoundManager`, `addSound`).
  The world (squares, light, rooms, doors and thumpables, corpses, sounds,
  path behaviour) is reached through `StateWorld` hooks; lore settings are
  `SimContext` getters with the sandbox defaults. A seeded run against the
  original (600 lore-matrix `DoZombieStats`/`makeInactive`/`toggleCrawling`
  cases with the `Rand` stream after each, 2000 `setTurnAlertedValues` and
  500 vision-radius cases, 40 rounds of `WorldSoundManager` sounds with
  1200 zombie hearing queries and aging, plus 1280 lines on a cell with
  rooms), 3000 spotting checks (`spottedNew`/`spottedOld` on a real cell
  with light, rooms, sneaking, traits and distances), 392 bites and
  scratches on dressed players (`AttackState.triggerPlayerReaction` with
  clothing holes, helmet falls and the BodyDamage update) and 24000 ticks
  of the zombie state machine on the real action groups match bit for bit,
  with the `Rand` stream (checked locally). `IsoMovingObject.postupdate`'s
  collision passes (`DoCollide`, collision flags, the obstacle clip through
  pz-pathfind) and `doDeferredMovement` are ported with unit tests only.
  Lunges and paths (the original's path finder is native), ragdolls,
  vehicles and the network states are not checked against the original;
  stairs, slopes and `slideAwayFromWalls` are TODO.
  original loader locally. Enum and lookup tables are runtime data
  (`SpriteTables`, `ObjectTables`); the jar loader awaits `pz-classfile`.

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.
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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Zombie population (`pz-popman`): a pure-Rust port of the native
  `PZPopMan` library (population cells, repopulation and redistribution,
  virtual zombies and their wall follower, world sounds, hordes, aggro
  targets, the collision grid and its path tasks, and the `zpop_*.bin` /
  `chunkdata_*.bin` save files), the single-player
  `ZombiePopulationManager` with `LoadedAreas`, `PlayerSpawns`,
  `RealZombieMap`, `ZombieStateFlags` and `ObjectPool`, and the population
  Lua globals (`bind_popman`). Four scripted runs of 600 to 12000 ticks
  match the shipped library line for line and save file for save file
  (checked locally). The comparison follows the original's allocator
  wherever a freed world sound was reused differently. Creating real
  zombies goes through `PopulationHost`, which waits on `IsoZombie` and
  `VirtualZombieManager`.

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Appearance tables and outfit draws (`pz-characters::population`):
  `ClothingDecals`, `HairStyles`, `BeardStyles`, `VoiceStyles` (JAXB
  binding rules, mod overrides), `PopTemplateManager` skin lists,
  `HairOutfitDefinitions`, `OutfitRNG`/`LocationRNG` (`pz-core`),
  `OutfitManager`/`Outfit`/`ClothingItemReference` with `Randomize`, the
  random outfit picks and `dressInOutfit`'s clothing choices,
  `DefaultClothing`, and a `HumanStyles` for `HumanVisual`. Lua bindings
  (`bind_appearance`). Loaded lists and seeded draws match the original on
  the installed media (every outfit randomized and dressed on three seeds)
  and on invented edge-case XML (checked locally).

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.

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

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup. The
  Studio object model (events, parameters, timelines, instruments, buses)
  parses from every game bank; event lengths, distances and parameter
  descriptions match the FMOD runtime for all 3,490 events but one unused
  legacy event (see `crates/pz-audio/FORMAT.md`).

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup. The
  Studio object model (events, parameters, timelines, instruments, buses)
  parses from every game bank; event lengths, distances and parameter
  descriptions match the FMOD runtime for all 3,490 events but one unused
  legacy event (see `crates/pz-audio/FORMAT.md`).

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Appearance tables and outfit draws (`pz-characters::population`):
  `ClothingDecals`, `HairStyles`, `BeardStyles`, `VoiceStyles` (JAXB
  binding rules, mod overrides), `PopTemplateManager` skin lists,
  `HairOutfitDefinitions`, `OutfitRNG`/`LocationRNG` (`pz-core`),
  `OutfitManager`/`Outfit`/`ClothingItemReference` with `Randomize`, the
  random outfit picks and `dressInOutfit`'s clothing choices,
  `DefaultClothing`, and a `HumanStyles` for `HumanVisual`. Lua bindings
  (`bind_appearance`). Loaded lists and seeded draws match the original on
  the installed media (every outfit randomized and dressed on three seeds)
  and on invented edge-case XML (checked locally).

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Appearance tables and outfit draws (`pz-characters::population`):
  `ClothingDecals`, `HairStyles`, `BeardStyles`, `VoiceStyles` (JAXB
  binding rules, mod overrides), `PopTemplateManager` skin lists,
  `HairOutfitDefinitions`, `OutfitRNG`/`LocationRNG` (`pz-core`),
  `OutfitManager`/`Outfit`/`ClothingItemReference` with `Randomize`, the
  random outfit picks and `dressInOutfit`'s clothing choices,
  `DefaultClothing`, and a `HumanStyles` for `HumanVisual`. Lua bindings
  (`bind_appearance`). Loaded lists and seeded draws match the original on
  the installed media (every outfit randomized and dressed on three seeds)
  and on invented edge-case XML (checked locally).
  pz-pathfind), `slideAwayFromWalls` (square edges here, vehicles through
  pz-pathfind's `resolve_collision_radius`) and `doDeferredMovement` are
  ported with unit tests only. Characters burn through pz-world's fire
  (`FireCharacter`, the `FireCheck`/`SpreadFire`/`ReduceHealthWhenBurning`
  step after the state machine): 60 `SetOnFire` runs with 16361 ticks of
  burning damage on players and zombies match the original with the
  `Rand` stream. A zombie chasing a player on real westpoint chunks (40
  runs of 400 ticks: path requests answered by the original's native
  library on one side and pz-pathfind's `PathHost` on the other, path
  following, walk toward, lunge, face target and attack, collisions and
  head sliding against the real squares) matches the original bit for bit
  with the `Rand` stream in 35 runs; the other 5 match until the zombie
  reaches a door or a fence (thumping and fence climbing need the world's
  mover-aware `testCollideAdjacent` and hoppable checks). Ragdolls,
  stairs and slopes are TODO.

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).
- `pz-clipper6`: the polygon code of the native pathfinder (Clipper 6.4.2,
  earcut.hpp and the game's `PZClipper` wrapper) in pure Rust. Local
  differential run against the shipped arm64 library: 1.15 million random
  cases bit-identical.

- Character state machine (`pz-characters::character::state`, `states`,
  `combat`): `State`/`StateMachine`/`State.Param`/`StateMachineComponent`,
  the action system (`ActionGroup` from `media/actiongroups`,
  `ActionState`, `ActionTransition`, the variable/event/Lua conditions,
  `ActionContext`), the character's animation variables (game variables
  plus the `IsoGameCharacter`/`IsoLivingCharacter`/`IsoPlayer` callback
  slots, as `pz_anim::AnimationVariableSource`), anim-event routing
  (`OnAnimEvent`), `pz_pathfind`'s `PathMover`/`PlayerMover` for
  characters, the player states (`IdleState`, `SwipeStatePlayer`,
  `StaggerBackState`, knocked down, fall down, hit reactions, get up,
  collide, aim, strafe) and the melee combat helpers (`Hit`,
  `processHitDamage`, `hitConsequences`, crit chance, endurance, damage
  split), with Lua bindings (`bind_states`, `bind_combat`). A seeded run of
  the real `player`, `zombie` and `zombie-crawler` action groups (6000
  scripted variable/event steps plus every state's sorted transition list)
  and 48 weapon/perk/trait setups of the combat helpers (weapon type and
  mods, crit chance, `processHitDamage` on zombie and player targets,
  endurance loss, a full `Hit` on a player) match the original bit for bit
  (checked locally). Climbing, window, sitting and timed-action states are
  stand-ins; ranged combat and target bones are TODO.
- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.

- Zombie population (`pz-popman`): a pure-Rust port of the native
  `PZPopMan` library (population cells, repopulation and redistribution,
  virtual zombies and their wall follower, world sounds, hordes, aggro
  targets, the collision grid and its path tasks, and the `zpop_*.bin` /
  `chunkdata_*.bin` save files), the single-player
  `ZombiePopulationManager` with `LoadedAreas`, `PlayerSpawns`,
  `RealZombieMap`, `ZombieStateFlags` and `ObjectPool`, and the population
  Lua globals (`bind_popman`). Four scripted runs of 600 to 12000 ticks
  match the shipped library line for line and save file for save file
  (checked locally). The comparison follows the original's allocator
  wherever a freed world sound was reused differently. Creating real
  zombies goes through `PopulationHost`, which waits on `IsoZombie` and
  `VirtualZombieManager`.

- Zombies (`pz-characters::character::zombie`, `states::zombie`):
  `IsoZombie`'s simulation (construction from a descriptor, the lore-driven
  `DoZombieStats`/speeds/`makeInactive`/`toggleCrawling`, sight
  (`spottedNew`/`spottedOld`, vision radius, cone, light, sneaking, traits,
  close-sneak-bonus sprites read from the jar), hearing through
  `pz_audio::world_sound` (`RespondToSound`, `setTurnAlertedValues`),
  memory and target selection, thumping, eating bodies, the corpse
  search, `updateInternal`/`postupdate`), the zombie animation variable
  slots, the zombie states (idle, walk toward, path find, lunge, attack
  with `AddRandomDamageFromZombie`, thump, fall down, get up, on ground,
  reanimate, eat body, face target, get down, sitting, crawler turn, fake
  dead, turn alerted, get up from crawl, falling), `initializeStates`,
  `pz_pathfind`'s `ZombieMover`, the `pz-popman` host's zombie side
  (`create_real_zombie_always` and the setters) and Lua bindings
  (`bind_zombie`: `IsoZombie` members, `getWorldSoundManager`, `addSound`).
  The world (squares, light, rooms, doors and thumpables, corpses, sounds,
  path behaviour) is reached through `StateWorld` hooks; lore settings are
  `SimContext` getters with the sandbox defaults. A seeded run against the
  original (600 lore-matrix `DoZombieStats`/`makeInactive`/`toggleCrawling`
  cases with the `Rand` stream after each, 2000 `setTurnAlertedValues` and
  500 vision-radius cases, 40 rounds of `WorldSoundManager` sounds with
  1200 zombie hearing queries and aging) matches bit for bit (checked
  locally). Spotting, the zombie states' world interplay, ragdolls,
  vehicles and the network states are not yet checked against the
  original; the movement code (`IsoGameCharacter.update`) is TODO.

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Appearance tables and outfit draws (`pz-characters::population`):
  `ClothingDecals`, `HairStyles`, `BeardStyles`, `VoiceStyles` (JAXB
  binding rules, mod overrides), `PopTemplateManager` skin lists,
  `HairOutfitDefinitions`, `OutfitRNG`/`LocationRNG` (`pz-core`),
  `OutfitManager`/`Outfit`/`ClothingItemReference` with `Randomize`, the
  random outfit picks and `dressInOutfit`'s clothing choices,
  `DefaultClothing`, and a `HumanStyles` for `HumanVisual`. Lua bindings
  (`bind_appearance`). Loaded lists and seeded draws match the original on
  the installed media (every outfit randomized and dressed on three seeds)
  and on invented edge-case XML (checked locally).

- Vehicles in the iso world (`pz-vehicle-render`, vehicle shaders in
  `pz-gpu`): `VehicleModelInstance` (skin textures; `updateLights`,
  `doDamageOverlay`, `doBloodOverlay` enable matrices),
  `ModelManager.addVehicle`'s body and part models (legacy `.txt` wheels
  included) with `playPartAnim`/`updateAnimationPlayer` door and hood
  poses, `BaseVehicle.updateTransform` (wheel suspension, steering and
  spin; attached part models), `ModelSlotRenderData`'s vehicle branch and
  `Model.DrawVehicle` as `DrawCmd::VehicleModel` (WGSL ports of `vehicle`,
  `vehicle_multiuv`, `vehicle_norandom_multiuv` with and without
  `_noreflect`, and `vehiclewheel`), `renderShadow` into
  `FBORenderShadows`, vehicles in `renderMovingObjects`, and headlight
  torches / lightbar lampposts for the lighting system. On SportsCar_ez,
  CarNormal, PickUpTruck and Van the render, vehicle and part-model
  transforms, all twelve enable matrices, rust/reflection/paint values,
  the vehicle camera, centre-of-mass offset and `targetDepth`, the shadow
  polygon and the headlight torches match the original's bit for bit
  (131 values, local harness). Headlights reach pz-lighting as torches
  (`checkLights`' vehicle loop) and the lightbar's alternating lights as
  cell lampposts (`updateWorldLights`); on a night floor the squares in a
  car's beams light up and the lightbar lamppost comes and goes (tests
  need the game's scripts). Not yet: the sky box reflection textures
  (the reflective programs run with whatever textures are passed), the
  square's result lights on the vehicle model, sloped-surface shadow
  height, `renderToTexture`.

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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).

- Appearance tables and outfit draws (`pz-characters::population`):
  `ClothingDecals`, `HairStyles`, `BeardStyles`, `VoiceStyles` (JAXB
  binding rules, mod overrides), `PopTemplateManager` skin lists,
  `HairOutfitDefinitions`, `OutfitRNG`/`LocationRNG` (`pz-core`),
  `OutfitManager`/`Outfit`/`ClothingItemReference` with `Randomize`, the
  random outfit picks and `dressInOutfit`'s clothing choices,
  `DefaultClothing`, and a `HumanStyles` for `HumanVisual`. Lua bindings
  (`bind_appearance`). Loaded lists and seeded draws match the original on
  the installed media (every outfit randomized and dressed on three seeds)
  and on invented edge-case XML (checked locally).

- Vehicles in the iso world (`pz-vehicle-render`, vehicle shaders in
  `pz-gpu`): `VehicleModelInstance` (skin textures; `updateLights`,
  `doDamageOverlay`, `doBloodOverlay` enable matrices),
  `ModelManager.addVehicle`'s body and part models (legacy `.txt` wheels
  included) with `playPartAnim`/`updateAnimationPlayer` door and hood
  poses, `BaseVehicle.updateTransform` (wheel suspension, steering and
  spin; attached part models), `ModelSlotRenderData`'s vehicle branch and
  `Model.DrawVehicle` as `DrawCmd::VehicleModel` (WGSL ports of `vehicle`,
  `vehicle_multiuv`, `vehicle_norandom_multiuv` with and without
  `_noreflect`, and `vehiclewheel`), `renderShadow` into
  `FBORenderShadows`, vehicles in `renderMovingObjects`, and headlight
  torches / lightbar lampposts for the lighting system. On SportsCar_ez,
  CarNormal, PickUpTruck and Van the render, vehicle and part-model
  transforms, all twelve enable matrices, rust/reflection/paint values,
  the vehicle camera, centre-of-mass offset and `targetDepth`, the shadow
  polygon and the headlight torches match the original's bit for bit
  (131 values, local harness). Headlights reach pz-lighting as torches
  (`checkLights`' vehicle loop) and the lightbar's alternating lights as
  cell lampposts (`updateWorldLights`); on a night floor the squares in a
  car's beams light up and the lightbar lamppost comes and goes (tests
  need the game's scripts). Not yet: the sky box reflection textures
  (the reflective programs run with whatever textures are passed), the
  square's result lights on the vehicle model, sloped-surface shadow
  height, `renderToTexture`.

- Character outlines and item textures: `ModelOutlines` (solid mask with
  `aim_outline_solid`, `aim_outline_h`/`_v` thickening, masked
  `aim_outline_blit`, the player masking outlines behind it) in the iso
  frame's object pass (vehicles, as in build 42, take none), and
  `ItemSmartTexture` for held items (`ModelInstanceTextureInitializer`/`EquippedTextureCreator`: tint and
  `TINT` masks, weapon blood, fluid) and for placed items in the
  `WorldItemAtlas` (`_Fluid` models, `FLUIDTINT` overlays) through the GPU
  texture combiner. The outline state machine and passes (buffers,
  viewports, uniforms, quads at nine zooms) and the item combine command
  lists match the original's (local harnesses). Not yet: sprite-object
  outlines (`FBORenderObjectOutline`, `vboRenderer_SpriteOutline`) and
  `IsoObjectModelDrawer`'s sprite-model outlines; split-screen viewports.

- One world frame for both render stacks (ws/render-join):
  `frame::world_frame_full` draws `FBORenderCell.performRenderTiles`'
  order after the chunk composite (players, corpse and mannequin shadows,
  corpses, items; puddles; moving objects; water; fireplace and barbecue
  flames; per-level translucent floors, puddles, shore, rain splashes,
  shadows and objects), then `IsoWorld.render`'s weather pass, with a test
  asserting the order. Not yet in the frame: `renderFlies`,
  `FBORenderObjectHighlight`, the cell's `renderFog`,
  `VisibilityPolygon2`, markers, `FBORenderObjectOutline`.

- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.
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
  the original bit for bit (checked locally). Food, pills, literature,
  weapons, the worn clothing (defense, spikes, dirt/blood, insulation,
  `ClothingWetness`), hand items (tissues, no-sprint) and the inventory
  weight are `pz-inventory` items on the character; the zombie side of an
  attack is still plain data. Eating, drinking, pills, clothing defense
  queries, inventory weight and weapon hits on real game items match the
  original's state bit for bit (checked locally).
- `pz-clipper6`: the polygon code of the native pathfinder (Clipper 6.4.2,
  earcut.hpp and the game's `PZClipper` wrapper) in pure Rust. Local
  differential run against the shipped arm64 library: 1.15 million random
  cases bit-identical.

- Character state machine (`pz-characters::character::state`, `states`,
  `combat`): `State`/`StateMachine`/`State.Param`/`StateMachineComponent`,
  the action system (`ActionGroup` from `media/actiongroups`,
  `ActionState`, `ActionTransition`, the variable/event/Lua conditions,
  `ActionContext`), the character's animation variables (game variables
  plus the `IsoGameCharacter`/`IsoLivingCharacter`/`IsoPlayer` callback
  slots, as `pz_anim::AnimationVariableSource`), anim-event routing
  (`OnAnimEvent`), `pz_pathfind`'s `PathMover`/`PlayerMover` for
  characters, the player states (`IdleState`, `SwipeStatePlayer`,
  `StaggerBackState`, knocked down, fall down, hit reactions, get up,
  collide, aim, strafe) and the melee combat helpers (`Hit`,
  `processHitDamage`, `hitConsequences`, crit chance, endurance, damage
  split), with Lua bindings (`bind_states`, `bind_combat`). A seeded run of
  the real `player`, `zombie` and `zombie-crawler` action groups (6000
  scripted variable/event steps plus every state's sorted transition list)
  and 48 weapon/perk/trait setups of the combat helpers (weapon type and
  mods, crit chance, `processHitDamage` on zombie and player targets,
  endurance loss, a full `Hit` on a player) match the original bit for bit
  (checked locally). Climbing, window, sitting and timed-action states are
  stand-ins; ranged combat and target bones are TODO.
- Audio: FSB5 sample banks inside the game's FMOD Studio banks are parsed
  and their Vorbis samples decoded in Rust (`pz-audio`). FSB5 omits the
  Vorbis setup headers; they are rebuilt at runtime from the FMOD library
  in the user's install and CRC-verified (all 161 in Build 42.21). Decoded
  output matches libvorbis (via ffmpeg) to within 5e-7. The strings bank
  table (paths to GUIDs) parses identically to FMOD's own lookup.

- Zombie population (`pz-popman`): a pure-Rust port of the native
  `PZPopMan` library (population cells, repopulation and redistribution,
  virtual zombies and their wall follower, world sounds, hordes, aggro
  targets, the collision grid and its path tasks, and the `zpop_*.bin` /
  `chunkdata_*.bin` save files), the single-player
  `ZombiePopulationManager` with `LoadedAreas`, `PlayerSpawns`,
  `RealZombieMap`, `ZombieStateFlags` and `ObjectPool`, and the population
  Lua globals (`bind_popman`). Four scripted runs of 600 to 12000 ticks
  match the shipped library line for line and save file for save file
  (checked locally). The comparison follows the original's allocator
  wherever a freed world sound was reused differently. Creating real
  zombies goes through `PopulationHost`, which waits on `IsoZombie` and
  `VirtualZombieManager`.

- Zombies (`pz-characters::character::zombie`, `states::zombie`):
  `IsoZombie`'s simulation (construction from a descriptor, the lore-driven
  `DoZombieStats`/speeds/`makeInactive`/`toggleCrawling`, sight
  (`spottedNew`/`spottedOld`, vision radius, cone, light, sneaking, traits,
  close-sneak-bonus sprites read from the jar), hearing through
  `pz_audio::world_sound` (`RespondToSound`, `setTurnAlertedValues`),
  memory and target selection, thumping, eating bodies, the corpse
  search, `updateInternal`/`postupdate`), the zombie animation variable
  slots, the zombie states (idle, walk toward, path find, lunge, attack
  with `AddRandomDamageFromZombie`, thump, fall down, get up, on ground,
  reanimate, eat body, face target, get down, sitting, crawler turn, fake
  dead, turn alerted, get up from crawl, falling), `initializeStates`,
  `pz_pathfind`'s `ZombieMover`, the `pz-popman` host's zombie side
  (`create_real_zombie_always` and the setters) and Lua bindings
  (`bind_zombie`: `IsoZombie` members, `getWorldSoundManager`, `addSound`).
  The world (squares, light, rooms, doors and thumpables, corpses, sounds,
  path behaviour) is reached through `StateWorld` hooks; lore settings are
  `SimContext` getters with the sandbox defaults. A seeded run against the
  original (600 lore-matrix `DoZombieStats`/`makeInactive`/`toggleCrawling`
  cases with the `Rand` stream after each, 2000 `setTurnAlertedValues` and
  500 vision-radius cases, 40 rounds of `WorldSoundManager` sounds with
  1200 zombie hearing queries and aging, plus 1280 lines on a cell with
  rooms), 3000 spotting checks (`spottedNew`/`spottedOld` on a real cell
  with light, rooms, sneaking, traits and distances), 392 bites and
  scratches on dressed players (`AttackState.triggerPlayerReaction` with
  clothing holes, helmet falls and the BodyDamage update) and 24000 ticks
  of the zombie state machine on the real action groups match bit for bit,
  with the `Rand` stream (checked locally). `IsoMovingObject.postupdate`'s
  collision passes (`DoCollide`, collision flags, the obstacle clip through
  pz-pathfind), `slideAwayFromWalls` (square edges here, vehicles through
  pz-pathfind's `resolve_collision_radius`) and `doDeferredMovement` are
  ported with unit tests only. Characters burn through pz-world's fire
  (`FireCharacter`, the `FireCheck`/`SpreadFire`/`ReduceHealthWhenBurning`
  step after the state machine): 60 `SetOnFire` runs with 16361 ticks of
  burning damage on players and zombies match the original with the
  `Rand` stream. A zombie chasing a player on real westpoint chunks (40
  runs of 400 ticks: path requests answered by the original's native
  library on one side and pz-pathfind's `PathHost` on the other, path
  following, walk toward, lunge, face target and attack, collisions and
  head sliding against the real squares) matches the original bit for bit
  with the `Rand` stream in all 40 runs (16000 ticks), thumping doors and
  climbing fences included (mover-aware `testCollideAdjacent` from
  pz-world, `IsoZombie.collideWith`, the hoppable checks and
  `ClimbOverFenceState`). Ragdolls,
  vehicles and the network states are not checked against the original;
  stairs and slopes are TODO.

## In progress
- UIElement/UIManager, Core options, client window loop.

## Next (main-menu milestone)
1. Kahlua stdlib; load `stdlib.lbc`/`stdlib.lua` from the install.
2. `LuaManager.init` + `LoadDirBase("shared")`, `("client")` with API stubs.
3. Fonts, translations, textures, UIElement/UIManager.
4. `MainScreenState` running `MainScreen.lua` in the client.
