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
  New items carry the vehicle-part values, `ItemCapacity`/`MaxCapacity`,
  radio `DeviceData` with presets and channel, and random map ids, with
  the original's `Rand` draws (channel lists and map definitions are
  runtime input); weights count fluid contents. Of 5,092 created script
  items, all match the original's draws, ids and vehicle values and 5,081
  every compared field (11 moveables differ in sprite-derived weight), and
  the part items of 242 `vehicles.db` vehicles match the original's
  weights and totals. Checked locally.
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
  dump field and Bullet call): 241 of 243 saved `vehicles.db` vehicles are
  bit-exact, one matches up to the original's own crash (a broken window
  with no square) and one holds a trailer animal the original cannot run
  headless; all 241 vanilla vehicle scripts as new vehicles are
  bit-exact; 3,000 single-zone spawns and 2,000 `AddVehicles` runs on
  synthetic zone lists give the original's vehicles and `Rand` state.
  End to end on pz-bullet (`VehiclePhysics` for `PzBullet`, with the
  natives' exceptions and step callbacks), against the original running
  the real libPZBullet on a Muldraugh region's chunk shapes: 240 of 241
  vehicle scripts are bit-exact for 400 driven frames (the other matches
  until the harness lacks a world-sound native). `IsoChunk
  .updatePhysicsForLevel` gives the original's `ToBullet` shapes for every
  one of 114,529 squares of four regions that load the same objects.
  Checked locally.
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

## In progress
- UIElement/UIManager, Core options, client window loop.

## Next (main-menu milestone)
1. Kahlua stdlib; load `stdlib.lbc`/`stdlib.lua` from the install.
2. `LuaManager.init` + `LoadDirBase("shared")`, `("client")` with API stubs.
3. Fonts, translations, textures, UIElement/UIManager.
4. `MainScreenState` running `MainScreen.lua` in the client.
