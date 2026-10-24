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
  match the game on all 130,560 squares of 17 regions, checked locally.
  Blending/attachments (chunk update), foraging zones and procedural
  basements are not ported.

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
