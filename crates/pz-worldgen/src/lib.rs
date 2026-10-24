//! Build 42 procedural world generation (`zombie.iso.worldgen`).
//!
//! When a chunk is loaded with no save file, `IsoChunk.LoadBrandNew` loads
//! the 2×2 block of chunks around it from the lotpacks and runs
//! `WorldGenChunk.generateChunks` over the block ([`WorldGenHook`]
//! implements `pz_world`'s `BrandNewHook` for it):
//!
//! * map chunks: per square, the biome named by the cell's
//!   `biomemap_X_Y.png` (through `biome_map_config`) replaces the lotpack's
//!   vegetation placeholders (trees, bushes, grass) with its features,
//!   possibly spanning several squares (jumbo trees, boulder groups), adds
//!   ore boulders on bare ground (unless a `WorldGen` map zone has
//!   `Rocks = false`), applies tile replacements (clay floors), and clears
//!   ground cover from sand and road floors; squares whose biome is
//!   `$random` are generated like the procedural area;
//! * chunks with empty level-0 squares (the procedural area around the
//!   map): ground, vegetation and ore from noise-selected biomes, dirt
//!   roads, ore veins, and the map's static modules (water, prefab roads).
//!
//! Everything the generator reads comes from the installed game at runtime:
//! the Lua `worldgen` tables (`media/lua/server/WorldGen`), the biome maps
//! and `objects.lua` zones of `media/maps`, each map's
//! `WorldGenOverride.lua`, and constants of `projectzomboid.jar` (simplex
//! tables, road points, sandbox defaults).

pub mod basements;
pub mod biome_map;
pub mod defs;
pub mod gen;
pub mod jar_consts;
pub mod java;
pub mod lua;
pub mod params;
pub mod pzby;
pub mod roads;
pub mod simplex;
pub mod veins;
pub mod zones;

use std::rc::Rc;

use pz_classfile::Jar;
use pz_fs::GameFs;
use pz_world::chunk::{BrandNewHook, ChunkLoadEnv, ChunksCache, IsoChunk};
use pz_world::IsoMetaGrid;

pub use gen::WorldGen;
pub use params::WorldGenParams;

use defs::{Reader, SandboxLookup, AXES};

/// What [`load`] needs besides the game files.
pub struct WorldGenSetup<'a> {
    /// The installed game directory (holding `media/`).
    pub fs: Rc<GameFs>,
    pub jar: &'a Jar,
    /// `WorldGenParams.seedString`.
    pub seed: &'a str,
    /// `IsoWorld.getMap()` split on `;` (the lot directories, in order).
    pub maps: &'a [&'a str],
    /// The meta grid's cell bounds `(min_x, min_y, max_x, max_y)` after
    /// `CreateStep1` (map bounds widened to the world-generation bounds).
    pub cell_bounds: (i32, i32, i32, i32),
    /// Double sandbox options by name (for `Sandbox.*` probabilities).
    pub sandbox: SandboxLookup<'a>,
}

