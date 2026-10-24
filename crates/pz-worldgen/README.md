# pz-worldgen

Build 42 procedural world generation (`zombie.iso.worldgen`), run when a
chunk without a save file is loaded (`IsoChunk.LoadBrandNew`). It plugs into
`pz_world::chunk::load_brand_new` through `WorldGenHook`, which implements
`pz_world::chunk::BrandNewHook`.

## What the original does

`LoadBrandNew` loads the 2×2 block of chunks containing the requested one
from the lotpacks and calls `WorldGenChunk.generateChunks` on the block
(all four chunks are generated; one is kept). A shared 16×16 `toBeDone` grid
carries the parts of multi-square features (jumbo trees, boulder groups)
over to the squares they cover.

* **Map chunks** (`genMapChunk`): the cell's `biomemap_X_Y.png` gives each
  square a pixel value; `biome_map_config` maps it to a biome name prefix
  (and an ore prefix). The concrete biome is picked from the matching
  `worldgen.biomes_map` entries by the bush and ore noise and a selector
  noise. Per square: tile replacements (clay floors near water, with the
  `ClayLakeChance`/`ClayRiverChance` sandbox odds), then the lotpack's
  vegetation placeholders (an `IsoTree`, a bush, a grass tile) are replaced
  by the biome's PLANT/BUSH/TREE features where the floor matches the
  biome's placement patterns and the tile isn't protected; bare squares
  (floor plus grass) may get an ORE feature (boulders, flint, limestone)
  unless a `WorldGen` map zone says `Rocks = false`. `cleanChunk` then
  removes `vegetation_groundcover_01` from sand and road floors. Pixel
  values whose biome is `$random` would be generated like the procedural
  area (none in the vanilla map).
* **Procedural chunks** (any empty level-0 square, i.e. the ±250-cell area
  around the map; `genRandomChunk`): ground, plants, bushes, trees and ore
  from the `worldgen.biomes` biome selected by six simplex noises
  (landscape, plant, bush, temperature, hygrometry, ore level) and the
  `worldgen.selection` ranges; dirt roads (a Delaunay graph of per-cell
  random points turned into L-shaped roads); ore veins (iron, copper) of the
  cells around the cell origin; and the map's static modules from
  `WorldGenOverride.lua` (water strips, prefab highway and road).
* Squares that roll the biome's zombie chance are recorded as
  `proceduralZombieSquares` (`WorldGen::procedural_zombie_squares`).

Blending and attachments (`Blending.applyBlending`,
`AttachmentsHandler.applyAttachments`) are not part of loading: they run
later from `IsoChunk.update()`. This crate sets the chunk flags they read
but does not port them. Generated procedural chunks get the `WORLDGEN`
generation status, as in the original. Foraging zones
(`ZoneGenerator.genForaging`) are not ported.

## Procedural basements

`WorldGenHook::on_new_chunk_loaded` runs `Basements.onNewChunkLoaded`
(module `basements`, `.pzby` reader in `pzby`). `load_basements` does the
new-world part of `Basements.beforeLoadMetaGrid`: the definitions and spawn
locations of each map's `basements.lua` (through a recorded
`Basements.getAPIv1()`), the `Basement` zones of `objects.lua`, the
`.pzby` headers of `media/binmap` and `media/basement_access`, and the
placements (`calculateBasementPlacements`: per spawn location, a random
fitting definition from a `Random(worldSeed)`, the overlap test with its
original quirk that any intersecting bounds overlap, and the location's
access piece). When a chunk is loaded, every placement within one square
of it is stamped in (`SpawnBasementInChunk` / `addToIsoChunk`: new squares
below ground, the keep-floors/walls/other attribute bits, tile creation,
the floor removed over the new stairs); header loads draw `Rand.Next(2)`
per room like the original.

Not ported: `map_basements.bin` (saving and reloading placements), and
adding the basements' buildings and rooms to the meta grid
(`createBasementBuildingDefs`, `addBasementBuildingDefsToLotHeaders` with
merging into an adjacent building, `addBasementBuildingDefsToMetaGrid`).
Squares below ground therefore have no room (`room_id` -1) and no
`IsoBuilding` is created for them. That needs `pz_world::IsoMetaGrid` to
let another crate add a building to a cell: a public
`get_cell_data_mut`, or better an `add_building(cell_x, cell_y, rooms,
emptyoutside)` that assigns `RoomID`/`BuildingID`/meta ids the way
`mergeRoomsOntoMetaCell` does, adds the rooms to the cell's meta chunks
(`IsoMetaCell.addRooms`, the private `add_room_to_cell`) and to adjacent
cells, and pushes the building to `buildings`; plus, for merging,
`getBuildingsIntersecting`, `BuildingDef.isAdjacent`/`addRoomsOf` and
removing a building from a cell with `recalculateBuildingAndRoomIDs`.

