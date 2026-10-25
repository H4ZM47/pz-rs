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
- Climate and clock (`pz-core::weather`, `GameTime`): `ClimateManager`
  with its float/colour/bool layers, `ClimateValues`, `WeatherPeriod`,
  `ThunderStorm`, `ClimateForecaster`, `ClimateMoon`, `ErosionSeason`,
  `SimplexNoise` and `GameTime.update`, bound to Lua (`bind_climate`) and
  feeding the character simulation's air temperature, humidity, rain and
  wind chill. Hourly and daily dumps match the original bit for bit over
  more than 400 simulated days across seasons, sandbox weather cycles and
  daylight-saving changes (checked locally).
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
  (131 values, local harness). Not yet: the sky box reflection textures
  (the reflective programs run with whatever textures are passed),
  headlight light on the ground (needs the lighting pass), sloped-surface
  shadow height, `renderToTexture`.

- Character outlines and item textures: `ModelOutlines` (solid mask with
  `aim_outline_solid`, `aim_outline_h`/`_v` thickening, masked
  `aim_outline_blit`, the player masking outlines behind it) in the iso
  frame's object pass, and `ItemSmartTexture` for held items
  (`ModelInstanceTextureInitializer`/`EquippedTextureCreator`: tint and
  `TINT` masks, weapon blood, fluid) and for placed items in the
  `WorldItemAtlas` (`_Fluid` models, `FLUIDTINT` overlays) through the GPU
  texture combiner. The outline state machine and passes (buffers,
  viewports, uniforms, quads at nine zooms) and the item combine command
  lists match the original's (local harnesses). Not yet: sprite-object
  outlines (`FBORenderObjectOutline`, `vboRenderer_SpriteOutline`) and
  `IsoObjectModelDrawer`'s sprite-model outlines; split-screen viewports.

## In progress
- UIElement/UIManager, Core options, client window loop.

## Next (main-menu milestone)
1. Kahlua stdlib; load `stdlib.lbc`/`stdlib.lua` from the install.
2. `LuaManager.init` + `LoadDirBase("shared")`, `("client")` with API stubs.
3. Fonts, translations, textures, UIElement/UIManager.
4. `MainScreenState` running `MainScreen.lua` in the client.