/// `new WorldGenChunk(seed)`, `new BiomeMap()` and the `WorldGen` map zones:
/// runs the world-generation Lua and reads every definition.
pub fn load(setup: &WorldGenSetup<'_>) -> Result<WorldGen, String> {
    let params = WorldGenParams::new(setup.seed);
    let tables = simplex::SimplexTables::from_jar(setup.jar)?;
    let simplex = simplex::WorldGenSimplexGenerator::new(tables, params.seed());
    let mut lua = lua::WorldGenLua::new(setup.fs.clone()).map_err(|e| e.to_string())?;
    lua.load_world_gen(setup.maps);
    let errors = lua.errors();
    let with_errors = |e: String| {
        if errors.is_empty() {
            e
        } else {
            format!("{e} (Lua errors: {})", errors.join("; "))
        }
    };
    let wg = lua
        .global_table("worldgen")
        .ok_or_else(|| with_errors("no worldgen table".into()))?;
    let reader = Reader {
        sandbox: setup.sandbox,
    };
    let key = |base: &str| {
        let o = format!("{base}_override");
        if wg.get_str(&o).is_nil() {
            base.to_string()
        } else {
            o
        }
    };
    let biomes = reader
        .load_biomes(&wg, &key("biomes"))
        .map_err(with_errors)?;
    let biomes_map = reader.load_biomes(&wg, "biomes_map").map_err(with_errors)?;
    let sel_key = key("selection");
    let mut selection: [defs::Selection; 6] = Default::default();
    for (i, axis) in AXES.iter().enumerate() {
        selection[i] = reader.load_selection(*axis, &wg, &sel_key)?;
    }
    let static_modules = reader.load_static_modules(&wg, "static_modules")?;
    let veins = veins::Veins::new(reader.load_veins(&wg, "veins")?);
    let priorities = reader.load_priorities(&wg, "priorities")?;
    let fixed = jar_consts::road_fixed_points(setup.jar)?;
    let (min_x, min_y, max_x, max_y) = setup.cell_bounds;
    let mut road_generators = Vec::new();
    let mut offset = 1000i64;
    for (i, cfg) in reader.load_roads(&wg, "roads")?.into_iter().enumerate() {
        road_generators.push(roads::RoadGenerator::new(
            &params,
            cfg,
            i,
            offset,
            (min_x, min_y),
            (max_x, max_y),
            &fixed,
        ));
        offset += 1000;
    }
    let bmc = lua
        .global_table("biome_map_config")
        .ok_or("no biome_map_config")?;
    let entries = reader.load_biome_map_config(&bmc)?;
    let biome_map = biome_map::BiomeMap::new(setup.fs.clone(), setup.maps, entries);
    let mut zones = zones::WorldGenZones::new(min_x, min_y, max_x, max_y);
    zones.load_maps(&setup.fs, setup.maps)?;
    Ok(WorldGen::new(
        params,
        simplex,
        biomes,
        biomes_map,
        selection,
        static_modules,
        veins,
        priorities,
        road_generators,
        biome_map,
        zones,
    ))
}

/// `Basements.beforeLoadMetaGrid` for a new world (no `map_basements.bin`):
/// the definitions of each lot directory's `basements.lua`, the `Basement`
/// zones `load` found in `objects.lua`, their `.pzby` headers, the
/// placements and their buildings in the meta grid. `rand` is `Rand` at that point of world loading (each
/// header room draws from it).
pub fn load_basements(
    fs: Rc<GameFs>,
    lot_dirs: &[&str],
    meta: &mut IsoMetaGrid,
    gen: &WorldGen,
    rand: &mut dyn pz_world::GameRand,
) -> Result<basements::Basements, String> {
    let mut b = basements::Basements::new(fs);
    b.load_lua(lot_dirs)?;
    for (dir, loc) in &gen.zones.basement_spawns {
        b.register_spawn_location(dir, loc.clone());
    }
    b.load_headers(rand);
    b.calculate_placements(meta, gen.params.seed());
    b.meta_buildings = b.add_buildings_to_meta_grid(meta, rand);
    Ok(b)
}

/// The world-generation side of `IsoChunk.LoadBrandNew`: world generation
/// and procedural basements.
pub struct WorldGenHook {
    pub gen: WorldGen,
    /// `None` leaves new chunks without procedural basements.
    pub basements: Option<basements::Basements>,
    /// `IsoChunk.doWorldgen`.
    pub do_worldgen: bool,
}

impl WorldGenHook {
    pub fn new(gen: WorldGen) -> Self {
        WorldGenHook {
            gen,
            basements: None,
            do_worldgen: true,
        }
    }
}

impl BrandNewHook for WorldGenHook {
    fn generate_chunks(
        &mut self,
        chunks: &mut ChunksCache,
        meta: &IsoMetaGrid,
        env: &mut ChunkLoadEnv<'_>,
    ) {
        if let Err(e) = self.gen.generate_chunks(chunks, meta, env) {
            self.gen.errors.push(format!(
                "generateChunks {},{}: {e}",
                chunks.min_chunk_x(),
                chunks.min_chunk_y()
            ));
        }
    }

    fn on_new_chunk_loaded(
        &mut self,
        chunk: &mut IsoChunk,
        meta: &IsoMetaGrid,
        env: &mut ChunkLoadEnv<'_>,
    ) {
        if let Some(b) = self.basements.as_mut() {
            b.on_new_chunk_loaded(chunk, meta, env);
        }
    }

    fn do_worldgen(&self) -> bool {
        self.do_worldgen
    }
}