## Runtime data

Nothing game-specific is in the code:

* the Lua definitions: `media/lua/server/WorldGen/**` (features, biomes,
  subbiomes, selection, veins, roads, priorities, prefabs), run with
  `pz-kahlua` in the game's order (`WorldGen/WorldGen` first, whose
  `WorldGenUtils.getFiles` walks happen in directory-listing order), then
  `metazones/BiomeMapConfig.lua` and each map's `WorldGenOverride.lua`;
* `media/maps/<map>/maps/biomemap_*.png` and the `WorldGen` rectangles of
  `media/maps/<map>/objects.lua`;
* from `projectzomboid.jar` (via `pz-classfile`): the `SimplexNoise`
  permutation and gradient tables, the two fixed road points of
  `RoadGenerator.genRoadDelaunayPoints`, and the `SandboxOptions` double
  defaults (callers pass the save's sandbox values instead when they have
  them).

Java behaviour reproduced: `java.util.Random` (including the JDK 17
`nextInt(origin, bound)`), `String.hashCode`, `HashMap` iteration order of
the biome, vein and road maps (it decides which biome a selector value
picks and the order veins draw), float/double arithmetic as written, the
wildcard regexes (`"^" + p`, `.`, `*`, `?`), and the original's quirks (the
in-chunk coordinates `checkFutureSquares` looks up relative to the block's
first chunk, index skipping while deleting objects, `int` overflow in
`Vector2i.lengthSquared`).

Not reproducible: the original keeps road points, Delaunay edges and roads
in `HashSet`s of identity-hashed objects, so when two roads cross the same
square the order in which they draw from the square's generator changes
from run to run of the game. This port uses a fixed order.

## Parity

`examples/worldgen_replay.rs` loads chunks through
`load_brand_new_cached` + `WorldGenHook` (seed `pzrs-pathfind-oracle`,
`Rand` reseeded per chunk with `wx << 32 ^ wy`, regions plus a one-chunk
margin in row-major order, then `recalc_chunk_squares` and
`load_in_main_thread`) and compares every square's object list (class and
sprite name, in order) with a local dump of the game's own
`IsoChunk.LoadChunk` on an empty save. Results on build 42:

| region | where | squares | identical |
|---|---|---|---|
| muldraugh | town and forest | 10,752 | 10,752 |
| westpoint | downtown, basements | 30,016 | 30,016 |
| riverside | town, river, `Rocks = false` zones, clay | 17,536 | 17,536 |
| louisville | high-rises | 38,592 | 38,592 |
| forest | PR forest | 4,160 | 4,160 |
| clay_shore, clay_lake | clay replacements | 4,608 | 4,608 |
| trailer, farms, ph_forest, farmmix | trailer park, farm, farmland, farm forest, birch forest, PH forest, farm-mix forest biomes | 12,608 | 12,608 |
| proc_forest | procedural biomes | 2,304 | 2,304 |
| proc_water | static module (water) | 2,304 | 2,304 |
| proc_highway | static module (prefab highway) | 2,304 | 2,304 |
| proc_road | prefab road, dirt roads, map edge | 3,072 | 3,072 |
| proc_vein | iron/copper vein (62 ore squares) | 2,304 | 2,304 |
| bsm_a … bsm_g | 7 town areas with 24 placed basements and 13 access pieces (exterior, interior and house types) | 36,672 | 36,672 |

All 167,232 squares match. Without world generation (lotpacks only) 7,025
of the first five regions' 101,056 squares differ; without basements 1,109
of the bsm regions' squares differ. All 785 basement placements match the
game's in order. Every vanilla basement is one level deep (z = -1), so
deeper levels are not exercised. Rooms differ as described above: 926
basement squares are in a room in the game and in none here. (2,303
westpoint squares also differ in room, on both oracles and unrelated to
basements: a `pz-world` room-assignment gap, e.g. 11880,6912,0.)
